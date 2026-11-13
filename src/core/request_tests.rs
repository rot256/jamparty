use proptest::prelude::*;

use crate::core::{
    ids::EndpointKey,
    port::Port,
    request::{
        CLIENT_PATH, LocalOrigin, Method, Refusal, RequestFacts, Route, SOCKET_PATH, Upgrade,
    },
};

const HOST: &str = "tzrjdfymwrg5sqai.localhost:4321";
const ORIGIN: &str = "http://tzrjdfymwrg5sqai.localhost:4321";
const OTHER_ORIGIN: &str = "https://evil.example";

fn local() -> LocalOrigin {
    LocalOrigin::new(&EndpointKey([0; 32]).sid(), 4321.try_into().unwrap())
}

/// A same-origin `fetch` from the session's own page.
fn fetch(method: Method, path: &str) -> RequestFacts<'_> {
    RequestFacts {
        host: Some(HOST),
        method,
        path,
        sec_fetch_site: Some("same-origin"),
        sec_fetch_mode: Some("cors"),
        sec_fetch_dest: Some("empty"),
        origin: match method {
            Method::Get | Method::Head => None,
            Method::Post | Method::Other => Some(ORIGIN),
        },
        upgrade: Upgrade::No,
    }
}

/// A top-level navigation from `site`.
fn nav<'a>(path: &'a str, site: &'a str) -> RequestFacts<'a> {
    RequestFacts {
        sec_fetch_site: Some(site),
        sec_fetch_mode: Some("navigate"),
        sec_fetch_dest: Some("document"),
        ..fetch(Method::Get, path)
    }
}

/// A non-browser client: no fetch metadata, no Origin.
fn bare(method: Method, path: &str) -> RequestFacts<'_> {
    RequestFacts {
        sec_fetch_site: None,
        sec_fetch_mode: None,
        sec_fetch_dest: None,
        origin: None,
        ..fetch(method, path)
    }
}

