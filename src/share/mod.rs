//! The share (`jamparty serve` / `jamparty proxy`): an iroh endpoint that
//! admits joiners presenting the party secret, serves their request
//! streams and takes their comments.

mod admission;
mod connection;
pub(crate) mod delivery;
mod files;
mod http;
pub(crate) mod hub;
mod inject;
mod listing;
mod live;
pub(crate) mod proxy;
mod resources;
mod socket;

use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use iroh::Endpoint;
use rootcause::prelude::{ResultExt, bail};
use tokio::{
    sync::{Semaphore, oneshot},
    task::{JoinHandle, JoinSet},
};

#[cfg(test)]
use crate::protocol::comment::{Author, Comment};
use crate::{
    Report,
    adapters::{
        endpoint::{Accept, NetConfig, bind, direct_addrs, generate_key},
        runtime::{self, SocketFile},
    },
    core::{
        clock::{Clock, SystemClock},
        ids::{EndpointKey, Sid},
        rng::SystemRng,
        text::Untrusted,
        token::{PartySecret, PartyToken},
    },
    share::{
        connection::VersionLog,
        delivery::Target,
        files::Files,
        http::Site,
        hub::Hub,
        live::LiveWatcher,
        proxy::{Proxy, UpstreamUrl},
    },
};

/// Connections waiting for admission at once, by default.
const MAX_UNADMITTED: usize = 32;

/// What to share.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// A directory (canonical path), with live reload.
    Files(PathBuf),
    /// One regular file (canonical parent), with live reload.
    File(PathBuf),
    /// A dev server.
    Proxy(UpstreamUrl),
}

impl Source {
    pub(crate) fn description(&self) -> String {
        match self {
            Self::Files(root) => format!(
                "everything under {} (except dotfiles) (live reload)",
                Untrusted(root.display())
            ),
            Self::File(file) => format!("file {} (live reload)", Untrusted(file.display())),
            Self::Proxy(upstream) => upstream.to_string(),
        }
    }

    pub(crate) fn from_path(input: &Path) -> Result<Self, Report> {
        let path = fs::canonicalize(input)
            .context_with(|| format!("could not share {}", input.display()))?;
        if path.is_dir() {
            return Ok(Self::Files(path));
        }
        if !fs::symlink_metadata(input)
            .context_with(|| format!("could not share {}", input.display()))?
            .is_file()
        {
            bail!("{} is not a regular file or directory", input.display());
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            bail!(
                "{} has a file name that is not valid UTF-8",
                input.display()
            );
        };
        if name.starts_with('.') {
            bail!("cannot share dotfile {}", input.display());
        }
        Ok(Self::File(path))
    }
}

#[derive(Clone)]
pub(crate) struct ShareConfig {
    pub(crate) source: Source,
    pub(crate) net: NetConfig,
    pub(crate) target: Target,
    /// Where the runtime socket goes.
    pub(crate) runtime_dir: PathBuf,
    /// How long a new connection has, from its first packet, to present
    /// its hello.
    pub(crate) admission_timeout: Duration,
    /// Connections waiting for admission at once; more are refused.
    pub(crate) max_unadmitted: usize,
    pub(crate) clock: Arc<dyn Clock>,
}

impl ShareConfig {
    #[must_use]
    pub(crate) fn new(
        source: Source,
        net: NetConfig,
        target: Target,
        runtime_dir: PathBuf,
    ) -> Self {
        Self {
            source,
            net,
            target,
            runtime_dir,
            admission_timeout: Duration::from_secs(10),
            max_unadmitted: MAX_UNADMITTED,
            clock: Arc::new(SystemClock),
        }
    }
}

/// What every connection task needs.
pub(crate) struct Shared {
    token: PartyToken,
    site: Site,
    hub: Arc<Hub>,
    admission_timeout: Duration,
    /// Joiners of another release line, said once.
    versions: VersionLog,
}

/// A running share. Dropping it stops serving; [`Share::stop`] also tells
/// joiners.
pub(crate) struct Share {
    endpoint: Endpoint,
    shared: Arc<Shared>,
    tasks: JoinSet<()>,
    /// Stops [`accept`], which then waits for its connections.
    stop_accepting: oneshot::Sender<()>,
    accepting: JoinHandle<()>,
    _watcher: Option<LiveWatcher>,
    socket: SocketFile,
}

