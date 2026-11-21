//! HTTP shared by the share's request streams and the joiner's local
//! server: response bodies, small response builders and the header read
//! timeout.

use std::{convert::Infallible, error::Error, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::{
    Response, StatusCode,
    body::{Body, Incoming},
    header::{self, ALLOW, CACHE_CONTROL, CONTENT_TYPE},
};
use serde::Serialize;
use tokio::time::timeout;

use crate::protocol::resource::MAX_FILE_BYTES;

/// How long a request's head may take to arrive, on the share's request
/// streams and the joiner's local server.
pub(crate) const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Bounds slow uploads separately from the HTTP header deadline.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) async fn read_upload<B>(body: B) -> Result<Bytes, RequestError>
where
    B: Body<Data = Bytes> + Unpin,
{
    timeout(UPLOAD_TIMEOUT, read_upload_bytes(body))
        .await
        .map_err(|_| {
            RequestError::new(StatusCode::REQUEST_TIMEOUT, "attachment upload timed out")
        })?
}

async fn read_upload_bytes<B>(mut body: B) -> Result<Bytes, RequestError>
where
    B: Body<Data = Bytes> + Unpin,
{
    let too_large = || {
        RequestError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "an attachment can be at most 8 MiB",
        )
    };
    let length = body.size_hint().exact().unwrap_or_default();
    if length > MAX_FILE_BYTES as u64 {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(length as usize);
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| {
            RequestError::new(StatusCode::BAD_REQUEST, "could not read the attachment")
        })?;
        if let Ok(data) = frame.into_data() {
            if data.len() > MAX_FILE_BYTES - bytes.len() {
                return Err(too_large());
            }
            let needed = bytes.len() + data.len();
            if needed > bytes.capacity() {
                let capacity = needed.max(bytes.capacity() * 2).min(MAX_FILE_BYTES);
                bytes.reserve_exact(capacity - bytes.len());
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(Bytes::from(bytes))
}

/// What hyper takes as a body's error.
pub(crate) type BodyError = Box<dyn Error + Send + Sync>;

pub(crate) type AppBody = UnsyncBoxBody<Bytes, BodyError>;

/// A refused request: status, message and, for 405, the allowed methods.
#[derive(Debug)]
pub(crate) struct RequestError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) allow: Option<&'static str>,
}

impl RequestError {
    pub(crate) fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            allow: None,
        }
    }

    pub(crate) fn method_not_allowed(allow: &'static str) -> Self {
        Self {
            status: StatusCode::METHOD_NOT_ALLOWED,
            message: "method not allowed".to_owned(),
            allow: Some(allow),
        }
    }
}

impl From<RequestError> for Response<AppBody> {
    fn from(error: RequestError) -> Self {
        let RequestError {
            status,
            message,
            allow,
        } = error;
        let mut response = json_response(status, &ErrorBody { error: &message });
        if let Some(allow) = allow {
            response
                .headers_mut()
                .insert(ALLOW, header::HeaderValue::from_static(allow));
        }
        response
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
}

pub(crate) fn json_response(status: StatusCode, body: &impl Serialize) -> Response<AppBody> {
    let bytes = serde_json::to_vec(body)
        .unwrap_or_else(|_| b"{\"error\":\"serialization failed\"}".to_vec());
    let mut response = Response::new(full_body(Bytes::from(bytes)));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        header::HeaderValue::from_static("application/json; charset=utf-8"),
    );
    no_store(response)
}

pub(crate) fn empty_response(status: StatusCode) -> Response<AppBody> {
    let mut response = Response::new(full_body(Bytes::new()));
    *response.status_mut() = status;
    no_store(response)
}

fn no_store(mut response: Response<AppBody>) -> Response<AppBody> {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    response
}

pub(crate) fn full_body(bytes: Bytes) -> AppBody {
    Full::new(bytes)
        .map_err(|never: Infallible| match never {})
        .boxed_unsync()
}

pub(crate) fn incoming_body(body: Incoming) -> AppBody {
    body.map_err(|error| -> BodyError { Box::new(error) })
        .boxed_unsync()
}

pub(crate) fn map_incoming_response(response: Response<Incoming>) -> Response<AppBody> {
    let (parts, body) = response.into_parts();
    Response::from_parts(parts, incoming_body(body))
}

pub(crate) fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod upload_tests {
    use std::{convert::Infallible, sync::Arc};

    use bytes::Bytes;
    use http_body_util::StreamBody;
    use hyper::body::Frame;

    use crate::adapters::http::read_upload;

    #[tokio::test]
    async fn fragmented_upload_releases_each_frame_before_reading_next() {
        const FRAMES: usize = 65_536;
        let owner: Arc<[u8]> = Arc::new([0x5a; 1024]);
        let frames = (0..FRAMES).map(|_| {
            assert_eq!(
                Arc::strong_count(&owner),
                1,
                "the previous frame is still retained"
            );
            let bytes = Bytes::from_owner(Arc::clone(&owner)).slice(..1);
            Ok::<_, Infallible>(Frame::data(bytes))
        });
        let result = read_upload(StreamBody::new(futures_util::stream::iter(frames)))
            .await
            .unwrap();
        assert_eq!(result.len(), FRAMES);
        assert!(result.iter().all(|byte| *byte == 0x5a));
    }
}
