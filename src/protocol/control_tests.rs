use crate::{
    core::{ids::EndpointKey, rng::SeededRng, settings::Switch, token::PartySecret},
    protocol::{
        MAX_LINE_BYTES,
        comment::{Author, DraftFields, MAX_COMMENT, MAX_PAGE, MAX_SELECTED_TEXT, MAX_SELECTOR},
        control::{
            AckOutcome, Anchor, CommentRef, Envelope, JoinerMessage, MAX_COMMENT_REF,
            MAX_STATUS_LINE, RefusedReason, ShareMessage, StatusLine,
        },
        decode_line, encode_line,
        presence::{MAX_PAGE_PATH, Person, Support},
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
            r#"{{"version":1,"message":{{"type":"hello","secret":"{secret}","name":"alice@laptop"}}}}"#
        ),
        r#"{"version":1,"message":{"type":"comment","ref":"7","draft":{"page":"/","selector":"main > h1","selectedText":"Hi","comment":"Bigger"}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"comment","ref":"a-B-9","draft":{"page":"/","selector":"","comment":"x"}}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"presence","name":"alice","url":"/docs?x=1#y","cursor":"on"}}"#.to_owned(),
        r#"{"version":1,"message":{"type":"presence","name":"alice","url":null,"cursor":"off"}}"#.to_owned(),
    ];
    for line in &lines {
        let parsed = joiner(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(
            *encode_line(&parsed).unwrap(),
            format!("{line}\n").into_bytes()
        );
    }
    let hello = joiner(lines.first().unwrap()).unwrap();
    let JoinerMessage::Hello {
        secret: presented,
        name,
    } = hello.message
    else {
        panic!("not a hello");
    };
    assert!(presented.matches(&secret.parse().unwrap()));
    assert_eq!(name.as_str(), "alice@laptop");
}

#[test]
fn invalid_joiner_messages_are_rejected() {
    let secret = secret();
    let hello = |body: &str| format!(r#"{{"version":1,"message":{{"type":"hello",{body}}}}}"#);
    let bad = [
        // Envelope.
        format!(r#"{{"version":2,"message":{{"type":"hello","secret":"{secret}","name":"a"}}}}"#),
        format!(r#"{{"message":{{"type":"hello","secret":"{secret}","name":"a"}}}}"#),
        format!(r#"{{"version":1,"message":{{"type":"hello","secret":"{secret}","name":"a"}},"x":1}}"#),
        r#"{"version":1,"message":{"type":"nope"}}"#.to_owned(),
        // Hello.
        hello(&format!(r#""secret":"{secret}","name":"a","admin":true"#)),
        hello(&format!(r#""secret":"{}","name":"a""#, secret.to_uppercase())),
        hello(r#""secret":"short","name":"a""#),
        hello(&format!(r#""secret":"{secret}","name":"a b""#)),
        hello(&format!(r#""secret":"{secret}","name":"""#)),
        hello(&format!(r#""secret":"{secret}""#)),
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
        page: escaped(MAX_PAGE),
        selector: escaped(MAX_SELECTOR),
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
            ShareMessage::Admitted { you: author.clone(), undelivered: vec![ID.parse().unwrap()], presence: Support::Supported },
            format!(r#"{{"type":"admitted","you":{{"name":"alice@laptop","fingerprint":"ababababab","key":"{key}"}},"undelivered":["{ID}"],"presence":"supported"}}"#),
        ),
        (
            ShareMessage::Refused { reason: RefusedReason::WrongSecret },
            r#"{"type":"refused","reason":"wrong_secret"}"#.to_owned(),
        ),
        (
            ShareMessage::Refused { reason: RefusedReason::Full },
            r#"{"type":"refused","reason":"full"}"#.to_owned(),
        ),
        (
            ShareMessage::CommentAck { reference: reference("7"), outcome: AckOutcome::Queued { id: ID.parse().unwrap() } },
            format!(r#"{{"type":"comment_ack","ref":"7","outcome":{{"result":"queued","id":"{ID}"}}}}"#),
        ),
        (
            ShareMessage::CommentAck { reference: reference("8"), outcome: AckOutcome::RateLimited { retry_after_secs: 6 } },
            r#"{"type":"comment_ack","ref":"8","outcome":{"result":"rate_limited","retry_after_secs":6}}"#.to_owned(),
        ),
        (
            ShareMessage::CommentAck { reference: reference("9"), outcome: AckOutcome::QueueFull },
            r#"{"type":"comment_ack","ref":"9","outcome":{"result":"queue_full"}}"#.to_owned(),
        ),
        (
            ShareMessage::Answer {
                id: ID.parse().unwrap(),
                message: "Done.".to_owned().try_into().unwrap(),
                anchor: Some(Anchor { comment: ID.parse().unwrap(), page: "/".to_owned(), selector: "h1".to_owned() }),
            },
            format!(r#"{{"type":"answer","id":"{ID}","message":"Done.","anchor":{{"comment":"{ID}","page":"/","selector":"h1"}}}}"#),
        ),
        (
            ShareMessage::Answer { id: ID.parse().unwrap(), message: "Hi".to_owned().try_into().unwrap(), anchor: None },
            format!(r#"{{"type":"answer","id":"{ID}","message":"Hi"}}"#),
        ),
        (
            ShareMessage::Delivered { ids: vec![ID.parse().unwrap()] },
            format!(r#"{{"type":"delivered","ids":["{ID}"]}}"#),
        ),
        (
            ShareMessage::Reload { revision: 3 },
            r#"{"type":"reload","revision":3}"#.to_owned(),
        ),
        (
            ShareMessage::Status { line: Some("agent unavailable".to_owned().try_into().unwrap()) },
            r#"{"type":"status","line":"agent unavailable"}"#.to_owned(),
        ),
        (ShareMessage::Status { line: None }, r#"{"type":"status","line":null}"#.to_owned()),
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
    // What an older share sends still parses: no presence support, no
    // `here`.
    let older = format!(
        r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"a","fingerprint":"ababababab","key":"{key}"}}}}}}"#
    );
    let Envelope {
        message: ShareMessage::Admitted { presence, .. },
        ..
    } = decode_line::<Envelope<ShareMessage>>(older.as_bytes()).unwrap()
    else {
        panic!("not admitted");
    };
    assert_eq!(presence, Support::Unsupported);
    let older = r#"{"version":1,"message":{"type":"presence","joiners":[]}}"#;
    assert!(decode_line::<Envelope<ShareMessage>>(older.as_bytes()).is_ok());
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
