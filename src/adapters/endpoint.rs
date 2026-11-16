//! iroh endpoints: how shares and joiners bind, find each other and keep
//! their connections alive.

use std::{net::SocketAddr, time::Duration};

use iroh::{
    Endpoint, EndpointAddr, PublicKey, RelayMap, RelayMode, RelayUrl, SecretKey,
    address_lookup::{DnsAddressLookup, PkarrResolver},
    endpoint::{self, Builder, Connection, QuicTransportConfig, VarInt, presets},
};
use rootcause::prelude::report;
use zeroize::Zeroizing;

use crate::{
    Report,
    core::{
        ids::EndpointKey,
        rng::{Rng, random_array},
    },
    protocol::ALPN,
};

/// How endpoints reach each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Discovery {
    /// n0's public relays and address lookup: the share publishes its
    /// addresses and joiners look them up; a joiner, which only dials
    /// out, publishes nothing.
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
    /// A self-hosted relay instead of n0's (`JAMPARTY_RELAY`); public only.
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
    let bytes = Zeroizing::new(random_array(rng));
    SecretKey::from_bytes(&bytes)
}

/// Binds an endpoint with `key`; `accept` says whether it accepts jamparty
/// connections (the share) or only dials out (the joiner).
pub(crate) async fn bind(
    config: &NetConfig,
    key: SecretKey,
    accept: Accept,
) -> Result<Endpoint, Report> {
    let transport = QuicTransportConfig::builder()
        .keep_alive_interval(config.keep_alive)
        .max_idle_timeout(Some(config.idle_timeout.try_into()?))
        // Nobody uses unidirectional streams.
        .max_concurrent_uni_streams(VarInt::from_u32(0));
    let transport = match accept {
        // Before admission a joiner may open only its control stream, and
        // the share buffers little for it; [`admitted`] raises both.
        Accept::Jamparty => transport
            .max_concurrent_bidi_streams(VarInt::from_u32(1))
            .stream_receive_window(VarInt::from_u32(STREAM_WINDOW))
            .receive_window(VarInt::from_u32(UNADMITTED_WINDOW)),
        // The share never opens streams to a joiner.
        Accept::Nothing => transport.max_concurrent_bidi_streams(VarInt::from_u32(0)),
    }
    .build();
    let builder = match config.discovery {
        Discovery::Public => {
            let builder = public(accept);
            match &config.relay {
                None => builder,
                Some(url) => builder.relay_mode(RelayMode::Custom(RelayMap::from(url.clone()))),
            }
        }
        Discovery::Direct => Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .clear_address_lookup()
            .bind_addr("127.0.0.1:0")?,
    };
    let alpns = match accept {
        Accept::Jamparty => vec![ALPN.to_vec()],
        Accept::Nothing => Vec::new(),
    };
    Ok(builder
        .secret_key(key)
        .alpns(alpns)
        .transport_config(transport)
        .bind()
        .await?)
}

/// n0's defaults ([`presets::N0`]) for an endpoint that accepts jamparty
/// connections. One that only dials out resolves addresses as those do,
/// and uses the same relays, but publishes none of its own: nobody needs
/// to find a joiner, and its key and addresses are nobody's business.
fn public(accept: Accept) -> Builder {
    match accept {
        Accept::Jamparty => Endpoint::builder(presets::N0),
        Accept::Nothing => Endpoint::builder(presets::Minimal)
            .address_lookup(PkarrResolver::n0_dns())
            .address_lookup(DnsAddressLookup::n0_dns())
            .relay_mode(endpoint::default_relay_mode()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Accept {
    Jamparty,
    Nothing,
}

/// Connects to the share `key`, at `direct` addresses if any (otherwise
/// found through address lookup). The handshake fails unless the peer
/// holds `key`.
pub(crate) async fn connect(
    endpoint: &Endpoint,
    key: EndpointKey,
    direct: &[SocketAddr],
) -> Result<Connection, Report> {
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
    type Error = Report;

    fn try_from(key: EndpointKey) -> Result<Self, Self::Error> {
        Self::from_bytes(&key.0).map_err(|_| report!("the token's key is not a valid public key"))
    }
}
