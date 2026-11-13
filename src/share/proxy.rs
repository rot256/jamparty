//! Forwards requests to the one dev server the share was given.
//!
//! The dev server sees requests as from a browser on its own origin: a
//! request from the session's page (which the joiner checked) carries the
//! dev server's origin in `Origin` and `Referer`, and no forwarding
//! headers, so its own origin checks (CSRF, WebSocket origin, host checks)
//! pass. A CORS answer for its own origin is for the session's.

use std::{error::Error, fmt, str::FromStr};

use hyper::{
    Request, Response, StatusCode, Uri,
    body::Incoming,
    header::{
        self, ACCESS_CONTROL_ALLOW_ORIGIN, CONNECTION, HOST, HeaderMap, HeaderName, HeaderValue,
        LOCATION, ORIGIN, REFERER, TE, TRAILER, TRANSFER_ENCODING, UPGRADE,
    },
    upgrade,
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo},
};
use tokio::io::copy_bidirectional;
use url::Url;

use crate::{
    adapters::http::{AppBody, RequestError, incoming_body, map_incoming_response},
    core::{ids::Sid, port::Port},
    share::inject::{ClientMode, maybe_inject_client},
};

/// An `http://` or `https://` base URL without credentials, query or
/// fragment, on a port other than 0; its path always ends with `/`.
#[derive(Clone, Debug)]
pub(crate) struct UpstreamUrl {
    url: Url,
    /// The effective port (the scheme's default if the URL names none).
    port: Port,
}

impl UpstreamUrl {
    /// The port the dev server listens on.
    #[must_use]
    pub(crate) fn port(&self) -> Port {
        self.port
    }
}

/// Why a string is not an [`UpstreamUrl`].
#[derive(Debug)]
pub(crate) enum UpstreamUrlError {
    Parse(url::ParseError),
    Scheme,
    NoHost,
    Credentials,
    QueryOrFragment,
    PortZero,
}

impl fmt::Display for UpstreamUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::Scheme => f.write_str("upstream must use http:// or https://"),
            Self::NoHost => f.write_str("upstream must include a host"),
            Self::Credentials => f.write_str("upstream credentials are not supported"),
            Self::QueryOrFragment => f.write_str("upstream must not include a query or fragment"),
            Self::PortZero => f.write_str("upstream port must not be 0"),
        }
    }
}

impl Error for UpstreamUrlError {}

impl FromStr for UpstreamUrl {
    type Err = UpstreamUrlError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let mut upstream = Url::parse(raw).map_err(UpstreamUrlError::Parse)?;
        if !matches!(upstream.scheme(), "http" | "https") {
            return Err(UpstreamUrlError::Scheme);
        }
        if upstream.host_str().is_none() {
            return Err(UpstreamUrlError::NoHost);
        }
        if !upstream.username().is_empty() || upstream.password().is_some() {
            return Err(UpstreamUrlError::Credentials);
        }
        if upstream.query().is_some() || upstream.fragment().is_some() {
            return Err(UpstreamUrlError::QueryOrFragment);
        }
        let port = upstream
            .port_or_known_default()
            .and_then(|port| Port::try_from(port).ok())
            .ok_or(UpstreamUrlError::PortZero)?;
        if !upstream.path().ends_with('/') {
            let path = format!("{}/", upstream.path());
            upstream.set_path(&path);
        }
        Ok(Self {
            url: upstream,
            port,
        })
    }
}

impl fmt::Display for UpstreamUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.url.fmt(f)
    }
}

/// The origin of the session a request came from,
/// `http://<sid>.localhost[:<port>]`, read from its `Host`, which the
/// joiner checked against the port it serves on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionOrigin(String);

impl SessionOrigin {
    /// The origin `headers`' `Host` names, if it is one of the session
    /// `sid`'s.
    #[must_use]
    pub(crate) fn new(sid: &Sid, headers: &HeaderMap) -> Option<Self> {
        let host = headers.get(HOST)?.to_str().ok()?;
        let name = format!("{sid}.localhost");
        let rest = host.strip_prefix(&name)?;
        let valid = rest.is_empty()
            || rest.strip_prefix(':').is_some_and(|port| {
                port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<Port>().is_ok()
            });
        valid.then(|| Self(format!("http://{host}")))
    }
}

