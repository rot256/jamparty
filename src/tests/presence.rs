//! Presence end to end: joiners' pages say where they are, the share
//! tells everyone who is here (and only people on the same page where the
//! others are), names change mid-session.

use crate::{
    core::settings::Switch,
    join::Joiner,
    protocol::presence::Person,
    tests::harness::{
        DRAFT, fixture_dir, get, join, post_comment, post_json, share_files, tab_events, until,
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
    let moved = post_json(&bob, "/_jampot/api/page", r#"{"tab":"tab-b","url":"/b"}"#).await;
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
    let unknown = post_json(&bob, "/_jampot/api/page", r#"{"tab":"tab-z","url":"/"}"#).await;
    assert_eq!(unknown.status, 404);
    for bad in [
        "/_jampot/api/events?tab=a",
        "/_jampot/api/events?tab=a%20b&url=%2F",
        "/_jampot/api/events?tab=a&url=http%3A%2F%2Fevil%2F",
    ] {
        assert_eq!(get(&bob, bad).await.status, 400, "{bad}");
    }
    let invalid = post_json(&bob, "/_jampot/api/page", r#"{"tab":"tab-b","url":"/\""}"#).await;
    assert_eq!(invalid.status, 400);
}
