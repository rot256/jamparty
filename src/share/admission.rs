//! Admission: the joiner's first control line must be a hello with the
//! party secret.

use crate::{
    Report,
    core::{name::JoinerName, token::PartySecret},
    protocol::control::{Envelope, Hello, JoinerMessage, RefusedReason},
};

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
                JoinerMessage::Hello(Hello {
                    secret: presented,
                    name,
                    jamparty: _,
                }),
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

#[cfg(test)]
mod tests {
    use rootcause::prelude::report;

    use crate::{
        core::version::JampartyVersion,
        core::{rng::SeededRng, token::PartySecret},
        protocol::{
            comment::{Draft, DraftFields},
            control::{Envelope, Hello, JoinerMessage, RefusedReason},
        },
        share::admission::admit,
    };

    fn hello(secret: &PartySecret) -> Envelope<JoinerMessage> {
        JoinerMessage::Hello(Hello {
            secret: secret.clone(),
            name: "alice".parse().unwrap(),
            jamparty: JampartyVersion::current(),
        })
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
            choice: None,
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
                RefusedReason::try_from(reason.as_str().as_bytes()),
                Ok(reason)
            );
            // The text form is the wire form.
            assert_eq!(
                serde_json::to_value(reason).unwrap(),
                serde_json::json!(reason.as_str())
            );
        }
        assert!(RefusedReason::try_from(b"other".as_slice()).is_err());
    }
}
