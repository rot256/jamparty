use std::time::Duration;

use crate::{
    core::{ids::EndpointKey, rng::SeededRng, settings::Switch, token::PartySecret},
    protocol::{
        MAX_LINE_BYTES,
        agent::AgentRequest,
        comment::{Author, DraftFields, MAX_COMMENT, MAX_PAGE, MAX_SELECTED_TEXT, MAX_SELECTOR},
        control::{
            AckOutcome, Anchor, CommentRef, Envelope, Hello, JoinerMessage, MAX_COMMENT_REF,
            MAX_STATUS_LINE, RefusedReason, Revision, ShareMessage, SiteInfo, StatusLine,
        },
        decode_line, encode_line,
        presence::{MAX_CURSOR_SELECTOR, MAX_PAGE_PATH, Person, Support},
    },
};

const ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

fn secret() -> String {
    PartySecret::generate(&SeededRng::new(1)).to_string()
}

fn joiner(line: &str) -> Result<Envelope<JoinerMessage>, crate::protocol::FrameError> {
    decode_line(line.as_bytes())
}

#[test]
fn joiner_messages_round_trip() {
    let secret = secret();
    let lines = [
        format!(
            r#"{{"version":1,"message":{{"type":"hello","secret":"{secret}","name":"alice@laptop","jamparty":"0.1.0"}}}}"#
        ),
        r#"{"version":1,"message":{"type":"comment","ref":"7","draft":{"page":"/","selector":"main > h1","selectedText":"Hi","comment":"Bigger"}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"comment","ref":"a-B-9","draft":{"page":"/","selector":"","comment":"x"}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"presence","name":"alice","url":"/docs?x=1#y","cursor":"on"}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"presence","name":"alice","url":null,"cursor":"off"}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"cursor","position":{"selector":"main > h1","x":0,"y":10000}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"cursor","position":null}}"#.to_owned(),
    ];
    for line in &lines {
        let parsed = joiner(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(
            *encode_line(&parsed).unwrap(),
            format!("{line}\n").into_bytes()
        );
    }
    let hello = joiner(lines.first().unwrap()).unwrap();
    let JoinerMessage::Hello(Hello {
        secret: presented,
        name,
        jamparty,
    }) = hello.message
    else {
        panic!("not a hello");
    };
    assert!(presented.matches(&secret.parse().unwrap()));
    assert_eq!(name.as_str(), "alice@laptop");
    assert_eq!(jamparty.to_string(), "0.1.0");
    // A later jamparty of this protocol may add to its hello.
    let later = joiner(&format!(
        r#"{{"version":1,"message":{{"type":"hello","secret":"{secret}","name":"a","jamparty":"9.0.0","wants":["more"]}}}}"#
    ));
    assert!(
        matches!(
            later,
            Ok(Envelope {
                message: JoinerMessage::Hello(_),
                ..
            })
        ),
        "{later:?}"
    );
}

#[test]
fn invalid_joiner_messages_are_rejected() {
    let secret = secret();
    let hello = |body: &str| format!(r#"{{"version":1,"message":{{"type":"hello",{body}}}}}"#);
    let bad = [
        // Envelope.
        format!(r#"{{"version":2,"message":{{"type":"hello","secret":"{secret}","name":"a","jamparty":"0.1.0"}}}}"#),
        format!(r#"{{"message":{{"type":"hello","secret":"{secret}","name":"a","jamparty":"0.1.0"}}}}"#),
        format!(r#"{{"version":1,"message":{{"type":"hello","secret":"{secret}","name":"a","jamparty":"0.1.0"}},"x":1}}"#),
        r#"{"version":1,"message":{"type":"nope"}}"#.to_owned(),
        // Hello.
        hello(&format!(r#""secret":"{}","name":"a","jamparty":"0.1.0""#, secret.to_uppercase())),
        hello(r#""secret":"short","name":"a","jamparty":"0.1.0""#),
        hello(&format!(r#""secret":"{secret}","name":"a b","jamparty":"0.1.0""#)),
        hello(&format!(r#""secret":"{secret}","name":"","jamparty":"0.1.0""#)),
        hello(&format!(r#""secret":"{secret}","jamparty":"0.1.0""#)),
        hello(&format!(r#""secret":"{secret}","name":"a""#)),
        // Comments: authorship is the share's, caps hold.
        r#"{"version":1,"message":{"type":"comment","ref":"1","draft":{"page":"/","selector":"","comment":"x","author":{"name":"bob","fingerprint":"0000000000"}}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"comment","ref":"1","draft":{"page":"/","selector":"","comment":" "}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"comment","ref":1,"draft":{"page":"/","selector":"","comment":"x"}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"comment","ref":"","draft":{"page":"/","selector":"","comment":"x"}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"comment","ref":"a b","draft":{"page":"/","selector":"","comment":"x"}}}"#.to_owned(),
        format!(
            r#"{{"version":1,"message":{{"type":"comment","ref":"{}","draft":{{"page":"/","selector":"","comment":"x"}}}}}}"#,
            "r".repeat(MAX_COMMENT_REF + 1)
        ),
        format!(
            r#"{{"version":1,"message":{{"type":"comment","ref":"1","draft":{{"page":"/","selector":"","comment":"{}"}}}}}}"#,
            "x".repeat(MAX_COMMENT + 1)
        ),
        // Presence: names and pages are checked, nothing else is taken.
        r#"{"version":1,"message":{"type":"presence","name":"a b","cursor":"on"}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"presence","name":"a","url":"http://evil/","cursor":"on"}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"presence","name":"a","url":"/\"","cursor":"on"}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"presence","name":"a","cursor":"maybe"}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"presence","name":"a","cursor":"on","colour":1}}"#.to_owned(),
        // Cursors: an element and fractions of its box, nothing else.
        r#"{"version":1,"message":{"type":"cursor","position":{"selector":"h1","x":10001,"y":0}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"cursor","position":{"selector":"h1","x":-1,"y":0}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"cursor","position":{"selector":"","x":1,"y":0}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"cursor","position":{"selector":"h1","x":1,"y":0,"clientX":5}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"cursor","position":{"selector":"h1","x":0.5,"y":0}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"cursor","url":"/","position":null}}"#.to_owned(),
        format!(
            r#"{{"version":1,"message":{{"type":"cursor","position":{{"selector":"{}","x":1,"y":0}}}}}}"#,
            "s".repeat(MAX_CURSOR_SELECTOR + 1)
        ),
        format!(
            r#"{{"version":1,"message":{{"type":"presence","name":"a","url":"/{}","cursor":"on"}}}}"#,
            "p".repeat(MAX_PAGE_PATH)
        ),
    ];
    for line in bad {
        assert!(joiner(&line).is_err(), "{line}");
    }
}

#[test]
fn a_maximal_draft_fits_in_a_line() {
    // Control characters are escaped as \u00XX: the worst case per char.
    let escaped = |n: usize| "\u{1}".repeat(n);
    let draft = DraftFields {
        page: escaped(MAX_PAGE).try_into().unwrap(),
        selector: escaped(MAX_SELECTOR).try_into().unwrap(),
        choice: None,
        selected_text: Some(escaped(MAX_SELECTED_TEXT)),
        comment: format!("x{}", escaped(MAX_COMMENT - 1)),
    };
    let message = Envelope::from(JoinerMessage::Comment {
        reference: "r".repeat(MAX_COMMENT_REF).try_into().unwrap(),
        draft: draft.try_into().unwrap(),
    });
    let line = encode_line(&message).unwrap();
    assert!(line.len() < MAX_LINE_BYTES / 2, "{}", line.len());
    assert!(decode_line::<Envelope<JoinerMessage>>(&line).is_ok());
}

#[test]
fn share_messages() {
    let author = Author::new(
        "alice@laptop".to_owned().try_into().unwrap(),
        EndpointKey([0xab; 32]),
    );
    let key = "ab".repeat(32);
    let reference = |raw: &str| CommentRef::try_from(raw.to_owned()).unwrap();
    let cases = [
        (
            ShareMessage::Admitted { you: author.clone(), undelivered: vec![ID.parse().unwrap()], presence: Support::Supported, site: SiteInfo::Proxy { port: 1313.try_into().unwrap() }, jamparty: "0.1.0".to_owned().try_into().unwrap() },
            format!(r#"{{"type":"admitted","you":{{"name":"alice@laptop","fingerprint":"ababababab","key":"{key}"}},"undelivered":["{ID}"],"presence":"supported","site":{{"kind":"proxy","port":1313}},"jamparty":"0.1.0"}}"#),
        ),
        (
            ShareMessage::Admitted { you: author.clone(), undelivered: vec![], presence: Support::Supported, site: SiteInfo::Files, jamparty: "0.1.0".to_owned().try_into().unwrap() },
            format!(r#"{{"type":"admitted","you":{{"name":"alice@laptop","fingerprint":"ababababab","key":"{key}"}},"undelivered":[],"presence":"supported","site":{{"kind":"files"}},"jamparty":"0.1.0"}}"#),
        ),
        (
            ShareMessage::Refused { reason: RefusedReason::WrongSecret, jamparty: None },
            r#"{"type":"refused","reason":"wrong_secret"}"#.to_owned(),
        ),
        (
            ShareMessage::Refused { reason: RefusedReason::Full, jamparty: None },
            r#"{"type":"refused","reason":"full"}"#.to_owned(),
        ),
        (
            ShareMessage::Refused { reason: RefusedReason::Incompatible, jamparty: Some("0.2.0".to_owned().try_into().unwrap()) },
            r#"{"type":"refused","reason":"incompatible","jamparty":"0.2.0"}"#.to_owned(),
        ),
        (
            ShareMessage::CommentAck { reference: reference("7"), outcome: AckOutcome::Queued { id: ID.parse().unwrap() } },
            format!(r#"{{"type":"comment_ack","ref":"7","outcome":{{"result":"queued","id":"{ID}"}}}}"#),
        ),
        (
            ShareMessage::CommentAck { reference: reference("8"), outcome: AckOutcome::RateLimited { retry_after: Duration::from_secs(6) } },
            r#"{"type":"comment_ack","ref":"8","outcome":{"result":"rate_limited","retry_after_secs":6}}"#.to_owned(),
        ),
        (
            ShareMessage::CommentAck { reference: reference("9"), outcome: AckOutcome::QueueFull },
            r#"{"type":"comment_ack","ref":"9","outcome":{"result":"queue_full"}}"#.to_owned(),
        ),
        (
            ShareMessage::Answer { choices: Default::default(),
                id: ID.parse().unwrap(),
                message: "Done.".to_owned().try_into().unwrap(),
                anchor: Some(Anchor { comment: ID.parse().unwrap(), page: "/".to_owned().try_into().unwrap(), selector: "h1".to_owned().try_into().unwrap() }),
                to: None,
            },
            format!(r#"{{"type":"answer","id":"{ID}","message":"Done.","anchor":{{"comment":"{ID}","page":"/","selector":"h1"}}}}"#),
        ),
        (
            ShareMessage::Answer { choices: Default::default(), id: ID.parse().unwrap(), message: "Hi".to_owned().try_into().unwrap(), anchor: None, to: None },
            format!(r#"{{"type":"answer","id":"{ID}","message":"Hi"}}"#),
        ),
        (
            ShareMessage::Answer { choices: Default::default(), id: ID.parse().unwrap(), message: "Hi".to_owned().try_into().unwrap(), anchor: None, to: Some(EndpointKey([0xab; 32])) },
            format!(r#"{{"type":"answer","id":"{ID}","message":"Hi","to":"{key}"}}"#),
        ),
        (
            ShareMessage::Delivered { ids: vec![ID.parse().unwrap()] },
            format!(r#"{{"type":"delivered","ids":["{ID}"]}}"#),
        ),
        (
            ShareMessage::Reload { revision: Revision::from(3) },
            r#"{"type":"reload","revision":3}"#.to_owned(),
        ),
        (
            ShareMessage::Status { line: Some("agent unavailable".to_owned().try_into().unwrap()) },
            r#"{"type":"status","line":"agent unavailable"}"#.to_owned(),
        ),
        (ShareMessage::Status { line: None }, r#"{"type":"status","line":null}"#.to_owned()),
        (
            ShareMessage::Cursor {
                key: EndpointKey([0xab; 32]),
                url: "/a".to_owned().try_into().unwrap(),
                position: Some(serde_json::from_str(r#"{"selector":"h1","x":1,"y":2}"#).unwrap()),
            },
            format!(r#"{{"type":"cursor","key":"{key}","url":"/a","position":{{"selector":"h1","x":1,"y":2}}}}"#),
        ),
        (
            ShareMessage::Presence {
                joiners: vec![author.clone()],
                here: vec![Person::new(author.clone(), Some("/a?b#c".to_owned().try_into().unwrap()), Switch::On)],
            },
            format!(r#"{{"type":"presence","joiners":[{{"name":"alice@laptop","fingerprint":"ababababab","key":"{key}"}}],"here":[{{"name":"alice@laptop","fingerprint":"ababababab","key":"{key}","colour":3,"url":"/a?b#c","cursor":"on"}}]}}"#),
        ),
    ];
    for (message, json) in cases {
        let line = format!(r#"{{"version":1,"message":{json}}}"#);
        assert_eq!(
            *encode_line(&Envelope::from(message.clone())).unwrap(),
            format!("{line}\n").into_bytes()
        );
        let parsed: Envelope<ShareMessage> = decode_line(line.as_bytes()).unwrap();
        assert_eq!(parsed.message, message);
    }
    // Every field a share writes is required: no build leaves one out.
    let partial = format!(
        r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"a","fingerprint":"ababababab","key":"{key}"}}}}}}"#
    );
    assert!(decode_line::<Envelope<ShareMessage>>(partial.as_bytes()).is_err());
    let partial = r#"{"version":1,"message":{"type":"presence","joiners":[]}}"#;
    assert!(decode_line::<Envelope<ShareMessage>>(partial.as_bytes()).is_err());
    // A newer share may add fields.
    let extra = r#"{"version":1,"message":{"type":"reload","revision":3,"at":"now"}}"#;
    assert!(decode_line::<Envelope<ShareMessage>>(extra.as_bytes()).is_ok());
}

#[test]
fn text_limits() {
    assert!(StatusLine::try_from("s".repeat(MAX_STATUS_LINE)).is_ok());
    assert!(StatusLine::try_from("s".repeat(MAX_STATUS_LINE + 1)).is_err());
    assert!(StatusLine::try_from("a\nb".to_owned()).is_err());
    let answer =
        format!(r#"{{"version":1,"message":{{"type":"answer","id":"{ID}","message":""}}}}"#);
    assert!(decode_line::<Envelope<ShareMessage>>(answer.as_bytes()).is_err());
}

#[test]
fn site_announcements() {
    let admitted = |site: &str| {
        let line = format!(
            r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"a","fingerprint":"ababababab","key":"{}"}},"undelivered":[],"presence":"supported","site":{site},"jamparty":"0.1.0"}}}}"#,
            "ab".repeat(32)
        );
        decode_line::<Envelope<ShareMessage>>(line.as_bytes()).map(|envelope| {
            let ShareMessage::Admitted { site, .. } = envelope.message else {
                panic!("not admitted: {:?}", envelope.message);
            };
            site
        })
    };
    assert_eq!(
        admitted(r#"{"kind":"proxy","port":80,"tls":true}"#).unwrap(),
        SiteInfo::Proxy {
            port: 80.try_into().unwrap()
        }
    );
    assert_eq!(admitted(r#"{"kind":"files"}"#).unwrap(), SiteInfo::Files);
    // A kind from a newer share is not this joiner's to mirror.
    assert_eq!(
        admitted(r#"{"kind":"tunnel","port":22}"#).unwrap(),
        SiteInfo::Other
    );
    for bad in [
        r#"{"kind":"proxy"}"#,
        r#"{"kind":"proxy","port":0}"#,
        r#"{"kind":"proxy","port":65536}"#,
        r#"{"kind":"proxy","port":"1313"}"#,
    ] {
        assert!(admitted(bad).is_err(), "{bad}");
    }
}

#[test]
fn choices_are_capped_on_both_sides() {
    use crate::protocol::control::{Choices, MAX_CHOICE, MAX_CHOICES};
    let valid = vec!["Keep it".to_owned(), "Make it bigger".to_owned()];
    assert!(Choices::try_from(valid.clone()).is_ok());
    for labels in [
        vec!["Only one".to_owned()],
        vec!["x".to_owned(); MAX_CHOICES + 1],
        vec!["x".repeat(MAX_CHOICE + 1), "y".to_owned()],
        vec!["x\ny".to_owned(), "y".to_owned()],
        vec![" ".to_owned(), "y".to_owned()],
        vec!["x".to_owned(), "x".to_owned()],
    ] {
        for message in [
            serde_json::json!({"type":"answer", "id": ID, "message":"Size?", "choices":labels}),
            serde_json::json!({"type":"send", "message":"Size?", "choices":labels}),
        ] {
            if message.get("type").and_then(serde_json::Value::as_str) == Some("answer") {
                assert!(serde_json::from_value::<ShareMessage>(message).is_err());
            } else {
                assert!(serde_json::from_value::<AgentRequest>(message).is_err());
            }
        }
    }
}
