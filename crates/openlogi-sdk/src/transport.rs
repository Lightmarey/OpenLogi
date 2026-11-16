//! Bounded, cancellation-safe JSON-lines framing on per-user local endpoints.

use futures_util::{SinkExt as _, StreamExt as _};
use std::{io, path::PathBuf};

use interprocess::local_socket::tokio::{Listener as LocalListener, Stream, prelude::*};
use interprocess::local_socket::{ListenerOptions, Name};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio_util::codec::{Framed, LinesCodec, LinesCodecError};

use crate::{MAX_FRAME_BYTES, ProtocolError};

/// Bound registration on both sides of a native application connection.
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Bound one queued send before its connection lease ends.
pub const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Framing or endpoint failure.
#[derive(Debug, Error)]
pub enum TransportError {
    /// The local socket could not be resolved or opened.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A malformed or oversized line was rejected.
    #[error(transparent)]
    Frame(#[from] LinesCodecError),
    /// A message failed typed JSON validation.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// A message exceeded the protocol's bound.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// The peer closed its lease.
    #[error("application connection closed")]
    Closed,
}

/// A profile-scoped endpoint. An explicit path also supports isolated host tests.
#[derive(Clone, Debug)]
pub struct Endpoint(PathBuf);

impl Endpoint {
    /// Resolve the current user's development or production profile.
    pub fn current() -> Result<Self, TransportError> {
        let directory = openlogi_core::paths::config_dir()
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(Self(directory.join("application-sdk.sock")))
    }

    /// Use an explicit endpoint namespace, for an embedded host or isolated test.
    #[must_use]
    pub const fn at(path: PathBuf) -> Self {
        Self(path)
    }

    fn name(&self) -> io::Result<Name<'static>> {
        #[cfg(unix)]
        {
            use interprocess::local_socket::{GenericFilePath, ToFsName};
            self.0.clone().to_fs_name::<GenericFilePath>()
        }
        #[cfg(windows)]
        {
            use interprocess::local_socket::{GenericNamespaced, ToNsName};
            use sha2::{Digest as _, Sha256};
            // The user-specific profile directory gives each account and each
            // development/production profile a distinct named-pipe namespace.
            let hash = Sha256::digest(self.0.to_string_lossy().as_bytes());
            format!("openlogi-sdk-{hash:x}").to_ns_name::<GenericNamespaced>()
        }
    }

    /// Bind the host endpoint. The agent must already own its single-instance lock.
    /// A stale Unix socket is reclaimed, and the socket itself is user-only.
    pub fn bind(&self) -> Result<Listener, TransportError> {
        #[cfg(unix)]
        if let Some(parent) = self.0.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let options = ListenerOptions::new()
            .name(self.name()?)
            .try_overwrite(true);
        #[cfg(unix)]
        let options = {
            use interprocess::os::unix::local_socket::ListenerOptionsExt as _;
            options.mode(0o600)
        };
        Ok(Listener(options.create_tokio()?))
    }

    /// Connect to a host using the same namespace and framing as its listener.
    pub async fn connect(&self) -> Result<JsonLines, TransportError> {
        Ok(frame(Stream::connect(self.name()?).await?))
    }
}

/// A native listener exposing the SDK's validated framing to a host.
pub struct Listener(LocalListener);

impl Listener {
    /// Accept an application connection using bounded, incremental framing.
    pub async fn accept(&self) -> Result<JsonLines, TransportError> {
        Ok(frame(self.0.accept().await?))
    }
}

/// Incrementally buffered framing. Cancelling `next()` preserves partial input.
pub type JsonLines = Framed<Stream, LinesCodec>;

/// Frame a stream accepted by a host listener.
#[must_use]
pub fn frame(stream: Stream) -> JsonLines {
    Framed::new(stream, LinesCodec::new_with_max_length(MAX_FRAME_BYTES))
}

/// Validate the outgoing size before it enters a transport queue.
pub fn encode(message: &impl Serialize) -> Result<String, TransportError> {
    let encoded = serde_json::to_string(message)?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameSize.into());
    }
    Ok(encoded)
}

/// Decode with the same bounded model validation used by every protocol client.
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, TransportError> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameSize.into());
    }
    Ok(serde_json::from_str(line)?)
}

/// Read one validated message. Cancelling the future retains partial framing.
pub async fn receive<T: DeserializeOwned>(stream: &mut JsonLines) -> Result<T, TransportError> {
    let line = stream.next().await.ok_or(TransportError::Closed)??;
    decode(&line)
}

/// Send one size-checked protocol message.
pub async fn send(stream: &mut JsonLines, message: &impl Serialize) -> Result<(), TransportError> {
    stream.send(encode(message)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unix_endpoint_permissions_and_distinct_namespaces_are_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sdk.sock");
        let endpoint = Endpoint::at(path.clone());
        let listener = endpoint.bind().unwrap();
        let client = endpoint.connect().await.unwrap();
        let _server = listener.accept().await.unwrap();
        drop(client);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        Endpoint::at(directory.path().join("other.sock"))
            .connect()
            .await
            .unwrap_err();
    }
}
