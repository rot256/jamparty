//! The joiner (`jampot join`): connects to the share and runs the local
//! HTTP server the browser talks to.

mod api;
mod connection;
mod control;
mod local;
mod markdown;
mod session;
#[cfg(test)]
pub(crate) use crate::join::session::REPORT_GAP;
mod tabs;

use std::{
    error::Error,
    fmt,
    io::{self, ErrorKind},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[cfg(test)]
use iroh::endpoint::Connection;
use iroh::{Endpoint, SecretKey};
use nix::libc;
use rootcause::prelude::{ResultExt, bail};
use tokio::{
    net::TcpListener,
    sync::{oneshot, watch},
    task::JoinSet,
};

use crate::{
    Report,
    adapters::{
        endpoint::{Accept, NetConfig, bind},
        settings,
    },
    core::{name::JoinerName, port::Port, request::LocalOrigin, rng::SystemRng, token::PartyToken},
    join::{
        connection::Dial,
        session::{Epoch, Preferences, Session, View},
    },
    protocol::control::{RefusedReason, SiteInfo},
};

pub(crate) struct JoinConfig {
    pub(crate) token: PartyToken,
    /// `--name`: the name for this run, over the saved one.
    pub(crate) name: Option<JoinerName>,
    /// The name when none is given or saved.
    pub(crate) default_name: JoinerName,
    /// The settings file (`adapters::settings`); read at start.
    pub(crate) settings: PathBuf,
    /// The joiner's own iroh key.
    pub(crate) key: SecretKey,
    pub(crate) net: NetConfig,
    /// Direct addresses of the share; empty to use address lookup.
    pub(crate) share_addrs: Vec<SocketAddr>,
    /// `--port`: the local port, whatever the share serves; none for the
    /// dev server's port when free here, or else a free one.
    pub(crate) port: Option<Port>,
    /// How long a page's request waits for the connection to the share
    /// (while reconnecting) before it is answered "not reachable".
    pub(crate) connect_wait: Duration,
}

/// The share refused this joiner for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Refused(pub(crate) RefusedReason);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the share refused this joiner ({:?}): the token is wrong or from an earlier run",
            self.0
        )
    }
}

impl Error for Refused {}

/// The dev server's port could not be had here, so the joiner serves on
/// another; said once at start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fallback {
    /// The dev server's port.
    pub(crate) wanted: Port,
    /// Where the joiner serves instead.
    pub(crate) serving: Port,
    /// Why `wanted` could not be had.
    pub(crate) why: Unavailable,
}

/// Why a port could not be had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unavailable {
    /// Something else listens on it (on either loopback address).
    InUse,
    /// Binding failed otherwise (port 80 without the privilege, say).
    Other(ErrorKind),
}

impl From<ErrorKind> for Unavailable {
    fn from(kind: ErrorKind) -> Self {
        if kind == ErrorKind::AddrInUse {
            Self::InUse
        } else {
            Self::Other(kind)
        }
    }
}

impl fmt::Display for Fallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            wanted,
            serving,
            why,
        } = self;
        match why {
            Unavailable::InUse => write!(f, "port {wanted} is in use here")?,
            Unavailable::Other(kind) => write!(f, "port {wanted} cannot be used here ({kind})")?,
        }
        write!(
            f,
            "; serving on {serving}. Live reload from a dev server that dials its own port works only if that dev server runs on this machine."
        )
    }
}

/// The local port the joiner asks for.
#[derive(Clone, Copy, Debug)]
enum Wanted {
    /// `--port`: this one or nothing.
    Given(Port),
    /// The dev server's port, or a free one if it cannot be had.
    Mirror(Port),
    /// A free one.
    Any,
}

impl Wanted {
    fn new(given: Option<Port>, site: Option<SiteInfo>) -> Self {
        match (given, site) {
            (Some(port), _) => Self::Given(port),
            (None, Some(SiteInfo::Proxy { port })) => Self::Mirror(port),
            (None, Some(SiteInfo::Files | SiteInfo::Other) | None) => Self::Any,
        }
    }
}

/// Listeners on one port of 127.0.0.1 and [::1] both.
struct Bound {
    listeners: Vec<TcpListener>,
    port: Port,
}

/// Free ports tried before giving up on one free on both loopback
/// addresses.
const BIND_ATTEMPTS: usize = 16;

/// Binds the local server as `wanted`; with [`Wanted::Mirror`], says why
/// it serves elsewhere if it does.
async fn bind_local(wanted: Wanted) -> Result<(Bound, Option<Fallback>), Report> {
    match wanted {
        Wanted::Given(port) => {
            let bound = bind_port(port)
                .await
                .context_with(|| format!("could not serve on port {port}"))?;
            Ok((bound, None))
        }
        Wanted::Mirror(port) => match bind_port(port).await {
            Ok(bound) => Ok((bound, None)),
            Err(error) => {
                tracing::debug!(%error, %port, "the dev server's port cannot be had here");
                let bound = bind_any().await?;
                let fallback = Fallback {
                    wanted: port,
                    serving: bound.port,
                    why: error.kind().into(),
                };
                Ok((bound, Some(fallback)))
            }
        },
        Wanted::Any => Ok((bind_any().await?, None)),
    }
}

