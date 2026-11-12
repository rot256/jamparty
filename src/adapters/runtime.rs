//! The runtime directory and the per-share Unix sockets in it, which
//! `jamjam recv` and `jamjam send` use to reach a running share.
//!
//! The directory is `$XDG_RUNTIME_DIR/jamjam/` or `~/.cache/jamjam/run/`,
//! mode 0700 and owned by the user; each share's socket is `<id>.sock`
//! (the id is the share's `<sid>`), mode 0600, removed when the share stops.

use std::{
    fs::{self, DirBuilder, Permissions},
    io::ErrorKind,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
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

/// Creates `dir` (mode 0700) if needed and checks that only the user can
/// use it.
fn prepare(dir: &Path) -> Result<(), BoxError> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
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
    let path = socket_path(dir, id);
    let listener = UnixListener::bind(&path)
        .map_err(|error| format!("could not listen on {}: {error}", path.display()))?;
    let file = SocketFile(path);
    fs::set_permissions(&file.0, Permissions::from_mode(0o600))?;
    Ok((listener, file))
}

/// Connects to share `id`, or to the only running share when `id` is
/// `None`. Sockets of shares that died without cleaning up are removed.
pub(crate) async fn connect(dir: &Path, id: Option<&Sid>) -> Result<UnixStream, BoxError> {
    if let Some(id) = id {
        return UnixStream::connect(socket_path(dir, id))
            .await
            .map_err(|_| format!("no running share with id {id}").into());
    }
    let mut running = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(NO_SHARE.into());
        }
        Err(error) => return Err(format!("could not read {}: {error}", dir.display()).into()),
    };
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
        match UnixStream::connect(&path).await {
            Ok(stream) => running.push((id, stream)),
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                let _ = fs::remove_file(&path);
            }
            Err(_) => {}
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

        // One that died without cleaning up is skipped and its socket removed.
        let stale = dir.join(format!("{}.sock", EndpointKey([4; 32]).sid()));
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        assert!(stale.exists());
        assert!(connect(&dir, None).await.is_ok());
        assert!(!stale.exists());

        // A directory others can read is refused.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(listen(&dir, &EndpointKey([3; 32]).sid()).is_err());
    }
}
