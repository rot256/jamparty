//! Path classification: the one place where paths become routes.
//!
//! Paths are matched exactly as received (no decoding, no case folding, no
//! slash merging): `//_jamjam/enter`, `/%5Fjamjam/enter` or
//! `/_jamjam/enter/` are not `/_jamjam/enter`. On a session host anything
//! that is not exactly a reserved path goes to the share.

use url::form_urlencoded;

use crate::core::{ids::Seed, safe_path::SafePath};

/// Routes on the front door.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DoorPath {
    Home,
    Entry { seed: Seed, next: SafePath },
    Login { next: Option<SafePath> },
    Callback,
    Logout,
    Devices,
    WellKnown,
}

/// Routes on a session host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionPath {
    Enter,
    Comments,
    Status,
    Messages,
    Proxy,
}

pub(crate) const ENTER_PATH: &str = "/_jamjam/enter";
pub(crate) const LOGIN_PATH: &str = "/auth/login";

/// The single value of query parameter `name`, decoded; `None` if absent
/// or given more than once.
pub(crate) fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    let mut values = form_urlencoded::parse(query?.as_bytes())
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned());
    match (values.next(), values.next()) {
        (Some(value), None) => Some(value),
        (Some(_), Some(_)) | (None, _) => None,
    }
}

fn next_param(query: Option<&str>) -> Option<SafePath> {
    query_param(query, "next").and_then(|next| str::parse::<SafePath>(next.as_str()).ok())
}

/// Classifies a front-door path; `None` is 404.
pub(crate) fn door_path(path: &str, query: Option<&str>) -> Option<DoorPath> {
    let route = match path {
        "/" => DoorPath::Home,
        LOGIN_PATH => DoorPath::Login {
            next: next_param(query),
        },
        "/auth/callback" => DoorPath::Callback,
        "/auth/logout" => DoorPath::Logout,
        "/devices" => DoorPath::Devices,
        "/.well-known/jamjam" => DoorPath::WellKnown,
        other => {
            let seed = other.strip_prefix("/s/")?.parse().ok()?;
            DoorPath::Entry {
                seed,
                next: next_param(query).unwrap_or_else(SafePath::root),
            }
        }
    };
    Some(route)
}

/// Classifies a session-host path.
#[must_use]
pub(crate) fn session_path(path: &str) -> SessionPath {
    match path {
        ENTER_PATH => SessionPath::Enter,
        "/_jamjam/api/comments" => SessionPath::Comments,
        "/_jamjam/api/status" => SessionPath::Status,
        "/_jamjam/api/messages" => SessionPath::Messages,
        _ => SessionPath::Proxy,
    }
}

/// `path?query` as a redirect target, if it is a safe one.
pub(crate) fn path_and_query(path: &str, query: Option<&str>) -> Option<SafePath> {
    let joined = match query {
        Some(query) => format!("{path}?{query}"),
        None => path.to_owned(),
    };
    str::parse::<SafePath>(joined.as_str()).ok()
}

/// `base?next=<encoded next>`, or `base` when there is no `next`.
pub(crate) fn with_next(base: String, next: Option<&SafePath>) -> String {
    match next {
        Some(next) => {
            let encoded: String =
                form_urlencoded::byte_serialize(next.as_str().as_bytes()).collect();
            format!("{base}?next={encoded}")
        }
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use crate::core::{
        access::routes::{DoorPath, SessionPath, door_path, query_param, session_path, with_next},
        safe_path::SafePath,
    };

    #[test]
    fn session_paths_match_exactly() {
        assert_eq!(session_path("/_jamjam/enter"), SessionPath::Enter);
        assert_eq!(session_path("/_jamjam/api/comments"), SessionPath::Comments);
        assert_eq!(session_path("/_jamjam/api/status"), SessionPath::Status);
        assert_eq!(session_path("/_jamjam/api/messages"), SessionPath::Messages);
        for other in [
            "/",
            "//_jamjam/enter",
            "/%5Fjamjam/enter",
            "/_JAMJAM/enter",
            "/_jamjam/enter/",
            "/_jamjam/Enter",
            "/_jamjam/api/comments/",
            "/_jamjam/client.js",
            "/_jamjam/enter;x",
        ] {
            assert_eq!(session_path(other), SessionPath::Proxy, "{other:?}");
        }
    }

    #[test]
    fn door_paths() {
        assert_eq!(door_path("/", None), Some(DoorPath::Home));
        assert_eq!(
            door_path("/s/aaaaaaaaaaaaaaaa", Some("next=%2Fa%3Fb%3D1")),
            Some(DoorPath::Entry {
                seed: "aaaaaaaaaaaaaaaa".parse().unwrap(),
                next: str::parse::<SafePath>("/a?b=1").unwrap(),
            })
        );
        assert_eq!(
            door_path("/s/aaaaaaaaaaaaaaaa", Some("next=//evil")),
            Some(DoorPath::Entry {
                seed: "aaaaaaaaaaaaaaaa".parse().unwrap(),
                next: SafePath::root(),
            })
        );
        assert_eq!(
            door_path("/auth/login", Some("next=https://evil")),
            Some(DoorPath::Login { next: None })
        );
        for missing in [
            "/s/AAAAAAAAAAAAAAAA",
            "/s/aaaaaaaaaaaaaaaa/",
            "/s/",
            "//",
            "/devices/",
            "/auth/login/",
            "/S/aaaaaaaaaaaaaaaa",
        ] {
            assert_eq!(door_path(missing, None), None, "{missing:?}");
        }
    }

    #[test]
    fn query_params() {
        assert_eq!(
            query_param(Some("a=1&next=%2Fx"), "next").as_deref(),
            Some("/x")
        );
        assert_eq!(query_param(Some("next=/a&next=/b"), "next"), None);
        assert_eq!(query_param(None, "next"), None);
        assert_eq!(
            with_next(
                "https://d/s/x".to_owned(),
                Some(&str::parse::<SafePath>("/a?b=1&c").unwrap())
            ),
            "https://d/s/x?next=%2Fa%3Fb%3D1%26c"
        );
    }
}
