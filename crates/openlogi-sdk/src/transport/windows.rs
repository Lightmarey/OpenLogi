//! Named pipes share one profile namespace and a bounded native connection wait.

use std::{io, path::Path};

use interprocess::ConnectWaitMode;
use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, ToNsName};
use interprocess::os::windows::named_pipe::{
    local_socket::tokio::Stream as PipeSocket, pipe_mode::Bytes, tokio::DuplexPipeStream,
};
use sha2::{Digest as _, Sha256};

fn pipe_name(path: &Path) -> String {
    // The user-specific directory isolates accounts and development profiles.
    let hash = Sha256::digest(path.to_string_lossy().as_bytes());
    format!("openlogi-sdk-{hash:x}")
}

pub(super) fn bind(path: &Path) -> io::Result<Listener> {
    ListenerOptions::new()
        .name(pipe_name(path).to_ns_name::<GenericNamespaced>()?)
        .try_overwrite(true)
        .create_tokio()
}

pub(super) async fn connect(path: &Path) -> io::Result<Stream> {
    // interprocess 2.4.2's Tokio local-socket adapter ignores its wait options.
    // Its typed named-pipe constructor bounds the underlying blocking wait too.
    let pipe = DuplexPipeStream::<Bytes>::connect_by_path_with_wait_mode(
        format!(r"\\.\pipe\{}", pipe_name(path)),
        ConnectWaitMode::Timeout(super::HANDSHAKE_TIMEOUT),
    )
    .await?;
    Ok(Stream::from(PipeSocket::from(pipe)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Endpoint;
    use tokio::net::windows::named_pipe::ServerOptions;

    #[tokio::test]
    async fn a_busy_native_pipe_has_a_bounded_connection_wait() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("busy.sock");
        let server = ServerOptions::new()
            .max_instances(1)
            .create(format!(r"\\.\pipe\{}", pipe_name(&path)))
            .unwrap();
        let endpoint = Endpoint::at(path);
        let first = endpoint.connect().await.unwrap();
        server.connect().await.unwrap();
        let result = tokio::time::timeout(
            super::super::HANDSHAKE_TIMEOUT + std::time::Duration::from_secs(1),
            endpoint.connect(),
        )
        .await;
        drop(first);
        drop(server);
        result
            .expect("the native connection wait must have a bounded deadline")
            .map(|_| ())
            .unwrap_err();
    }
}
