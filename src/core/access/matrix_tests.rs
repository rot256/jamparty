//! The access matrix: user x host x route, each with its expected decision.

use std::collections::BTreeMap;

use crate::core::{
    access::{
        Decision, Method, Refusal, RequestFacts, Upgrade,
        tests::{
            A, A_ORIGIN, ALLOWED, B, B_ORIGIN, DOOR_ORIGIN, NO_ACCESS, OUTSIDER, OWNER, UNKNOWN,
            cookie, fetch, fixture, nav, seed,
        },
    },
    cookie::CookieScope,
    host::HostKind,
    ids::UserId,
};

/// Decision kinds compared in the matrix (exact values are covered in
/// `tests.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    /// 303 (to sign-in or to the entry link).
    Redirect,
    /// 401.
    Unauth,
    /// 403.
    Forbid,
    /// 404.
    NotFound,
    /// Public route.
    Public,
    /// Signed-in front-door route.
    Door,
    /// Session route with access.
    Session,
}

use Want::{
    Door, Forbid as F, NotFound as N, Public as P, Redirect as R, Session as S, Unauth as U,
};

fn kind(decision: &Decision) -> Want {
    match decision {
        Decision::Refuse(Refusal::Redirect(_)) => R,
        Decision::Refuse(Refusal::Unauthorized) => U,
        Decision::Refuse(Refusal::Forbidden) => F,
        Decision::Refuse(Refusal::NotFound) => N,
        Decision::Public(_) => P,
        Decision::Door { .. } => Door,
        Decision::Session { .. } => S,
    }
}

/// Users, in column order.
const USERS: [Option<UserId>; 5] = [
    None,
    Some(OUTSIDER),
    Some(NO_ACCESS),
    Some(OWNER),
    Some(ALLOWED),
];

#[derive(Clone, Copy, Debug)]
enum Host {
    Door,
    SessionA,
    SessionB,
    Unknown,
}

impl Host {
    fn kind(self) -> HostKind {
        match self {
            Self::Door => HostKind::FrontDoor,
            Self::SessionA => HostKind::Session(seed(A)),
            Self::SessionB => HostKind::Session(seed(B)),
            Self::Unknown => HostKind::Session(seed(UNKNOWN)),
        }
    }

