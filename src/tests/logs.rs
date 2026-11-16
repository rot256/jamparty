//! What the share sends reaches the joiner's log with its control
//! characters escaped: it cannot forge log lines or drive the terminal.

use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, OnceLock, PoisonError},
    thread::{self, ThreadId},
};

use crate::tests::{
    fake_share::fake_share,
    harness::{fixture_dir, joiner, share_files, try_joiner, until, within},
};

fn admitted() -> String {
    format!(
        r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"alice","fingerprint":"ababababab","key":"{}"}},"undelivered":[],"presence":"supported","site":{{"kind":"files"}},"jamparty":"{}"}}}}"#,
        "ab".repeat(32),
        env!("CARGO_PKG_VERSION")
    )
}

/// Log output, kept for the test to read.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<HashMap<ThreadId, Vec<u8>>>>);

impl Captured {
    fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(
            bytes
                .get(&thread::current().id())
                .map(Vec::as_slice)
                .unwrap_or_default(),
        )
        .into_owned()
    }

    /// Everything logged, by any thread: for multi-threaded tests, which
    /// must look for something only they log.
    fn all(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        bytes
            .values()
            .map(|log| String::from_utf8_lossy(log).into_owned())
            .collect()
    }
}

impl io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(thread::current().id())
            .or_default()
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// This is the suite's only logging subscriber. Keep it global so cached
// callsites remain enabled, but separate captures by test runtime thread.
fn capture() -> Captured {
    static CAPTURE: OnceLock<Captured> = OnceLock::new();
    CAPTURE
        .get_or_init(|| {
            let captured = Captured::default();
            let writer = captured.clone();
            let subscriber = tracing_subscriber::fmt()
                .with_env_filter(
                    "off,jamparty::join::connection=debug,jamparty::share::connection=info",
                )
                .with_ansi(false)
                .without_time()
                .with_writer(move || writer.clone())
                .finish();
            tracing::subscriber::set_global_default(subscriber).unwrap();
            captured
        })
        .clone()
}

#[tokio::test(flavor = "current_thread")]
async fn stopping_a_joiner_does_not_report_a_lost_connection() {
    let captured = capture();
    let share = fake_share(vec![admitted()]).await;
    let joiner = within(
        "the joiner",
        try_joiner(share.token.clone(), share.addrs.clone()),
    )
    .await
    .unwrap();
    within("stopping the joiner", joiner.stop()).await;
    let log = captured.text();
    assert!(!log.contains("lost the connection"), "{log}");
    assert!(!log.contains("retrying"), "{log}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_share_does_not_report_joiners_leaving() {
    let captured = capture();
    let directory = fixture_dir();
    let share = share_files(&directory).await;
    let mut joiners = Vec::new();
    for _ in 0..4 {
        joiners.push(
            joiner(
                share.token().clone(),
                share.direct_addrs(),
                "stopping-share",
            )
            .await,
        );
    }
    within("stopping the share", share.stop()).await;
    let log = captured.all();
    assert!(log.contains("joiner admitted name=stopping-share"), "{log}");
    assert!(!log.contains("joiner left name=stopping-share"), "{log}");
    for joiner in joiners {
        joiner.stop().await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn losing_a_share_still_reports_reconnecting() {
    let captured = capture();
    let directory = fixture_dir();
    let share = share_files(&directory).await;
    let joiner = within(
        "the joiner",
        try_joiner(share.token().clone(), share.direct_addrs()),
    )
    .await
    .unwrap();
    within("stopping the share", share.stop()).await;
    until("the lost connection to be logged", || {
        captured
            .text()
            .contains("lost the connection to the share; reconnecting")
    })
    .await;
    joiner.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_control_line_with_escapes_is_logged_escaped() {
    let captured = capture();

    let forged = r#"{"version":1,"message":{"type":"\r\nFORGED LINE\u001b[2J"}}"#;
    let share = fake_share(vec![admitted(), forged.to_owned()]).await;
    let _joiner = within(
        "the joiner",
        try_joiner(share.token.clone(), share.addrs.clone()),
    )
    .await
    .unwrap();
    let is_forged =
        |line: &str| line.contains("control stream ended") && line.contains("FORGED LINE");
    until("the bad line to be logged", || {
        captured.text().lines().any(is_forged)
    })
    .await;
    let log = captured.text();
    let line = log.lines().find(|line| is_forged(line)).unwrap();
    // On the one line, its line break and escape spelled out.
    assert!(
        line.contains(r"\n │ FORGED LINE\u{1b}[2J"),
        "{line:?} in {log:?}"
    );
    assert!(!log.contains('\r'), "{log:?}");
    assert!(!log.contains('\u{1b}'), "{log:?}");
    assert!(
        !log.lines().any(|line| line.starts_with("FORGED")),
        "{log:?}"
    );
}
