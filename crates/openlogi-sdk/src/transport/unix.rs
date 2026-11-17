//! Filesystem local sockets retain their user-only mode and profile namespace.

use std::{io, path::Path};

use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use interprocess::local_socket::{GenericFilePath, ListenerOptions, Name, ToFsName};
use interprocess::os::unix::local_socket::ListenerOptionsExt as _;

fn name(path: &Path) -> io::Result<Name<'static>> {
    path.to_path_buf().to_fs_name::<GenericFilePath>()
}

pub(super) fn bind(path: &Path) -> io::Result<Listener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    ListenerOptions::new()
        .name(name(path)?)
        .try_overwrite(true)
        .mode(0o600)
        .create_tokio()
}

pub(super) async fn connect(path: &Path) -> io::Result<Stream> {
    Stream::connect(name(path)?).await
}