    fn origin(self) -> &'static str {
        match self {
            Self::Door => DOOR_ORIGIN,
            Self::SessionA => A_ORIGIN,
            Self::SessionB => B_ORIGIN,
            Self::Unknown => "https://uuuuuuuuuuuuuuuu.jam.example",
        }
    }

    fn scope(self) -> CookieScope {
        match self.kind() {
            HostKind::Session(seed) => CookieScope::Session(seed),
            HostKind::FrontDoor | HostKind::Unknown => CookieScope::Door,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Req {
    Nav(&'static str),
    Get(&'static str),
    Post(&'static str),
    Upgrade(&'static str),
}

fn facts(host: Host, req: Req) -> RequestFacts<'static> {
    let kind = host.kind();
    match req {
        Req::Nav(path) => nav(kind, path),
        Req::Get(path) => fetch(kind, Method::Get, path, None),
        Req::Post(path) => fetch(kind, Method::Post, path, Some(host.origin())),
        Req::Upgrade(path) => RequestFacts {
            upgrade: Upgrade::Yes,
            sec_fetch_mode: Some("websocket"),
            ..fetch(kind, Method::Get, path, Some(host.origin()))
        },
    }
}

#[rustfmt::skip]
const MATRIX: &[(Host, Req, [Want; 5])] = &[
    //                                                           none  outsider no-access owner allowed
    (Host::Door, Req::Nav("/"),                                  [R, F, Door, Door, Door]),
    (Host::Door, Req::Nav("/s/aaaaaaaaaaaaaaaa"),                [R, F, N, Door, Door]),
    (Host::Door, Req::Nav("/s/bbbbbbbbbbbbbbbb"),                [R, F, N, N, N]),
    (Host::Door, Req::Nav("/s/uuuuuuuuuuuuuuuu"),                [R, F, N, N, N]),
    (Host::Door, Req::Nav("/s/not-a-seed"),                      [N, N, N, N, N]),
    (Host::Door, Req::Nav("/devices"),                           [R, F, F, Door, F]),
    (Host::Door, Req::Post("/devices"),                          [U, F, F, Door, F]),
    (Host::Door, Req::Get("/devices"),                           [U, F, F, Door, F]),
    (Host::Door, Req::Post("/auth/logout"),                      [U, Door, Door, Door, Door]),
    (Host::Door, Req::Nav("/auth/login"),                        [P, P, P, P, P]),
    (Host::Door, Req::Nav("/auth/callback"),                     [P, P, P, P, P]),
    (Host::Door, Req::Get("/.well-known/jamjam"),                [P, P, P, P, P]),
    (Host::Door, Req::Nav("/nope"),                              [N, N, N, N, N]),

    (Host::SessionA, Req::Nav("/"),                              [R, N, N, S, S]),
    (Host::SessionA, Req::Get("/app.js"),                        [U, N, N, S, S]),
    (Host::SessionA, Req::Post("/form"),                         [U, N, N, S, S]),
    (Host::SessionA, Req::Upgrade("/ws"),                        [U, N, N, S, S]),
    (Host::SessionA, Req::Post("/_jamjam/api/comments"),         [U, N, N, S, S]),
    (Host::SessionA, Req::Get("/_jamjam/api/status"),            [U, N, N, S, S]),
    (Host::SessionA, Req::Get("/_jamjam/api/messages"),          [U, N, N, S, S]),
    (Host::SessionA, Req::Nav("/_jamjam/api/status"),            [U, N, N, S, S]),
    (Host::SessionA, Req::Nav("/_jamjam/enter"),                 [P, P, P, P, P]),

    (Host::SessionB, Req::Nav("/"),                              [R, N, N, N, N]),
    (Host::SessionB, Req::Get("/app.js"),                        [U, N, N, N, N]),
    (Host::SessionB, Req::Post("/form"),                         [U, N, N, N, N]),
    (Host::SessionB, Req::Upgrade("/ws"),                        [U, N, N, N, N]),
    (Host::SessionB, Req::Post("/_jamjam/api/comments"),         [U, N, N, N, N]),
    (Host::SessionB, Req::Get("/_jamjam/api/status"),            [U, N, N, N, N]),
    (Host::SessionB, Req::Get("/_jamjam/api/messages"),          [U, N, N, N, N]),
    (Host::SessionB, Req::Nav("/_jamjam/api/status"),            [U, N, N, N, N]),
    (Host::SessionB, Req::Nav("/_jamjam/enter"),                 [P, P, P, P, P]),

    (Host::Unknown, Req::Nav("/"),                               [R, N, N, N, N]),
    (Host::Unknown, Req::Get("/app.js"),                         [U, N, N, N, N]),
    (Host::Unknown, Req::Post("/form"),                          [U, N, N, N, N]),
    (Host::Unknown, Req::Upgrade("/ws"),                         [U, N, N, N, N]),
    (Host::Unknown, Req::Post("/_jamjam/api/comments"),          [U, N, N, N, N]),
    (Host::Unknown, Req::Get("/_jamjam/api/status"),             [U, N, N, N, N]),
    (Host::Unknown, Req::Get("/_jamjam/api/messages"),           [U, N, N, N, N]),
    (Host::Unknown, Req::Nav("/_jamjam/api/status"),             [U, N, N, N, N]),
    (Host::Unknown, Req::Nav("/_jamjam/enter"),                  [P, P, P, P, P]),
];

#[test]
fn access_matrix() {
    let f = fixture();
    let mut failures = Vec::new();
    for (host, req, wants) in MATRIX {
        for (user, want) in USERS.iter().zip(wants) {
            let facts = RequestFacts {
                cookie: user.and_then(|user| cookie(user, host.scope())),
                ..facts(*host, *req)
            };
            let got = f.decide(&facts);
            if kind(&got) != *want {
                failures.push(format!(
                    "{host:?} {req:?} {user:?}: want {want:?}, got {got:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The seed of B, once with B existing (no matrix user has access) and once
/// with B removed: every user, route and host gives the same decision.
#[test]
fn unknown_and_unauthorized_seeds_decide_identically() {
    let with_b = fixture();
    let mut without_b = fixture();
    without_b.sessions = without_b
        .sessions
        .into_iter()
        .filter(|(seed, _)| *seed != self::seed(B))
        .collect::<BTreeMap<_, _>>();

    let requests = MATRIX
        .iter()
        .filter(|(host, _, _)| matches!(host, Host::SessionB))
        .map(|(host, req, _)| (*host, *req))
        .chain([(Host::Door, Req::Nav("/s/bbbbbbbbbbbbbbbb"))]);
    for (host, req) in requests {
        for user in USERS {
            let facts = RequestFacts {
                cookie: user.and_then(|user| cookie(user, host.scope())),
                ..facts(host, req)
            };
            assert_eq!(
                with_b.decide(&facts),
                without_b.decide(&facts),
                "{host:?} {req:?} {user:?}"
            );
        }
    }
}
