//! The command target: success, failure with backoff on a manual clock,
//! a timeout, and holding comments when the command is gone.

use std::{ffi::OsString, path::Path, sync::Arc, time::Duration};

use jiff::{SignedDuration, Timestamp};
use tempfile::TempDir;
use tokio::net::UnixStream;

use crate::{
    agent,
    core::{clock::ManualClock, queue::DeliveryState},
    share::{
        Source,
        delivery::{CommandTarget, Target},
    },
    tests::harness::{
        DRAFT, TestShare, config, fixture_dir, join, post_comment, start, until, within,
    },
};

fn t0() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

async fn share_to(
    dir: &TempDir,
    argv: Vec<OsString>,
    timeout: Duration,
    clock: Arc<ManualClock>,
) -> TestShare {
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let command = CommandTarget { argv, timeout };
    let mut config = config(Source::Files(root), Target::Command(command));
    config.clock = clock;
    start(config).await
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
    std::fs::read_to_string(path)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_command_backs_off_on_the_clock_then_delivers() {
    let dir = fixture_dir();
    let work = tempfile::tempdir().unwrap();
    let (log, flag) = (work.path().join("log"), work.path().join("fail"));
    std::fs::write(&flag, "").unwrap();
    let clock = Arc::new(ManualClock::new(t0()));
    let argv = logging_command(&log, &flag);
    let share = share_to(&dir, argv, Duration::from_secs(60), Arc::clone(&clock)).await;
    let alice = join(&share, "alice").await;
    assert_eq!(post_comment(&alice, DRAFT).await.status, 202);

    until("the first attempt", || {
        lines(&log) == 1 && share.hub().wake_at() == Some(t0() + SignedDuration::from_secs(1))
    })
    .await;
    let mut view = alice.watch();
    let status = within(
        "the status line",
        view.wait_for(|view| view.status.is_some()),
    )
    .await
    .unwrap()
    .status
    .clone()
    .unwrap();
    assert!(String::from(status).contains("retrying"));

    // The retry waits for the clock, then backs off further.
    clock.advance(SignedDuration::from_secs(1));
    share.hub().wake();
    until("the second attempt", || {
        lines(&log) == 2 && share.hub().wake_at() == Some(t0() + SignedDuration::from_secs(3))
    })
    .await;
    assert_eq!(share.queued().len(), 1);

    std::fs::remove_file(&flag).unwrap();
    clock.advance(SignedDuration::from_secs(2));
    share.hub().wake();
    within(
        "delivery",
        view.wait_for(|view| view.pending == 0 && view.status.is_none()),
    )
    .await
    .unwrap();
    assert_eq!(lines(&log), 3);
    assert!(share.queued().is_empty());
    assert_eq!(share.hub().delivery_state(), DeliveryState::Ready);
    let last = std::fs::read_to_string(&log).unwrap();
    assert!(last.lines().last().unwrap().contains("\"Bigger, please\""));

    // recv is not how this share delivers.
    let stream = UnixStream::connect(share.socket_path()).await.unwrap();
    let error = agent::recv(stream, &mut Vec::new()).await.unwrap_err();
    assert!(error.to_string().contains("command"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_command_holds_comments() {
    let dir = fixture_dir();
    let missing = vec![OsString::from("/nonexistent/jamjam-agent")];
    let clock = Arc::new(ManualClock::new(t0()));
    let share = share_to(&dir, missing, Duration::from_secs(60), clock).await;
    let alice = join(&share, "alice").await;
    assert_eq!(post_comment(&alice, DRAFT).await.status, 202);
    until("the hold", || {
        share.hub().delivery_state() == DeliveryState::Held
    })
    .await;
    let mut view = alice.watch();
    let status = within(
        "the status line",
        view.wait_for(|view| view.status.is_some()),
    )
    .await
    .unwrap()
    .status
    .clone()
    .unwrap();
    assert!(String::from(status).contains("gone"));
    // New comments still queue, nothing is sent.
    assert_eq!(post_comment(&alice, DRAFT).await.status, 202);
    assert_eq!(share.queued().len(), 2);
    assert_eq!(share.hub().delivery_state(), DeliveryState::Held);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_that_takes_too_long_is_killed_and_retried() {
    let dir = fixture_dir();
    let work = tempfile::tempdir().unwrap();
    let pid = work.path().join("pid");
    let argv = vec![
        "sh".into(),
        "-c".into(),
        r#"echo $$ > "$0"; exec sleep 30"#.into(),
        pid.clone().into(),
    ];
    let clock = Arc::new(ManualClock::new(t0()));
    let share = share_to(&dir, argv, Duration::from_millis(300), clock).await;
    let alice = join(&share, "alice").await;
    assert_eq!(post_comment(&alice, DRAFT).await.status, 202);
    let mut view = alice.watch();
    let status = within(
        "the status line",
        view.wait_for(|view| view.status.is_some()),
    )
    .await
    .unwrap()
    .status
    .clone()
    .unwrap();
    let status = String::from(status);
    assert!(
        status.contains("killed") && status.contains("retrying"),
        "{status}"
    );
    assert_eq!(
        share.hub().wake_at(),
        Some(t0() + SignedDuration::from_secs(1))
    );
    assert_eq!(share.queued().len(), 1);
    let pid = std::fs::read_to_string(&pid).unwrap();
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    until("the command to be killed", || !alive()).await;
}
