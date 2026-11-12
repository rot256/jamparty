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
    join::session::{Event, Session, View},
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

/// The version of these endpoints, as the page's client speaks them. The
/// client is served by the share and the endpoints by the joiner, which
/// may run another jampot; the client compares this number (from
/// `api/status`) with its own and refuses to go on when they differ. Bump
/// it on any change an older client or joiner cannot handle.
pub(crate) const API_VERSION: u32 = 1;

/// `api/status`: the endpoints' version and this jampot's, then the view.
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
struct Status {
    api: u32,
    version: &'static str,
    #[serde(flatten)]
    #[cfg_attr(test, ts(flatten))]
    view: View,
}

/// What a page posts: its ref for the comment (sent again unchanged on
/// "send again") and the draft's fields.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[cfg_attr(test, derive(ts_rs::TS))]
struct Queued {
    id: CommentId,
    pending: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(ts_rs::TS))]
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
    let queued =
        |id: CommentId, pending| json_response(StatusCode::ACCEPTED, &Queued { id, pending });
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
    json_response(
        StatusCode::OK,
        &Status {
            api: API_VERSION,
            version: env!("CARGO_PKG_VERSION"),
            view: session.view(),
        },
    )
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

/// The TypeScript the page's client is checked against
/// (`client/src/generated/api.ts`), generated from the types above. The
/// test fails when the file is stale; `JAMPOT_UPDATE_BINDINGS=1 cargo test
/// bindings` rewrites it.
#[cfg(test)]
mod bindings {
    use std::{env, fs, path::Path};

    use ts_rs::{Config, TS};

    use crate::{
        core::{
            ids::{CommentId, EndpointKey, Fingerprint},
            name::JoinerName,
        },
        join::{
            api::{API_VERSION, Limited429, Queued, Status, Submission},
            session::{Answer, Event, View},
        },
        protocol::{
            comment::Author,
            control::{CommentRef, RefusedReason, StatusLine},
        },
        share::body::ErrorBody,
    };

    fn generate() -> String {
        // Numbers in this API (revisions, counts, seconds) stay far below 2^53.
        let cfg = Config::new().with_large_int("number");
        let decls = [
            CommentId::decl(&cfg),
            CommentRef::decl(&cfg),
            EndpointKey::decl(&cfg),
            Fingerprint::decl(&cfg),
            JoinerName::decl(&cfg),
            StatusLine::decl(&cfg),
            RefusedReason::decl(&cfg),
            Author::decl(&cfg),
            Answer::decl(&cfg),
            View::decl(&cfg),
            Status::decl(&cfg),
            Event::decl(&cfg),
            Submission::decl(&cfg),
            Queued::decl(&cfg),
            Limited429::decl(&cfg),
            ErrorBody::decl(&cfg),
        ];
        let mut out = String::from(
            "// Generated from the Rust types by `cargo test` (src/join/api.rs); do not edit.\n\
             // Regenerate: JAMPOT_UPDATE_BINDINGS=1 cargo test bindings\n\n",
        );
        out.push_str(&format!("export const API_VERSION = {API_VERSION};\n"));
        for decl in decls {
            out.push_str("\nexport ");
            out.push_str(&decl);
            out.push('\n');
        }
        out
    }

    #[test]
    fn bindings_are_current() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("client/src/generated/api.ts");
        let generated = generate();
        if env::var_os("JAMPOT_UPDATE_BINDINGS").is_some() {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &generated).unwrap();
            return;
        }
        let current = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == generated,
            "{} is stale: run JAMPOT_UPDATE_BINDINGS=1 cargo test bindings",
            path.display()
        );
    }
}
