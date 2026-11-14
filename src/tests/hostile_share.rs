//! A share that claims more of this joiner's comments than a share keeps
//! is dropped before the joiner's state grows with it.

use std::sync::atomic::Ordering;

use crate::{
    core::{ids::CommentId, rng::SystemRng},
    protocol::control::{AckOutcome, Envelope, MAX_QUEUED_PER_KEY, ShareMessage},
    tests::{
        fake_share::fake_share,
        harness::{try_joiner, until, within},
    },
};

fn admitted(undelivered: &[CommentId]) -> String {
    let key = "ab".repeat(32);
    let undelivered = serde_json::to_string(undelivered).unwrap();
    format!(
        r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"alice","fingerprint":"ababababab","key":"{key}"}},"undelivered":{undelivered},"presence":"supported","site":{{"kind":"files"}},"jampot":"{}"}}}}"#,
        env!("CARGO_PKG_VERSION")
    )
}

fn fresh_ids(count: usize) -> Vec<CommentId> {
    (0..count)
        .map(|_| CommentId::generate(&SystemRng))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn unsolicited_acks_past_what_a_share_keeps_drop_it() {
    let mut lines = vec![admitted(&[])];
    for (index, id) in fresh_ids(MAX_QUEUED_PER_KEY + 1).into_iter().enumerate() {
        let ack = ShareMessage::CommentAck {
            reference: format!("unasked-{index}").try_into().unwrap(),
            outcome: AckOutcome::Queued { id },
        };
        lines.push(serde_json::to_string(&Envelope::from(ack)).unwrap());
    }
    let share = fake_share(lines).await;
    let joiner = within(
        "the joiner",
        try_joiner(share.token.clone(), share.addrs.clone()),
    )
    .await
    .unwrap();
    // It took what a share can hold, then hung up and tries again.
    until("a second hello", || {
        share.hellos.load(Ordering::Relaxed) >= 2
    })
    .await;
    assert!(joiner.watch().borrow().pending() <= MAX_QUEUED_PER_KEY);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admission_with_too_many_pending_comments_is_not_taken() {
    let share = fake_share(vec![admitted(&fresh_ids(MAX_QUEUED_PER_KEY + 1))]).await;
    let joining = tokio::spawn(try_joiner(share.token.clone(), share.addrs.clone()));
    until("a second hello", || {
        share.hellos.load(Ordering::Relaxed) >= 2
    })
    .await;
    // Never admitted: the joiner is still trying.
    assert!(!joining.is_finished());
    joining.abort();
}
