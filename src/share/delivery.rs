//! Delivery: one task per share takes batches from the hub's queue and
//! hands them to the target.
//!
//! - `recv`: each batch waits for a `nabe recv` on the runtime socket;
//!   it is delivered once that client says it printed it, and returned to
//!   the queue (no backoff) if the client goes away first.
//! - `command`: the batch goes to the command's stdin.
//! - `t3`: the batch is a message to a T3 thread.

use std::{ffi::OsString, sync::Arc, time::Duration};

use tokio::sync::{mpsc, oneshot};

use crate::{
    adapters::{
        command,
        t3::{T3Delivery, T3Target},
    },
    core::queue::DeliveryResult,
    protocol::comment::Comment,
    share::hub::Hub,
};

/// Where comments go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// Held for `nabe recv`.
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

/// A waiting `nabe recv`: the batch goes out on `batch`; `printed`
/// resolves once the client printed it (dropped if it went away).
pub(crate) struct RecvRequest {
    pub(crate) batch: oneshot::Sender<Vec<Comment>>,
    pub(crate) printed: oneshot::Receiver<()>,
}

/// Starts delivery; returns where `nabe recv` clients queue up (`None`
/// unless the target is `recv`).
pub(crate) fn start(
    target: Target,
    hub: Arc<Hub>,
    tasks: &mut tokio::task::JoinSet<()>,
) -> Option<mpsc::Sender<RecvRequest>> {
    match target {
        Target::Recv => {
            let (requests, receiver) = mpsc::channel(16);
            tasks.spawn(to_recv(hub, receiver));
            Some(requests)
        }
        Target::Command(command) => {
            tasks.spawn(to_command(hub, command));
            None
        }
        Target::T3(target) => {
            tasks.spawn(to_t3(hub, T3Delivery::new(target)));
            None
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
            hub.finish(DeliveryResult::Returned, "recv went away");
            continue;
        }
        match printed.await {
            Ok(()) => hub.finish(DeliveryResult::Ok, ""),
            Err(_) => hub.finish(DeliveryResult::Returned, "recv went away"),
        }
    }
}

async fn to_command(hub: Arc<Hub>, target: CommandTarget) {
    loop {
        let batch = hub.next_batch().await;
        let (result, detail) = command::deliver(&target.argv, target.timeout, &batch).await;
        hub.finish(result, &detail);
    }
}

async fn to_t3(hub: Arc<Hub>, t3: T3Delivery) {
    loop {
        let batch = hub.next_batch().await;
        let (result, detail) = t3.deliver(&batch).await;
        hub.finish(result, &detail);
    }
}
