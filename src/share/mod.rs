//! The share (`jamjam serve` / `jamjam proxy`): an iroh endpoint that
//! admits joiners presenting the party secret, serves their request
//! streams and takes their comments.

mod admission;
pub(crate) mod body;
mod connection;
mod files;
mod http;
mod hub;
mod inject;
mod live;
pub(crate) mod proxy;

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use iroh::Endpoint;
use tokio::task::JoinSet;

use crate::{
    BoxError,
    adapters::endpoint::{Accept, NetConfig, bind, direct_addrs, generate_key},
    core::{
        clock::SystemClock,
        ids::EndpointKey,
        rng::SystemRng,
        token::{PartySecret, PartyToken},
    },
    share::{
        files::Files,
        http::Site,
        hub::Hub,
        live::LiveWatcher,
        proxy::{Proxy, UpstreamUrl},
    },
};

pub(crate) use crate::share::admission::REFUSED_CODE;

/// What to share.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// A directory (canonical path), with live reload.
    Files(PathBuf),
    /// A dev server.
    Proxy(UpstreamUrl),
}

#[derive(Clone, Debug)]
pub(crate) struct ShareConfig {
    pub(crate) source: Source,
    pub(crate) net: NetConfig,
    /// How long a new connection has to present its hello.
    pub(crate) hello_timeout: Duration,
}

impl ShareConfig {
    #[must_use]
    pub(crate) fn new(source: Source, net: NetConfig) -> Self {
        Self {
            source,
            net,
            hello_timeout: Duration::from_secs(10),
        }
    }
}

/// What every connection task needs.
pub(crate) struct Shared {
    token: PartyToken,
    site: Site,
    hub: Arc<Hub>,
    hello_timeout: Duration,
}

/// A running share. Dropping it stops serving; [`Share::stop`] also tells
/// joiners.
pub(crate) struct Share {
    endpoint: Endpoint,
    shared: Arc<Shared>,
    _tasks: JoinSet<()>,
    _watcher: Option<LiveWatcher>,
}

impl Share {
    /// Binds a fresh key, makes a fresh party secret and starts accepting.
    pub(crate) async fn start(config: ShareConfig) -> Result<Self, BoxError> {
        let ShareConfig {
            source,
            net,
            hello_timeout,
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
        let endpoint = bind(&net, key, Accept::Jamjam).await?;
        let shared = Arc::new(Shared {
            token,
            site,
            hub: Hub::new(Arc::new(SystemClock), Arc::new(SystemRng)),
            hello_timeout,
        });
        let mut tasks = JoinSet::new();
        tasks.spawn(accept(endpoint.clone(), Arc::clone(&shared)));
        tasks.spawn(connection::forward_reloads(Arc::clone(&shared)));
        Ok(Self {
            endpoint,
            shared,
            _tasks: tasks,
            _watcher: watcher,
        })
    }

    #[must_use]
    pub(crate) fn token(&self) -> &PartyToken {
        &self.shared.token
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

    /// Closes every connection and the endpoint.
    pub(crate) async fn stop(self) {
        self.endpoint.close().await;
    }
}

async fn accept(endpoint: Endpoint, shared: Arc<Shared>) {
    let mut connections = JoinSet::new();
    while let Some(incoming) = endpoint.accept().await {
        connections.spawn(connection::handle(incoming, Arc::clone(&shared)));
        // Reap finished connections so the set does not grow.
        while connections.try_join_next().is_some() {}
    }
}
