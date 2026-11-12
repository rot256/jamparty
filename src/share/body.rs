//! Response bodies and small response builders shared by the share's and
//! the joiner's HTTP code.

use std::{convert::Infallible, error::Error};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::{
    Response, StatusCode,
    body::Incoming,
    header::{self, ALLOW, CACHE_CONTROL, CONTENT_TYPE},
};
use serde::Serialize;

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

pub(crate) fn javascript_response(script: &'static str) -> Response<AppBody> {
    let mut response = Response::new(full_body(Bytes::from_static(script.as_bytes())));
    response.headers_mut().insert(
        CONTENT_TYPE,
        header::HeaderValue::from_static("text/javascript; charset=utf-8"),
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
