//! Delivery: one task per share takes batches from the hub's queue and
//! hands them to the target.
//!
//! - `recv`: each batch waits for a `jampot recv` on the runtime socket;
//!   it is delivered once that client says it printed it, and returned to
//!   the queue (no backoff) if the client goes away first.
//! - `command`: the batch goes to the command's stdin.
//! - `t3`: the batch is a message to a T3 thread.

use std::{ffi::OsString, fmt, sync::Arc, time::Duration};

use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
};

use crate::{
    adapters::{
        command,
        t3::{T3Delivery, T3Target, ThreadId},
    },
    core::queue::Attempt,
    protocol::comment::Comment,
    share::hub::Hub,
};

/// Where comments go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// Held for `jampot recv`.
    Recv,
    /// Piped to a command.
    Command(CommandTarget),
    /// Sent to a T3 thread (resolved at start).
    T3(T3Target),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandTarget {
    /// Not empty.
    pub(crate) argv: Vec<OsString>,
    /// A run taking longer is killed and counts as failed.
    pub(crate) timeout: Duration,
}

/// A waiting `jampot recv`: the batch goes out on `batch`; `printed`
/// resolves once the client printed it (dropped if it went away).
pub(crate) struct RecvRequest {
    pub(crate) batch: oneshot::Sender<Vec<Comment>>,
    pub(crate) printed: oneshot::Receiver<()>,
}

/// What a `jampot recv` finds on the runtime socket.
#[derive(Clone)]
pub(crate) enum RecvAccess {
    /// The target is `recv`: where clients queue up.
    Open(mpsc::Sender<RecvRequest>),
    /// Comments go elsewhere.
    Elsewhere(Elsewhere),
}

/// Where comments go when not to `jampot recv`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Elsewhere {
    /// A command, by its program.
    Command(OsString),
    /// A T3 thread.
    T3(ThreadId),
}

impl fmt::Display for Elsewhere {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(program) => write!(f, "the command {}", program.to_string_lossy()),
            Self::T3(thread) => write!(f, "the T3 thread {thread}"),
        }
    }
}

/// Starts delivery; returns what `jampot recv` clients get.
pub(crate) fn start(target: Target, hub: Arc<Hub>, tasks: &mut JoinSet<()>) -> RecvAccess {
    match target {
        Target::Recv => {
            let (requests, receiver) = mpsc::channel(16);
            tasks.spawn(to_recv(hub, receiver));
            RecvAccess::Open(requests)
        }
        Target::Command(command) => {
            let program = command.argv.first().cloned().unwrap_or_default();
            tasks.spawn(to_command(hub, command));
            RecvAccess::Elsewhere(Elsewhere::Command(program))
        }
        Target::T3(target) => {
            let thread = target.thread.clone();
            tasks.spawn(to_t3(hub, T3Delivery::new(target)));
            RecvAccess::Elsewhere(Elsewhere::T3(thread))
        }
    }
}

async fn to_recv(hub: Arc<Hub>, mut requests: mpsc::Receiver<RecvRequest>) {
    while let Some(RecvRequest { mut batch, printed }) = requests.recv().await {
        let comments = tokio::select! {
            comments = hub.next_batch() => comments,
            () = batch.closed() => continue,
        };
        if batch.send(comments).is_err() {
            // The `jampot recv` went away.
            hub.finish(Attempt::Returned);
            continue;
        }
        match printed.await {
            Ok(()) => hub.finish(Attempt::Delivered),
            Err(_) => hub.finish(Attempt::Returned),
        }
    }
}

async fn to_command(hub: Arc<Hub>, target: CommandTarget) {
    loop {
        let batch = hub.next_batch().await;
        hub.finish(command::deliver(&target.argv, target.timeout, &batch).await);
    }
}

async fn to_t3(hub: Arc<Hub>, t3: T3Delivery) {
    loop {
        let batch = hub.next_batch().await;
        hub.finish(t3.deliver(&batch).await);
    }
}
