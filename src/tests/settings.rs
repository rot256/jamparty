//! The joiner's settings: read at start, changed from the page, saved to
//! the settings file and sent to every page.

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use crate::{
    adapters::{endpoint::generate_key, settings},
    core::{rng::SystemRng, settings::Switch},
    join::{JoinConfig, Joiner},
    share::Share,
    tests::harness::{
        connected, events, fixture_dir, get, net, next_event, post_json, share_files,
    },
};

const SETTINGS: &str = "/_jampot/api/settings";

async fn joiner(share: &Share, name: Option<&str>, settings: &Path) -> Joiner {
    let joiner = Joiner::start(JoinConfig {
        token: share.token().clone(),
        name: name.map(|name| name.parse().unwrap()),
        default_name: "carol@box".parse().unwrap(),
        settings: settings.to_owned(),
        key: generate_key(&SystemRng),
        net: net(),
        share_addrs: share.direct_addrs(),
    })
    .await
    .unwrap();
    connected(&joiner).await;
    joiner
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_saved_read_back_and_sent_to_pages() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("jampot/settings.json");

    let first = joiner(&share, None, &path).await;
    let status = get(&first, "/_jampot/api/status").await.body;
    assert!(
        status.contains(r#""name":"carol@box","askName":true"#),
        "{status}"
    );
    assert!(
        status.contains(r#""commentMode":"on","showCursor":"off""#),
        "{status}"
    );
    assert!(!path.exists(), "nothing written before a change");

    let mut page = events(&first).await;
    let changed = post_json(&first, SETTINGS, r#"{"name":"dora","showCursor":"on"}"#).await;
    assert_eq!(changed.status, 200, "{}", changed.body);
    assert!(
        changed.body.contains(r#""name":"dora","askName":false"#),
        "{}",
        changed.body
    );
    let event = next_event(&mut page, "settings").await;
    assert!(event.contains(r#""showCursor":"on""#), "{event}");
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(fs::read_to_string(&path).unwrap().contains("dora"));
    // The name goes into the next hello.
    assert_eq!(first.watch().borrow().settings.name.as_str(), "dora");

    // Invalid changes are refused and change nothing.
    for bad in [
        r#"{"keys":{"commentMode":"k","showCursor":"K"}}"#,
        r#"{"keys":{"commentMode":"/","showCursor":"x"}}"#,
        r#"{"name":"a b"}"#,
        r#"{"admin":true}"#,
    ] {
        assert_eq!(post_json(&first, SETTINGS, bad).await.status, 400, "{bad}");
    }
    let keys = post_json(
        &first,
        SETTINGS,
        r#"{"keys":{"commentMode":"k","showCursor":"F2"},"commentMode":"off"}"#,
    )
    .await;
    assert_eq!(keys.status, 200, "{}", keys.body);
    first.stop().await;

    // A new run reads them back.
    let again = joiner(&share, None, &path).await;
    let status = get(&again, "/_jampot/api/status").await.body;
    for needle in [
        r#""name":"dora","askName":false"#,
        r#""keys":{"commentMode":"k","showCursor":"F2"}"#,
        r#""commentMode":"off","showCursor":"on""#,
    ] {
        assert!(status.contains(needle), "{needle} in {status}");
    }
    again.stop().await;

    // --name wins for the run, without asking, and is not saved.
    let named = joiner(&share, Some("erin"), &path).await;
    let status = get(&named, "/_jampot/api/status").await.body;
    assert!(
        status.contains(r#""name":"erin","askName":false"#),
        "{status}"
    );
    assert!(fs::read_to_string(&path).unwrap().contains("dora"));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_joiners_with_one_home_keep_each_others_changes() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("jampot/settings.json");
    let first = joiner(&share, None, &path).await;
    let second = joiner(&share, None, &path).await;

    let named = post_json(&first, SETTINGS, r#"{"name":"dora"}"#).await;
    assert_eq!(named.status, 200, "{}", named.body);
    let cursor = post_json(&second, SETTINGS, r#"{"showCursor":"on"}"#).await;
    assert_eq!(cursor.status, 200, "{}", cursor.body);
    // The second joiner saw the first one's name on disk.
    assert!(cursor.body.contains(r#""name":"dora""#), "{}", cursor.body);

    let saved = settings::load(&path).unwrap();
    assert_eq!(saved.name.map(String::from).as_deref(), Some("dora"));
    assert_eq!(saved.show_cursor, Switch::On);
}
