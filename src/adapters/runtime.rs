//! The runtime directory and the per-share Unix sockets in it, which
//! the agent commands use to reach running shares.
//!
//! The directory is `$XDG_RUNTIME_DIR/jamparty/` or `~/.cache/jamparty/run/`,
//! mode 0700 and owned by the user; each share's socket is `<id>.sock`
//! (the id is the share's `<sid>`), mode 0600, removed when the share stops.

use std::{
    env,
    fs::{self, DirBuilder, Permissions},
    io::ErrorKind,
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net,
    },
    path::{Path, PathBuf},
};

use nix::unistd;
use rootcause::{
    option_ext::OptionExt,
    prelude::{ResultExt, bail},
};
use tokio::net::{UnixListener, UnixStream};

use crate::{Report, core::ids::Sid};

/// The runtime directory, from the environment.
pub(crate) fn default_dir() -> Result<PathBuf, Report> {
    if let Some(dir) = env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir).join("jamparty"));
    }
    let home = env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".cache/jamparty/run"))
}

/// Creates `dir` (mode 0700) if needed and checks it.
fn prepare(dir: &Path) -> Result<(), Report> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    check_dir(dir)
}

/// Checks that only the user can use `dir`: owned by them, mode 0700.
fn check_dir(dir: &Path) -> Result<(), Report> {
    let meta = fs::metadata(dir)?;
    if meta.uid() != unistd::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
        bail!(
            "{} must be owned by you with mode 0700 (chmod 700 it)",
            dir.display()
        );
    }
    Ok(())
}

/// Checks that the other end of `stream` runs as the user.
pub(crate) fn check_peer(stream: &UnixStream) -> Result<(), Report> {
    let peer = stream.peer_cred()?.uid();
    if peer != unistd::geteuid().as_raw() {
        bail!("the runtime socket's other end runs as user {peer}, not you");
    }
    Ok(())
}

/// Removes sockets of the user's shares that died without cleaning up.
fn sweep(dir: &Path) -> Result<(), Report> {
    let me = unistd::geteuid().as_raw();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let ours = meta.file_type().is_socket() && meta.uid() == me;
        let stale = ours
            && path.extension().is_some_and(|ext| ext == "sock")
            && matches!(
                net::UnixStream::connect(&path),
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
pub(crate) fn listen(dir: &Path, id: &Sid) -> Result<(UnixListener, SocketFile), Report> {
    prepare(dir)?;
    sweep(dir)?;
    let path = socket_path(dir, id);
    let listener = UnixListener::bind(&path)
        .context_with(|| format!("could not listen on {}", path.display()))?;
    let file = SocketFile(path);
    fs::set_permissions(&file.0, Permissions::from_mode(0o600))?;
    Ok((listener, file))
}

/// Connects to share `id`, or to the only running share when `id` is
/// `None`. The directory and the share at the other end must be the
/// user's; sockets of shares that died are skipped (the next share to
/// start removes them), and one that is there but cannot be reached is
/// an error.
pub(crate) async fn connect(dir: &Path, id: Option<&Sid>) -> Result<UnixStream, Report> {
    if let Some(id) = id {
        check_runtime(dir)?;
        let stream = UnixStream::connect(socket_path(dir, id))
            .await
            .context_with(|| format!("no running share with id {id}"))?;
        check_peer(&stream)?;
        return Ok(stream);
    }
    let mut running = connected_shares(dir).await?.into_iter();
    match (running.next(), running.next()) {
        (None, _) => bail!("{NO_SHARE}"),
        (Some((_, stream)), None) => Ok(stream),
        (Some(first), Some(second)) => {
            let mut ids: Vec<String> = [first, second]
                .into_iter()
                .chain(running)
                .map(|(id, _)| id.to_string())
                .collect();
            ids.sort();
            bail!(
                "several shares are running ({}); pass --share <id>",
                ids.join(", ")
            )
        }
    }
}

fn check_runtime(dir: &Path) -> Result<(), Report> {
    if fs::metadata(dir).is_err_and(|error| error.kind() == ErrorKind::NotFound) {
        bail!("{NO_SHARE}");
    }
    check_dir(dir)
}

/// Candidate sockets, including any stale or unreachable ones.
pub(crate) fn share_ids(dir: &Path) -> Result<Vec<Sid>, Report> {
    if fs::metadata(dir).is_err_and(|error| error.kind() == ErrorKind::NotFound) {
        return Ok(Vec::new());
    }
    check_dir(dir)?;
    let mut ids = Vec::new();
    let entries = fs::read_dir(dir).context_with(|| format!("could not read {}", dir.display()))?;
    for entry in entries {
        let name = entry?.file_name();
        if let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".sock"))
            .and_then(|id| id.parse::<Sid>().ok())
        {
            ids.push(id);
        }
    }
    Ok(ids)
}

