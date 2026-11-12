//! Forwards requests to the one dev server the share was given.

use std::{fmt, str::FromStr};

use hyper::{
    Request, Response, StatusCode, Uri,
    body::Incoming,
    header::{
        self, CONNECTION, HOST, HeaderMap, HeaderName, HeaderValue, LOCATION, TE, TRAILER,
        TRANSFER_ENCODING, UPGRADE,
    },
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo},
};
use tokio::io::copy_bidirectional;
use url::Url;

use crate::{
    BoxError,
    share::{
        body::{AppBody, RequestError, incoming_body, map_incoming_response},
        inject::{ClientMode, maybe_inject_client},
    },
};

/// An `http://` or `https://` base URL without credentials, query or
/// fragment; its path always ends with `/`.
#[derive(Clone, Debug)]
pub(crate) struct UpstreamUrl(Url);

impl FromStr for UpstreamUrl {
    type Err = BoxError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let mut upstream = Url::parse(raw)?;
        if !matches!(upstream.scheme(), "http" | "https") {
            return Err("upstream must use http:// or https://".into());
        }
        if upstream.host_str().is_none() {
            return Err("upstream must include a host".into());
        }
        if !upstream.username().is_empty() || upstream.password().is_some() {
            return Err("upstream credentials are not supported".into());
        }
        if upstream.query().is_some() || upstream.fragment().is_some() {
            return Err("upstream must not include a query or fragment".into());
        }
        if !upstream.path().ends_with('/') {
            let path = format!("{}/", upstream.path());
            upstream.set_path(&path);
        }
        Ok(Self(upstream))
    }
}

impl fmt::Display for UpstreamUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone)]
pub(crate) struct Proxy {
    upstream: Url,
    client: Client<HttpsConnector<HttpConnector>, AppBody>,
}

impl Proxy {
    pub(crate) fn new(upstream: UpstreamUrl) -> Self {
        let connector = HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        Self {
            upstream: upstream.0,
            client: Client::builder(TokioExecutor::new()).build(connector),
        }
    }

    pub(crate) async fn handle(
        &self,
        mut request: Request<Incoming>,
    ) -> Result<Response<AppBody>, RequestError> {
        let upstream = &self.upstream;
        let downstream_upgrade = request
            .headers()
            .contains_key(header::UPGRADE)
            .then(|| hyper::upgrade::on(&mut request));
        let original_host = request.headers().get(HOST).cloned();
        let (mut parts, body) = request.into_parts();
        strip_hop_by_hop(&mut parts.headers, downstream_upgrade.is_some());
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
        if let Some(host) = original_host {
            parts
                .headers
                .insert(header::HeaderName::from_static("x-forwarded-host"), host);
        }
        parts.headers.insert(
            header::HeaderName::from_static("x-forwarded-proto"),
            header::HeaderValue::from_static("http"),
        );

        let outgoing = Request::from_parts(parts, incoming_body(body));
        let mut response = self.client.request(outgoing).await.map_err(|error| {
            tracing::warn!(%error, "upstream request failed");
            RequestError::new(StatusCode::BAD_GATEWAY, "upstream request failed")
        })?;

        rewrite_redirect(response.headers_mut(), upstream);
        let upgraded = response.status() == StatusCode::SWITCHING_PROTOCOLS;
        strip_hop_by_hop(response.headers_mut(), upgraded);

        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            if let Some(downstream_upgrade) = downstream_upgrade {
                let upstream_upgrade = hyper::upgrade::on(&mut response);
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

/// Removes hop-by-hop headers (RFC 9110, 7.6.1) and those `Connection`
/// names; `upgrade` keeps `Upgrade` and `Connection: upgrade`.
fn strip_hop_by_hop(headers: &mut HeaderMap, upgrade: bool) {
    let kept = headers.get(UPGRADE).cloned().filter(|_| upgrade);
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

fn rewrite_redirect(headers: &mut header::HeaderMap, upstream: &Url) {
    let Some(value) = headers.get(LOCATION) else {
        return;
    };
    let Ok(location) = value.to_str() else {
        return;
    };
    let Ok(url) = Url::parse(location) else {
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

    use crate::share::proxy::{UpstreamUrl, rewrite_redirect, strip_hop_by_hop, upstream_uri};

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
        strip_hop_by_hop(&mut plain, false);
        let names: Vec<&str> = plain.keys().map(|name| name.as_str()).collect();
        assert_eq!(names, ["x-kept"]);

        strip_hop_by_hop(&mut headers, true);
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
    }
}
