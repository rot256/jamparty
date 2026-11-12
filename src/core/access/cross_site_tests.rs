//! Cross-site and Origin rules on session hosts and the front door.

use crate::core::{
    access::{
        Decision, Method, PublicRoute, Refusal, RequestFacts, SessionRoute, Upgrade,
        tests::{
            A, A_ORIGIN, ALLOWED, B_ORIGIN, DOOR_ORIGIN, OWNER, a_cookie, cookie, fetch, fixture,
            nav, on_a, seed,
        },
    },
    cookie::CookieScope,
    host::HostKind,
};

#[test]
fn cross_site_requests_are_refused() {
    let f = fixture();
    let viewer = a_cookie(ALLOWED);
    // Session B's page POSTing to A with the viewer's A cookie attached.
    for site in ["same-site", "cross-site", "bogus"] {
        let facts = RequestFacts {
            sec_fetch_site: Some(site),
            cookie: viewer,
            ..fetch(
                on_a(),
                Method::Post,
                "/_jamjam/api/comments",
                Some(B_ORIGIN),
            )
        };
        assert_eq!(
            f.decide(&facts),
            Decision::Refuse(Refusal::Forbidden),
            "{site}"
        );
        // Even with a forged-looking matching Origin, the fetch site decides.
        let facts = RequestFacts {
            sec_fetch_site: Some(site),
            cookie: viewer,
            ..fetch(
                on_a(),
                Method::Post,
                "/_jamjam/api/comments",
                Some(A_ORIGIN),
            )
        };
        assert_eq!(
            f.decide(&facts),
            Decision::Refuse(Refusal::Forbidden),
            "{site}"
        );
        // Framing or fetching a page of A from B.
        let facts = RequestFacts {
            sec_fetch_site: Some(site),
            sec_fetch_dest: Some("iframe"),
            cookie: viewer,
            ..nav(on_a(), "/")
        };
        assert_eq!(
            f.decide(&facts),
            Decision::Refuse(Refusal::Forbidden),
            "{site}"
        );
        let facts = RequestFacts {
            sec_fetch_site: Some(site),
            cookie: viewer,
            ..fetch(on_a(), Method::Get, "/_jamjam/api/status", None)
        };
        assert_eq!(
            f.decide(&facts),
            Decision::Refuse(Refusal::Forbidden),
            "{site}"
        );
        // A WebSocket from B to A.
        let facts = RequestFacts {
            sec_fetch_site: Some(site),
            sec_fetch_mode: Some("websocket"),
            upgrade: Upgrade::Yes,
            cookie: viewer,
            ..fetch(on_a(), Method::Get, "/ws", Some(B_ORIGIN))
        };
        assert_eq!(
            f.decide(&facts),
            Decision::Refuse(Refusal::Forbidden),
            "{site}"
        );
        // Top-level navigation to the entry endpoint is the exception.
        let facts = RequestFacts {
            sec_fetch_site: Some(site),
            ..nav(on_a(), "/_jamjam/enter")
        };
        assert_eq!(
            f.decide(&facts),
            Decision::Public(PublicRoute::Enter { seed: seed(A) }),
            "{site}"
        );
        // A top-level link to a page goes through the entry link.
        let facts = RequestFacts {
            sec_fetch_site: Some(site),
            cookie: viewer,
            ..nav(on_a(), "/page")
        };
        assert_eq!(
            f.decide(&facts),
            Decision::Refuse(Refusal::Redirect(
                "https://jam.example/s/aaaaaaaaaaaaaaaa?next=%2Fpage".to_owned()
            )),
            "{site}"
        );
        // Entry endpoint, but not as a top-level navigation.
        let facts = RequestFacts {
            sec_fetch_site: Some(site),
            ..fetch(on_a(), Method::Get, "/_jamjam/enter", None)
        };
        assert_eq!(
            f.decide(&facts),
            Decision::Refuse(Refusal::Forbidden),
            "{site}"
        );
    }
}

#[test]
fn origin_must_match_exactly_on_writes_and_upgrades() {
    let f = fixture();
    let viewer = a_cookie(ALLOWED);
    for origin in [
        None,
        Some(B_ORIGIN),
        Some("https://aaaaaaaaaaaaaaaa.jam.example/"),
        Some("http://aaaaaaaaaaaaaaaa.jam.example"),
        Some("null"),
        Some(DOOR_ORIGIN),
    ] {
        for method in [Method::Post, Method::Other] {
            let facts = RequestFacts {
                cookie: viewer,
                ..fetch(on_a(), method, "/_jamjam/api/comments", origin)
            };
            assert_eq!(
                f.decide(&facts),
                Decision::Refuse(Refusal::Forbidden),
                "{origin:?}"
            );
        }
        let facts = RequestFacts {
            upgrade: Upgrade::Yes,
            cookie: viewer,
            ..fetch(on_a(), Method::Get, "/ws", origin)
        };
        assert_eq!(
            f.decide(&facts),
            Decision::Refuse(Refusal::Forbidden),
            "{origin:?}"
        );
        let facts = RequestFacts {
            cookie: cookie(OWNER, CookieScope::Door),
            ..fetch(HostKind::FrontDoor, Method::Post, "/auth/logout", origin)
        };
        if origin != Some(DOOR_ORIGIN) {
            assert_eq!(
                f.decide(&facts),
                Decision::Refuse(Refusal::Forbidden),
                "{origin:?}"
            );
        }
    }
    let facts = RequestFacts {
        upgrade: Upgrade::Yes,
        cookie: viewer,
        ..fetch(on_a(), Method::Get, "/ws", Some(A_ORIGIN))
    };
    assert!(matches!(
        f.decide(&facts),
        Decision::Session {
            route: SessionRoute::Proxy,
            ..
        }
    ));
    // GET and HEAD need no Origin.
    for method in [Method::Get, Method::Head] {
        let facts = RequestFacts {
            cookie: viewer,
            ..fetch(on_a(), method, "/x", None)
        };
        assert!(matches!(
            f.decide(&facts),
            Decision::Session {
                route: SessionRoute::Proxy,
                ..
            }
        ));
    }
}
