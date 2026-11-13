//! The agent's answers: to everyone, or to one person here.

use crate::{
    core::{ids::CommentId, invalid::Invalid},
    protocol::{
        agent::{Addressee, KEY_PREFIX},
        comment::Author,
        control::{AnswerText, ShareMessage},
    },
    share::hub::{Audience, Hub, MAX_ANSWERS, Outgoing},
};

impl Hub {
    /// Sends an answer, to everyone or to one person: `to` (resolved among
    /// the people here) or, without `to`, the author of the `anchor`
    /// comment. `anchor` must be a comment this share has seen recently.
    /// Returns who it is for (none: everyone).
    pub(crate) fn answer(
        &self,
        message: AnswerText,
        anchor: Option<CommentId>,
        to: Option<Addressee>,
    ) -> Result<Option<Author>, Invalid> {
        let named = match &to {
            Some(person @ (Addressee::Named(_) | Addressee::Key(_))) => {
                Some(self.addressee(person)?)
            }
            Some(Addressee::Everyone) | None => None,
        };
        let (message, recipient) = {
            let mut inner = self.lock();
            let anchor = match anchor {
                None => None,
                Some(id) => Some(
                    inner
                        .anchors
                        .iter()
                        .find(|(anchor, _)| anchor.comment == id)
                        .cloned()
                        .ok_or_else(|| {
                            Invalid::new(format!("no comment with id {id} on this share"))
                        })?,
                ),
            };
            let recipient = match (to, anchor.as_ref()) {
                (Some(Addressee::Named(_) | Addressee::Key(_)), _) => named,
                (Some(Addressee::Everyone), _) | (None, None) => None,
                (None, Some((_, author))) => Some(author.clone()),
            };
            let message = ShareMessage::Answer {
                id: CommentId::generate(self.rng.as_ref()),
                message,
                anchor: anchor.map(|(anchor, _)| anchor),
                to: recipient.as_ref().map(|author| author.key),
            };
            if inner.answers.len() == MAX_ANSWERS {
                inner.answers.pop_front();
            }
            let audience = Audience::from(recipient.as_ref().map(|author| author.key));
            inner.answers.push_back(Outgoing {
                audience,
                message: message.clone(),
            });
            (Outgoing { audience, message }, recipient)
        };
        match &recipient {
            Some(author) => {
                tracing::info!(to = %author.name, fingerprint = %author.fingerprint, "answer sent");
            }
            None => tracing::info!("answer sent to everyone"),
        }
        let Outgoing { audience, message } = message;
        self.send(audience, message);
        Ok(recipient)
    }

    /// The one person here with that name (names only), or that key.
    fn addressee(&self, to: &Addressee) -> Result<Author, Invalid> {
        let here = self.presence();
        let listed = |people: &[&Author]| {
            people
                .iter()
                .map(|author| format!("{} ({KEY_PREFIX}{})", author.name, author.fingerprint))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let (found, wanted): (Vec<&Author>, String) = match to {
            Addressee::Named(name) => (
                here.iter().filter(|author| author.name == *name).collect(),
                format!("called {name}"),
            ),
            Addressee::Key(key) => (
                here.iter().filter(|author| key.matches(author)).collect(),
                format!("with {}", String::from(to.clone())),
            ),
            Addressee::Everyone => return Err(Invalid::new("everyone is not one person")),
        };
        match found.as_slice() {
            [author] => Ok((*author).clone()),
            [] => {
                let everyone: Vec<&Author> = here.iter().collect();
                Err(Invalid::new(if everyone.is_empty() {
                    format!("nobody {wanted} is here; nobody is connected")
                } else {
                    format!("nobody {wanted} is here; here: {}", listed(&everyone))
                }))
            }
            several => Err(Invalid::new(format!(
                "several people here are {wanted}: {}; pass --to {KEY_PREFIX}<fingerprint>",
                listed(several)
            ))),
        }
    }
}
