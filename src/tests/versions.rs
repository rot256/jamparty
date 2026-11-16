//! A share and a joiner of different jamparty versions: the joiner says
//! which run where, gives up when the protocols differ, and goes on with
//! a note when only the versions do.

use std::sync::atomic::Ordering;

use crate::{
    core::version::JampartyVersion,
    describe,
    protocol::control::{Envelope, RefusedReason, ShareMessage},
    tests::{
        fake_share::fake_share,
        harness::{fixture_dir, raw_connection, share_files, try_joiner, until, within},
    },
};

/// What `jamparty join` fails with against a share that greets it so.
async fn refusal(line: &str) -> String {
    let share = fake_share(vec![line.to_owned()]).await;
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
async fn a_share_of_a_newer_protocol_says_which_jamparty_to_install() {
    let ours = JampartyVersion::current();
    // Protocol 2, with whatever else a newer share sends.
    let said = refusal(&format!(
        r#"{{"version":2,"message":{{"type":"welcome","jamparty":"{AHEAD}","new":[1,2]}}}}"#
    ))
    .await;
    assert_eq!(
        said,
        format!(
            "the share runs jamparty {AHEAD} (protocol 2); this is jamparty {ours} (protocol 1). \
             Install the share's version: cargo install --locked jamparty@{AHEAD}"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_of_another_protocol_without_a_version_is_named_so() {
    let ours = JampartyVersion::current();
    let said = refusal(r#"{"version":2,"message":{"type":"welcome"}}"#).await;
    assert_eq!(
        said,
        format!(
            "the share runs another jamparty (protocol 2); this is jamparty {ours} (protocol 1)."
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_that_refuses_this_protocol_names_its_version() {
    let said = refusal(&format!(
        r#"{{"version":1,"message":{{"type":"refused","reason":"incompatible","jamparty":"{AHEAD}"}}}}"#
    ))
    .await;
    assert!(
        said.starts_with(&format!("the share runs jamparty {AHEAD} (protocol 1)")),
        "{said}"
    );
    assert!(
        said.ends_with(&format!("cargo install --locked jamparty@{AHEAD}")),
        "{said}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn another_version_of_the_same_protocol_goes_on_with_a_note() {
    let key = "ab".repeat(32);
    let share = fake_share(vec![format!(
        r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"alice","fingerprint":"ababababab","key":"{key}"}},"undelivered":[],"presence":"supported","site":{{"kind":"files"}},"jamparty":"{AHEAD}"}}}}"#
    )])
    .await;
    let joiner = within(
        "the joiner",
        try_joiner(share.token.clone(), share.addrs.clone()),
    )
    .await
    .unwrap();
    let ours = JampartyVersion::current();
    assert_eq!(
        joiner.version_note().unwrap(),
        format!(
            "the share runs jamparty {AHEAD}; this is {ours}; consider updating: \
             cargo install --locked jamparty@{AHEAD}"
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
            "{{\"version\":2,\"message\":{{\"type\":\"hello\",\"secret\":\"{secret}\",\"jamparty\":\"{AHEAD}\",\"more\":true}}}}\n"
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
            jamparty: Some(JampartyVersion::current()),
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_that_never_answers_the_hello_is_tried_again() {
    let share = fake_share(Vec::new()).await;
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
