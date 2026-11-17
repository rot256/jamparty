//! Only recv delivers comments. Commands receive reminders, never comments.

use std::{ffi::OsString, sync::Arc, time::Duration};

use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
};

use crate::{
    adapters::command, core::queue::DeliveryResult, protocol::comment::Comment, share::hub::Hub,
};

/// Bounded output per recv keeps one agent turn a manageable size.
pub(crate) const MAX_BATCH_BYTES: usize = 115_000;

/// Comments always go to recv; optionally remind an idle agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// Held for `jamparty recv`.
    Recv,
    /// A reminder when nobody is listening.
    Nudge(CommandTarget),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandTarget {
    /// Not empty.
    pub(crate) argv: Vec<OsString>,
    /// A run taking longer is killed and counts as failed.
    pub(crate) timeout: Duration,
}

/// A waiting `jamparty recv`: the batch goes out on `batch`; `printed`
/// resolves once the client printed it (dropped if it went away).
pub(crate) struct RecvRequest {
    pub(crate) batch: oneshot::Sender<Vec<Comment>>,
    pub(crate) printed: oneshot::Receiver<()>,
}

/// Starts recv delivery and, optionally, a separate nudge task.
pub(crate) fn start(
    target: Target,
    hub: Arc<Hub>,
    share_id: String,
    tasks: &mut JoinSet<()>,
) -> mpsc::Sender<RecvRequest> {
    let (requests, receiver) = mpsc::channel(16);
    tasks.spawn(to_recv(Arc::clone(&hub), receiver));
    if let Target::Nudge(command) = target {
        tasks.spawn(nudge(hub, command, share_id));
    }
    requests
}

async fn nudge(hub: Arc<Hub>, target: CommandTarget, share_id: String) {
    loop {
        let (generation, count) = hub.next_nudge().await;
        let noun = if count == 1 { "comment" } else { "comments" };
        let notice = format!(
            "jamparty: {count} {noun} waiting on {share_id}; run jamparty recv --share {share_id}\n"
        );
        let result = command::nudge(&target.argv, target.timeout, &notice).await;
        hub.finish_nudge(generation, result);
    }
}

async fn to_recv(hub: Arc<Hub>, mut requests: mpsc::Receiver<RecvRequest>) {
    while let Some(RecvRequest { mut batch, printed }) = requests.recv().await {
        let comments = tokio::select! {
            comments = hub.next_batch() => comments,
            () = batch.closed() => continue,
        };
        if batch.send(comments).is_err() {
            // The `jamparty recv` went away.
            hub.finish(DeliveryResult::Returned);
            continue;
        }
        match printed.await {
            Ok(()) => hub.finish(DeliveryResult::Ok),
            Err(_) => hub.finish(DeliveryResult::Returned),
        }
    }
}
