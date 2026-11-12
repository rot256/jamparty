//! iroh endpoints: how shares and joiners bind, find each other and keep
//! their connections alive.

use std::{net::SocketAddr, time::Duration};

use iroh::{
    Endpoint, EndpointAddr, PublicKey, RelayMap, RelayMode, RelayUrl, SecretKey,
    endpoint::{Connection, QuicTransportConfig, VarInt, presets},
};

use crate::{
    BoxError,
    core::{
        ids::EndpointKey,
        rng::{Rng, random_array},
    },
    protocol::ALPN,
};

/// How endpoints reach each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Discovery {
    /// iroh's defaults: n0's public relays and address lookup.
    Public,
    /// No relays and no address lookup: bound on `127.0.0.1`, peers are
    /// dialled at direct addresses only.
    Direct,
}

#[derive(Clone, Debug)]
pub(crate) struct NetConfig {
    pub(crate) discovery: Discovery,
    pub(crate) keep_alive: Duration,
    pub(crate) idle_timeout: Duration,
    /// A self-hosted relay instead of n0's (`JAMPOT_RELAY`); public only.
    pub(crate) relay: Option<RelayUrl>,
}

impl NetConfig {
    /// 15 s keep-alive interval, 45 s idle timeout.
    #[must_use]
    pub(crate) fn new(discovery: Discovery) -> Self {
        Self {
            discovery,
            keep_alive: Duration::from_secs(15),
            idle_timeout: Duration::from_secs(45),
            relay: None,
        }
    }
}

/// What a share buffers per stream of a joiner's.
const STREAM_WINDOW: u32 = 1024 * 1024;
/// What a share buffers for a joiner it has not admitted yet, across its
/// streams (only the control stream is allowed then).
const UNADMITTED_WINDOW: u32 = 64 * 1024;
/// What a share buffers for an admitted joiner, across its streams.
const ADMITTED_WINDOW: u32 = 4 * 1024 * 1024;
/// Streams an admitted joiner may have open at once (requests, upgrades
/// and the control stream).
const ADMITTED_STREAMS: u32 = 64;

/// Lets an admitted joiner open request streams, with a larger receive
/// window.
pub(crate) fn admitted(connection: &Connection) {
    connection.set_max_concurrent_bi_streams(VarInt::from_u32(ADMITTED_STREAMS));
    connection.set_receive_window(VarInt::from_u32(ADMITTED_WINDOW));
}

/// A fresh iroh secret key from `rng`.
pub(crate) fn generate_key(rng: &dyn Rng) -> SecretKey {
    SecretKey::from_bytes(&random_array(rng))
}

/// Binds an endpoint with `key`; `accept` says whether it accepts jampot
/// connections (the share) or only dials out (the joiner).
pub(crate) async fn bind(
    config: &NetConfig,
    key: SecretKey,
    accept: Accept,
) -> Result<Endpoint, BoxError> {
    let transport = QuicTransportConfig::builder()
        .keep_alive_interval(config.keep_alive)
        .max_idle_timeout(Some(config.idle_timeout.try_into()?))
        // Nobody uses unidirectional streams.
        .max_concurrent_uni_streams(VarInt::from_u32(0));
    let transport = match accept {
        // Before admission a joiner may open only its control stream, and
        // the share buffers little for it; [`admitted`] raises both.
        Accept::Jampot => transport
            .max_concurrent_bidi_streams(VarInt::from_u32(1))
            .stream_receive_window(VarInt::from_u32(STREAM_WINDOW))
            .receive_window(VarInt::from_u32(UNADMITTED_WINDOW)),
        // The share never opens streams to a joiner.
        Accept::Nothing => transport.max_concurrent_bidi_streams(VarInt::from_u32(0)),
    }
    .build();
    let builder = match config.discovery {
        Discovery::Public => match &config.relay {
            None => Endpoint::builder(presets::N0),
            Some(url) => Endpoint::builder(presets::N0)
                .relay_mode(RelayMode::Custom(RelayMap::from(url.clone()))),
        },
        Discovery::Direct => Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .clear_address_lookup()
            .bind_addr("127.0.0.1:0")?,
    };
    let alpns = match accept {
        Accept::Jampot => vec![ALPN.to_vec()],
        Accept::Nothing => Vec::new(),
    };
    Ok(builder
        .secret_key(key)
        .alpns(alpns)
        .transport_config(transport)
        .bind()
        .await?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Accept {
    Jampot,
    Nothing,
}

/// Connects to the share `key`, at `direct` addresses if any (otherwise
/// found through address lookup). The handshake fails unless the peer
/// holds `key`.
pub(crate) async fn connect(
    endpoint: &Endpoint,
    key: EndpointKey,
    direct: &[SocketAddr],
) -> Result<Connection, BoxError> {
    let id = PublicKey::try_from(key)?;
    let addr = direct
        .iter()
        .fold(EndpointAddr::new(id), |addr, ip| addr.with_ip_addr(*ip));
    Ok(endpoint.connect(addr, ALPN).await?)
}

/// The endpoint's direct addresses, for joiners without address lookup.
#[must_use]
pub(crate) fn direct_addrs(endpoint: &Endpoint) -> Vec<SocketAddr> {
    endpoint.bound_sockets()
}

impl From<PublicKey> for EndpointKey {
    fn from(key: PublicKey) -> Self {
        Self(*key.as_bytes())
    }
}

impl TryFrom<EndpointKey> for PublicKey {
    type Error = BoxError;

    fn try_from(key: EndpointKey) -> Result<Self, Self::Error> {
        Ok(Self::from_bytes(&key.0).map_err(|_| "the token's key is not a valid public key")?)
    }
}
