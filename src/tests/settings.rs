//! The joiner's settings: read from the settings file at start, changed
//! from the page for this join only, and sent to every page. Nothing a
//! page does reaches the file, or another join.

use std::{
    fs::{self, Permissions},
    os::unix::fs::PermissionsExt,
    path::Path,
    time::SystemTime,
};

use serde_json::{Value, json};

use crate::{
    adapters::endpoint::generate_key,
    core::rng::SystemRng,
    join::{JoinConfig, Joiner, ProxyLimits},
    share::Share,
    tests::harness::{
        ADMISSION_WAIT, CONNECT_WAIT, PageSocket, about, connected, fixture_dir, net, share_files,
    },
};

async fn joiner(share: &Share, name: Option<&str>, settings: &Path) -> Joiner {
    let joiner = Joiner::start(JoinConfig {
        token: share.token().clone(),
        name: name.map(|name| name.parse().unwrap()),
        default_name: "carol@box".parse().unwrap(),
        settings: settings.to_owned(),
        overrides: Default::default(),
        key: generate_key(&SystemRng),
        net: net(),
        share_addrs: share.direct_addrs(),
        port: None,
        connect_wait: CONNECT_WAIT,
        admission_wait: ADMISSION_WAIT,
        proxy: ProxyLimits::JOIN,
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

/// A settings file holding `json`, as the person writes it.
fn write_settings(path: &Path, json: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, json).unwrap();
    fs::set_permissions(path, Permissions::from_mode(0o600)).unwrap();
}

/// What the file at `path` holds and when it last changed.
fn file_state(path: &Path) -> (String, SystemTime) {
    let modified = fs::metadata(path).unwrap().modified().unwrap();
    (fs::read_to_string(path).unwrap(), modified)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pages_changes_hold_for_this_join_and_never_reach_the_file() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("jampot/settings.json");

    // No file: the defaults, and the page asks for a name.
    let first = joiner(&share, None, &path).await;
    let settings = settings_of(&first).await;
    assert_eq!(settings["name"], "carol@box", "{settings}");
    assert_eq!(settings["askName"], true, "{settings}");
    assert_eq!(settings["commentMode"], "on", "{settings}");
    assert_eq!(settings["showCursor"], "off", "{settings}");
    assert_eq!(settings["answerTimeout"], 30, "{settings}");

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
    // Every page of this join hears of it, and a page that loads later
    // gets it.
    let told = other.next_of("settings").await;
    assert_eq!(told["showCursor"], "on", "{told}");
    assert_eq!(settings_of(&first).await["name"], "dora");
    // The name goes into the next hello to the share.
    assert_eq!(first.watch().borrow().settings.name.as_str(), "dora");
    // Nothing is written.
    assert!(!path.exists(), "a page's change created the settings file");
    assert!(!path.parent().unwrap().exists());
    first.stop().await;

    // With a file: its settings are the defaults, and a page's changes
    // leave it as it was.
    write_settings(
        &path,
        r#"{"name":"erin","keys":{"commentMode":"k","showCursor":"F2"},"commentMode":"off"}"#,
    );
    let before = file_state(&path);
    let second = joiner(&share, None, &path).await;
    let settings = settings_of(&second).await;
    assert_eq!(settings["name"], "erin", "{settings}");
    assert_eq!(settings["askName"], false, "{settings}");
    assert_eq!(
        settings["keys"],
        json!({"commentMode": "k", "showCursor": "F2", "clearAnswers": "d"}),
        "{settings}"
    );
    assert_eq!(settings["commentMode"], "off", "{settings}");
    let (mut page, _) = PageSocket::hello(&second).await;
    for change in [
        r#"{"name":"mallory","showCursor":"on"}"#,
        r#"{"keys":{"commentMode":"q","showCursor":"w"},"commentMode":"on"}"#,
        "{}",
    ] {
        assert!(saved(&page.settings(change).await), "{change}");
    }
    assert_eq!(settings_of(&second).await["name"], "mallory");
    assert_eq!(file_state(&path), before);
    let names: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["settings.json"]);
    second.stop().await;

    // --name wins over the file, without asking.
    let named = joiner(&share, Some("frank"), &path).await;
    let settings = settings_of(&named).await;
    assert_eq!(settings["name"], "frank", "{settings}");
    assert_eq!(settings["askName"], false, "{settings}");
    assert_eq!(settings["commentMode"], "off", "{settings}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_from_one_shares_page_does_not_reach_another_join() {
    let dir = fixture_dir();
    let hostile = share_files(&dir).await;
    let other = share_files(&dir).await;
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("jampot/settings.json");
    write_settings(&path, r#"{"name":"dora"}"#);

    let first = joiner(&hostile, None, &path).await;
    let (mut page, _) = PageSocket::hello(&first).await;
    assert!(saved(
        &page
            .settings(
                r#"{"name":"mallory","showCursor":"on","commentMode":"off","corner":"top-left"}"#
            )
            .await
    ));
    // A joiner of another share, while that one runs and after it.
    let second = joiner(&other, None, &path).await;
    let settings = settings_of(&second).await;
    assert_eq!(settings["name"], "dora", "{settings}");
    assert_eq!(settings["showCursor"], "off", "{settings}");
    assert_eq!(settings["commentMode"], "on", "{settings}");
    assert_eq!(settings["corner"], "bottom-right", "{settings}");
    first.stop().await;
    let third = joiner(&other, None, &path).await;
    assert_eq!(settings_of(&third).await["name"], "dora");
}

#[tokio::test(flavor = "multi_thread")]
async fn key_bindings_the_overlay_refuses_are_refused() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let home = tempfile::tempdir().unwrap();
    let joiner = joiner(&share, None, &home.path().join("settings.json")).await;
    let (mut page, _) = PageSocket::hello(&joiner).await;
    let before = settings_of(&joiner).await;
    for key in [
        "Shift", "Control", "Alt", "Meta", "CapsLock", "AltGraph", "Enter", "Tab", "Escape",
        "escape", "/", " ", "",
    ] {
        let change = json!({"keys": {"commentMode": key, "showCursor": "x"}}).to_string();
        let outcome = page.settings(&change).await;
        assert!(
            outcome["refused"]["error"].is_string(),
            "{key:?}: {outcome}"
        );
    }
    // As are other invalid changes.
    for bad in [
        r#"{"keys":{"commentMode":"k","showCursor":"K"}}"#,
        r#"{"keys":{"commentMode":"c","showCursor":"x","clearAnswers":"C"}}"#,
        r#"{"answerTimeout":3601}"#,
        r#"{"answerTimeout":-1}"#,
        r#"{"answerTimeout":1.5}"#,
        r#"{"name":"a b"}"#,
        r#"{"admin":true}"#,
    ] {
        let outcome = page.settings(bad).await;
        assert!(outcome["refused"]["error"].is_string(), "{bad}: {outcome}");
    }
    assert_eq!(settings_of(&joiner).await, before);
    let keys = page
        .settings(r#"{"keys":{"commentMode":"k","showCursor":"F2","clearAnswers":"q"},"answerTimeout":10}"#)
        .await;
    assert!(saved(&keys), "{keys}");
    let changed = settings_of(&joiner).await;
    assert_eq!(changed["answerTimeout"], 10);
    assert_eq!(changed["keys"]["clearAnswers"], "q");
}
