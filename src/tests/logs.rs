//! What the share sends reaches the joiner's log with its control
//! characters escaped: it cannot forge log lines or drive the terminal.

use std::{
    io,
    sync::{Arc, Mutex, PoisonError},
};

use crate::tests::{
    fake_share::fake_share,
    harness::{try_joiner, until, within},
};

fn admitted() -> String {
    format!(
        r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"alice","fingerprint":"ababababab","key":"{}"}},"undelivered":[],"presence":"supported","site":{{"kind":"files"}},"jampot":"{}"}}}}"#,
        "ab".repeat(32),
        env!("CARGO_PKG_VERSION")
    )
}

/// Log output, kept for the test to read.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// This is the suite's only logging subscriber. Keep it global: a callsite
// first reached by another test must register against this subscriber too.
// A scoped subscriber can cache "never" from that thread's empty default.
#[tokio::test(flavor = "current_thread")]
async fn a_control_line_with_escapes_is_logged_escaped() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter("off,jampot::join::connection=debug")
        .with_ansi(false)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

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
