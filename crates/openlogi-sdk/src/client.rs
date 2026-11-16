//! Rust application client. It owns registration and handshake version policy.

use thiserror::Error;

use crate::transport::{
    self, Endpoint, HANDSHAKE_TIMEOUT, JsonLines, TransportError, WRITE_TIMEOUT,
};
use crate::{
    ClientMessage, ControlId, ErrorCode, ParameterState, ProtocolError, Provider, ServerMessage,
    SessionId, ToolSet, require_version,
};

/// Client connection or handshake failure.
#[derive(Debug, Error)]
pub enum ClientError {
    /// A local stream or typed frame failed.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// The host speaks a different application protocol.
    #[error(transparent)]
    Version(#[from] ProtocolError),
    /// The host refused registration.
    #[error("application registration rejected ({code:?}): {message}")]
    Rejected { code: ErrorCode, message: String },
    /// The host sent an event before acknowledging registration.
    #[error("unexpected application registration response")]
    Handshake,
    /// A handshake or send exceeded its bounded connection deadline.
    #[error("application connection timed out")]
    Timeout,
}

/// One connection lease. Dropping it withdraws the provider and its dynamic tools.
pub struct Client {
    stream: Option<JsonLines>,
    connection: u64,
}

impl Client {
    /// Register on the current user's active OpenLogi profile.
    pub async fn connect(provider: Provider) -> Result<Self, ClientError> {
        Self::connect_at(&Endpoint::current()?, provider).await
    }

    /// Register on an explicitly isolated endpoint.
    pub async fn connect_at(endpoint: &Endpoint, provider: Provider) -> Result<Self, ClientError> {
        let mut stream = endpoint.connect().await?;
        let response = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            transport::send(&mut stream, &ClientMessage::register(provider)).await?;
            transport::receive::<ServerMessage>(&mut stream).await
        })
        .await
        .map_err(|_| ClientError::Timeout)??;
        match response {
            ServerMessage::Ready {
                version,
                connection,
            } => {
                require_version(version)?;
                Ok(Self {
                    stream: Some(stream),
                    connection,
                })
            }
            ServerMessage::Error { code, message } => Err(ClientError::Rejected { code, message }),
            _ => Err(ClientError::Handshake),
        }
    }

    /// Opaque registration identity, useful when recording reconnection behavior.
    #[must_use]
    pub const fn connection(&self) -> u64 {
        self.connection
    }

    /// Replace the application's current tools, or withdraw them with `None`.
    pub async fn set_context(
        &mut self,
        revision: u64,
        tools: Option<ToolSet>,
    ) -> Result<(), ClientError> {
        self.send(&ClientMessage::Context { revision, tools }).await
    }

    /// Report a real application value for this activation and context.
    pub async fn set_parameter(
        &mut self,
        session: SessionId,
        context_revision: u64,
        tool: ControlId,
        value: ParameterState,
    ) -> Result<(), ClientError> {
        self.send(&ClientMessage::Parameter {
            session,
            context_revision,
            tool,
            value,
        })
        .await
    }

    async fn send(&mut self, message: &ClientMessage) -> Result<(), ClientError> {
        let stream = self.stream.as_mut().ok_or(TransportError::Closed)?;
        let result = tokio::time::timeout(WRITE_TIMEOUT, transport::send(stream, message))
            .await
            .map_err(|_| ClientError::Timeout)
            .and_then(|result| result.map_err(ClientError::from));
        if result.is_err() {
            self.stream.take();
        }
        result
    }

    /// Wait for the next ordered event. Partial frames survive cancellation.
    pub async fn next(&mut self) -> Result<ServerMessage, ClientError> {
        let stream = self.stream.as_mut().ok_or(TransportError::Closed)?;
        let result = transport::receive(stream).await.map_err(ClientError::from);
        if result.is_err() {
            self.stream.take();
        }
        result
    }
}
