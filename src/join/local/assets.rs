//! The joiner's embedded frontend and its HTTP representations.

use std::sync::OnceLock;

use brotli_decompressor::BrotliDecompress;
use bytes::Bytes;
use hyper::{
    Response, StatusCode,
    header::{self, HeaderMap, HeaderValue},
};

use crate::adapters::http::{AppBody, RequestError, full_body};

macro_rules! embed {
    ($name:literal) => {
        Asset {
            compressed: include_bytes!(concat!(env!("OUT_DIR"), "/", $name, ".br")),
            identity: OnceLock::new(),
        }
    };
}

// The client always speaks this joiner's page API.
pub(super) static CLIENT: Asset = embed!("client.js");
pub(super) static FAVICON: Asset = embed!("favicon.svg");
pub(super) static PDF_SCRIPT: Asset = embed!("pdf.js");
pub(super) static PDF_WORKER: Asset = embed!("pdf-worker.js");
pub(super) static PDF_STYLE: Asset = embed!("pdf.css");
pub(super) static PDF_VIEWER: Asset = embed!("pdf.html");
pub(super) static PDF_CMAPS: Asset = embed!("pdf-cmaps.bin");

pub(super) const JAVASCRIPT: &str = "text/javascript; charset=utf-8";

pub(super) struct Asset {
    compressed: &'static [u8],
    identity: OnceLock<Vec<u8>>,
}

impl Asset {
    pub(super) fn identity(&self) -> &[u8] {
        self.identity.get_or_init(|| {
            let mut data = Vec::new();
            let mut compressed = self.compressed;
            BrotliDecompress(&mut compressed, &mut data)
                .expect("embedded Brotli asset must decompress");
            data
        })
    }

    pub(super) fn response(
        &'static self,
        request: &HeaderMap,
        content_type: &'static str,
    ) -> Response<AppBody> {
        let mut response = match negotiate(request) {
            Some(Encoding::Brotli) => {
                let mut response = asset_response(self.compressed, content_type);
                response
                    .headers_mut()
                    .insert(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
                response
            }
            Some(Encoding::Identity) => asset_response(self.identity(), content_type),
            None => RequestError::new(
                StatusCode::NOT_ACCEPTABLE,
                "neither Brotli nor identity encoding is acceptable",
            )
            .into(),
        };
        response
            .headers_mut()
            .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
        response
    }
}

pub(super) fn asset_response(data: &'static [u8], content_type: &'static str) -> Response<AppBody> {
    // Hyper derives Content-Length from this exact-size body for GET and HEAD.
    let mut response = Response::new(full_body(Bytes::from_static(data)));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// How an embedded asset is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Brotli,
    Identity,
}

/// Picks the representation by `Accept-Encoding` weights (RFC 9110
/// §12.5.3), preferring Brotli on a tie; `None` if both are excluded.
/// Without the header any coding is acceptable, but identity is sent, as
/// clients that omit it rarely decode Brotli. An entry with a malformed
/// weight is ignored.
fn negotiate(headers: &HeaderMap) -> Option<Encoding> {
    let mut values = headers.get_all(header::ACCEPT_ENCODING).iter().peekable();
    if values.peek().is_none() {
        return Some(Encoding::Identity);
    }
    let (mut brotli, mut identity, mut wildcard) = (None, None, None);
    // Split before decoding, so one malformed entry drops only itself.
    for entry in values
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b','))
        .filter_map(|entry| std::str::from_utf8(entry).ok())
    {
        let (coding, weight) = match entry.split_once(';') {
            Some((coding, parameter)) => (coding.trim(), weight(parameter)),
            None => (entry.trim(), Some(1000)),
        };
        let Some(weight) = weight else { continue };
        if coding.eq_ignore_ascii_case("br") {
            brotli = Some(weight);
        } else if coding.eq_ignore_ascii_case("identity") {
            identity = Some(weight);
        } else if coding == "*" {
            wildcard = Some(weight);
        }
    }
    // Identity is acceptable unless excluded, explicitly or by `*`; left
    // unweighted, it ranks below any coding the client lists.
    let brotli = brotli.or(wildcard).unwrap_or(0);
    let identity = identity.or(wildcard).unwrap_or(1);
    match (brotli, identity) {
        (0, 0) => None,
        (brotli, identity) if brotli >= identity => Some(Encoding::Brotli),
        _ => Some(Encoding::Identity),
    }
}

