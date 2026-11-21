//! The joiner's local request decision: one pure function from what a
//! request carries to a route or a refusal.
//!
//! - `Host` must be exactly `<sid>.localhost:<port>` (defeats DNS
//!   rebinding); `<sid>.localhost` on port 80.
//! - Requests need `Sec-Fetch-Site: same-origin` or no `Sec-Fetch-Site`
//!   (non-browser clients). Top-level document navigations (GET/HEAD,
//!   `Sec-Fetch-Mode: navigate`, `Sec-Fetch-Dest: document`) also allow
//!   `Sec-Fetch-Site: none` (address bar, bookmarks and browser launches).
//! - Non-GET/HEAD requests and upgrades also need `Origin` to be exactly
//!   `http://<sid>.localhost:<port>`; other non-navigation requests with an
//!   `Origin` need that one too.
//!
//! Paths are matched exactly as received (no decoding or normalisation).
//! The joiner answers `/_jamparty/client.js` (GET or HEAD) itself, so a
//! page's client is always its joiner's. Everything under `/_jamparty/api/`
//! is the joiner's own too: the page's socket (GET with an upgrade), the
//! attachment upload (POST), and a 404 for any other path in the prefix.
//! Top-level PDF navigations and the viewer's assets are local too.
//! Everything else goes to the share.

use crate::{
    core::{ids::Sid, port::Port},
    protocol::resource::UPLOAD_PATH,
};

/// Every response carries `Content-Security-Policy` with this value.
pub(crate) const FRAME_ANCESTORS: &str = "frame-ancestors 'self'";

/// The joiner’s PDF viewer assets.
pub(crate) const PDF_VIEWER_PATH: &str = "/_jamparty/pdf.js";
pub(crate) const PDF_WORKER_PATH: &str = "/_jamparty/pdf-worker.js";
pub(crate) const PDF_STYLE_PATH: &str = "/_jamparty/pdf.css";

pub(crate) const PDF_CMAP_PREFIX: &str = "/_jamparty/pdf-cmaps/";

/// The browser client the share's pages load.
pub(crate) const CLIENT_PATH: &str = "/_jamparty/client.js";
/// The joiner's API paths start with this.
const API_PREFIX: &str = "/_jamparty/api/";
pub(crate) const SOCKET_PATH: &str = "/_jamparty/api/socket";

/// The request method, as far as the decision cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Head,
    Post,
    /// Any other method (PUT, DELETE, OPTIONS, ...).
    Other,
}

impl From<&str> for Method {
    fn from(name: &str) -> Self {
        match name {
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            _ => Self::Other,
        }
    }
}

/// Whether the request asks for a protocol upgrade (WebSocket, HMR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Upgrade {
    No,
    Yes,
}

/// What the request carries, as plain header values.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestFacts<'a> {
    pub(crate) host: Option<&'a str>,
    pub(crate) method: Method,
    /// The raw path, without the query.
    pub(crate) path: &'a str,
    pub(crate) sec_fetch_site: Option<&'a str>,
    pub(crate) sec_fetch_mode: Option<&'a str>,
    pub(crate) sec_fetch_dest: Option<&'a str>,
    pub(crate) origin: Option<&'a str>,
    pub(crate) upgrade: Upgrade,
}

/// Who answers an accepted request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// GET or HEAD the browser client.
    Client,
    PdfViewer,
    PdfScript,
    PdfWorker,
    PdfStyle,
    PdfCMap,
    /// GET with an upgrade: the page's WebSocket.
    Socket,
    /// POST the bytes of one comment attachment.
    Upload,
    /// Another path under [`API_PREFIX`]: not found, and not the share's.
    Unknown,
    /// Forward to the share.
    Proxy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// `Host` is not this session's (wrong name, port, or missing).
    WrongHost,
    /// Fetch metadata does not permit the request.
    CrossSite,
    /// A foreign `Origin`, or a write or upgrade without an `Origin`.
    BadOrigin,
    /// A reserved path with the wrong method, or with an upgrade or
    /// without one when it takes the other.
    MethodNotAllowed,
}

