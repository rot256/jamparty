//! A stand-in share that greets every joiner with fixed lines.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use iroh::Endpoint;
use tokio::task::JoinHandle;

use crate::{
    adapters::endpoint::{Accept, bind, direct_addrs, generate_key},
    core::{
        ids::EndpointKey,
        rng::SystemRng,
        token::{PartySecret, PartyToken},
    },
    tests::harness::net,
};

/// A share that answers every joiner's hello with `lines` (none: not at
/// all), and keeps the connection open.
pub(crate) struct FakeShare {
    pub(crate) token: PartyToken,
    pub(crate) addrs: Vec<SocketAddr>,
    /// Hellos read so far.
    pub(crate) hellos: Arc<AtomicUsize>,
    _endpoint: Endpoint,
    _task: JoinHandle<()>,
}

pub(crate) async fn fake_share(lines: Vec<String>) -> FakeShare {
    let key = generate_key(&SystemRng);
    let share = EndpointKey::from(key.public());
    let endpoint = bind(&net(), key, Accept::Jamparty).await.unwrap();
    let addrs = direct_addrs(&endpoint);
    let accepting = endpoint.clone();
    let hellos = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&hellos);
    let task = tokio::spawn(async move {
        let mut kept = Vec::new();
        while let Some(incoming) = accepting.accept().await {
            let Ok(connection) = incoming.await else {
                continue;
            };
            let Ok((mut send, mut recv)) = connection.accept_bi().await else {
                continue;
            };
            // The hello, up to its newline.
            let mut byte = [0];
            while recv.read_exact(&mut byte).await.is_ok() && byte != *b"\n" {}
            counted.fetch_add(1, Ordering::Relaxed);
            for line in &lines {
                let _ = send.write_all(format!("{line}\n").as_bytes()).await;
            }
            kept.push((connection, send, recv));
        }
    });
    FakeShare {
        token: PartyToken {
            share,
            secret: PartySecret::generate(&SystemRng),
        },
        addrs,
        hellos,
        _endpoint: endpoint,
        _task: task,
    }
}