/// A `q=` weight in thousandths, `None` if malformed.
fn weight(parameter: &str) -> Option<u16> {
    let (name, value) = parameter.trim().split_once('=')?;
    if !name.trim().eq_ignore_ascii_case("q") {
        return None;
    }
    let (whole, fraction) = value.trim().split_once('.').unwrap_or((value.trim(), ""));
    if fraction.len() > 3 || !fraction.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let thousandths = format!("{fraction:0<3}").parse::<u16>().ok()?;
    match whole {
        "0" => Some(thousandths),
        "1" if thousandths == 0 => Some(1000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use hyper::header::{self, HeaderMap, HeaderValue};

    use crate::join::local::assets::{
        CLIENT, Encoding, PDF_CMAPS, PDF_SCRIPT, PDF_STYLE, PDF_VIEWER, PDF_WORKER, negotiate,
    };

    #[test]
    fn embedded_assets_round_trip() {
        for (asset, raw) in [
            (
                &CLIENT,
                include_bytes!("../../../assets/client.js").as_slice(),
            ),
            (
                &PDF_SCRIPT,
                include_bytes!("../../../assets/pdf.js").as_slice(),
            ),
            (
                &PDF_WORKER,
                include_bytes!("../../../assets/pdf-worker.js").as_slice(),
            ),
            (
                &PDF_STYLE,
                include_bytes!("../../../assets/pdf.css").as_slice(),
            ),
            (
                &PDF_VIEWER,
                include_bytes!("../../../assets/pdf.html").as_slice(),
            ),
            (
                &PDF_CMAPS,
                include_bytes!("../../../assets/pdf-cmaps.bin").as_slice(),
            ),
        ] {
            assert_eq!(asset.identity(), raw);
        }
    }

    #[test]
    fn negotiates_by_weight() {
        use Encoding::{Brotli, Identity};
        for (values, expected) in [
            (&[][..], Some(Identity)),
            (&[""][..], Some(Identity)),
            (&["gzip, deflate"][..], Some(Identity)),
            (&["gzip, BR; q=0.001"][..], Some(Brotli)),
            (&["gzip", "br; q=1.000"][..], Some(Brotli)),
            (&["gzip, *;q=0.5"][..], Some(Brotli)),
            (&["*", "br;q=0.000"][..], Some(Identity)),
            (&["br;q=bogus"][..], Some(Identity)),
            (&["br;q=1.001"][..], Some(Identity)),
            (&["identity;q=1, br;q=0.1"][..], Some(Identity)),
            (&["identity;q=1", "br;q=0.1"][..], Some(Identity)),
            (&["br;q=0.5, identity;q=0.5"][..], Some(Brotli)),
            (&["identity;q=0"][..], None),
            (&["br;q=0, identity;q=0"][..], None),
            (&["br;q=0", "identity;q=0"][..], None),
            (&["*;q=0"][..], None),
            (&["*;q=0, br"][..], Some(Brotli)),
            (&["*;q=0, identity"][..], Some(Identity)),
        ] {
            let mut headers = HeaderMap::new();
            for value in values {
                headers.append(header::ACCEPT_ENCODING, HeaderValue::from_static(value));
            }
            assert_eq!(negotiate(&headers), expected, "{values:?}");
        }
    }

    #[test]
    fn a_malformed_entry_drops_only_itself() {
        let mut headers = HeaderMap::new();
        headers.append(
            header::ACCEPT_ENCODING,
            HeaderValue::from_bytes(b"x\xff, *;q=0").unwrap(),
        );
        assert_eq!(negotiate(&headers), None);
    }
}
