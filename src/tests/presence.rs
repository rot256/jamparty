//! Presence end to end: joiners' pages say where they are, the share
//! tells everyone who is here (and only people on the same page where the
//! others are), names change mid-session.

use std::time::{Duration, Instant};

use http_body_util::BodyExt;
use hyper::body::Incoming;
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
        DRAFT, fixture_dir, get, join, next_event, post_comment, post_json, raw_connection,
        share_files, tab_events, until, within,
    },
};

/// `joiner`'s view of the person called `name`.
fn person(joiner: &Joiner, name: &str) -> Option<Person> {
    joiner
        .watch()
        .borrow()
        .here
        .iter()
        .find(|person| person.name.to_string() == name)
        .cloned()
}

fn url(person: Option<Person>) -> Option<String> {
    person.and_then(|person| person.url).map(String::from)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_page_changes_ends_where_the_page_is() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let bob = join(&share, "bob").await;
    let first = bob.connection().unwrap().stable_id();
    let _page = tab_events(&bob, "tab-b", "/").await;
    // More than the share takes per 10 s: the joiner coalesces.
    for index in 0..150 {
        let seq = index + 1;
        let body = format!(r#"{{"tab":"tab-b","seq":{seq},"url":"/r{index}"}}"#);
        assert_eq!(
            post_json(&bob, "/_jampot/api/page", &body).await.status,
            204
        );
    }
    until("the share at /r149", || {
        share
            .hub()
            .here()
            .iter()
            .any(|person| person.url.clone().map(String::from).as_deref() == Some("/r149"))
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
    let _alice_page = tab_events(&alice, "tab-a", "/r99").await;
    let _bob_page = tab_events(&bob, "tab-b", "/").await;
    let on = post_json(&bob, "/_jampot/api/settings", r#"{"showCursor":"on"}"#).await;
    assert_eq!(on.status, 200);
    until("bob's switch at the share", || {
        share
            .hub()
            .here()
            .iter()
            .any(|person| person.cursor == Switch::On)
    })
    .await;
    let before = share.hub().presence_updates();
    let started = Instant::now();
    // A page that changes and a pointer that moves, every 10 ms.
    for index in 0..100_u64 {
        let seq = index + 1;
        let page = format!(r#"{{"tab":"tab-b","seq":{seq},"url":"/r{index}"}}"#);
        assert_eq!(
            post_json(&bob, "/_jampot/api/page", &page).await.status,
            204
        );
        let cursor = format!(
            r#"{{"tab":"tab-b","seq":{seq},"url":"/r{index}","position":{{"selector":"h1","x":1,"y":1}}}}"#
        );
        assert_eq!(
            post_json(&bob, "/_jampot/api/cursor", &cursor).await.status,
            204
        );
        time::sleep(Duration::from_millis(10)).await;
    }
    until("the share at /r99", || {
        share
            .hub()
            .here()
            .iter()
            .any(|person| person.url.clone().map(String::from).as_deref() == Some("/r99"))
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

    let alice_page = tab_events(&alice, "tab-a", "/a?x=1#top").await;
    let bob_page = tab_events(&bob, "tab-b", "/a?x=1#top").await;
    until("both on /a", || {
        url(person(&bob, "alice")).as_deref() == Some("/a?x=1#top")
            && url(person(&alice, "bob")).as_deref() == Some("/a?x=1#top")
    })
    .await;
    let alice_colour = person(&bob, "alice").unwrap().colour;
    assert_eq!(person(&alice, "alice").unwrap().colour, alice_colour);

    // Bob moves on: nobody learns where he went, and he no longer sees
    // where alice is.
    let moved = post_json(
        &bob,
        "/_jampot/api/page",
        r#"{"tab":"tab-b","seq":1,"url":"/b"}"#,
    )
    .await;
    assert_eq!(moved.status, 204, "{}", moved.body);
    until("bob on /b", || {
        share.hub().here().iter().any(|person| {
            person.name.to_string() == "bob"
                && person.url.clone().map(String::from).as_deref() == Some("/b")
        })
    })
    .await;
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
    let renamed = post_json(
        &alice,
        "/_jampot/api/settings",
        r#"{"name":"alicia","showCursor":"on"}"#,
    )
    .await;
    assert_eq!(renamed.status, 200, "{}", renamed.body);
    until("the rename", || {
        person(&bob, "alicia").is_some_and(|person| person.cursor == Switch::On)
            && person(&bob, "alice").is_none()
    })
    .await;
    assert_eq!(post_comment(&alice, DRAFT).await.status, 202);
    let queued = share.queued();
    assert_eq!(queued.first().unwrap().author.name.to_string(), "alicia");
    let status = get(&bob, "/_jampot/api/status").await.body;
    assert!(status.contains(r#""name":"alicia""#), "{status}");

    // Closing the page's event stream is leaving the page.
    drop(alice_page);
    until("alice nowhere", || {
        share
            .hub()
            .here()
            .iter()
            .any(|person| person.name.to_string() == "alicia" && person.url.is_none())
    })
    .await;
    drop(bob_page);

    // A page must name an open tab, with a valid page.
    let unknown = post_json(
        &bob,
        "/_jampot/api/page",
        r#"{"tab":"tab-z","seq":1,"url":"/"}"#,
    )
    .await;
    assert_eq!(unknown.status, 404);
    for bad in [
        "/_jampot/api/events?url=%2F",
        "/_jampot/api/events?tab=a%20b&url=%2F",
        "/_jampot/api/events?tab=a&url=http%3A%2F%2Fevil%2F",
    ] {
        assert_eq!(get(&bob, bad).await.status, 400, "{bad}");
    }
    let invalid = post_json(
        &bob,
        "/_jampot/api/page",
        r#"{"tab":"tab-b","seq":2,"url":"/\""}"#,
    )
    .await;
    assert_eq!(invalid.status, 400);
}

/// Fails if `body` carries an event of type `kind` within `wait`.
async fn no_event(body: &mut Incoming, kind: &str, wait: Duration) {
    let needle = format!("\"type\":\"{kind}\"");
    let _ = time::timeout(wait, async {
        while let Some(Ok(frame)) = body.frame().await {
            let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
            assert!(!text.contains(&needle), "unexpected {text}");
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cursors_reach_only_tabs_on_the_same_page() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;
    let carol = join(&share, "carol").await;
    let mut alice_page = tab_events(&alice, "tab-a", "/a").await;
    let _bob_page = tab_events(&bob, "tab-b", "/a").await;
    let mut carol_page = tab_events(&carol, "tab-c", "/c").await;
    until("everyone placed", || {
        share.hub().here().iter().all(|person| person.url.is_some())
    })
    .await;

    let cursor =
        r#"{"tab":"tab-b","seq":0,"url":"/a","position":{"selector":"h1","x":2500,"y":7500}}"#;
    // Off by default: the joiner sends nothing, and nobody sees it.
    let refused = post_json(&bob, "/_jampot/api/cursor", cursor).await;
    assert_eq!(refused.status, 409, "{}", refused.body);
    no_event(&mut alice_page, "cursor", Duration::from_millis(300)).await;

    let on = post_json(&bob, "/_jampot/api/settings", r#"{"showCursor":"on"}"#).await;
    assert_eq!(on.status, 200);
    until("bob's switch at the share", || {
        share
            .hub()
            .here()
            .iter()
            .any(|person| person.name.to_string() == "bob" && person.cursor == Switch::On)
    })
    .await;
    assert_eq!(
        post_json(&bob, "/_jampot/api/cursor", cursor).await.status,
        204
    );
    let seen = next_event(&mut alice_page, "cursor").await;
    let bob_key = bob.watch().borrow().you.clone().unwrap().key.to_string();
    assert!(seen.contains(&bob_key), "{seen}");
    assert!(
        seen.contains(r#""url":"/a","position":{"selector":"h1","x":2500,"y":7500}"#),
        "{seen}"
    );

    // Leaving the page: a cursor without a position.
    let left = post_json(
        &bob,
        "/_jampot/api/cursor",
        r#"{"tab":"tab-b","seq":0,"url":"/a"}"#,
    )
    .await;
    assert_eq!(left.status, 204);
    let seen = next_event(&mut alice_page, "cursor").await;
    assert!(seen.contains(r#""position":null"#), "{seen}");
    no_event(&mut carol_page, "cursor", Duration::from_millis(300)).await;

    // Out of range, unknown tab.
    let far = r#"{"tab":"tab-b","seq":0,"url":"/a","position":{"selector":"h1","x":10001,"y":0}}"#;
    assert_eq!(
        post_json(&bob, "/_jampot/api/cursor", far).await.status,
        400
    );
    let unknown = r#"{"tab":"tab-z","seq":0,"url":"/a","position":{"selector":"h1","x":0,"y":0}}"#;
    assert_eq!(
        post_json(&bob, "/_jampot/api/cursor", unknown).await.status,
        404
    );

    // A cursor for another page than the joiner has for the tab (a stale
    // registration) is refused, never routed.
    let stale = r#"{"tab":"tab-b","seq":0,"url":"/c","position":{"selector":"h1","x":0,"y":0}}"#;
    assert_eq!(
        post_json(&bob, "/_jampot/api/cursor", stale).await.status,
        409
    );
    no_event(&mut carol_page, "cursor", Duration::from_millis(300)).await;

    // Reports that come out of order: an older one changes nothing, and a
    // cursor from before the latest report is refused.
    let newer = r#"{"tab":"tab-b","seq":2,"url":"/a?v=2"}"#;
    assert_eq!(
        post_json(&bob, "/_jampot/api/page", newer).await.status,
        204
    );
    let older = r#"{"tab":"tab-b","seq":1,"url":"/a"}"#;
    assert_eq!(
        post_json(&bob, "/_jampot/api/page", older).await.status,
        204
    );
    until("bob at /a?v=2", || {
        share.hub().here().iter().any(|person| {
            person.name.to_string() == "bob"
                && person.url.clone().map(String::from).as_deref() == Some("/a?v=2")
        })
    })
    .await;
    let early = r#"{"tab":"tab-b","seq":1,"url":"/a","position":{"selector":"h1","x":0,"y":0}}"#;
    assert_eq!(
        post_json(&bob, "/_jampot/api/cursor", early).await.status,
        409
    );
    let fresh =
        r#"{"tab":"tab-b","seq":2,"url":"/a?v=2","position":{"selector":"h1","x":0,"y":0}}"#;
    assert_eq!(
        post_json(&bob, "/_jampot/api/cursor", fresh).await.status,
        204
    );

    // A page that cannot say where it is: nowhere, and no cursor.
    let nowhere = post_json(
        &bob,
        "/_jampot/api/page",
        r#"{"tab":"tab-b","seq":3,"url":null}"#,
    )
    .await;
    assert_eq!(nowhere.status, 204, "{}", nowhere.body);
    until("bob nowhere", || {
        share
            .hub()
            .here()
            .iter()
            .any(|person| person.name.to_string() == "bob" && person.url.is_none())
    })
    .await;
    assert_eq!(
        post_json(&bob, "/_jampot/api/cursor", cursor).await.status,
        409
    );
    no_event(&mut alice_page, "cursor", Duration::from_millis(300)).await;
}

/// Whether `name` is at the share with its cursor switch `cursor`.
fn switch_at_share(share: &Share, name: &str, cursor: Switch) -> bool {
    share
        .hub()
        .here()
        .iter()
        .any(|person| person.name.to_string() == name && person.cursor == cursor)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_share_forwards_no_cursor_while_the_switch_is_off() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let mut alice_page = tab_events(&alice, "tab-a", "/a").await;

    // Mallory speaks the protocol by hand, so nothing on its side holds
    // the cursor back: only the share can.
    let (_endpoint, connection) = raw_connection(&share).await;
    let (mut send, recv) = connection.open_bi().await.unwrap();
    let secret = share.token().secret.to_string();
    let mut recv = BufReader::new(recv);
    let line = |message: String| format!("{{\"version\":1,\"message\":{message}}}\n").into_bytes();
    send.write_all(&line(format!(
        r#"{{"type":"hello","secret":"{secret}","name":"mallory"}}"#
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
    no_event(&mut alice_page, "cursor", Duration::from_millis(500)).await;

    // The same messages with the switch on do reach alice.
    send.write_all(&line(presence("on"))).await.unwrap();
    until("mallory's switch on", || {
        switch_at_share(&share, "mallory", Switch::On)
    })
    .await;
    send.write_all(&line(cursor.to_owned())).await.unwrap();
    let seen = next_event(&mut alice_page, "cursor").await;
    assert!(seen.contains(r#""selector":"h1""#), "{seen}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_joiner_sends_one_cursor_message_per_report_and_none_while_off() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;
    let _alice_page = tab_events(&alice, "tab-a", "/a").await;
    let _bob_page = tab_events(&bob, "tab-b", "/a").await;
    until("both placed", || {
        share.hub().here().iter().all(|person| person.url.is_some())
    })
    .await;
    let report = |x: u32| {
        format!(
            r#"{{"tab":"tab-b","seq":0,"url":"/a","position":{{"selector":"h1","x":{x},"y":5000}}}}"#
        )
    };

    // Off: the joiner refuses the page's reports and sends the share nothing.
    for x in [1000, 2000, 3000] {
        assert_eq!(
            post_json(&bob, "/_jampot/api/cursor", &report(x))
                .await
                .status,
            409
        );
    }
    time::sleep(Duration::from_millis(500)).await;
    assert_eq!(share.hub().cursors_received(), 0);

    // On: at most one message per report, and none while the pointer rests.
    let on = post_json(&bob, "/_jampot/api/settings", r#"{"showCursor":"on"}"#).await;
    assert_eq!(on.status, 200);
    until("bob's switch at the share", || {
        switch_at_share(&share, "bob", Switch::On)
    })
    .await;
    for x in [1000, 2000, 3000, 4000, 5000] {
        assert_eq!(
            post_json(&bob, "/_jampot/api/cursor", &report(x))
                .await
                .status,
            204
        );
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