/// The page opening its socket.
fn socket(site: Option<&'static str>) -> RequestFacts<'static> {
    RequestFacts {
        path: SOCKET_PATH,
        ..upgrade(site)
    }
}

fn upgrade(site: Option<&'static str>) -> RequestFacts<'static> {
    RequestFacts {
        upgrade: Upgrade::Yes,
        sec_fetch_site: site,
        sec_fetch_mode: Some("websocket"),
        origin: Some(ORIGIN),
        ..fetch(Method::Get, "/__vite_hmr")
    }
}

#[test]
fn origin_and_host() {
    let local = local();
    assert_eq!(local.host(), HOST);
    assert_eq!(local.origin(), ORIGIN);
}

#[test]
fn the_default_port_is_left_out() {
    // As browsers write it: no `:80` in `Host` or `Origin`.
    let local = LocalOrigin::new(&EndpointKey([0; 32]).sid(), Port::HTTP);
    assert_eq!(local.host(), "tzrjdfymwrg5sqai.localhost");
    assert_eq!(local.origin(), "http://tzrjdfymwrg5sqai.localhost");
    let post = RequestFacts {
        host: Some("tzrjdfymwrg5sqai.localhost"),
        origin: Some("http://tzrjdfymwrg5sqai.localhost"),
        ..socket(Some("same-origin"))
    };
    assert_eq!(local.decide(&post), Ok(Route::Socket));
    let explicit = RequestFacts {
        host: Some("tzrjdfymwrg5sqai.localhost:80"),
        ..post
    };
    assert_eq!(local.decide(&explicit), Err(Refusal::WrongHost));
    let explicit_origin = RequestFacts {
        origin: Some("http://tzrjdfymwrg5sqai.localhost:80"),
        ..post
    };
    assert_eq!(local.decide(&explicit_origin), Err(Refusal::BadOrigin));
}

#[test]
fn table() {
    let cases: Vec<(&str, RequestFacts<'_>, Result<Route, Refusal>)> = vec![
        // Same-origin use of the session.
        ("page", nav("/", "none"), Ok(Route::Proxy)),
        ("asset", fetch(Method::Get, "/app.js"), Ok(Route::Proxy)),
        ("head", fetch(Method::Head, "/"), Ok(Route::Proxy)),
        ("socket", socket(Some("same-origin")), Ok(Route::Socket)),
        ("site post", fetch(Method::Post, "/form"), Ok(Route::Proxy)),
        ("site put", fetch(Method::Other, "/api/x"), Ok(Route::Proxy)),
        (
            "client.js",
            fetch(Method::Get, CLIENT_PATH),
            Ok(Route::Client),
        ),
        (
            "head client.js",
            fetch(Method::Head, CLIENT_PATH),
            Ok(Route::Client),
        ),
        (
            "client.js as a module",
            RequestFacts {
                origin: Some(ORIGIN),
                sec_fetch_dest: Some("script"),
                ..fetch(Method::Get, CLIENT_PATH)
            },
            Ok(Route::Client),
        ),
        ("upgrade", upgrade(Some("same-origin")), Ok(Route::Proxy)),
        ("upgrade no site", upgrade(None), Ok(Route::Proxy)),
        // Navigation from anywhere.
        (
            "nav cross-site",
            nav("/docs", "cross-site"),
            Ok(Route::Proxy),
        ),
        ("nav same-site", nav("/", "same-site"), Ok(Route::Proxy)),
        (
            "nav to client.js",
            nav(CLIENT_PATH, "cross-site"),
            Ok(Route::Client),
        ),
        // Non-browser clients.
        ("curl get", bare(Method::Get, "/"), Ok(Route::Proxy)),
        (
            "curl socket no origin",
            RequestFacts {
                origin: None,
                ..socket(None)
            },
            Err(Refusal::BadOrigin),
        ),
        ("curl socket with origin", socket(None), Ok(Route::Socket)),
        // Reserved paths: exact path, method and upgrade.
        (
            "post client.js",
            fetch(Method::Post, CLIENT_PATH),
            Err(Refusal::MethodNotAllowed),
        ),
        (
            "upgrade client.js",
            RequestFacts {
                path: CLIENT_PATH,
                ..upgrade(Some("same-origin"))
            },
            Err(Refusal::MethodNotAllowed),
        ),
        (
            "cross-site client.js",
            RequestFacts {
                sec_fetch_site: Some("cross-site"),
                ..fetch(Method::Get, CLIENT_PATH)
            },
            Err(Refusal::CrossSite),
        ),
        (
            "socket without upgrade",
            fetch(Method::Get, SOCKET_PATH),
            Err(Refusal::MethodNotAllowed),
        ),
        (
            "post socket",
            RequestFacts {
                method: Method::Post,
                ..socket(Some("same-origin"))
            },
            Err(Refusal::MethodNotAllowed),
        ),
        // The rest of the joiner's paths: its own, and not found.
        (
            "the removed status",
            fetch(Method::Get, "/_jampot/api/status"),
            Ok(Route::Unknown),
        ),
        (
            "trailing slash",
            fetch(Method::Get, "/_jampot/api/status/"),
            Ok(Route::Unknown),
        ),
        (
            "an old endpoint",
            fetch(Method::Get, "/_jampot/api/events"),
            Ok(Route::Unknown),
        ),
        (
            "an old write",
            fetch(Method::Post, "/_jampot/api/comments"),
            Ok(Route::Unknown),
        ),
        (
            "an upgrade elsewhere",
            RequestFacts {
                path: "/_jampot/api/other",
                ..upgrade(Some("same-origin"))
            },
            Ok(Route::Unknown),
        ),
        (
            "the prefix itself",
            fetch(Method::Get, "/_jampot/api/"),
            Ok(Route::Unknown),
        ),
        (
            "not under it",
            fetch(Method::Get, "/_jampot/api"),
            Ok(Route::Proxy),
        ),
        (
            "encoded",
            fetch(Method::Get, "/%5Fjampot/api/status"),
            Ok(Route::Proxy),
        ),
        (
            "double slash",
            fetch(Method::Get, "//_jampot/api/status"),
            Ok(Route::Proxy),
        ),
        // Cross-site rules still come first.
        (
            "cross-site unknown",
            RequestFacts {
                sec_fetch_site: Some("cross-site"),
                ..fetch(Method::Get, "/_jampot/api/events")
            },
            Err(Refusal::CrossSite),
        ),
    ];
    let local = local();
    for (name, facts, expected) in cases {
        assert_eq!(local.decide(&facts), expected, "{name}");
    }
}

#[test]
fn wrong_hosts_are_refused() {
    // DNS rebinding: another name resolving to 127.0.0.1 still sends its
    // own name; so does any other session or port.
    let hosts = [
        None,
        Some(""),
        Some("evil.example:4321"),
        Some("evil.example"),
        Some("127.0.0.1:4321"),
        Some("localhost:4321"),
        Some("[::1]:4321"),
        Some("tzrjdfymwrg5sqai.localhost"),
        Some("tzrjdfymwrg5sqai.localhost:4322"),
        Some("tzrjdfymwrg5sqai.localhost:4321."),
        Some("tzrjdfymwrg5sqai.localhost.:4321"),
        Some("MZUHVLPYMK6XO3EP.localhost:4321"),
        Some("aaaaaaaaaaaaaaaa.localhost:4321"),
        Some("x.tzrjdfymwrg5sqai.localhost:4321"),
        Some("tzrjdfymwrg5sqai.localhost:4321@evil.example"),
        Some("tzrjdfymwrg5sqai.localhost.evil.example:4321"),
    ];
    let local = local();
    for host in hosts {
        for facts in [nav("/", "none"), socket(Some("same-origin"))] {
            let facts = RequestFacts { host, ..facts };
            assert_eq!(local.decide(&facts), Err(Refusal::WrongHost), "{host:?}");
        }
    }
}

#[test]
fn cross_site_requests_are_refused() {
    let local = local();
    for site in ["cross-site", "same-site", "none", "bogus"] {
        let cases = [
            // A page elsewhere opening the socket, with or without a forged
            // matching Origin (the fetch site decides first).
            socket(Some("same-origin")),
            RequestFacts {
                origin: Some(OTHER_ORIGIN),
                ..socket(Some("same-origin"))
            },
            // Reading the client, or any page by fetch.
            fetch(Method::Get, CLIENT_PATH),
            fetch(Method::Get, "/"),
            // An upgrade.
            upgrade(None),
            // A frame or form navigation is not a top-level GET.
            RequestFacts {
                sec_fetch_dest: Some("iframe"),
                ..nav("/", "cross-site")
            },
            RequestFacts {
                method: Method::Post,
                ..nav("/form", "cross-site")
            },
        ];
        for facts in cases {
            let facts = RequestFacts {
                sec_fetch_site: Some(site),
                ..facts
            };
            assert_eq!(
                local.decide(&facts),
                Err(Refusal::CrossSite),
                "{site} {facts:?}"
            );
        }
    }
}

#[test]
fn reads_with_another_origin_are_refused() {
    let local = local();
    for origin in [Some("null"), Some(OTHER_ORIGIN)] {
        for facts in [fetch(Method::Get, "/"), bare(Method::Head, CLIENT_PATH)] {
            let facts = RequestFacts { origin, ..facts };
            assert_eq!(local.decide(&facts), Err(Refusal::BadOrigin), "{origin:?}");
        }
    }
    let own = RequestFacts {
        origin: Some(ORIGIN),
        ..fetch(Method::Get, CLIENT_PATH)
    };
    assert_eq!(local.decide(&own), Ok(Route::Client));
    // Navigations are allowed from anywhere, with their Origin.
    let navigation = RequestFacts {
        origin: Some(OTHER_ORIGIN),
        ..nav("/", "cross-site")
    };
    assert_eq!(local.decide(&navigation), Ok(Route::Proxy));
}

#[test]
fn writes_and_upgrades_need_the_exact_origin() {
    let local = local();
    let origins = [
        None,
        Some("null"),
        Some(OTHER_ORIGIN),
        Some("http://tzrjdfymwrg5sqai.localhost:4322"),
        Some("https://tzrjdfymwrg5sqai.localhost:4321"),
        Some("http://tzrjdfymwrg5sqai.localhost:4321/"),
        Some("http://aaaaaaaaaaaaaaaa.localhost:4321"),
    ];
    for origin in origins {
        for facts in [
            socket(Some("same-origin")),
            fetch(Method::Post, "/form"),
            fetch(Method::Other, "/"),
            RequestFacts {
                upgrade: Upgrade::Yes,
                ..fetch(Method::Get, "/ws")
            },
        ] {
            let facts = RequestFacts { origin, ..facts };
            assert_eq!(local.decide(&facts), Err(Refusal::BadOrigin), "{origin:?}");
        }
    }
}

#[test]
fn refusal_statuses() {
    assert_eq!(Refusal::WrongHost.status(), 421);
    assert_eq!(Refusal::CrossSite.status(), 403);
    assert_eq!(Refusal::BadOrigin.status(), 403);
    assert_eq!(Refusal::MethodNotAllowed.status(), 405);
}

proptest! {
    /// Whatever else a request carries, only the exact host is served, and
    /// the socket opens only for a same-origin (or bare) request with the
    /// exact Origin.
    #[test]
    fn the_socket_needs_host_site_and_origin(
        host in prop_oneof![Just(HOST.to_owned()), "\\PC{0,40}"],
        site in prop::option::of(prop_oneof![
            Just("same-origin"), Just("same-site"), Just("cross-site"), Just("none")
        ]),
        mode in prop::option::of(prop_oneof![
            Just("navigate"), Just("cors"), Just("no-cors"), Just("websocket")
        ]),
        dest in prop::option::of(prop_oneof![Just("document"), Just("empty"), Just("iframe")]),
        origin in prop::option::of(prop_oneof![Just(ORIGIN.to_owned()), "\\PC{0,40}"]),
    ) {
        let facts = RequestFacts {
            host: Some(host.as_str()),
            method: Method::Get,
            path: SOCKET_PATH,
            sec_fetch_site: site,
            sec_fetch_mode: mode,
            sec_fetch_dest: dest,
            origin: origin.as_deref(),
            upgrade: Upgrade::Yes,
        };
        let accepted = local().decide(&facts) == Ok(Route::Socket);
        let allowed = host == HOST
            && matches!(site, None | Some("same-origin"))
            && origin.as_deref() == Some(ORIGIN);
        prop_assert_eq!(accepted, allowed);
    }
}
