//! The authentication decision: one pure function from request facts to
//! either a refusal or a route plus who may use it.
//!
//! Deny by default: a route is protected unless listed in
//! [`PublicRoute`]. Unknown and unauthorized seeds produce identical
//! decisions.

mod routes;

use std::collections::BTreeMap;

use jiff::Timestamp;

use crate::core::{
    access::routes::{
        DoorPath, LOGIN_PATH, SessionPath, door_path, path_and_query, session_path, with_next,
    },
    allowlist::Allowlist,
    cookie::{CookieScope, VerifiedCookie},
    host::{HostConfig, HostKind},
    ids::{Seed, UserId},
    safe_path::SafePath,
    session::Session,
};

/// The request method, as far as the decision cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Head,
    Post,
    /// Any other method (PUT, DELETE, OPTIONS, ...): proxied, and
    /// Origin-checked like POST.
    Other,
}

impl Method {
    #[must_use]
    pub(crate) fn from_name(name: &str) -> Self {
        match name {
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            _ => Self::Other,
        }
    }

    /// GET and HEAD: browsers send no `Origin` on same-origin GET and HEAD.
    fn is_safe(self) -> bool {
        match self {
            Self::Get | Self::Head => true,
            Self::Post | Self::Other => false,
        }
    }
}

/// Whether the request asks for a protocol upgrade (WebSocket, HMR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Upgrade {
    No,
    Yes,
}

/// What the request carries, as plain values.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestFacts<'a> {
    pub(crate) host: HostKind,
    pub(crate) method: Method,
    /// The raw path, without the query.
    pub(crate) path: &'a str,
    pub(crate) query: Option<&'a str>,
    pub(crate) sec_fetch_site: Option<&'a str>,
    pub(crate) sec_fetch_mode: Option<&'a str>,
    pub(crate) sec_fetch_dest: Option<&'a str>,
    pub(crate) origin: Option<&'a str>,
    pub(crate) upgrade: Upgrade,
    /// The host's jamjam cookie, if it verified (for any scope; the scope
    /// is checked again here).
    pub(crate) cookie: Option<VerifiedCookie>,
}

/// Finds live sessions by seed.
pub(crate) trait SessionDirectory {
    fn session(&self, seed: &Seed) -> Option<&Session>;
}

impl SessionDirectory for BTreeMap<Seed, Session> {
    fn session(&self, seed: &Seed) -> Option<&Session> {
        self.get(seed)
    }
}

/// Server state the decision reads.
#[derive(Clone, Copy)]
pub(crate) struct AccessContext<'a> {
    pub(crate) hosts: &'a HostConfig,
    pub(crate) allowlist: &'a Allowlist,
    pub(crate) sessions: &'a dyn SessionDirectory,
    pub(crate) now: Timestamp,
}

/// A response decided without a handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// 303 to this absolute URL.
    Redirect(String),
    Unauthorized,
    Forbidden,
    NotFound,
}

/// Routes anyone may reach: the explicit public list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PublicRoute {
    Login {
        next: Option<SafePath>,
    },
    Callback,
    WellKnown,
    /// `/_jamjam/enter` on a session host (the token is checked by the handler).
    Enter {
        seed: Seed,
    },
}

/// Front-door routes for a signed-in person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DoorRoute {
    Home,
    Devices,
    Logout,
    /// `/s/<seed>`: access already checked; the handler issues a token.
    Entry {
        seed: Seed,
        next: SafePath,
    },
}

/// Session-host routes for a viewer with access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionRoute {
    Comments,
    Status,
    Messages,
    /// Everything else: forwarded to the share.
    Proxy,
}

/// Every route, for dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Public(PublicRoute),
    Door(DoorRoute),
    Session(SessionRoute),
}

/// A viewer allowed into a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Access {
    pub(crate) user: UserId,
    pub(crate) seed: Seed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Refuse(Refusal),
    Public(PublicRoute),
    Door { route: DoorRoute, user: UserId },
    Session { route: SessionRoute, access: Access },
}

impl Decision {
    /// The route to dispatch, if the request is allowed.
    #[must_use]
    pub(crate) fn route(&self) -> Option<Route> {
        match self {
            Self::Refuse(_) => None,
            Self::Public(route) => Some(Route::Public(route.clone())),
            Self::Door { route, .. } => Some(Route::Door(route.clone())),
            Self::Session { route, .. } => Some(Route::Session(*route)),
        }
    }
}

const fn refuse(refusal: Refusal) -> Decision {
    Decision::Refuse(refusal)
}

/// `Sec-Fetch-Site` is `same-origin` or `none`, or absent (not a browser
/// that sends it: browsers always do, and pages cannot remove it).
fn same_origin_fetch(facts: &RequestFacts<'_>) -> bool {
    matches!(facts.sec_fetch_site, None | Some("same-origin" | "none"))
}

/// A top-level GET navigation: `Sec-Fetch-Mode: navigate` with
/// `Sec-Fetch-Dest: document`, or a GET without fetch metadata.
fn top_level_get(facts: &RequestFacts<'_>) -> bool {
    let navigation = match (facts.sec_fetch_mode, facts.sec_fetch_dest) {
        (Some("navigate"), Some("document")) | (None, None) => true,
        (Some(_) | None, Some(_) | None) => false,
    };
    facts.method == Method::Get && facts.upgrade == Upgrade::No && navigation
}

