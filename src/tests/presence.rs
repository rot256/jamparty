//! Presence end to end: joiners' pages say where they are, the share
//! tells everyone who is here (and only people on the same page where the
//! others are), names change mid-session, cursors follow.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::{io::BufReader, time};

use crate::{
    adapters::lines,
    core::{ratelimit::RateLimit, settings::Switch},
    join::{Joiner, REPORT_GAP},
    protocol::{
        control::{Envelope, ShareMessage},
        presence::Person,
    },
    share::Share,
    tests::harness::{
        DRAFT, PageSocket, commented, fixture_dir, join, post_comment, raw_connection, share_files,
        until, within,
    },
};

/// `joiner`'s view of the person called `name`.
fn person(joiner: &Joiner, name: &str) -> Option<Person> {
    joiner
        .watch()
        .borrow()
        .people
        .iter()
        .find(|person| person.name.to_string() == name)
        .cloned()
}

fn url(person: Option<Person>) -> Option<String> {
    person.and_then(|person| person.url).map(String::from)
}

/// Whether the share has `name` on `page`.
fn at_share(share: &Share, name: &str, page: Option<&str>) -> bool {
    share.hub().here().iter().any(|person| {
        person.name.to_string() == name && person.url.clone().map(String::from).as_deref() == page
    })
}

/// Whether `name` is at the share with its cursor switch `cursor`.
fn switch_at_share(share: &Share, name: &str, cursor: Switch) -> bool {
    share
        .hub()
        .here()
        .iter()
        .any(|person| person.name.to_string() == name && person.cursor == cursor)
}

fn position(x: u32) -> Option<Value> {
    Some(json!({"selector": "h1", "x": x, "y": 5000}))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_page_changes_ends_where_the_page_is() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let bob = join(&share, "bob").await;
    let first = bob.connection().unwrap().stable_id();
    let mut page = PageSocket::on(&bob, "/").await;
    // More than the share takes per 10 s: the joiner coalesces.
    for index in 0..150 {
        page.visit(Some(&format!("/r{index}"))).await;
    }
    until("the share at /r149", || {
        at_share(&share, "bob", Some("/r149"))
    })
    .await;
    // Within the share's budget: never cut off for it.
    assert_eq!(bob.connection().unwrap().stable_id(), first);
}

