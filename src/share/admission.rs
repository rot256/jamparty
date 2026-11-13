//! Admission: the joiner's first control line must be a hello with the
//! party secret.

use crate::{
    Report,
    core::{invalid::Invalid, name::JoinerName, token::PartySecret},
    protocol::control::{Envelope, JoinerMessage, RefusedReason},
};

/// The QUIC close code for a refused joiner; the reason is the
/// [`RefusedReason`] in snake case.
pub(crate) const REFUSED_CODE: u32 = 1;

/// What the first line said: a message, a clean end, or why there was none
/// (garbage, or nothing before the deadline).
pub(crate) type FirstLine = Result<Option<Envelope<JoinerMessage>>, Report>;

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
                    jampot: _,
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
                message:
                    JoinerMessage::Comment { .. }
                    | JoinerMessage::Presence { .. }
                    | JoinerMessage::Cursor { .. },
            })
            | None,
        )
        | Err(_) => Err(RefusedReason::BadHello),
    }
}

/// The close reason sent with [`REFUSED_CODE`].
#[must_use]
pub(crate) fn reason_text(reason: RefusedReason) -> &'static str {
    match reason {
        RefusedReason::WrongSecret => "wrong_secret",
        RefusedReason::BadHello => "bad_hello",
        RefusedReason::Full => "full",
        RefusedReason::Incompatible => "incompatible",
    }
}

impl TryFrom<&[u8]> for RefusedReason {
    type Error = Invalid;

    fn try_from(text: &[u8]) -> Result<Self, Self::Error> {
        match text {
            b"wrong_secret" => Ok(Self::WrongSecret),
            b"bad_hello" => Ok(Self::BadHello),
            b"full" => Ok(Self::Full),
            b"incompatible" => Ok(Self::Incompatible),
            other => Err(Invalid::new(format!(
                "unknown refusal reason {:?}",
                String::from_utf8_lossy(other)
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use rootcause::prelude::report;

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
            jampot: None,
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
            page: "/".to_owned().try_into().unwrap(),
            selector: "body".to_owned().try_into().unwrap(),
            selected_text: None,
            comment: "hi".to_owned(),
        })
        .unwrap();
        let comment = JoinerMessage::Comment {
            reference: "1".to_owned().try_into().unwrap(),
            draft,
        };
        assert_eq!(
            admit(Ok(Some(comment.into())), &secret),
            Err(RefusedReason::BadHello)
        );
        assert_eq!(admit(Ok(None), &secret), Err(RefusedReason::BadHello));
        assert_eq!(
            admit(Err(report!("garbage")), &secret),
            Err(RefusedReason::BadHello)
        );
    }

    #[test]
    fn reasons_round_trip() {
        for reason in [
            RefusedReason::WrongSecret,
            RefusedReason::BadHello,
            RefusedReason::Full,
            RefusedReason::Incompatible,
        ] {
            assert_eq!(
                RefusedReason::try_from(reason_text(reason).as_bytes()),
                Ok(reason)
            );
        }
        assert!(RefusedReason::try_from(b"other".as_slice()).is_err());
    }
}
