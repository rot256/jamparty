//! The t3 target against a fake T3 MCP endpoint: reading the thread at
//! start, one `t3_thread_send` per batch, backoff, a gone thread, token
//! renewal and both response forms. Plus the opt-in contract test against a
//! real T3.

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use jiff::{SignedDuration, Timestamp};
use tempfile::TempDir;
use tokio::net::UnixStream;

use crate::{
    adapters::t3::{self, T3Delivery, T3Target, Thread, ThreadId},
    agent,
    core::{clock::ManualClock, queue::DeliveryResult},
    protocol::comment::{Author, Comment, Draft},
    share::{
        Source,
        delivery::Target,
        hub::{HELD, RETRYING},
    },
    tests::{
        fake_t3::{FakeT3, FakeThread, State},
        harness::{
            DRAFT, TestShare, commented, config, fixture_dir, join, post_comment, start, until,
            within,
        },
    },
};

fn t0() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

fn threads() -> State {
    let mut state = State::default();
    let thread = |title, parent| FakeThread {
        title,
        parent,
        archived: false,
    };
    state.threads.insert("top", thread("Fix the header", None));
    state
        .threads
        .insert("child", thread("Delegated", Some(("top", "subagent"))));
    state
        .threads
        .insert("fork", thread("Forked", Some(("top", "fork"))));
    state.threads.insert(
        "old",
        FakeThread {
            title: "Old",
            parent: None,
            archived: true,
        },
    );
    state
}

/// A token file in `dir` holding `token`.
fn token_file(dir: &TempDir, token: &str) -> PathBuf {
    let path = dir.path().join("token");
    fs::write(&path, format!("{token}\n")).unwrap();
    path
}

fn target(fake: &FakeT3, token: &Path, thread: &str) -> T3Target {
    T3Target {
        url: fake.url().parse().unwrap(),
        token_file: token.to_owned(),
        thread: thread.to_owned().try_into().unwrap(),
        site: "/srv/site".to_owned(),
    }
}

async fn share_to(dir: &TempDir, target: T3Target, clock: Arc<ManualClock>) -> TestShare {
    let root = fs::canonicalize(dir.path()).unwrap();
    let mut config = config(Source::Files(root), Target::T3(target));
    config.clock = clock;
    start(config).await
}

#[tokio::test(flavor = "multi_thread")]
async fn start_reads_the_thread_and_resolves_subagents() {
    let fake = FakeT3::start(threads()).await;
    let work = tempfile::tempdir().unwrap();
    let token = token_file(&work, "secret-token");

    let top = t3::open(&target(&fake, &token, "top")).await.unwrap();
    assert_eq!(
        top,
        Thread {
            id: "top".to_owned().try_into().unwrap(),
            title: "Fix the header".to_owned(),
            resolved_from: None,
        }
    );
    let child = t3::open(&target(&fake, &token, "child")).await.unwrap();
    assert_eq!(
        (
            child.id.as_str(),
            child.resolved_from.as_ref().map(ThreadId::as_str)
        ),
        ("top", Some("child"))
    );
    // A fork is a thread of its own.
    let fork = t3::open(&target(&fake, &token, "fork")).await.unwrap();
    assert_eq!((fork.id.as_str(), fork.resolved_from), ("fork", None));

    let unknown = t3::open(&target(&fake, &token, "nope")).await.unwrap_err();
    assert!(unknown.to_string().contains("no thread nope"), "{unknown}");
    let archived = t3::open(&target(&fake, &token, "old")).await.unwrap_err();
    assert!(archived.to_string().contains("archived"), "{archived}");

    // MCP done properly: initialize, initialized, then calls in the session.
    let calls = fake.state().calls.clone();
    let methods: Vec<&str> = calls
        .iter()
        .take(3)
        .map(|call| call.method.as_str())
        .collect();
    assert_eq!(
        methods,
        ["initialize", "notifications/initialized", "tools/call"]
    );
    for call in &calls {
        assert_eq!(
            call.session.is_none(),
            call.method == "initialize",
            "{call:?}"
        );
        assert_eq!(call.token, "secret-token");
    }
    let read = fake.calls_to("t3_thread_read");
    let read = read.first().unwrap();
    assert_eq!(read.arguments.thread_id.as_deref(), Some("top"));
}

