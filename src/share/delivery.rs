//! Delivery: one task per share takes batches from the hub's queue and
//! hands them to the target.
//!
//! - `recv`: each batch waits for a `jamjam recv` on the runtime socket;
//!   it is delivered once that client says it printed it, and returned to
//!   the queue (no backoff) if the client goes away first.
//! - `command`: the batch goes to the command's stdin.

use std::{ffi::OsString, sync::Arc};

use tokio::sync::{mpsc, oneshot};

use crate::{
    adapters::command, core::queue::DeliveryResult, protocol::comment::Comment, share::hub::Hub,
};

/// Where comments go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// Held for `jamjam recv`.
    Recv,
    /// Piped to this argv (not empty).
    Command(Vec<OsString>),
}

/// A waiting `jamjam recv`: the batch goes out on `batch`; `printed`
/// resolves once the client printed it (dropped if it went away).
pub(crate) struct RecvRequest {
    pub(crate) batch: oneshot::Sender<Vec<Comment>>,
    pub(crate) printed: oneshot::Receiver<()>,
}

/// Starts delivery; returns where `jamjam recv` clients queue up (`None`
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
        Target::Command(argv) => {
            tasks.spawn(to_command(hub, argv));
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

async fn to_command(hub: Arc<Hub>, argv: Vec<OsString>) {
    loop {
        let batch = hub.next_batch().await;
        let (result, detail) = command::deliver(&argv, &batch).await;
        hub.finish(result, &detail);
    }
}