impl Share {
    /// Binds a fresh key, makes a fresh party secret and starts accepting.
    pub(crate) async fn start(config: ShareConfig) -> Result<Self, Report> {
        let ShareConfig {
            source,
            net,
            target,
            runtime_dir,
            admission_timeout,
            max_unadmitted,
            clock,
        } = config;
        let key = generate_key(&SystemRng);
        let token = PartyToken {
            share: EndpointKey::from(key.public()),
            secret: PartySecret::generate(&SystemRng),
        };
        let (site, watcher) = match source {
            Source::Files(root) => static_site(root, None)?,
            Source::File(file) => {
                let Some(parent) = file.parent() else {
                    bail!("shared file has no parent directory");
                };
                static_site(parent.to_owned(), Some(file))?
            }
            Source::Proxy(upstream) => {
                let proxy = Proxy::new(upstream, token.share.sid());
                (Site::Proxy(Box::new(proxy)), None)
            }
        };
        let (listener, socket) = runtime::listen(&runtime_dir, &token.share.sid())?;
        let endpoint = bind(&net, key, Accept::Jamparty).await?;
        let shared = Arc::new(Shared {
            token,
            site,
            hub: Hub::new(clock, Arc::new(SystemRng)),
            admission_timeout,
            versions: VersionLog::default(),
        });
        let mut tasks = JoinSet::new();
        let recv = delivery::start(
            target,
            Arc::clone(&shared.hub),
            shared.token.share.sid().to_string(),
            &mut tasks,
        );
        tasks.spawn(socket::serve(listener, Arc::clone(&shared.hub), recv));
        let (stop_accepting, stopped) = oneshot::channel();
        let accepting = tokio::spawn(accept(
            endpoint.clone(),
            Arc::new(Semaphore::new(max_unadmitted)),
            Arc::clone(&shared),
            stopped,
        ));
        tasks.spawn(connection::forward_reloads(Arc::clone(&shared)));
        tasks.spawn(hub::flush_presence(Arc::clone(&shared.hub)));
        Ok(Self {
            endpoint,
            shared,
            tasks,
            stop_accepting,
            accepting,
            _watcher: watcher,
            socket,
        })
    }

    #[must_use]
    pub(crate) fn token(&self) -> &PartyToken {
        &self.shared.token
    }

    /// The id `jamparty recv --share` and `jamparty send --share` take.
    #[must_use]
    pub(crate) fn id(&self) -> Sid {
        self.shared.token.share.sid()
    }

    /// The runtime socket.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket.0
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn hub(&self) -> &Hub {
        &self.shared.hub
    }

    #[must_use]
    pub(crate) fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Where joiners without address lookup can reach the share.
    #[must_use]
    pub(crate) fn direct_addrs(&self) -> Vec<SocketAddr> {
        direct_addrs(&self.endpoint)
    }

    /// Comments waiting for delivery.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn queued(&self) -> Vec<Comment> {
        self.shared.hub.queued()
    }

    /// Who is connected.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn presence(&self) -> Vec<Author> {
        self.shared.hub.presence()
    }

    /// Closes every connection and the endpoint; removes the socket.
    pub(crate) async fn stop(mut self) {
        // Cancel every task, and wait until each connection's is gone,
        // before closing the endpoint: a connection still running would
        // report the closing as its joiner leaving.
        let _ = self.stop_accepting.send(());
        let _ = self.accepting.await;
        self.tasks.shutdown().await;
        self.endpoint.close().await;
        drop(self.socket);
    }
}

fn static_site(
    root: PathBuf,
    single: Option<PathBuf>,
) -> Result<(Site, Option<LiveWatcher>), Report> {
    let (watcher, reload) = live::start(&root, single.clone())?;
    let files = Files::new(root, single);
    Ok((Site::Files { files, reload }, Some(watcher)))
}

/// Accepts joiners until `stop` fires or is dropped; then cancels the
/// connections and returns once all of them have finished.
async fn accept(
    endpoint: Endpoint,
    unadmitted: Arc<Semaphore>,
    shared: Arc<Shared>,
    mut stop: oneshot::Receiver<()>,
) {
    let mut connections = JoinSet::new();
    loop {
        let incoming = tokio::select! {
            _ = &mut stop => break,
            incoming = endpoint.accept() => match incoming {
                Some(incoming) => incoming,
                None => break,
            },
        };
        let Ok(permit) = Arc::clone(&unadmitted).try_acquire_owned() else {
            tracing::debug!("too many connections waiting for admission; refusing one");
            incoming.refuse();
            continue;
        };
        connections.spawn(connection::handle(incoming, permit, Arc::clone(&shared)));
        // Reap finished connections so the set does not grow.
        while connections.try_join_next().is_some() {}
    }
    connections.shutdown().await;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::share::Source;

    #[test]
    fn describes_each_source() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let file = root.join("report.html");
        fs::write(&file, "").unwrap();
        assert_eq!(
            Source::from_path(&root).unwrap().description(),
            format!(
                "everything under {} (except dotfiles) (live reload)",
                root.display()
            )
        );
        assert_eq!(
            Source::from_path(&file).unwrap().description(),
            format!("file {} (live reload)", file.display())
        );
        assert_eq!(
            Source::Proxy("http://localhost:8000".parse().unwrap()).description(),
            "http://localhost:8000/"
        );
    }
}