/// Shares to thread `top`, has alice comment, waits for the delivery.
async fn deliver_one(sse: bool) {
    let mut state = threads();
    state.sse = sse;
    let fake = FakeT3::start(state).await;
    let work = tempfile::tempdir().unwrap();
    let token = token_file(&work, "secret-token");
    let dir = fixture_dir();
    let target = target(&fake, &token, "top");
    t3::open(&target).await.unwrap();
    let share = share_to(&dir, target, Arc::new(ManualClock::new(t0()))).await;
    let alice = join(&share, "alice").await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    let mut view = alice.watch();
    within("delivery", view.wait_for(|view| view.pending() == 0))
        .await
        .unwrap();
    let [send] = fake.calls_to("t3_thread_send").try_into().unwrap();
    assert_eq!(send.arguments.thread_id.as_deref(), Some("top"));
    assert_eq!(send.arguments.mode.as_deref(), Some("queue"));
    assert!(send.arguments.client_request_id.is_some());
    let message = send.arguments.message.unwrap();
    let mut lines = message.lines();
    assert_eq!(lines.next(), Some(t3::header("/srv/site").as_str()));
    let comment: Comment = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(comment.comment, "Bigger, please");
    assert_eq!(comment.author.name.to_string(), "alice");
    assert!(lines.next().is_none());

    // recv is not how this share delivers.
    let stream = UnixStream::connect(share.socket_path()).await.unwrap();
    let error = agent::recv(stream, &mut Vec::new()).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("the T3 thread top, not to jampot recv"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_is_one_queued_message_in_the_thread() {
    deliver_one(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn event_stream_responses_work_too() {
    deliver_one(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_http_error_backs_off_on_the_clock_then_delivers() {
    let mut state = threads();
    state.http_failures = 1;
    let fake = FakeT3::start(state).await;
    let work = tempfile::tempdir().unwrap();
    let token = token_file(&work, "secret-token");
    let dir = fixture_dir();
    let clock = Arc::new(ManualClock::new(t0()));
    let share = share_to(&dir, target(&fake, &token, "top"), Arc::clone(&clock)).await;
    let alice = join(&share, "alice").await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    until("the backoff", || {
        share.hub().wake_at() == Some(t0() + SignedDuration::from_secs(1))
    })
    .await;
    let mut view = alice.watch();
    let status = within(
        "the status line",
        view.wait_for(|view| view.link.status.is_some()),
    )
    .await
    .unwrap()
    .link
    .status
    .clone()
    .unwrap();
    // The HTTP error stays in the share's log.
    assert_eq!(String::from(status), RETRYING);
    assert_eq!(fake.calls_to("t3_thread_send").len(), 1);
    // A comment arriving meanwhile goes in a batch of its own.
    let later = DRAFT.replace("Bigger, please", "Later");
    assert!(commented(&post_comment(&alice, &later).await).is_some());

    clock.advance(SignedDuration::from_secs(1));
    share.hub().wake();
    within(
        "delivery",
        view.wait_for(|view| view.pending() == 0 && view.link.status.is_none()),
    )
    .await
    .unwrap();
    let sends = fake.calls_to("t3_thread_send");
    let [failed, retried, next] = sends.as_slice() else {
        panic!("{sends:?}");
    };
    // The retry is the same message under the same key.
    assert_eq!(retried.arguments.message, failed.arguments.message);
    assert_eq!(
        retried.arguments.client_request_id,
        failed.arguments.client_request_id
    );
    assert_ne!(
        next.arguments.client_request_id,
        failed.arguments.client_request_id
    );
    assert!(next.arguments.message.as_deref().unwrap().contains("Later"));
    assert!(
        !retried
            .arguments
            .message
            .as_deref()
            .unwrap()
            .contains("Later")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_gone_during_delivery_holds_comments() {
    let fake = FakeT3::start(threads()).await;
    let work = tempfile::tempdir().unwrap();
    let token = token_file(&work, "secret-token");
    let dir = fixture_dir();
    let target = target(&fake, &token, "top");
    t3::open(&target).await.unwrap();
    fake.state().unsendable.insert("top".to_owned());
    let share = share_to(&dir, target, Arc::new(ManualClock::new(t0()))).await;
    let alice = join(&share, "alice").await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    let mut view = alice.watch();
    let status = within(
        "the status line",
        view.wait_for(|view| view.link.status.is_some()),
    )
    .await
    .unwrap()
    .link
    .status
    .clone()
    .unwrap();
    assert_eq!(String::from(status), HELD);
    assert_eq!(share.queued().len(), 1);
    assert_eq!(share.hub().wake_at(), None);
}

fn comment(text: &str) -> Comment {
    let author: Author = Author::new("alice".parse().unwrap(), "01".repeat(32).parse().unwrap());
    let draft: Draft =
        serde_json::from_value(serde_json::json!({"page": "/", "selector": "h1", "comment": text}))
            .unwrap();
    Comment::new(
        crate::core::ids::CommentId::generate(&crate::core::rng::SystemRng),
        author,
        draft,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_token_is_read_at_each_delivery_and_sessions_are_renewed() {
    let fake = FakeT3::start(threads()).await;
    let work = tempfile::tempdir().unwrap();
    let token = token_file(&work, "first-token");
    let t3 = T3Delivery::new(target(&fake, &token, "top"));
    let (result, _) = t3.deliver(&[comment("one")]).await;
    assert_eq!(result, DeliveryResult::Ok);

    token_file(&work, "renewed-token");
    // T3 restarted meanwhile: the old session is unknown.
    fake.forget_sessions();
    let (result, detail) = t3.deliver(&[comment("two")]).await;
    assert_eq!(result, DeliveryResult::Ok, "{detail}");

    let sends = fake.calls_to("t3_thread_send");
    let tokens: Vec<&str> = sends.iter().map(|call| call.token.as_str()).collect();
    assert_eq!(tokens, ["first-token", "renewed-token", "renewed-token"]);
    let initializes = fake
        .state()
        .calls
        .iter()
        .filter(|call| call.method == "initialize")
        .count();
    assert_eq!(initializes, 2);

    fs::remove_file(&token).unwrap();
    let (result, detail) = t3.deliver(&[comment("three")]).await;
    assert_eq!(result, DeliveryResult::Error);
    assert!(detail.contains("token file"), "{detail}");
}

/// Against a real T3: reads the thread and sends one message to it. Runs
/// only when `JAMPOT_T3_URL`, `JAMPOT_T3_TOKEN_FILE` and `JAMPOT_T3_THREAD`
/// are set (use a throwaway thread):
///
/// ```sh
/// JAMPOT_T3_URL=http://127.0.0.1:<port>/mcp JAMPOT_T3_TOKEN_FILE=<file> \
///   JAMPOT_T3_THREAD=<thread id> cargo test t3_contract -- --nocapture
/// ```
#[tokio::test]
async fn t3_contract() {
    let vars = ["JAMPOT_T3_URL", "JAMPOT_T3_TOKEN_FILE", "JAMPOT_T3_THREAD"].map(env::var);
    let [Ok(url), Ok(token_file), Ok(thread)] = vars else {
        eprintln!("t3_contract skipped: set JAMPOT_T3_URL, JAMPOT_T3_TOKEN_FILE, JAMPOT_T3_THREAD");
        return;
    };
    let mut target = T3Target {
        url: url.parse().unwrap(),
        token_file: token_file.into(),
        thread: thread.try_into().unwrap(),
        site: "the jampot contract test".to_owned(),
    };
    let thread = t3::open(&target).await.unwrap();
    eprintln!("t3_contract: thread {:?} ({})", thread.title, thread.id);
    target.thread = thread.id;
    let (result, detail) = T3Delivery::new(target)
        .deliver(&[comment("jampot contract test: ignore this message.")])
        .await;
    assert_eq!(result, DeliveryResult::Ok, "{detail}");
}
