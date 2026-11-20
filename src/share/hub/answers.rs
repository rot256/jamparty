//! The agent's answers: to everyone, or to one person here.

use crate::{
    core::{ids::CommentId, invalid::Invalid},
    protocol::{
        agent::{Addressee, KEY_PREFIX},
        answer::AnswerAnchor,
        comment::Author,
        control::{AnswerText, Choices, ShareMessage},
    },
    share::hub::{Addressed, Audience, Hub, MAX_ANSWERS},
};

impl Hub {
    /// Sends an answer, to everyone or to one person: `to` (resolved among
    /// the people here) or, for an answer to a comment, its author.
    /// Comment anchors must refer to a comment this share has seen recently.
    /// Returns who it is for (none: everyone).
    pub(crate) fn answer(
        &self,
        message: AnswerText,
        anchor: Option<AnswerAnchor<CommentId>>,
        to: Option<Addressee>,
        choices: Choices,
    ) -> Result<Option<Author>, Invalid> {
        let named = match &to {
            Some(person @ (Addressee::Named(_) | Addressee::Key(_))) => {
                Some(self.addressee(person)?)
            }
            Some(Addressee::Everyone) | None => None,
        };
        let recipient = {
            let mut inner = self.lock();
            let (anchor, author) = match anchor {
                None => (None, None),
                Some(AnswerAnchor::Place { place }) => (Some(AnswerAnchor::Place { place }), None),
                Some(AnswerAnchor::Comment { comment: id }) => {
                    let (comment, author) = inner
                        .anchors
                        .iter()
                        .find(|(anchor, _)| anchor.comment == id)
                        .cloned()
                        .ok_or_else(|| {
                            Invalid::new(format!("no comment with id {id} on this share"))
                        })?;
                    (Some(AnswerAnchor::Comment { comment }), Some(author))
                }
            };
            let recipient = match to {
                Some(Addressee::Named(_) | Addressee::Key(_)) => named,
                Some(Addressee::Everyone) => None,
                None => author,
            };
            let message = ShareMessage::Answer {
                id: CommentId::generate(self.rng.as_ref()),
                message,
                choices,
                anchor,
                to: recipient.as_ref().map(|author| author.key),
            };
            if inner.answers.len() == MAX_ANSWERS {
                inner.answers.pop_front();
            }
            let audience = Audience::from(recipient.as_ref().map(|author| author.key));
            inner.answers.push_back(Addressed {
                audience,
                message: message.clone(),
            });
            self.send(audience, message);
            recipient
        };
        match &recipient {
            Some(author) => {
                tracing::info!(to = %author.name, fingerprint = %author.fingerprint, "answer sent");
            }
            None => tracing::info!("answer sent to everyone"),
        }
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
