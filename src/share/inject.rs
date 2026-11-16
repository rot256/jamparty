//! Injects the browser client into HTML responses: before `</body>` in
//! HTML of a known length (up to 2 MB, read whole), at the end of HTML
//! streamed without one (passed on as it comes). Encoded (compressed) HTML
//! passes untouched; the proxy asks dev servers for none.

use std::{borrow::Cow, future};

use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt, stream};
use http_body_util::{BodyExt, BodyStream, StreamBody};
use hyper::{
    Method, Response, StatusCode,
    body::Frame,
    header::{self, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG},
};

use crate::{
    adapters::http::{AppBody, RequestError, full_body},
    core::text::Untrusted,
    protocol::control::Revision,
};

const MAX_HTML_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const EDIT_SCRIPT: &[u8] = b"\n<script type=\"module\" src=\"/_jamparty/client.js\"></script>\n";

/// How the client is loaded: plain, or with the live-reload revision the
/// page was served at.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ClientMode {
    Edit,
    Live(Revision),
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
    let content_length = match response.headers().get(CONTENT_LENGTH) {
        None => return Ok(append_client(response, request_method, mode)),
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|value| value.parse::<usize>().ok()),
    };
    // Large HTML (or a length that is not one) passes through untouched.
    let Some(content_length) = content_length.filter(|length| *length <= MAX_HTML_RESPONSE_BYTES)
    else {
        tracing::debug!("large HTML was not annotated");
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
        tracing::warn!(error = %Untrusted(message), "could not read HTML response");
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

/// Streamed HTML (no length): passed on as it arrives, the client after
/// its last chunk. Trailers are dropped (the client comes after the data).
fn append_client(
    response: Response<AppBody>,
    request_method: &Method,
    mode: ClientMode,
) -> Response<AppBody> {
    let (mut parts, body) = response.into_parts();
    annotate_headers(&mut parts.headers, None);
    if request_method == Method::HEAD {
        return Response::from_parts(parts, body);
    }
    let script = Bytes::from(client_script(mode).into_owned());
    let frames = BodyStream::new(body)
        .try_filter(|frame| future::ready(frame.is_data()))
        .chain(stream::once(future::ready(Ok(Frame::data(script)))));
    Response::from_parts(parts, StreamBody::new(frames).boxed_unsync())
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
                "\n<script type=\"module\" src=\"/_jamparty/client.js?live={revision}\"></script>\n"
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
    use std::convert::Infallible;

    use bytes::Bytes;
    use futures_util::stream;
    use http_body_util::{BodyExt, Full, StreamBody};
    use hyper::{Method, Response, body::Frame, header};
    use tokio::sync::mpsc;

    use crate::{
        adapters::http::{AppBody, BodyError},
        protocol::control::Revision,
        share::inject::{
            ClientMode, EDIT_SCRIPT, MAX_HTML_RESPONSE_BYTES, inject_client, maybe_inject_client,
        },
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

    /// A streamed HTML response whose chunks come from `chunks`.
    fn streamed(chunks: mpsc::Receiver<&'static str>) -> Response<AppBody> {
        let frames = stream::unfold(chunks, |mut chunks| async move {
            let chunk = chunks.recv().await?;
            Some((
                Ok(Frame::data(Bytes::from_static(chunk.as_bytes()))),
                chunks,
            ))
        });
        let mut response = Response::new(
            StreamBody::new(frames)
                .map_err(|never: Infallible| match never {})
                .boxed_unsync(),
        );
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, "text/html".parse().unwrap());
        headers.insert(header::ETAG, "\"1\"".parse().unwrap());
        response
    }

    #[tokio::test]
    async fn streamed_html_gets_the_client_at_its_end_unbuffered() {
        let (send, chunks) = mpsc::channel(4);
        let response = maybe_inject_client(streamed(chunks), &Method::GET, ClientMode::Edit)
            .await
            .unwrap();
        let headers = response.headers();
        assert!(headers.get(header::CONTENT_LENGTH).is_none());
        assert!(headers.get(header::ETAG).is_none());
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
        let mut body = response.into_body();
        // The first chunk passes while the rest is still to come.
        send.send("<html><body><h1>Hi</h1>").await.unwrap();
        let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(first, Bytes::from_static(b"<html><body><h1>Hi</h1>"));
        send.send("</body></html>").await.unwrap();
        drop(send);
        let rest = body.collect().await.unwrap().to_bytes();
        let rest = String::from_utf8(rest.to_vec()).unwrap();
        let script = String::from_utf8(EDIT_SCRIPT.to_vec()).unwrap();
        assert_eq!(rest, format!("</body></html>{script}"));
    }

    #[tokio::test]
    async fn a_head_for_streamed_html_is_annotated_alone() {
        let (send, chunks) = mpsc::channel(1);
        drop(send);
        let response = maybe_inject_client(streamed(chunks), &Method::HEAD, ClientMode::Edit)
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert!(response.headers().get(header::CONTENT_LENGTH).is_none());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty());
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
        let output = inject_client(b"<body>Hello</body>", ClientMode::Live(Revision::from(7)));
        let output = String::from_utf8_lossy(&output).into_owned();
        assert!(output.contains("client.js?live=7\""));
    }
}