/// Non-GET/HEAD requests and upgrades must carry exactly `expected` as
/// `Origin`, from a same-origin fetch.
fn writes_from(facts: &RequestFacts<'_>, expected: &str) -> bool {
    let needs_origin = !facts.method.is_safe() || facts.upgrade == Upgrade::Yes;
    !needs_origin || (facts.origin == Some(expected) && same_origin_fetch(facts))
}

fn cookie_user(facts: &RequestFacts<'_>, scope: CookieScope) -> Option<UserId> {
    facts
        .cookie
        .filter(|cookie| cookie.scope == scope)
        .map(|cookie| cookie.user)
}

fn may_view(ctx: &AccessContext<'_>, seed: &Seed, user: UserId) -> bool {
    ctx.sessions.session(seed).is_some_and(|session| {
        !session.is_expired(ctx.now) && session.may_view(user, ctx.allowlist)
    })
}

/// The authentication decision for one request.
#[must_use]
pub(crate) fn decide(facts: &RequestFacts<'_>, ctx: &AccessContext<'_>) -> Decision {
    match facts.host {
        HostKind::FrontDoor => decide_door(facts, ctx),
        HostKind::Session(seed) => decide_session(seed, facts, ctx),
        HostKind::Unknown => refuse(Refusal::NotFound),
    }
}

fn decide_door(facts: &RequestFacts<'_>, ctx: &AccessContext<'_>) -> Decision {
    let Some(path) = door_path(facts.path, facts.query) else {
        return refuse(Refusal::NotFound);
    };
    let origin = ctx.hosts.front_door_origin();
    if !writes_from(facts, &origin) {
        return refuse(Refusal::Forbidden);
    }
    let route = match path {
        DoorPath::Login { next } => return Decision::Public(PublicRoute::Login { next }),
        DoorPath::Callback => return Decision::Public(PublicRoute::Callback),
        DoorPath::WellKnown => return Decision::Public(PublicRoute::WellKnown),
        DoorPath::Home => DoorRoute::Home,
        DoorPath::Logout => DoorRoute::Logout,
        DoorPath::Devices => DoorRoute::Devices,
        DoorPath::Entry { seed, next } => DoorRoute::Entry { seed, next },
    };
    let Some(user) = cookie_user(facts, CookieScope::Door) else {
        return if top_level_get(facts) {
            let back = path_and_query(facts.path, facts.query);
            refuse(Refusal::Redirect(with_next(
                format!("{origin}{LOGIN_PATH}"),
                back.as_ref(),
            )))
        } else {
            refuse(Refusal::Unauthorized)
        };
    };
    let allowed = match &route {
        DoorRoute::Logout => true,
        DoorRoute::Home => ctx.allowlist.is_allowed(user),
        DoorRoute::Devices => ctx.allowlist.may_share(user),
        DoorRoute::Entry { seed, .. } => {
            if !ctx.allowlist.is_allowed(user) {
                return refuse(Refusal::Forbidden);
            }
            if !may_view(ctx, seed, user) {
                return refuse(Refusal::NotFound);
            }
            true
        }
    };
    if allowed {
        Decision::Door { route, user }
    } else {
        refuse(Refusal::Forbidden)
    }
}

fn decide_session(seed: Seed, facts: &RequestFacts<'_>, ctx: &AccessContext<'_>) -> Decision {
    let path = session_path(facts.path);
    let entry_link = || {
        let next = path_and_query(facts.path, facts.query);
        let base = format!("{}/s/{seed}", ctx.hosts.front_door_origin());
        refuse(Refusal::Redirect(with_next(base, next.as_ref())))
    };

    if !same_origin_fetch(facts) {
        let enter = path == SessionPath::Enter && top_level_get(facts);
        if !enter {
            // A cross-site link to a session page goes through the entry
            // link (which checks access); anything else is refused.
            return if top_level_get(facts) {
                entry_link()
            } else {
                refuse(Refusal::Forbidden)
            };
        }
    }
    if !writes_from(facts, &ctx.hosts.session_origin(&seed)) {
        return refuse(Refusal::Forbidden);
    }
    let route = match path {
        SessionPath::Enter => return Decision::Public(PublicRoute::Enter { seed }),
        SessionPath::Comments => SessionRoute::Comments,
        SessionPath::Status => SessionRoute::Status,
        SessionPath::Messages => SessionRoute::Messages,
        SessionPath::Proxy => SessionRoute::Proxy,
    };
    let Some(user) = cookie_user(facts, CookieScope::Session(seed)) else {
        return match route {
            SessionRoute::Comments | SessionRoute::Status | SessionRoute::Messages => {
                refuse(Refusal::Unauthorized)
            }
            SessionRoute::Proxy if top_level_get(facts) => entry_link(),
            SessionRoute::Proxy => refuse(Refusal::Unauthorized),
        };
    };
    if !may_view(ctx, &seed, user) {
        return refuse(Refusal::NotFound);
    }
    Decision::Session {
        route,
        access: Access { user, seed },
    }
}

#[cfg(test)]
mod cross_site_tests;
#[cfg(test)]
mod matrix_tests;
#[cfg(test)]
mod tests;
