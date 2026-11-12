//! Injects the browser client into HTML responses.

use std::borrow::Cow;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::{
    Method, Response, StatusCode,
    header::{self, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG},
};

use crate::share::body::{AppBody, RequestError, full_body};

const MAX_HTML_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// The client learns this jampot's version from its URL, to name it when
/// the joiner it talks to runs another.
const EDIT_SCRIPT: &[u8] = concat!(
    "\n<script type=\"module\" src=\"/_jampot/client.js?version=",
    env!("CARGO_PKG_VERSION"),
    "\"></script>\n"
)
.as_bytes();

/// How the client is loaded: plain, or with the live-reload revision the
/// page was served at (both carry this jampot's version).
#[derive(Clone, Copy, Debug)]
pub(crate) enum ClientMode {
    Edit,
    Live(u64),
}

pub(crate) async fn maybe_inject_client(
    response: Response<AppBody>,
    request_method: &Method,
    mode: ClientMode,
) -> Result<Response<AppBody>, RequestError> {
    let is_html = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/html"));
    let is_encoded = response.headers().contains_key(header::CONTENT_ENCODING);
    let status_allows_injection =
        response.status().is_success() && response.status() != StatusCode::PARTIAL_CONTENT;
    if !matches!(*request_method, Method::GET | Method::HEAD)
        || !status_allows_injection
        || !is_html
        || is_encoded
    {
        if is_html && is_encoded {
            tracing::warn!("encoded HTML was not annotated");
        }
        return Ok(response);
    }
    let content_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    // Streamed HTML (no length) passes through untouched, as does large HTML.
    let Some(content_length) = content_length.filter(|length| *length <= MAX_HTML_RESPONSE_BYTES)
    else {
        tracing::debug!("streamed or large HTML was not annotated");
        return Ok(response);
    };

    if request_method == Method::HEAD {
        let mut response = response;
        let annotated_length = content_length.checked_add(client_script(mode).len());
        annotate_headers(response.headers_mut(), annotated_length);
        return Ok(response);
    }

    let (mut parts, body) = response.into_parts();
    let body_error_status = match mode {
        ClientMode::Edit => StatusCode::BAD_GATEWAY,
        ClientMode::Live(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    // Bounded, whatever the body turns out to be: its Content-Length was
    // checked above, and the limit holds even if it lied.
    let read_error = |message: &str| {
        tracing::warn!(%message, "could not read HTML response");
        RequestError::new(body_error_status, "could not read HTML response")
    };
    let mut body = body;
    let mut bytes = Vec::with_capacity(content_length);
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| read_error(&error.to_string()))?;
        if let Ok(data) = frame.into_data() {
            if bytes.len() + data.len() > MAX_HTML_RESPONSE_BYTES {
                return Err(read_error("longer than its Content-Length"));
            }
            bytes.extend_from_slice(&data);
        }
    }

    let bytes = inject_client(&bytes, mode);
    annotate_headers(&mut parts.headers, Some(bytes.len()));
    Ok(Response::from_parts(parts, full_body(bytes)))
}

fn annotate_headers(headers: &mut header::HeaderMap, content_length: Option<usize>) {
    headers.remove(CONTENT_LENGTH);
    if let Some(content_length) = content_length
        && let Ok(value) = header::HeaderValue::from_str(&content_length.to_string())
    {
        headers.insert(CONTENT_LENGTH, value);
    }
    headers.remove(ETAG);
    headers.remove(header::CONTENT_ENCODING);
    headers.remove(header::TRANSFER_ENCODING);
    headers.insert(CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
}

fn client_script(mode: ClientMode) -> Cow<'static, [u8]> {
    match mode {
        ClientMode::Edit => Cow::Borrowed(EDIT_SCRIPT),
        ClientMode::Live(revision) => Cow::Owned(
            format!(
                "\n<script type=\"module\" src=\"/_jampot/client.js?live={revision}&amp;version={}\"></script>\n",
                env!("CARGO_PKG_VERSION")
            )
            .into_bytes(),
        ),
    }
}

fn inject_client(html: &[u8], mode: ClientMode) -> Bytes {
    let script = client_script(mode);
    let position = html
        .windows(b"</body>".len())
        .rposition(|window| window.eq_ignore_ascii_case(b"</body>"));
    let mut output = Vec::with_capacity(html.len() + script.len());
    if let Some(position) = position {
        output.extend_from_slice(html.get(..position).unwrap_or_default());
        output.extend_from_slice(&script);
        output.extend_from_slice(html.get(position..).unwrap_or_default());
    } else {
        output.extend_from_slice(html);
        output.extend_from_slice(&script);
    }
    Bytes::from(output)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::{Method, Response, header};

    use crate::share::{
        body::BodyError,
        inject::{ClientMode, MAX_HTML_RESPONSE_BYTES, inject_client, maybe_inject_client},
    };

    #[tokio::test]
    async fn a_body_longer_than_its_length_is_not_buffered() {
        let body = Full::new(Bytes::from(vec![b'x'; MAX_HTML_RESPONSE_BYTES + 1]))
            .map_err(|never| -> BodyError { match never {} })
            .boxed_unsync();
        let mut response = Response::new(body);
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, "text/html".parse().unwrap());
        response
            .headers_mut()
            .insert(header::CONTENT_LENGTH, "10".parse().unwrap());
        let refused = maybe_inject_client(response, &Method::GET, ClientMode::Edit)
            .await
            .unwrap_err();
        assert_eq!(refused.status, 502);
    }

    #[test]
    fn injects_before_a_case_insensitive_body_close() {
        let output = inject_client(b"<html><body>Hello</BODY></html>", ClientMode::Edit);
        let output = String::from_utf8(output.to_vec()).expect("UTF-8 HTML");
        assert!(output.contains("Hello\n<script type=\"module\""));
        assert!(output.ends_with("</BODY></html>"));
    }

    #[test]
    fn appends_when_html_has_no_body_close() {
        let output = inject_client(b"<p>Hello</p>", ClientMode::Edit);
        assert!(String::from_utf8_lossy(&output).ends_with("</script>\n"));
    }

    #[test]
    fn marks_the_live_client_script() {
        let output = inject_client(b"<body>Hello</body>", ClientMode::Live(7));
        let output = String::from_utf8_lossy(&output).into_owned();
        let version = env!("CARGO_PKG_VERSION");
        assert!(output.contains(&format!("client.js?live=7&amp;version={version}")));
    }
}