async fn connected_shares(dir: &Path) -> Result<Vec<(Sid, UnixStream)>, Report> {
    let mut running = Vec::new();
    for id in share_ids(dir)? {
        let path = socket_path(dir, &id);
        let stream = match UnixStream::connect(&path).await {
            Ok(stream) => stream,
            // Its share died, or stopped since the directory was read. (On
            // macOS a share whose queue of connections is full refuses too,
            // and looks like one that died; on Linux it is the error below.)
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::ConnectionRefused | ErrorKind::NotFound
                ) =>
            {
                continue;
            }
            Err(error) => Err(error)
                .context_with(|| format!("could not reach the share at {}", path.display()))?,
        };
        check_peer(&stream)?;
        running.push((id, stream));
    }
    Ok(running)
}

const NO_SHARE: &str = "no share is running (start one with jamparty serve or jamparty proxy)";

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::PermissionsExt,
        path::Path,
    };

    use socket2::{Domain, SockAddr, Socket, Type};

    use crate::{
        adapters::runtime::{connect, listen},
        core::ids::{EndpointKey, Sid},
        describe,
    };

    #[tokio::test]
    async fn sockets_are_private_and_found() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("run");
        assert!(connect(&dir, None).await.is_err());

        let first = EndpointKey([1; 32]).sid();
        let (_listener, socket) = listen(&dir, &first).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
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
        // next share to remove. Its socket file is bound but listened on
        // by no one. Closing a listener here would not do: a child that
        // another test forks at that moment holds the listener open until
        // it execs, and connect would find it running. So bind without
        // listening, and hold the socket open as such a child would.
        let stale = dir.join(format!("{}.sock", EndpointKey([4; 32]).sid()));
        let dead = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
        dead.bind(&SockAddr::unix(&stale).unwrap()).unwrap();
        assert!(stale.exists());
        let skipped = connect(&dir, None).await.map(drop);
        assert!(
            skipped.is_ok(),
            "{:?}",
            skipped.map_err(|error| describe(&error))
        );
        assert!(stale.exists(), "connect removed a socket");
        drop(dead);
        let (_listener, _next) = listen(&dir, &EndpointKey([5; 32]).sid()).unwrap();
        assert!(!stale.exists());
        assert!(connect(&dir, Some(&first)).await.is_ok());

        // A directory others can read is refused, by both ends.
        fs::set_permissions(&dir, Permissions::from_mode(0o755)).unwrap();
        assert!(listen(&dir, &EndpointKey([3; 32]).sid()).is_err());
        let error = connect(&dir, Some(&first)).await.unwrap_err().to_string();
        assert!(error.contains("0700"), "{error}");
        assert!(connect(&dir, None).await.is_err());
    }

    /// A share that is there but cannot be reached is reported, not taken
    /// for one that died: skipping it could pick another share instead.
    /// Linux only: there a full queue fails the connect with EAGAIN, while
    /// macOS takes a backlog of 0 as its maximum and refuses when full.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn an_unreachable_share_is_reported() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("run");
        let (_listener, _socket) = listen(&dir, &EndpointKey([1; 32]).sid()).unwrap();

        // A share whose queue of connections is full: one waits, and the
        // queue holds no more.
        let busy = dir.join(format!("{}.sock", EndpointKey([2; 32]).sid()));
        let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
        listener.bind(&SockAddr::unix(&busy).unwrap()).unwrap();
        listener.listen(0).unwrap();
        let _waiting = std::os::unix::net::UnixStream::connect(&busy).unwrap();

        let error = connect(&dir, None).await.map(drop).unwrap_err();
        let error = describe(&error);
        assert!(error.contains(&busy.display().to_string()), "{error}");
    }
}
