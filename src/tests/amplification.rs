//! A share's answers cannot make the joiner's memory, or what it sends
//! its pages, grow past fixed bounds: Markdown that renders large is shown
//! as plain text, and the answers kept are bounded in bytes.

use std::time::Duration;

use crate::{
    core::{ids::CommentId, rng::SystemRng},
    join::{MAX_ANSWER_HTML, MAX_ANSWERS_BYTES, MAX_FRAME},
    tests::{
        fake_share::fake_share,
        harness::{PageSocket, try_joiner, within},
    },
};

fn admitted() -> String {
    let key = "ab".repeat(32);
    format!(
        r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"alice","fingerprint":"ababababab","key":"{key}"}},"undelivered":[],"site":{{"kind":"files"}},"jamparty":"{}"}}}}"#,
        env!("CARGO_PKG_VERSION")
    )
}

/// 10,000 characters of Markdown that render to about 2.4 MB as a table.
fn table() -> String {
    let mut markdown = "|a".repeat(1250);
    markdown.push('\n');
    markdown.push_str(&"|-".repeat(1250));
    markdown.push('\n');
    markdown.push_str(&"x\n".repeat(2499));
    markdown
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_that_render_large_stay_bounded() {
    let message = serde_json::to_string(&table()).unwrap();
    let mut lines = vec![admitted()];
    // More than the bytes kept hold, as plain text.
    for _ in 0..20 {
        let id = CommentId::generate(&SystemRng);
        lines.push(format!(
            r#"{{"version":1,"message":{{"type":"answer","id":"{id}","message":{message}}}}}"#
        ));
    }
    lines.push(r#"{"version":1,"message":{"type":"status","line":"done"}}"#.to_owned());
    let share = fake_share(lines).await;
    let joiner = within(
        "the joiner",
        try_joiner(share.token.clone(), share.addrs.clone()),
    )
    .await
    .unwrap();
    let mut view = joiner.watch();
    let view = within(
        "the last line",
        view.wait_for(|view| view.link.status.clone().map(String::from).as_deref() == Some("done")),
    )
    .await
    .unwrap()
    .clone();
    assert!(!view.answers.is_empty());
    let mut stored = 0;
    for answer in &view.answers {
        assert!(
            answer.html.len() <= MAX_ANSWER_HTML,
            "{}",
            answer.html.len()
        );
        assert!(!answer.html.as_str().contains("<table>"));
        stored += answer.html.len();
    }
    assert!(stored <= MAX_ANSWERS_BYTES, "{stored}");
    assert!(view.answers.len() < 20, "{}", view.answers.len());
    // What a page is told when it attaches fits a frame.
    let (_page, hello) = within(
        "the hello",
        tokio::time::timeout(Duration::from_secs(10), PageSocket::hello(&joiner)),
    )
    .await
    .unwrap();
    let size = serde_json::to_vec(&hello).unwrap().len();
    assert!(size <= MAX_FRAME, "{size}");
    assert_eq!(
        hello["answers"].as_array().unwrap().len(),
        view.answers.len()
    );
}