impl Refusal {
    /// The HTTP status code to answer with.
    #[must_use]
    pub(crate) fn status(self) -> u16 {
        match self {
            Self::WrongHost => 421,
            Self::CrossSite | Self::BadOrigin => 403,
            Self::MethodNotAllowed => 405,
        }
    }
}

/// The joiner's local origin: `<sid>.localhost:<port>`, or without the
/// port when it is 80, as browsers write it.
#[derive(Clone, Debug)]
pub(crate) struct LocalOrigin {
    host: String,
    origin: String,
}

impl LocalOrigin {
    #[must_use]
    pub(crate) fn new(sid: &Sid, port: Port) -> Self {
        let host = if port == Port::HTTP {
            format!("{sid}.localhost")
        } else {
            format!("{sid}.localhost:{port}")
        };
        let origin = format!("http://{host}");
        Self { host, origin }
    }

    /// The `Host` header value, also the address to open in the browser.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    /// The `Origin` header value, `http://<host>`.
    #[must_use]
    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    /// Decides who answers `facts`, or why it is refused.
    pub(crate) fn decide(&self, facts: &RequestFacts<'_>) -> Result<Route, Refusal> {
        if facts.host != Some(self.host.as_str()) {
            return Err(Refusal::WrongHost);
        }
        match facts.sec_fetch_site {
            None | Some("same-origin") => {}
            Some("none") if is_navigation(facts) => {}
            Some(_) => return Err(Refusal::CrossSite),
        }
        let writes = match facts.method {
            Method::Get | Method::Head => facts.upgrade == Upgrade::Yes,
            Method::Post | Method::Other => true,
        };
        let origin_ok = match facts.origin {
            Some(origin) => origin == self.origin,
            None => !writes,
        };
        if !origin_ok {
            return Err(Refusal::BadOrigin);
        }
        route(facts)
    }
}

/// A top-level document navigation by GET or HEAD.
pub(crate) fn is_navigation(facts: &RequestFacts<'_>) -> bool {
    let safe = match facts.method {
        Method::Get | Method::Head => true,
        Method::Post | Method::Other => false,
    };
    safe && facts.upgrade == Upgrade::No
        && facts.sec_fetch_mode == Some("navigate")
        && facts.sec_fetch_dest == Some("document")
}

/// Whether the raw path has a PDF extension.
pub(crate) fn is_pdf_path(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("pdf"))
}

fn route(facts: &RequestFacts<'_>) -> Result<Route, Refusal> {
    let (route, method, upgrade) = match facts.path {
        CLIENT_PATH | PDF_VIEWER_PATH | PDF_WORKER_PATH | PDF_STYLE_PATH => {
            let read = match facts.method {
                Method::Get | Method::Head => facts.upgrade == Upgrade::No,
                Method::Post | Method::Other => false,
            };
            return if read {
                Ok(match facts.path {
                    PDF_VIEWER_PATH => Route::PdfScript,
                    PDF_WORKER_PATH => Route::PdfWorker,
                    PDF_STYLE_PATH => Route::PdfStyle,
                    _ => Route::Client,
                })
            } else {
                Err(Refusal::MethodNotAllowed)
            };
        }
        path if path.starts_with(PDF_CMAP_PREFIX) => {
            return match facts.method {
                Method::Get | Method::Head if facts.upgrade == Upgrade::No => Ok(Route::PdfCMap),
                Method::Get | Method::Head | Method::Post | Method::Other => {
                    Err(Refusal::MethodNotAllowed)
                }
            };
        }
        UPLOAD_PATH => (Route::Upload, Method::Post, Upgrade::No),
        SOCKET_PATH => (Route::Socket, Method::Get, Upgrade::Yes),
        path if path.starts_with(API_PREFIX) => return Ok(Route::Unknown),
        path if is_navigation(facts) && is_pdf_path(path) => return Ok(Route::PdfViewer),
        _ => return Ok(Route::Proxy),
    };
    if facts.method == method && facts.upgrade == upgrade {
        Ok(route)
    } else {
        Err(Refusal::MethodNotAllowed)
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
