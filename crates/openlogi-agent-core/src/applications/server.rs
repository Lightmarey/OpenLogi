//! Socket tasks own their registration lease and all asynchronous delivery.

use openlogi_sdk::transport::{
    self, Endpoint, HANDSHAKE_TIMEOUT, JsonLines, TransportError, WRITE_TIMEOUT,
};
use openlogi_sdk::{ClientMessage, ErrorCode, ServerMessage, require_version};
use std::sync::Arc;
use tokio::task::JoinSet;

use super::ApplicationHost;

/// Lifetime of the independently versioned application server.
pub struct ApplicationServer {
    host: Arc<ApplicationHost>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl ApplicationServer {
    /// Bind the current user's development/production endpoint and serve it.
    pub fn start(host: Arc<ApplicationHost>) -> Result<Self, TransportError> {
        Self::start_at(host, &Endpoint::current()?)
    }

    /// Serve an explicit local endpoint, including real isolated integration tests.
    pub fn start_at(
        host: Arc<ApplicationHost>,
        endpoint: &Endpoint,
    ) -> Result<Self, TransportError> {
        let listener = endpoint.bind()?;
        let owner = Arc::clone(&host);
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    incoming = listener.accept() => match incoming {
                        Ok(stream) => { clients.spawn(serve(Arc::clone(&owner), stream)); }
                        Err(error) => {
                            tracing::warn!(%error, "application SDK listener failed");
                            break;
                        }
                    },
                    Some(result) = clients.join_next() => {
                        if let Err(error) = result { tracing::warn!(%error, "application SDK connection task failed"); }
                    }
                }
            }
            owner.disconnect_all();
        });
        Ok(Self {
            host,
            task: Some(task),
        })
    }

    /// Close all leases and release the native endpoint before a replacement binds.
    pub async fn stop(mut self) {
        self.host.disconnect_all();
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for ApplicationServer {
    fn drop(&mut self) {
        self.host.disconnect_all();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn write(stream: &mut JsonLines, message: &ServerMessage) -> bool {
    matches!(
        tokio::time::timeout(WRITE_TIMEOUT, transport::send(stream, message)).await,
        Ok(Ok(()))
    )
}

async fn serve(host: Arc<ApplicationHost>, mut stream: JsonLines) {
    let Ok(Ok(ClientMessage::Register { version, provider })) =
        tokio::time::timeout(HANDSHAKE_TIMEOUT, transport::receive(&mut stream)).await
    else {
        write(
            &mut stream,
            &ServerMessage::Error {
                code: ErrorCode::InvalidMessage,
                message: "first message must be a valid registration".into(),
            },
        )
        .await;
        return;
    };
    if let Err(error) = require_version(version) {
        write(
            &mut stream,
            &ServerMessage::Error {
                code: ErrorCode::Version,
                message: error.to_string(),
            },
        )
        .await;
        return;
    }
    let registration = match host.register(provider) {
        Ok(registration) => registration,
        Err(error) => {
            write(&mut stream, &error.response()).await;
            return;
        }
    };
    if !write(&mut stream, &ServerMessage::ready(registration.connection)).await {
        return;
    }
    loop {
        let message = tokio::select! {
            request = transport::receive::<ClientMessage>(&mut stream) => match request {
                Ok(request) => host.handle(registration.connection, request).err().map(|error| error.response()),
                Err(TransportError::Closed) => break,
                Err(error) => {
                    write(&mut stream, &ServerMessage::Error { code: ErrorCode::InvalidMessage, message: error.to_string() }).await;
                    break;
                }
            },
            outgoing = registration.queue.next() => match outgoing {
                Some(message) => Some(message),
                None => break,
            },
        };
        if let Some(message) = message
            && !write(&mut stream, &message).await
        {
            break;
        }
    }
}
