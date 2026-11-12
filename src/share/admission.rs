//! Admission: the joiner's first control line must be a hello with the
//! party secret.

use crate::{
    core::{name::JoinerName, token::PartySecret},
    protocol::control::{Envelope, JoinerMessage, RefusedReason},
};

/// The QUIC close code for a refused joiner; the reason is the
/// [`RefusedReason`] in snake case.
pub(crate) const REFUSED_CODE: u32 = 1;

/// What the first line said: a message, a clean end, or garbage.
pub(crate) type FirstLine = Result<Option<Envelope<JoinerMessage>>, ()>;

/// Admits the joiner (its display name) or says why not. The secret is
/// compared in constant time.
pub(crate) fn admit(first: FirstLine, secret: &PartySecret) -> Result<JoinerName, RefusedReason> {
    match first {
        Ok(Some(Envelope {
            version: _,
            message:
                JoinerMessage::Hello {
                    secret: presented,
                    name,
                },
        })) => {
            if secret.matches(&presented) {
                Ok(name)
            } else {
                Err(RefusedReason::WrongSecret)
            }
        }
        Ok(
            Some(Envelope {
                version: _,
                message: JoinerMessage::Comment { .. },
            })
            | None,
        )
        | Err(()) => Err(RefusedReason::BadHello),
    }
}

/// The close reason sent with [`REFUSED_CODE`].
#[must_use]
pub(crate) fn reason_text(reason: RefusedReason) -> &'static str {
    match reason {
        RefusedReason::WrongSecret => "wrong_secret",
        RefusedReason::BadHello => "bad_hello",
    }
}

impl TryFrom<&[u8]> for RefusedReason {
    type Error = ();

    fn try_from(text: &[u8]) -> Result<Self, Self::Error> {
        match text {
            b"wrong_secret" => Ok(Self::WrongSecret),
            b"bad_hello" => Ok(Self::BadHello),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::{rng::SeededRng, token::PartySecret},
        protocol::{
            comment::{Draft, DraftFields},
            control::{Envelope, JoinerMessage, RefusedReason},
        },
        share::admission::{admit, reason_text},
    };

    fn hello(secret: &PartySecret) -> Envelope<JoinerMessage> {
        JoinerMessage::Hello {
            secret: secret.clone(),
            name: "alice".parse().unwrap(),
        }
        .into()
    }

    #[test]
    fn only_the_right_hello_admits() {
        let secret = PartySecret::generate(&SeededRng::new(1));
        let other = PartySecret::generate(&SeededRng::new(2));
        assert_eq!(
            admit(Ok(Some(hello(&secret))), &secret).unwrap().as_str(),
            "alice"
        );
        assert_eq!(
            admit(Ok(Some(hello(&other))), &secret),
            Err(RefusedReason::WrongSecret)
        );
        let draft = Draft::try_from(DraftFields {
            page: "/".to_owned(),
            selector: "body".to_owned(),
            selected_text: None,
            comment: "hi".to_owned(),
        })
        .unwrap();
        let comment = JoinerMessage::Comment {
            reference: 1,
            draft,
        };
        assert_eq!(
            admit(Ok(Some(comment.into())), &secret),
            Err(RefusedReason::BadHello)
        );
        assert_eq!(admit(Ok(None), &secret), Err(RefusedReason::BadHello));
        assert_eq!(admit(Err(()), &secret), Err(RefusedReason::BadHello));
    }

    #[test]
    fn reasons_round_trip() {
        for reason in [RefusedReason::WrongSecret, RefusedReason::BadHello] {
            assert_eq!(
                RefusedReason::try_from(reason_text(reason).as_bytes()),
                Ok(reason)
            );
        }
        assert!(RefusedReason::try_from(b"other".as_slice()).is_err());
    }
}
