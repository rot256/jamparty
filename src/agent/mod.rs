//! The agent's side: `jamparty recv` and `jamparty send` over the runtime
//! socket, and the fixed texts (`jamparty`, `jamparty agent`).

pub(crate) mod fetch;
pub(crate) mod guides;

use std::io::Write;

use rootcause::{option_ext::OptionExt, prelude::bail};
use tokio::{io::BufReader, net::UnixStream};

use crate::{
    Report,
    adapters::lines,
    core::ids::CommentId,
    protocol::{
        agent::{Addressee, AgentReply, AgentRequest},
        answer::AnswerAnchor,
        control::{AnswerText, Choices, Envelope},
    },
};

/// Waits for pending comments, prints them to `out` as NDJSON and tells the
/// share they were printed; returns how many.
pub(crate) async fn recv(stream: UnixStream, out: &mut impl Write) -> Result<usize, Report> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    lines::write(&mut write, &Envelope::from(AgentRequest::Recv)).await?;
    let mut comments = Vec::new();
    loop {
        let reply = lines::read::<Envelope<AgentReply>, _>(&mut read)
            .await?
            .context("the share stopped")?;
        match reply.message {
            AgentReply::Comment { comment } => comments.push(*comment),
            AgentReply::End => break,
            AgentReply::Error { error } => bail!("{error}"),
            AgentReply::Sent | AgentReply::Resource | AgentReply::ResourceMissing => {
                bail!("unexpected reply from the share")
            }
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

/// Sends an answer. A comment anchor defaults to its author, while a
/// place anchor or an unanchored answer defaults to everyone.
pub(crate) async fn send(
    stream: UnixStream,
    message: AnswerText,
    choices: Choices,
    anchor: Option<AnswerAnchor<CommentId>>,
    to: Option<Addressee>,
) -> Result<(), Report> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    lines::write(
        &mut write,
        &Envelope::from(AgentRequest::Send {
            message,
            choices,
            anchor,
            to,
        }),
    )
    .await?;
    let reply = lines::read::<Envelope<AgentReply>, _>(&mut read)
        .await?
        .context("the share stopped")?;
    match reply.message {
        AgentReply::Sent => Ok(()),
        AgentReply::Error { error } => bail!("{error}"),
        AgentReply::Comment { .. }
        | AgentReply::End
        | AgentReply::Resource
        | AgentReply::ResourceMissing => bail!("unexpected reply from the share"),
    }
}
