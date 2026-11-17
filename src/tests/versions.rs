//! A share and a joiner of different release lines: incompatible peers refuse
//! before parsing the rest of the first message.

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
async fn an_older_share_asks_the_sharer_to_update() {
    let said = refusal(r#"{"version":1,"message":{"type":"welcome","jamparty":"0.0.1"}}"#).await;
    assert!(
        said.ends_with("Ask the person sharing to update jamparty."),
        "{said}"
    );
    assert!(!said.contains("cargo install"), "{said}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_of_a_newer_line_says_which_jamparty_to_install() {
    let ours = JampartyVersion::current();
    // Protocol 2, with whatever else a newer share sends.
    let said = refusal(&format!(
        r#"{{"version":2,"message":{{"type":"welcome","jamparty":"{AHEAD}","new":[1,2]}}}}"#
    ))
    .await;
    assert_eq!(
        said,
        format!(
            "the share runs jamparty {AHEAD}; this is jamparty {ours}. \
             Install the share's version: cargo install --locked jamparty@{AHEAD}"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_that_refuses_this_line_names_its_version() {
    let said = refusal(&format!(
        r#"{{"version":1,"message":{{"type":"refused","reason":"incompatible","jamparty":"{AHEAD}"}}}}"#
    ))
    .await;
    assert!(
        said.starts_with(&format!("the share runs jamparty {AHEAD}")),
        "{said}"
    );
    assert!(
        said.ends_with(&format!("cargo install --locked jamparty@{AHEAD}")),
        "{said}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn another_patch_of_the_same_line_is_admitted() {
    let key = "ab".repeat(32);
    let mut patch = semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
    patch.patch += 1;
    let share = fake_share(vec![format!(
        r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"alice","fingerprint":"ababababab","key":"{key}"}},"undelivered":[],"site":{{"kind":"files"}},"jamparty":"{patch}"}}}}"#
    )])
    .await;
    let joiner = within(
        "the joiner",
        try_joiner(share.token.clone(), share.addrs.clone()),
    )
    .await
    .unwrap();
    drop(joiner);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_joiner_of_another_line_is_told_the_shares_version() {
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

#[tokio::test(flavor = "multi_thread")]
async fn a_v0_1_2_shaped_refusal_is_final() {
    let said = refusal(
        r#"{"version":1,"message":{"type":"refused","reason":"incompatible","jamparty":"0.1.2"}}"#,
    )
    .await;
    assert!(said.starts_with("the share runs jamparty 0.1.2;"), "{said}");
    if !JampartyVersion::current().compatible(&"0.1.2".to_owned().try_into().unwrap()) {
        assert!(
            said.ends_with("Ask the person sharing to update jamparty."),
            "{said}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_v0_1_2_shaped_hello_gets_a_readable_reply() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    let secret = share.token().secret.to_string();
    let hello = format!(
        r#"{{"version":1,"message":{{"type":"hello","secret":"{secret}","name":"alice","jamparty":"0.1.2"}}}}"#
    );
    send.write_all(format!("{hello}\n").as_bytes())
        .await
        .unwrap();
    let mut recv = tokio::io::BufReader::new(recv);
    let reply = within(
        "the reply to v0.1.2",
        crate::adapters::lines::read::<Envelope<ShareMessage>, _>(&mut recv),
    )
    .await
    .unwrap()
    .unwrap();
    // This branch intentionally retains 0.1.2 until release. Once it is
    // bumped to 0.2, the same legacy hello must receive a refusal.
    if JampartyVersion::current().compatible(&"0.1.2".to_owned().try_into().unwrap()) {
        assert!(matches!(reply.message, ShareMessage::Admitted { .. }));
    } else {
        assert_eq!(
            reply.message,
            ShareMessage::Refused {
                reason: RefusedReason::Incompatible,
                jamparty: Some(JampartyVersion::current()),
            }
        );
    }
}
