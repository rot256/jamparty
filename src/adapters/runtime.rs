//! The runtime directory and the per-share Unix sockets in it, which
//! `jamjam recv` and `jamjam send` use to reach a running share.
//!
//! The directory is `$XDG_RUNTIME_DIR/jamjam/` or `~/.cache/jamjam/run/`,
//! mode 0700 and owned by the user; each share's socket is `<id>.sock`
//! (the id is the share's `<sid>`), mode 0600, removed when the share stops.

use std::{
    fs::{self, DirBuilder, Permissions},
    io::ErrorKind,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use tokio::net::{UnixListener, UnixStream};

use crate::{BoxError, core::ids::Sid};

/// The runtime directory, from the environment.
pub(crate) fn default_dir() -> Result<PathBuf, BoxError> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir).join("jamjam"));
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home).join(".cache/jamjam/run"))
}

/// Creates `dir` (mode 0700) if needed and checks it.
fn prepare(dir: &Path) -> Result<(), BoxError> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    check_dir(dir)
}

/// Checks that only the user can use `dir`: owned by them, mode 0700.
fn check_dir(dir: &Path) -> Result<(), BoxError> {
    let meta = fs::metadata(dir)?;
    if meta.uid() != nix::unistd::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err(format!(
            "{} must be owned by you with mode 0700 (chmod 700 it)",
            dir.display()
        )
        .into());
    }
    Ok(())
}

/// Checks that the other end of `stream` runs as the user.
pub(crate) fn check_peer(stream: &UnixStream) -> Result<(), BoxError> {
    let peer = stream.peer_cred()?.uid();
    if peer == nix::unistd::geteuid().as_raw() {
        Ok(())
    } else {
        Err(format!("the runtime socket's other end runs as user {peer}, not you").into())
    }
}

/// Removes sockets of the user's shares that died without cleaning up.
fn sweep(dir: &Path) -> Result<(), BoxError> {
    let me = nix::unistd::geteuid().as_raw();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let ours = meta.file_type().is_socket() && meta.uid() == me;
        let stale = ours
            && path.extension().is_some_and(|ext| ext == "sock")
            && matches!(
                std::os::unix::net::UnixStream::connect(&path),
                Err(error) if error.kind() == ErrorKind::ConnectionRefused
            );
        if stale {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

fn socket_path(dir: &Path, id: &Sid) -> PathBuf {
    dir.join(format!("{id}.sock"))
}

/// A share's socket file; removed on drop.
pub(crate) struct SocketFile(pub(crate) PathBuf);

impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Listens on the socket for share `id` in `dir`.
pub(crate) fn listen(dir: &Path, id: &Sid) -> Result<(UnixListener, SocketFile), BoxError> {
    prepare(dir)?;
    sweep(dir)?;
    let path = socket_path(dir, id);
    let listener = UnixListener::bind(&path)
        .map_err(|error| format!("could not listen on {}: {error}", path.display()))?;
    let file = SocketFile(path);
    fs::set_permissions(&file.0, Permissions::from_mode(0o600))?;
    Ok((listener, file))
}

/// Connects to share `id`, or to the only running share when `id` is
/// `None`. The directory and the share at the other end must be the
/// user's; sockets of shares that died are skipped (the next share to
/// start removes them).
pub(crate) async fn connect(dir: &Path, id: Option<&Sid>) -> Result<UnixStream, BoxError> {
    match check_dir(dir) {
        Ok(()) => {}
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
        {
            return Err(NO_SHARE.into());
        }
        Err(error) => return Err(error),
    }
    if let Some(id) = id {
        let stream = UnixStream::connect(socket_path(dir, id))
            .await
            .map_err(|_| format!("no running share with id {id}"))?;
        check_peer(&stream)?;
        return Ok(stream);
    }
    let mut running = Vec::new();
    let entries =
        fs::read_dir(dir).map_err(|error| format!("could not read {}: {error}", dir.display()))?;
    for entry in entries {
        let path = entry?.path();
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".sock"))
            .and_then(|id| id.parse::<Sid>().ok())
        else {
            continue;
        };
        if let Ok(stream) = UnixStream::connect(&path).await {
            check_peer(&stream)?;
            running.push((id, stream));
        }
    }
    match running.len() {
        0 => Err(NO_SHARE.into()),
        1 => Ok(running.pop().map(|(_, stream)| stream).ok_or(NO_SHARE)?),
        _ => {
            let mut ids: Vec<String> = running.iter().map(|(id, _)| id.to_string()).collect();
            ids.sort();
            Err(format!(
                "several shares are running ({}); pass --share <id>",
                ids.join(", ")
            )
            .into())
        }
    }
}

const NO_SHARE: &str = "no share is running (start one with jamjam serve or jamjam proxy)";

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use crate::{
        adapters::runtime::{connect, listen},
        core::ids::{EndpointKey, Sid},
    };

    #[tokio::test]
    async fn sockets_are_private_and_found() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("run");
        assert!(connect(&dir, None).await.is_err());

        let first = EndpointKey([1; 32]).sid();
        let (_listener, socket) = listen(&dir, &first).unwrap();
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&socket.0), 0o600);
        assert!(connect(&dir, None).await.is_ok());
        assert!(connect(&dir, Some(&first)).await.is_ok());

        let second = EndpointKey([2; 32]).sid();
        let (listener, other) = listen(&dir, &second).unwrap();
        let error = connect(&dir, None).await.unwrap_err().to_string();
        assert!(error.contains("--share"), "{error}");
        assert!(connect(&dir, Some(&second)).await.is_ok());

        // A share that stopped leaves nothing behind.
        let path = other.0.clone();
        drop((listener, other));
        assert!(!path.exists());
        assert!(connect(&dir, None).await.is_ok());
        let gone: Sid = second;
        assert!(connect(&dir, Some(&gone)).await.is_err());

        // One that died without cleaning up is skipped, and left for the
        // next share to remove.
        let stale = dir.join(format!("{}.sock", EndpointKey([4; 32]).sid()));
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        assert!(stale.exists());
        assert!(connect(&dir, None).await.is_ok());
        assert!(stale.exists(), "connect removed a socket");
        let (_listener, _next) = listen(&dir, &EndpointKey([5; 32]).sid()).unwrap();
        assert!(!stale.exists());
        assert!(connect(&dir, Some(&first)).await.is_ok());

        // A directory others can read is refused, by both ends.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(listen(&dir, &EndpointKey([3; 32]).sid()).is_err());
        let error = connect(&dir, Some(&first)).await.unwrap_err().to_string();
        assert!(error.contains("0700"), "{error}");
        assert!(connect(&dir, None).await.is_err());
    }
}
