use crate::{
    core::{ids::Seq, version::Version1},
    protocol::{
        ErrorCode, ProtocolError,
        control::{
            AccessUpdate, ControlRequest, ControlResponse, MAX_STATUS_LINE, Outcome, Reply,
            Request, StatusLine,
        },
    },
};

fn parse(line: &str) -> Result<ControlRequest, serde_json::Error> {
    serde_json::from_str(line)
}

#[test]
fn requests_parse_and_round_trip() {
    let lines = [
        r#"{"version":1,"id":1,"request":{"op":"create","label":"landing page","access":["alice","bob"],"expires_in_secs":259200,"client":{"kind":"t3","thread":"t"}}}"#,
        r#"{"version":1,"id":2,"request":{"op":"resume","seed":"aaaaaaaaaaaaaaaa"}}"#,
        r#"{"version":1,"id":3,"request":{"op":"update","seed":"aaaaaaaaaaaaaaaa","access":"everyone"}}"#,
        r#"{"version":1,"id":4,"request":{"op":"update","seed":"aaaaaaaaaaaaaaaa","access":{"only":["alice"]},"client":{"kind":"command","name":"x"}}}"#,
        r#"{"version":1,"id":5,"request":{"op":"end","seed":"aaaaaaaaaaaaaaaa"}}"#,
        r#"{"version":1,"id":6,"request":{"op":"set_status","seed":"aaaaaaaaaaaaaaaa","status":"agent unavailable: thread archived"}}"#,
        r#"{"version":1,"id":7,"request":{"op":"set_status","seed":"aaaaaaaaaaaaaaaa","status":null}}"#,
        r#"{"version":1,"id":8,"request":{"op":"answer","seed":"aaaaaaaaaaaaaaaa","message":"Done.","anchor":"0f8fad5b-d9cb-469f-a165-70867728950e"}}"#,
        r#"{"version":1,"id":9,"request":{"op":"status","seed":"aaaaaaaaaaaaaaaa"}}"#,
        r#"{"version":1,"id":10,"request":{"op":"list"}}"#,
        r#"{"version":1,"id":11,"request":{"op":"ack","seed":"aaaaaaaaaaaaaaaa","up_to":12}}"#,
    ];
    for line in lines {
        let request = parse(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(serde_json::to_string(&request).unwrap(), line);
    }
    let update = parse(lines.get(2).unwrap()).unwrap();
    assert!(matches!(
        update.request,
        Request::Update {
            access: Some(AccessUpdate::Everyone),
            ..
        }
    ));
    let ack = parse(lines.get(10).unwrap()).unwrap();
    assert!(matches!(ack.request, Request::Ack { up_to: Seq(12), .. }));
}

#[test]
fn invalid_requests_are_rejected() {
    for bad in [
        r#"{"version":2,"id":1,"request":{"op":"list"}}"#,
        r#"{"id":1,"request":{"op":"list"}}"#,
        r#"{"version":1,"id":1,"request":{"op":"list"},"extra":1}"#,
        r#"{"version":1,"id":1,"request":{"op":"nope"}}"#,
        r#"{"version":1,"id":1,"request":{"op":"end","seed":"aaaaaaaaaaaaaaaa","x":1}}"#,
        r#"{"version":1,"id":1,"request":{"op":"end","seed":"short"}}"#,
        r#"{"version":1,"id":1,"request":{"op":"create","label":"  "}}"#,
        r#"{"version":1,"id":1,"request":{"op":"create","label":"x","access":["not a login"]}}"#,
        r#"{"version":1,"id":1,"request":{"op":"answer","seed":"aaaaaaaaaaaaaaaa","message":""}}"#,
        r#"{"version":1,"id":1,"request":{"op":"answer","seed":"aaaaaaaaaaaaaaaa","message":"x","anchor":"main > h1"}}"#,
        r#"{"version":1,"id":1,"request":{"op":"set_status","seed":"aaaaaaaaaaaaaaaa","status":"a\nb"}}"#,
        r#"{"version":1,"id":-1,"request":{"op":"list"}}"#,
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
    let big = format!(
        r#"{{"version":1,"id":1,"request":{{"op":"create","label":"x","client":"{}"}}}}"#,
        "x".repeat(5000)
    );
    assert!(parse(&big).is_err());
    assert!(StatusLine::try_from("s".repeat(MAX_STATUS_LINE)).is_ok());
    assert!(StatusLine::try_from("s".repeat(MAX_STATUS_LINE + 1)).is_err());
}

#[test]
fn responses() {
    let done = ControlResponse {
        version: Version1,
        id: 5,
        outcome: Outcome::Ok(Reply::Done),
    };
    assert_eq!(
        serde_json::to_string(&done).unwrap(),
        r#"{"version":1,"id":5,"ok":{"kind":"done"}}"#
    );
    let acked = ControlResponse {
        version: Version1,
        id: 6,
        outcome: Outcome::Ok(Reply::Acked { removed: 3 }),
    };
    assert_eq!(
        serde_json::to_string(&acked).unwrap(),
        r#"{"version":1,"id":6,"ok":{"kind":"acked","removed":3}}"#
    );
    let error = ControlResponse {
        version: Version1,
        id: 7,
        outcome: Outcome::Error(ProtocolError {
            code: ErrorCode::NotFound,
            message: "no such session".to_owned(),
        }),
    };
    let line = serde_json::to_string(&error).unwrap();
    assert_eq!(
        line,
        r#"{"version":1,"id":7,"error":{"code":"not_found","message":"no such session"}}"#
    );
    assert_eq!(
        serde_json::from_str::<ControlResponse>(&line).unwrap(),
        error
    );

    let session = r#"{"version":1,"id":1,"ok":{"kind":"session","seed":"aaaaaaaaaaaaaaaa","label":"x","url":"https://jam.example/s/aaaaaaaaaaaaaaaa","created":"2026-11-12T16:00:00Z","expires":"2026-11-19T16:00:00Z","client":null}}"#;
    let parsed: ControlResponse = serde_json::from_str(session).unwrap();
    assert!(matches!(parsed.outcome, Outcome::Ok(Reply::Session(_))));
    assert_eq!(serde_json::to_string(&parsed).unwrap(), session);
}
