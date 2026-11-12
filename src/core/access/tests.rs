//! Fixture and targeted tests for [`crate::core::access::decide`]; the full
//! matrix is in `matrix_tests.rs`.

use std::collections::BTreeMap;

use jiff::{SignedDuration, Timestamp};

use crate::core::{
    access::{
        AccessContext, Decision, DoorRoute, Method, PublicRoute, Refusal, RequestFacts,
        SessionRoute, Upgrade, decide,
    },
    allowlist::Allowlist,
    cookie::{CookieScope, VerifiedCookie},
    host::{HostConfig, HostKind, Scheme},
    ids::{Seed, ShareKey, UserId},
    safe_path::SafePath,
    session::{ClientBlob, Session, SessionLabel, SessionRecord},
};

pub(super) const A: &str = "aaaaaaaaaaaaaaaa";
pub(super) const B: &str = "bbbbbbbbbbbbbbbb";
pub(super) const UNKNOWN: &str = "uuuuuuuuuuuuuuuu";
pub(super) const OWNER: UserId = UserId(1);
pub(super) const ALLOWED: UserId = UserId(2);
pub(super) const NO_ACCESS: UserId = UserId(3);
pub(super) const OWNER_B: UserId = UserId(4);
pub(super) const ALLOWED_B: UserId = UserId(5);
pub(super) const OUTSIDER: UserId = UserId(9);

pub(super) fn seed(raw: &str) -> Seed {
    raw.parse().unwrap()
}

pub(super) fn now() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

fn session(raw: &str, owner: UserId, access: Option<Vec<UserId>>) -> Session {
    Session::try_from(SessionRecord {
        seed: seed(raw),
        label: SessionLabel::try_from("review".to_owned()).unwrap(),
        owner,
        key: ShareKey([1; 32]),
        created: now(),
        expires: now().checked_add(SignedDuration::from_hours(24)).unwrap(),
        access,
        client: ClientBlob::default(),
    })
    .unwrap()
}

pub(super) struct Fixture {
    pub(super) hosts: HostConfig,
    pub(super) allowlist: Allowlist,
    pub(super) sessions: BTreeMap<Seed, Session>,
    pub(super) now: Timestamp,
}

pub(super) fn fixture() -> Fixture {
    let sessions = [
        session(A, OWNER, Some(vec![ALLOWED])),
        session(B, OWNER_B, Some(vec![ALLOWED_B])),
    ]
    .into_iter()
    .map(|session| (session.seed(), session))
    .collect();
    Fixture {
        hosts: HostConfig::new(Scheme::Https, "jam.example", None).unwrap(),
        allowlist: Allowlist::new(
            [OWNER, ALLOWED, NO_ACCESS, OWNER_B, ALLOWED_B],
            [OWNER, OWNER_B],
        )
        .unwrap(),
        sessions,
        now: now(),
    }
}

impl Fixture {
    pub(super) fn decide(&self, facts: &RequestFacts<'_>) -> Decision {
        decide(
            facts,
            &AccessContext {
                hosts: &self.hosts,
                allowlist: &self.allowlist,
                sessions: &self.sessions,
                now: self.now,
            },
        )
    }
}

pub(super) fn cookie(user: UserId, scope: CookieScope) -> Option<VerifiedCookie> {
    Some(VerifiedCookie {
        user,
        scope,
        expires: now().checked_add(SignedDuration::from_hours(1)).unwrap(),
    })
}

/// A same-origin top-level GET navigation.
pub(super) fn nav(host: HostKind, path: &'static str) -> RequestFacts<'static> {
    RequestFacts {
        host,
        method: Method::Get,
        path,
        query: None,
        sec_fetch_site: Some("same-origin"),
        sec_fetch_mode: Some("navigate"),
        sec_fetch_dest: Some("document"),
        origin: None,
        upgrade: Upgrade::No,
        cookie: None,
    }
}

/// A same-origin `fetch` with `method`, carrying `origin`.
pub(super) fn fetch(
    host: HostKind,
    method: Method,
    path: &'static str,
    origin: Option<&'static str>,
) -> RequestFacts<'static> {
    RequestFacts {
        host,
        method,
        path,
        query: None,
        sec_fetch_site: Some("same-origin"),
        sec_fetch_mode: Some("cors"),
        sec_fetch_dest: Some("empty"),
        origin,
        upgrade: Upgrade::No,
        cookie: None,
    }
}

