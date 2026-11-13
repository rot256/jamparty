//! A share and a joiner of different jampot versions: the joiner says
//! which run where, gives up when the protocols differ, and goes on with
//! a note when only the versions do.

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
        version::JampotVersion,
    },
    describe,
    protocol::control::{Envelope, RefusedReason, ShareMessage},
    tests::harness::{fixture_dir, net, raw_connection, share_files, try_joiner, until, within},
};

/// A share that answers every joiner's hello with `line` (none: not at
/// all), and keeps the connection open.
struct FakeShare {
    token: PartyToken,
    addrs: Vec<SocketAddr>,
    /// Hellos read so far.
    hellos: Arc<AtomicUsize>,
    _endpoint: Endpoint,
    _task: JoinHandle<()>,
}

async fn fake_share(line: Option<String>) -> FakeShare {
    let key = generate_key(&SystemRng);
    let share = EndpointKey::from(key.public());
    let endpoint = bind(&net(), key, Accept::Jampot).await.unwrap();
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
            if let Some(line) = &line {
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

/// What `jampot join` fails with against a share that greets it so.
async fn refusal(line: &str) -> String {
    let share = fake_share(Some(line.to_owned())).await;
    let error = within(
        "the joiner to give up",
        try_joiner(share.token.clone(), share.addrs.clone()),
    )
    .await
    .err()
    .expect("the joiner gave up");
    describe(&error)
}

const AHEAD: &str = "999.0.0";

#[tokio::test(flavor = "multi_thread")]
async fn a_share_of_a_newer_protocol_says_which_jampot_to_install() {
    let ours = JampotVersion::current();
    // Protocol 2, with whatever else a newer share sends.
    let said = refusal(&format!(
        r#"{{"version":2,"message":{{"type":"welcome","jampot":"{AHEAD}","new":[1,2]}}}}"#
    ))
    .await;
    assert_eq!(
        said,
        format!(
            "the share runs jampot {AHEAD} (protocol 2); this is jampot {ours} (protocol 1). \
             Install the share's version: cargo install --locked jampot@{AHEAD}"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_of_another_protocol_without_a_version_is_named_so() {
    let ours = JampotVersion::current();
    let said = refusal(r#"{"version":2,"message":{"type":"welcome"}}"#).await;
    assert_eq!(
        said,
        format!("the share runs another jampot (protocol 2); this is jampot {ours} (protocol 1).")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_that_refuses_this_protocol_names_its_version() {
    let said = refusal(&format!(
        r#"{{"version":1,"message":{{"type":"refused","reason":"incompatible","jampot":"{AHEAD}"}}}}"#
    ))
    .await;
    assert!(
        said.starts_with(&format!("the share runs jampot {AHEAD} (protocol 1)")),
        "{said}"
    );
    assert!(
        said.ends_with(&format!("cargo install --locked jampot@{AHEAD}")),
        "{said}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn another_version_of_the_same_protocol_goes_on_with_a_note() {
    let key = "ab".repeat(32);
    let share = fake_share(Some(format!(
        r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"alice","fingerprint":"ababababab","key":"{key}"}},"undelivered":[],"presence":"supported","site":{{"kind":"files"}},"jampot":"{AHEAD}"}}}}"#
    )))
    .await;
    let joiner = within(
        "the joiner",
        try_joiner(share.token.clone(), share.addrs.clone()),
    )
    .await
    .unwrap();
    let ours = JampotVersion::current();
    assert_eq!(
        joiner.version_note().unwrap(),
        format!(
            "the share runs jampot {AHEAD}; this is {ours}; consider updating: \
             cargo install --locked jampot@{AHEAD}"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_joiner_of_another_protocol_is_told_the_shares_version() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    let secret = share.token().secret.to_string();
    send.write_all(
        format!(
            "{{\"version\":2,\"message\":{{\"type\":\"hello\",\"secret\":\"{secret}\",\"jampot\":\"{AHEAD}\",\"more\":true}}}}\n"
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut recv = tokio::io::BufReader::new(recv);
    let reply = within(
        "the refusal",
        crate::adapters::lines::read::<Envelope<ShareMessage>, _>(&mut recv),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        reply.message,
        ShareMessage::Refused {
            reason: RefusedReason::Incompatible,
            jampot: Some(JampotVersion::current()),
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_that_never_answers_the_hello_is_tried_again() {
    let share = fake_share(None).await;
    let joining = tokio::spawn(try_joiner(share.token.clone(), share.addrs.clone()));
    // The first attempt gives up on the silent share, and the joiner
    // tries again.
    until("a second hello", || {
        share.hellos.load(Ordering::Relaxed) >= 2
    })
    .await;
    assert!(!joining.is_finished());
    joining.abort();
}