#[test]
fn the_joiner_reports_well_below_the_share_flood_limit() {
    // The share disconnects past one update per `interval` sustained; the
    // joiner sends at most one per `REPORT_GAP`: at least twice as long.
    let flood = RateLimit::PRESENCE_FLOOD.interval().unsigned_abs();
    assert!(REPORT_GAP >= flood * 2, "{REPORT_GAP:?} vs {flood:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cursors_on_moving_pages_stay_within_the_presence_budget() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;
    let first = bob.connection().unwrap().stable_id();
    let _alice_page = PageSocket::on(&alice, "/r99").await;
    let mut bob_page = PageSocket::on(&bob, "/").await;
    assert_eq!(bob_page.settings(r#"{"showCursor":"on"}"#).await, "saved");
    until("bob's switch at the share", || {
        switch_at_share(&share, "bob", Switch::On)
    })
    .await;
    let before = share.hub().presence_updates();
    let started = Instant::now();
    // A page that changes and a pointer that moves, every 10 ms.
    for index in 0..100_u32 {
        bob_page.visit(Some(&format!("/r{index}"))).await;
        bob_page.cursor(position(index)).await;
        time::sleep(Duration::from_millis(10)).await;
    }
    until("the share at /r99", || {
        at_share(&share, "bob", Some("/r99"))
    })
    .await;
    let elapsed = started.elapsed();
    let sent = share.hub().presence_updates() - before;
    // One report per gap at most, plus the one that may start the burst.
    let budget = elapsed.as_millis() / REPORT_GAP.as_millis() + 2;
    assert!(
        u128::from(sent) <= budget,
        "{sent} presence updates in {elapsed:?}"
    );
    assert_eq!(bob.connection().unwrap().stable_id(), first);
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_names_and_switches_reach_everyone() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;

    let mut alice_page = PageSocket::on(&alice, "/a?x=1#top").await;
    let mut bob_page = PageSocket::on(&bob, "/a?x=1#top").await;
    until("both on /a", || {
        url(person(&bob, "alice")).as_deref() == Some("/a?x=1#top")
            && url(person(&alice, "bob")).as_deref() == Some("/a?x=1#top")
    })
    .await;
    let alice_colour = person(&bob, "alice").unwrap().colour;
    assert_eq!(person(&alice, "alice").unwrap().colour, alice_colour);

    // Bob moves on: nobody learns where he went, and he no longer sees
    // where alice is.
    bob_page.visit(Some("/b")).await;
    until("bob on /b", || at_share(&share, "bob", Some("/b"))).await;
    until("bob's page hidden from alice", || {
        person(&alice, "bob").is_some() && url(person(&alice, "bob")).is_none()
    })
    .await;
    until("alice's page hidden from bob", || {
        url(person(&bob, "alice")).is_none()
    })
    .await;
    assert_eq!(url(person(&bob, "bob")).as_deref(), Some("/b"));

    // A rename and the cursor switch reach the others; comments are
    // signed with the new name.
    let renamed = alice_page
        .settings(r#"{"name":"alicia","showCursor":"on"}"#)
        .await;
    assert_eq!(renamed, "saved");
    until("the rename", || {
        person(&bob, "alicia").is_some_and(|person| person.cursor == Switch::On)
            && person(&bob, "alice").is_none()
    })
    .await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    let queued = share.queued();
    assert_eq!(queued.first().unwrap().author.name.to_string(), "alicia");
    let (_, hello) = PageSocket::hello(&bob).await;
    assert!(hello["people"].to_string().contains("alicia"), "{hello}");

    // Closing the page's socket is leaving the page.
    drop(alice_page);
    until("alice nowhere", || at_share(&share, "alicia", None)).await;
    // A page that cannot say where it is is nowhere too.
    bob_page.visit(None).await;
    until("bob nowhere", || at_share(&share, "bob", None)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_joiner_is_where_its_last_active_page_is() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let bob = join(&share, "bob").await;
    let mut first = PageSocket::on(&bob, "/one").await;
    until("bob at /one", || at_share(&share, "bob", Some("/one"))).await;
    // A second tab: the joiner follows it.
    let second = PageSocket::on(&bob, "/two").await;
    until("bob at /two", || at_share(&share, "bob", Some("/two"))).await;
    // The first comes to the front again.
    first.visit(Some("/one")).await;
    until("bob back at /one", || at_share(&share, "bob", Some("/one"))).await;
    // Closing it falls back on the other.
    drop(first);
    until("bob at /two again", || {
        at_share(&share, "bob", Some("/two"))
    })
    .await;
    drop(second);
    until("bob nowhere", || at_share(&share, "bob", None)).await;
    // A page that never says where it is does not count.
    let _silent = PageSocket::hello(&bob).await;
    time::sleep(REPORT_GAP * 2).await;
    assert!(at_share(&share, "bob", None));
}

#[tokio::test(flavor = "multi_thread")]
async fn cursors_reach_only_pages_on_the_same_page() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;
    let carol = join(&share, "carol").await;
    let mut alice_page = PageSocket::on(&alice, "/a").await;
    let mut bob_page = PageSocket::on(&bob, "/a").await;
    let mut carol_page = PageSocket::on(&carol, "/c").await;
    until("everyone placed", || {
        share.hub().here().iter().all(|person| person.url.is_some())
    })
    .await;

    let at = Some(json!({"selector": "h1", "x": 2500, "y": 7500}));
    // Off by default: the joiner sends nothing, and nobody sees it.
    bob_page.cursor(at.clone()).await;
    assert!(!alice_page.hears("cursor", Duration::from_millis(300)).await);
    assert_eq!(share.hub().cursors_received(), 0);

    assert_eq!(bob_page.settings(r#"{"showCursor":"on"}"#).await, "saved");
    until("bob's switch at the share", || {
        switch_at_share(&share, "bob", Switch::On)
    })
    .await;
    bob_page.cursor(at.clone()).await;
    let seen = alice_page.next_of("cursor").await;
    let bob_key = bob
        .watch()
        .borrow()
        .link
        .you
        .clone()
        .unwrap()
        .key
        .to_string();
    assert_eq!(seen["key"], bob_key, "{seen}");
    assert_eq!(seen["url"], "/a", "{seen}");
    assert_eq!(seen["position"], at.clone().unwrap(), "{seen}");

    // Leaving the page: a cursor without a position.
    bob_page.cursor(None).await;
    let seen = alice_page.next_of("cursor").await;
    assert_eq!(seen["position"], Value::Null, "{seen}");
    assert!(!carol_page.hears("cursor", Duration::from_millis(300)).await);

    // A pointer on a page that cannot say where it is goes nowhere.
    bob_page.visit(None).await;
    until("bob nowhere", || at_share(&share, "bob", None)).await;
    let before = share.hub().cursors_received();
    bob_page.cursor(at).await;
    assert!(!alice_page.hears("cursor", Duration::from_millis(300)).await);
    assert_eq!(share.hub().cursors_received(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_share_forwards_no_cursor_while_the_switch_is_off() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let mut alice_page = PageSocket::on(&alice, "/a").await;

    // Mallory speaks the protocol by hand, so nothing on its side holds
    // the cursor back: only the share can.
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    let secret = share.token().secret.to_string();
    let mut recv = BufReader::new(recv);
    let line = |message: String| format!("{{\"version\":1,\"message\":{message}}}\n").into_bytes();
    send.write_all(&line(format!(
        r#"{{"type":"hello","secret":"{secret}","name":"mallory","jamparty":"{}"}}"#,
        env!("CARGO_PKG_VERSION")
    )))
    .await
    .unwrap();
    let admitted = within(
        "admission",
        lines::read::<Envelope<ShareMessage>, _>(&mut recv),
    )
    .await
    .unwrap()
    .unwrap()
    .message;
    assert!(
        matches!(admitted, ShareMessage::Admitted { .. }),
        "{admitted:?}"
    );
    let presence = |cursor: &str| {
        format!(r#"{{"type":"presence","name":"mallory","url":"/a","cursor":"{cursor}"}}"#)
    };
    let cursor = r#"{"type":"cursor","position":{"selector":"h1","x":2500,"y":7500}}"#;

    send.write_all(&line(presence("off"))).await.unwrap();
    until("mallory on /a, switch off", || {
        switch_at_share(&share, "mallory", Switch::Off)
    })
    .await;
    for _ in 0..3 {
        send.write_all(&line(cursor.to_owned())).await.unwrap();
    }
    assert!(!alice_page.hears("cursor", Duration::from_millis(500)).await);

    // The same messages with the switch on do reach alice.
    send.write_all(&line(presence("on"))).await.unwrap();
    until("mallory's switch on", || {
        switch_at_share(&share, "mallory", Switch::On)
    })
    .await;
    send.write_all(&line(cursor.to_owned())).await.unwrap();
    let seen = alice_page.next_of("cursor").await;
    assert_eq!(seen["position"]["selector"], "h1", "{seen}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_joiner_sends_one_cursor_message_per_report_and_none_while_off() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;
    let _alice_page = PageSocket::on(&alice, "/a").await;
    let mut bob_page = PageSocket::on(&bob, "/a").await;
    until("both placed", || {
        share.hub().here().iter().all(|person| person.url.is_some())
    })
    .await;

    // Off: the joiner sends the share nothing.
    for x in [1000, 2000, 3000] {
        bob_page.cursor(position(x)).await;
    }
    time::sleep(Duration::from_millis(500)).await;
    assert_eq!(share.hub().cursors_received(), 0);

    // On: at most one message per report, and none while the pointer rests.
    assert_eq!(bob_page.settings(r#"{"showCursor":"on"}"#).await, "saved");
    until("bob's switch at the share", || {
        switch_at_share(&share, "bob", Switch::On)
    })
    .await;
    for x in [1000, 2000, 3000, 4000, 5000] {
        bob_page.cursor(position(x)).await;
    }
    until("the reports at the share", || {
        share.hub().cursors_received() >= 1
    })
    .await;
    time::sleep(Duration::from_millis(500)).await;
    let sent = share.hub().cursors_received();
    assert!((1..=5).contains(&sent), "{sent} messages for 5 reports");
    time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        share.hub().cursors_received(),
        sent,
        "messages while the pointer rests"
    );
}
