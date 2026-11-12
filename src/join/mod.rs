//! The joiner (`jampot join`): connects to the share and runs the local
//! HTTP server the browser talks to.

mod api;
mod connection;
mod local;
mod markdown;
mod session;

use std::{
    io::ErrorKind,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
};

#[cfg(test)]
use iroh::endpoint::Connection;
use iroh::{Endpoint, SecretKey};
use nix::libc;
use rootcause::{option_ext::OptionExt, prelude::bail};
use tokio::{net::TcpListener, sync::watch, task::JoinSet};

use crate::{
    Report,
    adapters::endpoint::{Accept, NetConfig, bind},
    core::{name::JoinerName, request::LocalOrigin, token::PartyToken},
    join::{
        connection::Dial,
        session::{Session, View},
    },
};

pub(crate) struct JoinConfig {
    pub(crate) token: PartyToken,
    pub(crate) name: JoinerName,
    /// The joiner's own iroh key.
    pub(crate) key: SecretKey,
    pub(crate) net: NetConfig,
    /// Direct addresses of the share; empty to use address lookup.
    pub(crate) share_addrs: Vec<SocketAddr>,
}

/// Ports tried before giving up on one free on both loopback addresses.
const BIND_ATTEMPTS: usize = 16;

/// Listeners on one port of 127.0.0.1 and [::1] both (`localhost` may
/// resolve to either), so no other local user can take the IPv6 side.
/// Without IPv6 loopback, nobody can, and 127.0.0.1 alone will do.
async fn bind_local() -> Result<Vec<TcpListener>, Report> {
    for _ in 0..BIND_ATTEMPTS {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = v4.local_addr()?.port();
        match TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await {
            Ok(v6) => return Ok(vec![v4, v6]),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {}
            Err(error)
                if error.kind() == ErrorKind::AddrNotAvailable
                    || error.raw_os_error() == Some(libc::EAFNOSUPPORT) =>
            {
                tracing::debug!(%error, "no IPv6 loopback; listening on 127.0.0.1 only");
                return Ok(vec![v4]);
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!("could not find a port free on both 127.0.0.1 and [::1]")
}

/// A running joiner; dropping it stops it.
pub(crate) struct Joiner {
    endpoint: Endpoint,
    session: Arc<Session>,
    #[cfg(test)]
    port: u16,
    _tasks: JoinSet<()>,
}

impl Joiner {
    pub(crate) async fn start(config: JoinConfig) -> Result<Self, Report> {
        let JoinConfig {
            token,
            name,
            key,
            net,
            share_addrs,
        } = config;
        let listeners = bind_local().await?;
        let port = listeners
            .first()
            .context("no local listener")?
            .local_addr()?
            .port();
        let session = Arc::new(Session::new(LocalOrigin::new(&token.share.sid(), port)));
        let endpoint = bind(&net, key, Accept::Nothing).await?;
        let dial = Dial {
            endpoint: endpoint.clone(),
            token,
            name,
            share_addrs,
        };
        let mut tasks = JoinSet::new();
        tasks.spawn(connection::run(dial, Arc::clone(&session)));
        for listener in listeners {
            tasks.spawn(local::serve(listener, Arc::clone(&session)));
        }
        Ok(Self {
            endpoint,
            session,
            #[cfg(test)]
            port,
            _tasks: tasks,
        })
    }

    /// The address to open: `http://<sid>.localhost:<port>/`.
    #[must_use]
    pub(crate) fn url(&self) -> String {
        format!("{}/", self.session.origin.origin())
    }

    /// The local server's address.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn local_addr(&self) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, self.port))
    }

    /// The local origin (`Host` and `Origin` values).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn origin(&self) -> &LocalOrigin {
        &self.session.origin
    }

    /// The session state, as `api/status` reports it.
    #[must_use]
    pub(crate) fn watch(&self) -> watch::Receiver<View> {
        self.session.watch()
    }

    /// The current connection to the share, if admitted.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn connection(&self) -> Option<Connection> {
        self.session
            .control()
            .map(|control| control.connection.clone())
    }

    /// The current control stream, if admitted.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn control(&self) -> Option<Arc<session::Control>> {
        self.session.control()
    }

    pub(crate) async fn stop(self) {
        self.endpoint.close().await;
    }
}
