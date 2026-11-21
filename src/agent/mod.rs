//! The agent's side: `jamparty recv` and `jamparty send` over the runtime
//! socket, and the fixed texts (`jamparty`, `jamparty agent`).

pub(crate) mod fetch;
pub(crate) mod guides;

use std::io::Write;

use rootcause::{option_ext::OptionExt, prelude::bail};
use serde::Serialize;
use serde_json::{
    Serializer,
    ser::{CompactFormatter, Formatter},
};
use tokio::{io::BufReader, net::UnixStream};

use crate::{
    Report,
    adapters::lines,
    core::ids::CommentId,
    protocol::{
        agent::{Addressee, AgentReply, AgentRequest},
        answer::AnswerAnchor,
        comment::Comment,
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
        write_comment_line(out, comment)?;
    }
    out.flush()?;
    lines::write(&mut write, &Envelope::from(AgentRequest::Printed)).await?;
    Ok(comments.len())
}

fn write_comment_line(out: &mut impl Write, comment: &Comment) -> Result<(), Report> {
    comment.serialize(&mut Serializer::with_formatter(&mut *out, AgentJson))?;
    out.write_all(b"\n")?;
    Ok(())
}

struct AgentJson;

impl Formatter for AgentJson {
    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> std::io::Result<()>
    where
        W: ?Sized + Write,
    {
        for part in fragment.split_inclusive(needs_json_escape) {
            if let Some(character) = part.chars().next_back().filter(|c| needs_json_escape(*c)) {
                let (text, _) = part.split_at(part.len() - character.len_utf8());
                CompactFormatter.write_string_fragment(writer, text)?;
                write!(writer, "\\u{:04x}", u32::from(character))?;
            } else {
                CompactFormatter.write_string_fragment(writer, part)?;
            }
        }
        Ok(())
    }
}

fn needs_json_escape(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::write_comment_line;
    use crate::{
        core::{ids::EndpointKey, name::JoinerName},
        protocol::{comment::Comment, resource::ResourceId},
    };

    #[test]
    fn comment_ndjson_escapes_bidi_controls_and_line_separators() {
        let controls = [
            '\u{061c}', '\u{200e}', '\u{200f}', '\u{2028}', '\u{2029}', '\u{202a}', '\u{202b}',
            '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ];
        let controls_text: String = controls.into_iter().collect();
        let text = format!("é{controls_text}字");
        let key = EndpointKey([0; 32]);
        let parsed: Comment = serde_json::from_value(json!({
            "id": "0f8fad5b-d9cb-469f-a165-70867728950e",
            "author": { "name": "reviewer", "key": key, "fingerprint": key.fingerprint() },
            "page": text,
            "selector": text,
            "selectedText": text,
            "comment": text,
            "choice": text,
            "attachments": [{
                "type": "file",
                "file": ResourceId::digest(b"attachment"),
                "name": text,
            }],
        }))
        .unwrap();

        // The author type rejects all these characters at admission.
        for character in controls {
            assert!(JoinerName::try_from(format!("reviewer{character}")).is_err());
        }

        let mut line = Vec::new();
        write_comment_line(&mut line, &parsed).unwrap();
        let line = String::from_utf8(line).unwrap();
        assert_eq!(line.lines().count(), 1);
        assert!(line.ends_with('\n'));
        for character in controls {
            assert!(!line.contains(character));
            // Five text fields preserve every character. Attachment names
            // already discard most bidi controls but preserve these three.
            let fields = if matches!(character, '\u{061c}' | '\u{2028}' | '\u{2029}') {
                6
            } else {
                5
            };
            assert_eq!(
                line.matches(&format!("\\u{:04x}", u32::from(character)))
                    .count(),
                fields
            );
        }
        assert_eq!(serde_json::from_str::<Comment>(&line).unwrap(), parsed);
    }
}