pub(super) const DOOR_ORIGIN: &str = "https://jam.example";
pub(super) const A_ORIGIN: &str = "https://aaaaaaaaaaaaaaaa.jam.example";
pub(super) const B_ORIGIN: &str = "https://bbbbbbbbbbbbbbbb.jam.example";

pub(super) fn on_a() -> HostKind {
    HostKind::Session(seed(A))
}

pub(super) fn a_cookie(user: UserId) -> Option<VerifiedCookie> {
    cookie(user, CookieScope::Session(seed(A)))
}

#[test]
fn allowed_requests_carry_route_and_access() {
    let f = fixture();
    let facts = RequestFacts {
        cookie: a_cookie(ALLOWED),
        ..fetch(
            on_a(),
            Method::Post,
            "/_jamjam/api/comments",
            Some(A_ORIGIN),
        )
    };
    let decision = f.decide(&facts);
    assert_eq!(
        decision,
        Decision::Session {
            route: SessionRoute::Comments,
            access: crate::core::access::Access {
                user: ALLOWED,
                seed: seed(A)
            },
        }
    );
    assert!(decision.route().is_some());

    let facts = RequestFacts {
        cookie: cookie(OWNER, CookieScope::Door),
        query: Some("next=%2Fdocs%3Fp%3D2"),
        ..nav(HostKind::FrontDoor, "/s/aaaaaaaaaaaaaaaa")
    };
    assert_eq!(
        f.decide(&facts),
        Decision::Door {
            route: DoorRoute::Entry {
                seed: seed(A),
                next: str::parse::<SafePath>("/docs?p=2").unwrap()
            },
            user: OWNER,
        }
    );
}

#[test]
fn redirects_point_at_sign_in_and_entry() {
    let f = fixture();
    let facts = RequestFacts {
        query: Some("next=/x"),
        ..nav(HostKind::FrontDoor, "/s/aaaaaaaaaaaaaaaa")
    };
    assert_eq!(
        f.decide(&facts),
        Decision::Refuse(Refusal::Redirect(
            "https://jam.example/auth/login?next=%2Fs%2Faaaaaaaaaaaaaaaa%3Fnext%3D%2Fx".to_owned()
        ))
    );
    let facts = RequestFacts {
        query: Some("q=1"),
        ..nav(on_a(), "/docs/page")
    };
    assert_eq!(
        f.decide(&facts),
        Decision::Refuse(Refusal::Redirect(
            "https://jam.example/s/aaaaaaaaaaaaaaaa?next=%2Fdocs%2Fpage%3Fq%3D1".to_owned()
        ))
    );
    // A path that is not a safe `next` is dropped rather than echoed.
    let facts = nav(on_a(), "//evil.example/x");
    assert_eq!(
        f.decide(&facts),
        Decision::Refuse(Refusal::Redirect(
            "https://jam.example/s/aaaaaaaaaaaaaaaa".to_owned()
        ))
    );
}

#[test]
fn cookies_count_only_for_their_own_host() {
    let f = fixture();
    // A's cookie on B, the door cookie on A, and A's cookie on the door.
    let a_on_b = RequestFacts {
        cookie: a_cookie(ALLOWED_B),
        ..fetch(
            HostKind::Session(seed(B)),
            Method::Get,
            "/_jamjam/api/status",
            None,
        )
    };
    assert_eq!(f.decide(&a_on_b), Decision::Refuse(Refusal::Unauthorized));
    let door_on_a = RequestFacts {
        cookie: cookie(ALLOWED, CookieScope::Door),
        ..fetch(on_a(), Method::Get, "/_jamjam/api/status", None)
    };
    assert_eq!(
        f.decide(&door_on_a),
        Decision::Refuse(Refusal::Unauthorized)
    );
    let a_on_door = RequestFacts {
        cookie: a_cookie(ALLOWED),
        ..fetch(
            HostKind::FrontDoor,
            Method::Post,
            "/auth/logout",
            Some(DOOR_ORIGIN),
        )
    };
    assert_eq!(
        f.decide(&a_on_door),
        Decision::Refuse(Refusal::Unauthorized)
    );
}