impl From<SessionOrigin> for String {
    fn from(origin: SessionOrigin) -> Self {
        origin.0
    }
}

#[derive(Clone)]
pub(crate) struct Proxy {
    upstream: Url,
    port: Port,
    /// The session's host label: its pages' origins.
    sid: Sid,
    client: Client<HttpsConnector<HttpConnector>, AppBody>,
}

impl Proxy {
    pub(crate) fn new(upstream: UpstreamUrl, sid: Sid) -> Self {
        let connector = HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        Self {
            port: upstream.port(),
            upstream: upstream.url,
            sid,
            client: Client::builder(TokioExecutor::new()).build(connector),
        }
    }

    /// The dev server's port.
    #[must_use]
    pub(crate) fn port(&self) -> Port {
        self.port
    }

    pub(crate) async fn handle(
        &self,
        mut request: Request<Incoming>,
    ) -> Result<Response<AppBody>, RequestError> {
        let upstream = &self.upstream;
        let downstream_upgrade = request
            .headers()
            .contains_key(header::UPGRADE)
            .then(|| upgrade::on(&mut request));
        let session = SessionOrigin::new(&self.sid, request.headers());
        let (mut parts, body) = request.into_parts();
        let hop = match downstream_upgrade {
            Some(_) => Hop::Upgrade,
            None => Hop::Plain,
        };
        strip_hop_by_hop(&mut parts.headers, hop);
        let request_method = parts.method.clone();
        parts.uri = upstream_uri(upstream, &parts.uri)?;
        parts.headers.insert(
            HOST,
            header::HeaderValue::from_str(upstream.authority())
                .map_err(|_| RequestError::new(StatusCode::BAD_GATEWAY, "invalid upstream host"))?,
        );
        parts.headers.insert(
            header::ACCEPT_ENCODING,
            header::HeaderValue::from_static("identity"),
        );
        translate_request(&mut parts.headers, session.as_ref(), upstream);

        let outgoing = Request::from_parts(parts, incoming_body(body));
        let mut response = self.client.request(outgoing).await.map_err(|error| {
            tracing::warn!(%error, "upstream request failed");
            RequestError::new(StatusCode::BAD_GATEWAY, "upstream request failed")
        })?;

        translate_response(response.headers_mut(), session.as_ref(), upstream);
        let hop = if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            Hop::Upgrade
        } else {
            Hop::Plain
        };
        strip_hop_by_hop(response.headers_mut(), hop);

        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            if let Some(downstream_upgrade) = downstream_upgrade {
                let upstream_upgrade = upgrade::on(&mut response);
                tokio::spawn(async move {
                    match tokio::try_join!(downstream_upgrade, upstream_upgrade) {
                        Ok((downstream, upstream)) => {
                            let mut downstream = TokioIo::new(downstream);
                            let mut upstream = TokioIo::new(upstream);
                            if let Err(error) =
                                copy_bidirectional(&mut downstream, &mut upstream).await
                            {
                                tracing::debug!(%error, "upgraded tunnel closed with an error");
                            }
                        }
                        Err(error) => tracing::debug!(%error, "upgrade failed"),
                    }
                });
            }
            return Ok(map_incoming_response(response));
        }

        maybe_inject_client(
            map_incoming_response(response),
            &request_method,
            ClientMode::Edit,
        )
        .await
    }
}

/// The upstream URL for `incoming`: its path under the upstream's base
/// path. Paths with `.` or `..` segments (also percent-encoded) are
/// refused: resolving them could leave the base path.
fn upstream_uri(upstream: &Url, incoming: &Uri) -> Result<Uri, RequestError> {
    let bad_path = || RequestError::new(StatusCode::BAD_REQUEST, "invalid request path");
    let incoming_path = incoming.path();
    let dot_segment = incoming_path.split('/').any(|segment| {
        let decoded = percent_encoding::percent_decode_str(segment).collect::<Vec<u8>>();
        matches!(decoded.as_slice(), b"." | b"..")
    });
    if dot_segment {
        return Err(bad_path());
    }
    let mut target = upstream.clone();
    let base = upstream.path().trim_end_matches('/');
    let path = if base.is_empty() {
        incoming_path.to_owned()
    } else {
        format!("{base}{incoming_path}")
    };
    target.set_path(&path);
    if !target.path().starts_with(upstream.path()) {
        return Err(bad_path());
    }
    target.set_query(incoming.query());
    target
        .as_str()
        .parse()
        .map_err(|_| RequestError::new(StatusCode::BAD_GATEWAY, "could not construct upstream URL"))
}