/// Both loopback addresses on `port`.
async fn bind_port(port: Port) -> io::Result<Bound> {
    let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, u16::from(port))).await?;
    Ok(Bound {
        listeners: with_ipv6(v4).await?,
        port,
    })
}

/// Both loopback addresses on a free port.
async fn bind_any() -> Result<Bound, Report> {
    for _ in 0..BIND_ATTEMPTS {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = Port::try_from(v4.local_addr()?.port())?;
        match with_ipv6(v4).await {
            Ok(listeners) => return Ok(Bound { listeners, port }),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {}
            Err(error) => return Err(error.into()),
        }
    }
    bail!("could not find a port free on both 127.0.0.1 and [::1]")
}

/// `v4` and a listener on [::1] at its port (`localhost` may resolve to
/// either), so no other local user can take the IPv6 side. Without IPv6
/// loopback, nobody can, and 127.0.0.1 alone will do.
async fn with_ipv6(v4: TcpListener) -> io::Result<Vec<TcpListener>> {
    let port = v4.local_addr()?.port();
    match TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await {
        Ok(v6) => Ok(vec![v4, v6]),
        Err(error)
            if error.kind() == ErrorKind::AddrNotAvailable
                || error.raw_os_error() == Some(libc::EAFNOSUPPORT) =>
        {
            tracing::debug!(%error, "no IPv6 loopback; listening on 127.0.0.1 only");
            Ok(vec![v4])
        }
        Err(error) => Err(error),
    }
}

/// A running joiner; dropping it stops it.
pub(crate) struct Joiner {
    endpoint: Endpoint,
    session: Arc<Session>,
    origin: Arc<LocalOrigin>,
    #[cfg(test)]
    port: Port,
    fallback: Option<Fallback>,
    _tasks: JoinSet<()>,
}

impl Joiner {
    /// Connects to the share and, once admitted, serves the site locally:
    /// on the dev server's port when it can (see [`Fallback`]). The local
    /// origin stays the same across reconnects. Fails if the share refuses
    /// this joiner; waits as long as the share cannot be reached.
    pub(crate) async fn start(config: JoinConfig) -> Result<Self, Report> {
        let JoinConfig {
            token,
            name,
            default_name,
            settings: path,
            key,
            net,
            share_addrs,
            port,
            connect_wait,
        } = config;
        let preferences = Preferences {
            saved: settings::load(&path)?,
            path,
            given: name,
            default: default_name,
        };
        let session = Arc::new(Session::new(preferences, Epoch::generate(&SystemRng)));
        let sid = token.share.sid();
        let endpoint = bind(&net, key, Accept::Nothing).await?;
        let dial = Dial {
            endpoint: endpoint.clone(),
            token,
            share_addrs,
        };
        let mut tasks = JoinSet::new();
        let (first, admitted) = oneshot::channel();
        tasks.spawn(connection::run(dial, Arc::clone(&session), first));
        tasks.spawn(session::report_changes(Arc::clone(&session)));
        let site = match admitted.await {
            Ok(Ok(site)) => site,
            Ok(Err(reason)) => {
                endpoint.close().await;
                return Err(Refused(reason).into());
            }
            Err(_) => bail!("the connection to the share ended before admission"),
        };
        let (bound, fallback) = match bind_local(Wanted::new(port, site)).await {
            Ok(bound) => bound,
            Err(error) => {
                endpoint.close().await;
                return Err(error);
            }
        };
        let Bound { listeners, port } = bound;
        let origin = Arc::new(LocalOrigin::new(&sid, port));
        for listener in listeners {
            tasks.spawn(local::serve(
                listener,
                Arc::clone(&session),
                Arc::clone(&origin),
                connect_wait,
            ));
        }
        Ok(Self {
            endpoint,
            session,
            origin,
            #[cfg(test)]
            port,
            fallback,
            _tasks: tasks,
        })
    }

    /// Why the joiner does not serve on the dev server's port, if it
    /// does not.
    #[must_use]
    pub(crate) fn fallback(&self) -> Option<&Fallback> {
        self.fallback.as_ref()
    }

    /// The address to open: `http://<sid>.localhost:<port>/`.
    #[must_use]
    pub(crate) fn url(&self) -> String {
        format!("{}/", self.origin.origin())
    }

    /// The local server's address.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn local_addr(&self) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, u16::from(self.port)))
    }

    /// The local origin (`Host` and `Origin` values).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn origin(&self) -> &LocalOrigin {
        &self.origin
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
    pub(crate) fn control(&self) -> Option<Arc<control::Control>> {
        self.session.control()
    }

    pub(crate) async fn stop(self) {
        self.endpoint.close().await;
    }
}
