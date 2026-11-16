//! The share's end of the runtime socket: `recv` and `send` requests from
//! the user's own processes (see [`crate::protocol::agent`]).

use std::{sync::Arc, time::Duration};

use tokio::{
    io::BufReader,
    net::{
        UnixListener, UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::{mpsc, oneshot},
    task::JoinSet,
    time,
};

use crate::{
    adapters::{lines, runtime},
    core::text::OneLineReport,
    protocol::{
        agent::{AgentReply, AgentRequest},
        control::Envelope,
    },
    share::{
        delivery::{RecvAccess, RecvRequest},
        hub::Hub,
    },
};

/// How long `jamparty recv` has to confirm it printed a batch; after that
/// the batch goes back to the queue.
const PRINTED_TIMEOUT: Duration = Duration::from_secs(30);
/// The pause after a failed accept.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Serves the user's own processes only (checked by their credentials).
pub(crate) async fn serve(listener: UnixListener, hub: Arc<Hub>, recv: RecvAccess) {
    let mut clients = JoinSet::new();
    loop {
        match listener.accept().await {
            Ok((stream, _)) => match runtime::check_peer(&stream) {
                Ok(()) => {
                    clients.spawn(handle(stream, Arc::clone(&hub), recv.clone()));
                }
                Err(error) => {
                    tracing::warn!(error = %OneLineReport(&error), "refused a runtime socket client")
                }
            },
            Err(error) => {
                tracing::warn!(%error, "runtime socket accept failed");
                // Out of descriptors, say: let it pass rather than spin.
                time::sleep(ACCEPT_BACKOFF).await;
            }
        }
        while clients.try_join_next().is_some() {}
    }
}

async fn handle(stream: UnixStream, hub: Arc<Hub>, recv: RecvAccess) {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    let request = match lines::read::<Envelope<AgentRequest>, _>(&mut read).await {
        Ok(Some(Envelope {
            version: _,
            message,
        })) => message,
        Ok(None) => return,
        Err(error) => {
            reply(
                &mut write,
                AgentReply::Error {
                    error: format!("invalid request: {}", OneLineReport(&error)),
                },
            )
            .await;
            return;
        }
    };
    let answer = match request {
        AgentRequest::Recv => match recv {
            RecvAccess::Open(requests) => return serve_recv(read, write, &requests).await,
            RecvAccess::Command(program) => AgentReply::Error {
                error: format!(
                    "this share delivers comments to the command {}, not to jamparty recv",
                    program.to_string_lossy()
                ),
            },
        },
        AgentRequest::Send {
            choices,
            message,
            anchor,
            to,
        } => match hub.answer(message, anchor, to, choices) {
            Ok(_) => AgentReply::Sent,
            Err(error) => AgentReply::Error {
                error: error.to_string(),
            },
        },
        AgentRequest::Printed => AgentReply::Error {
            error: "printed without recv".to_owned(),
        },
    };
    reply(&mut write, answer).await;
}

async fn reply(write: &mut OwnedWriteHalf, message: AgentReply) {
    let _ = lines::write(write, &Envelope::from(message)).await;
}

/// Queues up for a batch, sends it, and reports it delivered only once the
/// client says it printed it. Returning early drops `printed`, which puts
/// the batch back.
async fn serve_recv(
    mut read: BufReader<OwnedReadHalf>,
    mut write: OwnedWriteHalf,
    requests: &mpsc::Sender<RecvRequest>,
) {
    let (batch_tx, batch_rx) = oneshot::channel();
    let (printed_tx, printed_rx) = oneshot::channel();
    let request = RecvRequest {
        batch: batch_tx,
        printed: printed_rx,
    };
    if requests.send(request).await.is_err() {
        return;
    }
    let comments = tokio::select! {
        comments = batch_rx => match comments {
            Ok(comments) => comments,
            Err(_) => return,
        },
        // The client sends nothing while waiting: any read result means it
        // went away.
        _ = lines::read::<Envelope<AgentRequest>, _>(&mut read) => return,
    };
    for comment in comments {
        if lines::write(&mut write, &Envelope::from(AgentReply::Comment { comment }))
            .await
            .is_err()
        {
            return;
        }
    }
    if lines::write(&mut write, &Envelope::from(AgentReply::End))
        .await
        .is_err()
    {
        return;
    }
    let printed = time::timeout(
        PRINTED_TIMEOUT,
        lines::read::<Envelope<AgentRequest>, _>(&mut read),
    )
    .await;
    if let Ok(Ok(Some(Envelope {
        version: _,
        message: AgentRequest::Printed,
    }))) = printed
    {
        let _ = printed_tx.send(());
    }
}
