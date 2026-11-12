//! The share (`jampot serve` / `jampot proxy`): an iroh endpoint that
//! admits joiners presenting the party secret, serves their request
//! streams and takes their comments.

mod admission;
pub(crate) mod body;
mod connection;
pub(crate) mod delivery;
mod files;
mod http;
pub(crate) mod hub;
mod inject;
mod live;
pub(crate) mod proxy;
mod socket;

#[cfg(test)]
use std::path::Path;
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use iroh::Endpoint;
use tokio::{sync::Semaphore, task::JoinSet};

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
        token::{PartySecret, PartyToken},
    },
    share::{
        delivery::Target,
        files::Files,
        http::Site,
        hub::Hub,
        live::LiveWatcher,
        proxy::{Proxy, UpstreamUrl},
    },
};

pub(crate) use crate::share::{admission::REFUSED_CODE, http::HEADER_READ_TIMEOUT};

/// Connections waiting for admission at once, by default.
const MAX_UNADMITTED: usize = 32;

/// What to share.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// A directory (canonical path), with live reload.
    Files(PathBuf),
    /// A dev server.
    Proxy(UpstreamUrl),
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
}

/// A running share. Dropping it stops serving; [`Share::stop`] also tells
/// joiners.
pub(crate) struct Share {
    endpoint: Endpoint,
    shared: Arc<Shared>,
    _tasks: JoinSet<()>,
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
        let (site, watcher) = match source {
            Source::Files(root) => {
                let (watcher, reload) = live::start(&root)?;
                let files = Files::new(root);
                (Site::Files { files, reload }, Some(watcher))
            }
            Source::Proxy(upstream) => (Site::Proxy(Box::new(Proxy::new(upstream))), None),
        };
        let key = generate_key(&SystemRng);
        let token = PartyToken {
            share: EndpointKey::from(key.public()),
            secret: PartySecret::generate(&SystemRng),
        };
        let (listener, socket) = runtime::listen(&runtime_dir, &token.share.sid())?;
        let endpoint = bind(&net, key, Accept::Jampot).await?;
        let shared = Arc::new(Shared {
            token,
            site,
            hub: Hub::new(clock, Arc::new(SystemRng)),
            admission_timeout,
        });
        let mut tasks = JoinSet::new();
        let recv = delivery::start(target, Arc::clone(&shared.hub), &mut tasks);
        tasks.spawn(socket::serve(listener, Arc::clone(&shared.hub), recv));
        tasks.spawn(accept(
            endpoint.clone(),
            Arc::new(Semaphore::new(max_unadmitted)),
            Arc::clone(&shared),
        ));
        tasks.spawn(connection::forward_reloads(Arc::clone(&shared)));
        Ok(Self {
            endpoint,
            shared,
            _tasks: tasks,
            _watcher: watcher,
            socket,
        })
    }

    #[must_use]
    pub(crate) fn token(&self) -> &PartyToken {
        &self.shared.token
    }

    /// The id `jampot recv --share` and `jampot send --share` take.
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
    pub(crate) fn queued(&self) -> Vec<crate::protocol::comment::Comment> {
        self.shared.hub.queued()
    }

    /// Who is connected.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn presence(&self) -> Vec<crate::protocol::comment::Author> {
        self.shared.hub.presence()
    }

    /// Closes every connection and the endpoint; removes the socket.
    pub(crate) async fn stop(self) {
        self.endpoint.close().await;
        drop(self.socket);
    }
}

async fn accept(endpoint: Endpoint, unadmitted: Arc<Semaphore>, shared: Arc<Shared>) {
    let mut connections = JoinSet::new();
    while let Some(incoming) = endpoint.accept().await {
        let Ok(permit) = Arc::clone(&unadmitted).try_acquire_owned() else {
            tracing::debug!("too many connections waiting for admission; refusing one");
            incoming.refuse();
            continue;
        };
        connections.spawn(connection::handle(incoming, permit, Arc::clone(&shared)));
        // Reap finished connections so the set does not grow.
        while connections.try_join_next().is_some() {}
    }
}
