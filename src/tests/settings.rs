//! The joiner's settings: read at start, changed from the page, saved to
//! the settings file and sent to every page.

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use serde_json::{Value, json};

use crate::{
    adapters::{endpoint::generate_key, settings},
    core::{rng::SystemRng, settings::Switch},
    join::{JoinConfig, Joiner},
    share::Share,
    tests::harness::{CONNECT_WAIT, PageSocket, about, connected, fixture_dir, net, share_files},
};

async fn joiner(share: &Share, name: Option<&str>, settings: &Path) -> Joiner {
    let joiner = Joiner::start(JoinConfig {
        token: share.token().clone(),
        name: name.map(|name| name.parse().unwrap()),
        default_name: "carol@box".parse().unwrap(),
        settings: settings.to_owned(),
        key: generate_key(&SystemRng),
        net: net(),
        share_addrs: share.direct_addrs(),
        port: None,
        connect_wait: CONNECT_WAIT,
    })
    .await
    .unwrap();
    connected(&joiner).await;
    joiner
}

/// The settings a page of `joiner` is told when it connects.
async fn settings_of(joiner: &Joiner) -> Value {
    let (_, hello) = PageSocket::hello(joiner).await;
    hello["settings"].clone()
}

fn saved(outcome: &Value) -> bool {
    *outcome == json!("saved")
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_saved_read_back_and_sent_to_pages() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("jampot/settings.json");

    let first = joiner(&share, None, &path).await;
    let settings = settings_of(&first).await;
    assert_eq!(settings["name"], "carol@box", "{settings}");
    assert_eq!(settings["askName"], true, "{settings}");
    assert_eq!(settings["commentMode"], "on", "{settings}");
    assert_eq!(settings["showCursor"], "off", "{settings}");
    assert!(!path.exists(), "nothing written before a change");

    let (mut page, _) = PageSocket::hello(&first).await;
    let (mut other, _) = PageSocket::hello(&first).await;
    let seen = about(
        page.request(json!({"type": "settings", "change": {"name": "dora", "showCursor": "on"}}))
            .await,
    );
    // The settings message comes first, then the reply.
    let [settings, reply] = seen.as_slice() else {
        panic!("{seen:?}");
    };
    assert_eq!(settings["type"], "settings", "{settings}");
    assert_eq!(settings["name"], "dora", "{settings}");
    assert_eq!(settings["askName"], false, "{settings}");
    assert!(saved(&reply["outcome"]), "{reply}");
    // Every page hears of it.
    let told = other.next_of("settings").await;
    assert_eq!(told["showCursor"], "on", "{told}");
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
        let outcome = page.settings(bad).await;
        assert!(outcome["refused"]["error"].is_string(), "{bad}: {outcome}");
    }
    let keys = page
        .settings(r#"{"keys":{"commentMode":"k","showCursor":"F2"},"commentMode":"off"}"#)
        .await;
    assert!(saved(&keys), "{keys}");
    first.stop().await;

    // A new run reads them back.
    let again = joiner(&share, None, &path).await;
    let settings = settings_of(&again).await;
    assert_eq!(settings["name"], "dora", "{settings}");
    assert_eq!(settings["askName"], false, "{settings}");
    assert_eq!(
        settings["keys"],
        json!({"commentMode": "k", "showCursor": "F2"}),
        "{settings}"
    );
    assert_eq!(settings["commentMode"], "off", "{settings}");
    assert_eq!(settings["showCursor"], "on", "{settings}");
    again.stop().await;

    // --name wins for the run, without asking, and is not saved.
    let named = joiner(&share, Some("erin"), &path).await;
    let settings = settings_of(&named).await;
    assert_eq!(settings["name"], "erin", "{settings}");
    assert_eq!(settings["askName"], false, "{settings}");
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

    let (mut first_page, _) = PageSocket::hello(&first).await;
    let (mut second_page, _) = PageSocket::hello(&second).await;
    assert!(saved(&first_page.settings(r#"{"name":"dora"}"#).await));
    let seen = about(
        second_page
            .request(json!({"type": "settings", "change": {"showCursor": "on"}}))
            .await,
    );
    // The second joiner saw the first one's name on disk.
    let settings = seen.first().unwrap();
    assert_eq!(settings["name"], "dora", "{settings}");

    let saved = settings::load(&path).unwrap();
    assert_eq!(saved.name.map(String::from).as_deref(), Some("dora"));
    assert_eq!(saved.show_cursor, Switch::On);
}
