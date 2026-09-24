//! Unix-socket listener: mode 0600, stale-socket cleanup, and a same-uid peer check.

use osprey_domain::{DomainError, DomainResult};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};

/// A Unix listener that drops connections from other users.
pub(crate) struct PeerCheckedListener {
    inner: UnixListener,
    uid: u32,
}

fn unavailable(path: &Path, e: impl std::fmt::Display) -> DomainError {
    DomainError::Unavailable(format!("cannot bind {}: {e}", path.display()))
}

/// Bind the socket at `path`. A stale socket (nobody listening) is removed; a live one means
/// another instance is running and is an error; a non-socket file is never touched.
pub(crate) fn bind(path: &Path) -> DomainResult<PeerCheckedListener> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            return Err(unavailable(path, "path exists and is not a socket"));
        }
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(DomainError::Conflict(format!(
                "another Osprey instance is listening on {}",
                path.display()
            )));
        }
        std::fs::remove_file(path).map_err(|e| unavailable(path, e))?;
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| unavailable(path, e))?;

    // Bind inside a private (0700) staging directory, tighten the mode, then move the socket into
    // place, so there is no window in which another user could connect.
    let listener = match bind_staged(parent, path) {
        Ok(l) => l,
        Err(e) => {
            tracing::debug!(error = %e, "staged socket bind failed; binding in place");
            let l = UnixListener::bind(path).map_err(|e| unavailable(path, e))?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| unavailable(path, e))?;
            l
        }
    };
    let uid = std::fs::metadata(path)
        .map_err(|e| unavailable(path, e))?
        .uid();
    Ok(PeerCheckedListener {
        inner: listener,
        uid,
    })
}

fn bind_staged(parent: &Path, path: &Path) -> io::Result<UnixListener> {
    let staging: PathBuf = parent.join(format!(".osk{:08x}", rand_suffix()));
    std::fs::create_dir(&staging)?;
    let cleanup = |staging: &Path| {
        let _ = std::fs::remove_file(staging.join("s"));
        let _ = std::fs::remove_dir(staging);
    };
    let result = (|| {
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700))?;
        let tmp = staging.join("s");
        let l = UnixListener::bind(&tmp)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&tmp, path)?;
        Ok(l)
    })();
    cleanup(&staging);
    result
}

fn rand_suffix() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    h.write_u32(std::process::id());
    h.finish() as u32
}

impl axum::serve::Listener for PeerCheckedListener {
    type Io = UnixStream;
    type Addr = tokio::net::unix::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.inner.accept().await {
                Ok((stream, addr)) => match stream.peer_cred() {
                    Ok(cred) if cred.uid() == self.uid => return (stream, addr),
                    Ok(cred) => {
                        tracing::warn!(peer_uid = cred.uid(), "refused local API connection from another user");
                    }
                    Err(e) => tracing::warn!(error = %e, "cannot read peer credentials"),
                },
                Err(e) => {
                    // EMFILE and friends: back off instead of spinning.
                    tracing::warn!(error = %e, "unix socket accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}
