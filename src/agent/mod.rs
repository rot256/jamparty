//! The agent's side: `nabe recv` and `nabe send` over the runtime
//! socket, and the fixed texts (`nabe`, `nabe agent`).

pub(crate) mod guides;

use std::io::Write;

use tokio::{io::BufReader, net::UnixStream};

use crate::{
    BoxError,
    adapters::lines,
    core::ids::CommentId,
    protocol::{
        agent::{AgentReply, AgentRequest},
        control::{AnswerText, Envelope},
    },
};

/// Waits for pending comments, prints them to `out` as NDJSON and tells the
/// share they were printed; returns how many.
pub(crate) async fn recv(stream: UnixStream, out: &mut impl Write) -> Result<usize, BoxError> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    lines::write(&mut write, &Envelope::from(AgentRequest::Recv)).await?;
    let mut comments = Vec::new();
    loop {
        let reply = lines::read::<Envelope<AgentReply>, _>(&mut read)
            .await?
            .ok_or("the share stopped")?;
        match reply.message {
            AgentReply::Comment { comment } => comments.push(comment),
            AgentReply::End => break,
            AgentReply::Error { error } => return Err(error.into()),
            AgentReply::Sent => return Err("unexpected reply from the share".into()),
        }
    }
    for comment in &comments {
        serde_json::to_writer(&mut *out, comment)?;
        out.write_all(b"\n")?;
    }
    out.flush()?;
    lines::write(&mut write, &Envelope::from(AgentRequest::Printed)).await?;
    Ok(comments.len())
}

/// Sends an answer to every joiner, next to comment `anchor` if given.
pub(crate) async fn send(
    stream: UnixStream,
    message: AnswerText,
    anchor: Option<CommentId>,
) -> Result<(), BoxError> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    lines::write(
        &mut write,
        &Envelope::from(AgentRequest::Send { message, anchor }),
    )
    .await?;
    let reply = lines::read::<Envelope<AgentReply>, _>(&mut read)
        .await?
        .ok_or("the share stopped")?;
    match reply.message {
        AgentReply::Sent => Ok(()),
        AgentReply::Error { error } => Err(error.into()),
        AgentReply::Comment { .. } | AgentReply::End => {
            Err("unexpected reply from the share".into())
        }
    }
}