/// Whether a message's `Upgrade` survives [`strip_hop_by_hop`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hop {
    /// Everything hop-by-hop goes.
    Plain,
    /// An upgrade in progress: `Upgrade` and `Connection: upgrade` stay.
    Upgrade,
}

/// Removes hop-by-hop headers (RFC 9110, 7.6.1) and those `Connection`
/// names; [`Hop::Upgrade`] keeps `Upgrade` and `Connection: upgrade`.
fn strip_hop_by_hop(headers: &mut HeaderMap, hop: Hop) {
    let kept = match hop {
        Hop::Plain => None,
        Hop::Upgrade => headers.get(UPGRADE).cloned(),
    };
    let named: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in [
        CONNECTION,
        HeaderName::from_static("keep-alive"),
        HeaderName::from_static("proxy-connection"),
        TE,
        TRAILER,
        TRANSFER_ENCODING,
        UPGRADE,
    ] {
        headers.remove(name);
    }
    if let Some(value) = kept {
        headers.insert(UPGRADE, value);
        headers.insert(CONNECTION, HeaderValue::from_static("upgrade"));
    }
}

/// Makes a request from the session's page one from the dev server's own
/// origin: `Origin` and a same-origin `Referer` name the dev server, and
/// forwarding headers (the client's too) go. A foreign or `null` `Origin`
/// stays as it is; none is invented.
fn translate_request(headers: &mut HeaderMap, session: Option<&SessionOrigin>, upstream: &Url) {
    let forwarding: Vec<HeaderName> = headers
        .keys()
        .filter(|name| name.as_str() == "forwarded" || name.as_str().starts_with("x-forwarded-"))
        .cloned()
        .collect();
    for name in forwarding {
        headers.remove(name);
    }
    let Some(SessionOrigin(session)) = session else {
        return;
    };
    let own = upstream.origin().ascii_serialization();
    if headers
        .get(ORIGIN)
        .is_some_and(|origin| origin == session.as_str())
        && let Ok(value) = HeaderValue::from_str(&own)
    {
        headers.insert(ORIGIN, value);
    }
    let referer = headers
        .get(REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Url::parse(value).ok())
        .filter(|referer| referer.origin().ascii_serialization() == *session);
    if let Some(referer) = referer {
        let path = referer.path();
        let mapped = match referer.query() {
            Some(query) => format!("{path}?{query}"),
            None => path.to_owned(),
        };
        // Unmappable (dot segments): better none than the session's.
        match mapped
            .parse::<Uri>()
            .ok()
            .and_then(|uri| upstream_uri(upstream, &uri).ok())
            .and_then(|uri| HeaderValue::from_str(&uri.to_string()).ok())
        {
            Some(value) => headers.insert(REFERER, value),
            None => headers.remove(REFERER),
        };
    }
}

