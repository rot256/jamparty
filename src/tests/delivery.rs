//! Nudges are independent of recv delivery, including failed reminders.

use std::{ffi::OsString, fs, path::Path, sync::Arc, time::Duration};

use jiff::SignedDuration;
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, net::UnixStream};

use crate::{
    agent,
    core::clock::ManualClock,
    protocol::comment::{Draft, DraftFields},
    share::delivery::{CommandTarget, Target},
    tests::harness::{
        DRAFT, TestShare, commented, fixture_dir, join, post_comment, share_files_to, t0, until,
        within,
    },
};

async fn share_to(dir: &TempDir, argv: Vec<OsString>, clock: Arc<ManualClock>) -> TestShare {
    share_files_to(
        dir,
        Target::Nudge(CommandTarget {
            argv,
            timeout: Duration::from_secs(60),
        }),
        clock,
    )
    .await
}

async fn recv(share: &TestShare) {
    let stream = UnixStream::connect(share.socket_path()).await.unwrap();
    within("recv", agent::recv(stream, &mut Vec::new()))
        .await
        .unwrap();
    until("delivery confirmed", || share.queued().is_empty()).await;
}

/// `sh -c <script> <log> <flag>`: appends stdin to `log`, fails while
/// `flag` exists.
fn logging_command(log: &Path, flag: &Path) -> Vec<OsString> {
    vec![
        "sh".into(),
        "-c".into(),
        r#"cat >> "$0"; test ! -e "$1""#.into(),
        log.into(),
        flag.into(),
    ]
}

fn lines(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_nudges_retry_but_only_recv_delivers() {
    let dir = fixture_dir();
    let work = tempfile::tempdir().unwrap();
    let (log, flag) = (work.path().join("log"), work.path().join("fail"));
    fs::write(&flag, "").unwrap();
    let clock = Arc::new(ManualClock::new(t0()));
    let share = share_to(&dir, logging_command(&log, &flag), Arc::clone(&clock)).await;
    let alice = join(&share, "alice").await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    until("first failure", || {
        lines(&log) == 1 && share.hub().wake_at() == Some(t0() + SignedDuration::from_secs(1))
    })
    .await;
    clock.advance(SignedDuration::from_secs(1));
    share.hub().wake();
    until("second failure", || {
        lines(&log) == 2 && share.hub().wake_at() == Some(t0() + SignedDuration::from_secs(3))
    })
    .await;
    assert_eq!(share.queued().len(), 1);
    fs::remove_file(&flag).unwrap();
    clock.advance(SignedDuration::from_secs(2));
    share.hub().wake();
    until("successful reminder", || lines(&log) == 3).await;
    assert_eq!(share.queued().len(), 1);
    let text = fs::read_to_string(&log).unwrap();
    let notice = format!(
        "jamparty: 1 comment waiting on {}; run jamparty recv --share {}",
        share.id(),
        share.id()
    );
    assert!(text.lines().all(|line| line == notice));
    assert!(!text.contains("Bigger, please"));
    recv(&share).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_nudge_command_does_not_hold_delivery() {
    let dir = fixture_dir();
    let clock = Arc::new(ManualClock::new(t0()));
    let share = share_to(&dir, vec!["/nonexistent/jamparty-nudge".into()], clock).await;
    let alice = join(&share, "alice").await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    until("failure", || share.hub().wake_at().is_some()).await;
    assert_eq!(share.queued().len(), 1);
    recv(&share).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn listening_disconnects_and_between_batches() {
    let dir = fixture_dir();
    let work = tempfile::tempdir().unwrap();
    let log = work.path().join("log");
    let flag = work.path().join("absent");
    let share = share_to(
        &dir,
        logging_command(&log, &flag),
        Arc::new(ManualClock::new(t0())),
    )
    .await;
    let alice = join(&share, "alice").await;
    let mut waiting = UnixStream::connect(share.socket_path()).await.unwrap();
    waiting
        .write_all(b"{\"version\":1,\"message\":{\"type\":\"recv\"}}\n")
        .await
        .unwrap();
    until("listener", || share.hub().listeners() == 1).await;
    drop(waiting);
    until("disconnected listener", || share.hub().listeners() == 0).await;

    let path = share.socket_path().to_owned();
    let waiting = tokio::spawn(async move {
        let stream = UnixStream::connect(path).await.unwrap();
        agent::recv(stream, &mut Vec::new()).await.unwrap();
    });
    until("listener", || share.hub().listeners() == 1).await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    within("waiting recv", waiting).await.unwrap();
    until("delivery confirmed", || share.queued().is_empty()).await;
    assert_eq!(share.hub().listeners(), 0);
    assert_eq!(lines(&log), 0);

    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    until("nudge between batches", || lines(&log) == 1).await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    assert_eq!(share.queued().len(), 2);
    recv(&share).await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    until("new backlog reminder", || lines(&log) == 2).await;
    recv(&share).await;
    assert_eq!(lines(&log), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn recv_does_not_wait_for_a_running_nudge() {
    let dir = fixture_dir();
    let work = tempfile::tempdir().unwrap();
    let ready = work.path().join("ready");
    let argv = vec![
        "sh".into(),
        "-c".into(),
        r#"cat > "$0"; exec tail -f /dev/null"#.into(),
        ready.clone().into(),
    ];
    let share = share_to(&dir, argv, Arc::new(ManualClock::new(t0()))).await;
    let alice = join(&share, "alice").await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    until("nudge running", || {
        fs::read_to_string(&ready).is_ok_and(|text| text.ends_with('\n'))
    })
    .await;
    recv(&share).await;
    // The nudge never exits itself. Recv still completes and confirms the batch.
    share.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_partial_recv_triggers_another_nudge() {
    let dir = fixture_dir();
    let work = tempfile::tempdir().unwrap();
    let log = work.path().join("log");
    let flag = work.path().join("absent");
    let share = share_to(
        &dir,
        logging_command(&log, &flag),
        Arc::new(ManualClock::new(t0())),
    )
    .await;
    let alice = join(&share, "alice").await;
    let mut fields = DraftFields::from(serde_json::from_str::<Draft>(DRAFT).unwrap());
    fields.comment = "\u{0001}".repeat(10_000);
    let draft = serde_json::to_string(&Draft::try_from(fields).unwrap()).unwrap();
    assert!(commented(&post_comment(&alice, &draft).await).is_some());
    until("first nudge", || lines(&log) == 1).await;
    assert!(commented(&post_comment(&alice, &draft).await).is_some());
    assert_eq!(share.queued().len(), 2);

    let stream = UnixStream::connect(share.socket_path()).await.unwrap();
    let mut output = Vec::new();
    within("first batch", agent::recv(stream, &mut output))
        .await
        .unwrap();
    assert_eq!(String::from_utf8(output).unwrap().lines().count(), 1);
    until("first batch confirmed", || share.queued().len() == 1).await;
    until("remaining backlog nudge", || lines(&log) == 2).await;
    recv(&share).await;
    assert_eq!(lines(&log), 2);
}
