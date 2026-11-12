//! The joiner's own endpoints: `api/comments`, `api/status` and
//! `api/events` (SSE).

use std::{convert::Infallible, time::Duration};

use bytes::Bytes;
use futures_util::stream;
use http_body_util::{BodyExt, Limited, StreamBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Frame, Incoming},
    header::{self, CACHE_CONTROL, CONTENT_TYPE},
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::broadcast::{Receiver, error::RecvError},
    time::{Instant, Interval, MissedTickBehavior, interval_at},
};

use crate::{
    core::ids::CommentId,
    join::session::{Event, Session},
    protocol::{
        MAX_LINE_BYTES,
        comment::{Draft, DraftFields},
        control::{AckOutcome, CommentRef},
    },
    share::body::{AppBody, RequestError, json_response},
};

/// SSE keep-alive interval: a `ping` event, which pages also use as a
/// watchdog.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// What a page posts: its ref for the comment (sent again unchanged on
/// "send again") and the draft's fields.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Submission {
    #[serde(rename = "ref")]
    reference: CommentRef,
    page: String,
    selector: String,
    #[serde(default)]
    selected_text: Option<String>,
    comment: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Queued {
    id: String,
    pending: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Limited429 {
    error: &'static str,
    retry_after_secs: u64,
}

/// `POST api/comments`: a JSON draft, handed to the share.
pub(crate) async fn comment(request: Request<Incoming>, session: &Session) -> Response<AppBody> {
    submit(request, session)
        .await
        .unwrap_or_else(Response::from)
}

async fn submit(
    request: Request<Incoming>,
    session: &Session,
) -> Result<Response<AppBody>, RequestError> {
    let json = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"));
    if !json {
        return Err(RequestError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content-type must be application/json",
        ));
    }
    let body = Limited::new(request.into_body(), MAX_LINE_BYTES)
        .collect()
        .await
        .map_err(|_| RequestError::new(StatusCode::PAYLOAD_TOO_LARGE, "comment is too large"))?
        .to_bytes();
    let bad = |error: String| RequestError::new(StatusCode::BAD_REQUEST, error);
    let Submission {
        reference,
        page,
        selector,
        selected_text,
        comment,
    } = serde_json::from_slice(&body).map_err(|error| bad(error.to_string()))?;
    let draft = Draft::try_from(DraftFields {
        page,
        selector,
        selected_text,
        comment,
    })
    .map_err(|error| bad(error.to_string()))?;
    let not_sent = || {
        RequestError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "not sent: the share is not reachable",
        )
    };
    let control = session.control().ok_or_else(not_sent)?;
    let outcome = control
        .comment(reference, draft)
        .await
        .map_err(|_| not_sent())?;
    let queued = |id: CommentId, pending| {
        json_response(
            StatusCode::ACCEPTED,
            &Queued {
                id: id.to_string(),
                pending,
            },
        )
    };
    Ok(match outcome {
        AckOutcome::Queued { id } => queued(id, session.queued(id)),
        AckOutcome::Delivered { id } => queued(id, session.already_delivered(id)),
        AckOutcome::RateLimited { retry_after_secs } => json_response(
            StatusCode::TOO_MANY_REQUESTS,
            &Limited429 {
                error: "too many comments; try again shortly",
                retry_after_secs,
            },
        ),
        AckOutcome::QueueFull => {
            return Err(RequestError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "the share's queue is full",
            ));
        }
    })
}

/// `GET api/status`: the session as the joiner sees it.
pub(crate) fn status(session: &Session) -> Response<AppBody> {
    json_response(StatusCode::OK, &session.view())
}

/// `GET api/events`: the connection state first, then every event, with a
/// `ping` every 15 s.
pub(crate) fn events(session: &Session) -> Response<AppBody> {
    let receiver = session.subscribe();
    let first = Event::Share {
        connected: session.view().connected,
    };
    let mut keep_alive = interval_at(Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
    keep_alive.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let frames = stream::unfold(
        (Some(first), receiver, keep_alive),
        |(first, receiver, keep_alive)| next_frame(first, receiver, keep_alive),
    );
    let body = StreamBody::new(frames)
        .map_err(|never: Infallible| match never {})
        .boxed_unsync();
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        header::HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    response
}

type FrameState = (Option<Event>, Receiver<Event>, Interval);

async fn next_frame(
    first: Option<Event>,
    mut receiver: Receiver<Event>,
    mut keep_alive: Interval,
) -> Option<(Result<Frame<Bytes>, Infallible>, FrameState)> {
    if let Some(event) = first {
        return Some((Ok(event_frame(&event)), (None, receiver, keep_alive)));
    }
    loop {
        tokio::select! {
            event = receiver.recv() => match event {
                Ok(event) => return Some((Ok(event_frame(&event)), (None, receiver, keep_alive))),
                Err(RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "event stream fell behind");
                }
                Err(RecvError::Closed) => return None,
            },
            _ = keep_alive.tick() => {
                let frame = Frame::data(Bytes::from_static(b"data: {\"type\":\"ping\"}\n\n"));
                return Some((Ok(frame), (None, receiver, keep_alive)));
            }
        }
    }
}

fn event_frame(event: &Event) -> Frame<Bytes> {
    let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_owned());
    Frame::data(Bytes::from(format!("data: {data}\n\n")))
}