#[test]
fn reserved_path_variants_are_protected() {
    let f = fixture();
    for path in [
        "//_jamjam/enter",
        "/%5Fjamjam/enter",
        "/_JAMJAM/enter",
        "/_jamjam/enter/",
        "/_jamjam/api/comments/",
        "/_jamjam/client.js",
        "/anything",
    ] {
        let facts = fetch(on_a(), Method::Get, path, None);
        assert_eq!(
            f.decide(&facts),
            Decision::Refuse(Refusal::Unauthorized),
            "{path}"
        );
        let facts = nav(on_a(), path);
        assert!(
            matches!(f.decide(&facts), Decision::Refuse(Refusal::Redirect(_))),
            "{path}"
        );
    }
    for path in [
        "/auth/login/",
        "/AUTH/login",
        "/.well-known/jamjam/",
        "//auth/callback",
    ] {
        assert_eq!(
            f.decide(&nav(HostKind::FrontDoor, path)),
            Decision::Refuse(Refusal::NotFound),
            "{path}"
        );
    }
}

#[test]
fn unknown_hosts_are_not_found() {
    let f = fixture();
    let facts = RequestFacts {
        cookie: cookie(OWNER, CookieScope::Door),
        ..nav(HostKind::Unknown, "/")
    };
    assert_eq!(f.decide(&facts), Decision::Refuse(Refusal::NotFound));
}

#[test]
fn expired_sessions_and_removed_access_are_denied_on_the_next_request() {
    let mut f = fixture();
    let facts = RequestFacts {
        cookie: a_cookie(ALLOWED),
        ..fetch(on_a(), Method::Get, "/x", None)
    };
    assert!(matches!(f.decide(&facts), Decision::Session { .. }));
    f.allowlist = Allowlist::new([OWNER, NO_ACCESS], [OWNER]).unwrap();
    assert_eq!(f.decide(&facts), Decision::Refuse(Refusal::NotFound));

    let mut f = fixture();
    f.now = now().checked_add(SignedDuration::from_hours(24)).unwrap();
    assert_eq!(f.decide(&facts), Decision::Refuse(Refusal::NotFound));
    let entry = RequestFacts {
        cookie: cookie(ALLOWED, CookieScope::Door),
        ..nav(HostKind::FrontDoor, "/s/aaaaaaaaaaaaaaaa")
    };
    assert_eq!(f.decide(&entry), Decision::Refuse(Refusal::NotFound));
}

#[test]
fn requests_without_fetch_metadata() {
    let f = fixture();
    let bare = RequestFacts {
        sec_fetch_site: None,
        sec_fetch_mode: None,
        sec_fetch_dest: None,
        ..nav(on_a(), "/")
    };
    assert!(matches!(
        f.decide(&bare),
        Decision::Refuse(Refusal::Redirect(_))
    ));
    let post = RequestFacts {
        sec_fetch_site: None,
        sec_fetch_mode: None,
        sec_fetch_dest: None,
        cookie: a_cookie(ALLOWED),
        ..fetch(
            on_a(),
            Method::Post,
            "/_jamjam/api/comments",
            Some(A_ORIGIN),
        )
    };
    assert!(matches!(
        f.decide(&post),
        Decision::Session {
            route: SessionRoute::Comments,
            ..
        }
    ));
    let well_known = RequestFacts {
        sec_fetch_site: None,
        sec_fetch_mode: None,
        sec_fetch_dest: None,
        ..nav(HostKind::FrontDoor, "/.well-known/jamjam")
    };
    assert_eq!(
        f.decide(&well_known),
        Decision::Public(PublicRoute::WellKnown)
    );
}

#[test]
fn methods_parse() {
    assert_eq!(Method::from_name("GET"), Method::Get);
    assert_eq!(Method::from_name("HEAD"), Method::Head);
    assert_eq!(Method::from_name("POST"), Method::Post);
    assert_eq!(Method::from_name("get"), Method::Other);
    assert_eq!(Method::from_name("DELETE"), Method::Other);
}
