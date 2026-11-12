use jiff::{SignedDuration, Timestamp};

use crate::core::{
    allowlist::Allowlist,
    ids::{Seed, ShareKey, UserId},
    session::{
        ClientBlob, Creator, MAX_CLIENT_BYTES, Session, SessionError, SessionLabel, SessionLimits,
        SessionRequest,
    },
};

const OWNER: UserId = UserId(1);

fn allowlist() -> Allowlist {
    Allowlist::new([OWNER, UserId(2), UserId(3)], [OWNER]).unwrap()
}

fn now() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

fn request(access: Option<Vec<UserId>>, expires_in: Option<SignedDuration>) -> SessionRequest {
    SessionRequest {
        label: SessionLabel::try_from("review".to_owned()).unwrap(),
        access,
        expires_in,
        client: ClientBlob::default(),
    }
}

fn create(request: SessionRequest, owner: UserId) -> Result<Session, SessionError> {
    Session::create(
        request,
        "aaaaaaaaaaaaaaaa".parse::<Seed>().unwrap(),
        Creator {
            owner,
            key: ShareKey([1; 32]),
        },
        now(),
        SessionLimits::default(),
        &allowlist(),
    )
}

#[test]
fn defaults_and_maximum_expiry() {
    let session = create(request(None, None), OWNER).unwrap();
    assert_eq!(
        session.record().expires.duration_since(now()),
        SignedDuration::from_hours(7 * 24)
    );
    assert!(!session.is_expired(now()));
    let thirty = SignedDuration::from_hours(30 * 24);
    assert!(create(request(None, Some(thirty)), OWNER).is_ok());
    for bad in [
        thirty + SignedDuration::from_secs(1),
        SignedDuration::ZERO,
        SignedDuration::from_secs(-1),
    ] {
        assert_eq!(
            create(request(None, Some(bad)), OWNER),
            Err(SessionError::Expiry)
        );
    }
    let short = create(request(None, Some(SignedDuration::from_secs(10))), OWNER).unwrap();
    assert!(short.is_expired(now().checked_add(SignedDuration::from_secs(10)).unwrap()));
}

#[test]
fn access_rules() {
    assert_eq!(
        create(request(Some(vec![]), None), OWNER),
        Err(SessionError::EmptyAccess)
    );
    assert_eq!(
        create(request(Some(vec![UserId(2), UserId(9)]), None), OWNER),
        Err(SessionError::NotAllowlisted(UserId(9)))
    );
    assert_eq!(
        create(request(None, None), UserId(2)),
        Err(SessionError::NotSharer(UserId(2)))
    );

    let session = create(request(Some(vec![UserId(3), UserId(3)]), None), OWNER).unwrap();
    assert_eq!(session.record().access, Some(vec![UserId(3)]));
    let list = allowlist();
    assert!(session.may_view(OWNER, &list));
    assert!(session.may_view(UserId(3), &list));
    assert!(!session.may_view(UserId(2), &list));
    assert!(!session.may_view(UserId(9), &list));

    let open = create(request(None, None), OWNER).unwrap();
    assert!(open.may_view(UserId(2), &list));
    assert!(!open.may_view(UserId(9), &list));

    // Effective access stays within the allowlist when it shrinks.
    let shrunk = Allowlist::new([OWNER], [OWNER]).unwrap();
    assert!(!session.may_view(UserId(3), &shrunk));
    let gone = Allowlist::new([UserId(3)], []).unwrap();
    assert!(!session.may_view(OWNER, &gone));
}

#[test]
fn updates_are_validated() {
    let mut session = create(request(None, None), OWNER).unwrap();
    assert_eq!(
        session.set_access(Some(vec![]), &allowlist()),
        Err(SessionError::EmptyAccess)
    );
    session
        .set_access(Some(vec![UserId(2)]), &allowlist())
        .unwrap();
    assert!(!session.may_view(UserId(3), &allowlist()));
    session.set_access(None, &allowlist()).unwrap();
    assert!(session.may_view(UserId(3), &allowlist()));
}

#[test]
fn labels_and_client_blobs() {
    assert_eq!(
        SessionLabel::try_from("  x  ".to_owned()).unwrap().as_str(),
        "x"
    );
    assert!(SessionLabel::try_from("   ".to_owned()).is_err());
    assert!(SessionLabel::try_from("a\nb".to_owned()).is_err());
    assert!(SessionLabel::try_from("é".repeat(100)).is_ok());
    assert!(SessionLabel::try_from("é".repeat(101)).is_err());

    let fits = serde_json::Value::String("x".repeat(MAX_CLIENT_BYTES - 2));
    assert!(ClientBlob::try_from(fits).is_ok());
    let too_big = serde_json::Value::String("x".repeat(MAX_CLIENT_BYTES - 1));
    assert!(ClientBlob::try_from(too_big).is_err());
}

#[test]
fn limits_are_validated() {
    let day = SignedDuration::from_hours(24);
    assert!(SessionLimits::new(day, day).is_ok());
    assert!(SessionLimits::new(day + day, day).is_err());
    assert!(SessionLimits::new(SignedDuration::ZERO, day).is_err());
}