/// Makes the dev server's answer one for the session: redirects to its
/// own origin stay in the session, and a CORS grant to its own origin is
/// for the session's.
fn translate_response(headers: &mut HeaderMap, session: Option<&SessionOrigin>, upstream: &Url) {
    rewrite_redirect(headers, upstream);
    let own = upstream.origin().ascii_serialization();
    if let Some(SessionOrigin(session)) = session
        && headers
            .get(ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_some_and(|allowed| allowed == own.as_str())
        && let Ok(value) = HeaderValue::from_str(session)
    {
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
}

fn rewrite_redirect(headers: &mut header::HeaderMap, upstream: &Url) {
    let Some(value) = headers.get(LOCATION) else {
        return;
    };
    let Ok(location) = value.to_str() else {
        return;
    };
    // Absolute, or scheme-relative (`//host/path`); relative ones are
    // relative to the request's URL, which the browser resolves.
    let absolute = match location.strip_prefix("//") {
        Some(rest) => format!("{}://{rest}", upstream.scheme()),
        None => location.to_owned(),
    };
    let Ok(url) = Url::parse(&absolute) else {
        return;
    };
    if url.origin() != upstream.origin() {
        return;
    }
    let upstream_base = upstream.path().trim_end_matches('/');
    let redirected_path = url.path();
    let proxy_path = if upstream_base.is_empty() {
        redirected_path
    } else if redirected_path == upstream_base {
        "/"
    } else {
        redirected_path
            .strip_prefix(upstream_base)
            .filter(|suffix| suffix.starts_with('/'))
            .unwrap_or(redirected_path)
    };
    let mut rewritten = proxy_path.to_owned();
    if let Some(query) = url.query() {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    if let Some(fragment) = url.fragment() {
        rewritten.push('#');
        rewritten.push_str(fragment);
    }
    if let Ok(value) = header::HeaderValue::from_str(&rewritten) {
        headers.insert(LOCATION, value);
    }
}

#[cfg(test)]
mod tests {
    use hyper::header::{HeaderMap, HeaderValue, LOCATION};

    use hyper::{StatusCode, Uri};

    use crate::{
        core::ids::{EndpointKey, Sid},
        share::proxy::{
            Hop, SessionOrigin, UpstreamUrl, rewrite_redirect, strip_hop_by_hop, translate_request,
            translate_response, upstream_uri,
        },
    };

    /// The session's host label (`EndpointKey([0; 32])`'s).
    const SID: &str = "tzrjdfymwrg5sqai";

    fn sid() -> Sid {
        EndpointKey([0; 32]).sid()
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(name, value)| (name.parse().unwrap(), HeaderValue::from_static(value)))
            .collect()
    }

    fn get<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).map(|value| value.to_str().unwrap())
    }

    /// What the dev server is sent for a request with `pairs`, `Host`
    /// included, to `upstream`.
    fn translated(upstream: &str, pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let upstream = url::Url::parse(upstream).unwrap();
        let mut sent = headers(pairs);
        let session = SessionOrigin::new(&sid(), &sent);
        translate_request(&mut sent, session.as_ref(), &upstream);
        sent
    }

    #[test]
    fn the_session_origin_is_read_from_its_host() {
        assert_eq!(SID, sid().as_str());
        let origin = |host: &'static str| {
            SessionOrigin::new(&sid(), &headers(&[("host", host)])).map(String::from)
        };
        assert_eq!(
            origin("tzrjdfymwrg5sqai.localhost:1313").as_deref(),
            Some("http://tzrjdfymwrg5sqai.localhost:1313")
        );
        assert_eq!(
            origin("tzrjdfymwrg5sqai.localhost").as_deref(),
            Some("http://tzrjdfymwrg5sqai.localhost")
        );
        for other in [
            "aaaaaaaaaaaaaaaa.localhost:1313",
            "tzrjdfymwrg5sqai.localhost:0",
            "tzrjdfymwrg5sqai.localhost:x",
            "tzrjdfymwrg5sqai.localhost:1313@evil.example",
            "evil.example",
            "127.0.0.1:1313",
        ] {
            assert_eq!(origin(other), None, "{other}");
        }
        assert!(SessionOrigin::new(&sid(), &HeaderMap::new()).is_none());
    }

    #[test]
    fn the_dev_server_sees_its_own_origin() {
        let sent = translated(
            "http://127.0.0.2:1313/",
            &[
                ("host", "tzrjdfymwrg5sqai.localhost:1313"),
                ("origin", "http://tzrjdfymwrg5sqai.localhost:1313"),
                ("referer", "http://tzrjdfymwrg5sqai.localhost:1313/a/b?x=1"),
                ("forwarded", "for=192.0.2.1;host=evil.example"),
                ("x-forwarded-host", "evil.example"),
                ("x-forwarded-proto", "https"),
                ("x-forwarded-for", "192.0.2.1"),
                ("x-kept", "1"),
            ],
        );
        assert_eq!(get(&sent, "origin"), Some("http://127.0.0.2:1313"));
        assert_eq!(get(&sent, "referer"), Some("http://127.0.0.2:1313/a/b?x=1"));
        for dropped in [
            "forwarded",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-forwarded-for",
        ] {
            assert_eq!(get(&sent, dropped), None, "{dropped}");
        }
        assert_eq!(get(&sent, "x-kept"), Some("1"));
    }

    #[test]
    fn a_base_path_and_the_default_port_carry_over() {
        let sent = translated(
            "http://localhost/app/",
            &[
                ("host", "tzrjdfymwrg5sqai.localhost"),
                ("origin", "http://tzrjdfymwrg5sqai.localhost"),
                ("referer", "http://tzrjdfymwrg5sqai.localhost/docs/"),
            ],
        );
        assert_eq!(get(&sent, "origin"), Some("http://localhost"));
        assert_eq!(get(&sent, "referer"), Some("http://localhost/app/docs/"));
    }

    #[test]
    fn foreign_or_missing_origins_are_not_translated() {
        let host = ("host", "tzrjdfymwrg5sqai.localhost:1313");
        let upstream = "http://127.0.0.2:1313/";
        for foreign in [
            "http://evil.example",
            "null",
            "https://tzrjdfymwrg5sqai.localhost:1313",
            "http://tzrjdfymwrg5sqai.localhost:1314",
        ] {
            let sent = translated(upstream, &[host, ("origin", foreign)]);
            assert_eq!(get(&sent, "origin"), Some(foreign));
        }
        let sent = translated(upstream, &[host, ("referer", "http://evil.example/x")]);
        assert_eq!(get(&sent, "referer"), Some("http://evil.example/x"));
        // None is invented.
        let sent = translated(upstream, &[host]);
        assert_eq!(get(&sent, "origin"), None);
        assert_eq!(get(&sent, "referer"), None);
        // Not this session's host: nothing is translated, forwarding
        // headers still go.
        let sent = translated(
            upstream,
            &[
                ("host", "aaaaaaaaaaaaaaaa.localhost:1313"),
                ("origin", "http://aaaaaaaaaaaaaaaa.localhost:1313"),
                ("x-forwarded-host", "evil.example"),
            ],
        );
        assert_eq!(
            get(&sent, "origin"),
            Some("http://aaaaaaaaaaaaaaaa.localhost:1313")
        );
        assert_eq!(get(&sent, "x-forwarded-host"), None);
    }

    #[test]
    fn cors_answers_for_the_dev_server_are_for_the_session() {
        let upstream = url::Url::parse("http://127.0.0.2:1313/").unwrap();
        let session = SessionOrigin::new(
            &sid(),
            &headers(&[("host", "tzrjdfymwrg5sqai.localhost:1313")]),
        );
        let answered = |allowed: &'static str| {
            let mut received = headers(&[("access-control-allow-origin", allowed)]);
            translate_response(&mut received, session.as_ref(), &upstream);
            get(&received, "access-control-allow-origin").map(str::to_owned)
        };
        assert_eq!(
            answered("http://127.0.0.2:1313").as_deref(),
            Some("http://tzrjdfymwrg5sqai.localhost:1313")
        );
        assert_eq!(answered("*").as_deref(), Some("*"));
        assert_eq!(
            answered("http://elsewhere.example").as_deref(),
            Some("http://elsewhere.example")
        );
    }

    #[test]
    fn hop_by_hop_headers_are_dropped() {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("connection", "keep-alive, x-private"),
            ("keep-alive", "timeout=5"),
            ("x-private", "1"),
            ("te", "trailers"),
            ("transfer-encoding", "chunked"),
            ("upgrade", "websocket"),
            ("x-kept", "1"),
        ] {
            headers.insert(name, HeaderValue::from_static(value));
        }
        let mut plain = headers.clone();
        strip_hop_by_hop(&mut plain, Hop::Plain);
        let names: Vec<&str> = plain.keys().map(|name| name.as_str()).collect();
        assert_eq!(names, ["x-kept"]);

        strip_hop_by_hop(&mut headers, Hop::Upgrade);
        assert_eq!(headers.get("upgrade").unwrap(), "websocket");
        assert_eq!(headers.get("connection").unwrap(), "upgrade");
        assert!(headers.get("x-private").is_none());
        assert_eq!(headers.len(), 3);
    }

    #[test]
    fn requests_stay_under_the_base_path() {
        let upstream = url::Url::parse("http://localhost:3000/app/").unwrap();
        let target = |path: &str| upstream_uri(&upstream, &path.parse::<Uri>().unwrap());
        assert_eq!(
            target("/page?q=1").unwrap().to_string(),
            "http://localhost:3000/app/page?q=1"
        );
        assert_eq!(
            target("/").unwrap().to_string(),
            "http://localhost:3000/app/"
        );
        assert_eq!(
            target("/a%2fb/..x").unwrap().to_string(),
            "http://localhost:3000/app/a%2fb/..x"
        );
        for escape in [
            "/../admin",
            "/%2e%2e/admin",
            "/%2E%2e/admin",
            "/.%2e/admin",
            "/a/../../admin",
            "/./x",
            "/x/%2e",
        ] {
            let refused = target(escape).unwrap_err();
            assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{escape}");
        }
        let root = url::Url::parse("http://localhost:3000/").unwrap();
        assert!(upstream_uri(&root, &"/../etc".parse().unwrap()).is_err());
    }

    #[test]
    fn validates_upstream() {
        assert!("http://localhost:3000".parse::<UpstreamUrl>().is_ok());
        assert!("https://localhost:3000".parse::<UpstreamUrl>().is_ok());
        assert!("ftp://localhost:3000".parse::<UpstreamUrl>().is_err());
        assert!(
            "http://user:pass@localhost:3000"
                .parse::<UpstreamUrl>()
                .is_err()
        );
        assert_eq!(
            "http://localhost:3000/app"
                .parse::<UpstreamUrl>()
                .unwrap()
                .to_string(),
            "http://localhost:3000/app/"
        );
        assert!("http://localhost:0".parse::<UpstreamUrl>().is_err());
    }

    #[test]
    fn the_upstream_port_is_its_effective_port() {
        let port = |raw: &str| u16::from(raw.parse::<UpstreamUrl>().unwrap().port());
        assert_eq!(port("http://localhost:3000/app"), 3000);
        assert_eq!(port("http://localhost"), 80);
        assert_eq!(port("https://localhost/"), 443);
        assert_eq!(port("http://127.0.0.1:443"), 443);
    }

    #[test]
    fn rewrites_only_redirects_to_the_upstream_origin() {
        let upstream = url::Url::parse("http://127.0.0.1:8000").expect("URL");
        let mut headers = HeaderMap::new();
        headers.insert(
            LOCATION,
            HeaderValue::from_static("http://127.0.0.1:8000/next?q=1#part"),
        );
        rewrite_redirect(&mut headers, &upstream);
        assert_eq!(headers.get(LOCATION).expect("location"), "/next?q=1#part");

        let upstream_with_base = url::Url::parse("http://127.0.0.1:8000/base/").expect("URL");
        headers.insert(
            LOCATION,
            HeaderValue::from_static("http://127.0.0.1:8000/base/next"),
        );
        rewrite_redirect(&mut headers, &upstream_with_base);
        assert_eq!(headers.get(LOCATION).expect("location"), "/next");

        headers.insert(LOCATION, HeaderValue::from_static("http://elsewhere/next"));
        rewrite_redirect(&mut headers, &upstream);
        assert_eq!(
            headers.get(LOCATION).expect("location"),
            "http://elsewhere/next"
        );

        // Scheme-relative, as some servers write them.
        headers.insert(LOCATION, HeaderValue::from_static("//127.0.0.1:8000/next"));
        rewrite_redirect(&mut headers, &upstream);
        assert_eq!(headers.get(LOCATION).expect("location"), "/next");
        headers.insert(LOCATION, HeaderValue::from_static("//elsewhere/next"));
        rewrite_redirect(&mut headers, &upstream);
        assert_eq!(headers.get(LOCATION).expect("location"), "//elsewhere/next");
        // Relative ones are left to the browser.
        headers.insert(LOCATION, HeaderValue::from_static("next"));
        rewrite_redirect(&mut headers, &upstream);
        assert_eq!(headers.get(LOCATION).expect("location"), "next");
    }
}
