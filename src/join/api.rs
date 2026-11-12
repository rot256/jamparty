//! The joiner's own endpoints: `api/comments`, `api/status` and
//! `api/events` (SSE).

use std::{convert::Infallible, sync::Arc, time::Duration};

use bytes::Bytes;
use futures_util::stream;
use http_body_util::{BodyExt, Limited, StreamBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Frame, Incoming},
    header::{self, CACHE_CONTROL, CONTENT_TYPE},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    sync::broadcast::{Receiver, error::RecvError},
    time::{Instant, Interval, MissedTickBehavior, interval_at},
};
use url::form_urlencoded;

use crate::{
    core::{ids::CommentId, settings::SettingsChange},
    describe,
    join::{
        session::{Event, Session, View},
        tabs::{TabError, TabId},
    },
    protocol::{
        MAX_LINE_BYTES,
        comment::{Draft, DraftFields},
        control::{AckOutcome, CommentRef},
        presence::PagePath,
    },
    share::body::{AppBody, RequestError, empty_response, json_response},
};

/// SSE keep-alive interval: a `ping` event, which pages also use as a
/// watchdog.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The version of these endpoints, as the page's client speaks them. The
/// client is served by the share and the endpoints by the joiner, which
/// may run another jampot; the client compares this number (from
/// `api/status`) with its own and refuses to go on when they differ. Bump
/// it on any change an older client or joiner cannot handle.
pub(crate) const API_VERSION: u32 = 2;

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

/// The JSON body of a page's POST, as `T`.
async fn read_json<T: DeserializeOwned>(request: Request<Incoming>) -> Result<T, RequestError> {
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
        .map_err(|_| RequestError::new(StatusCode::PAYLOAD_TOO_LARGE, "body is too large"))?
        .to_bytes();
    serde_json::from_slice(&body).map_err(|error| bad(error.to_string()))
}

fn bad(error: String) -> RequestError {
    RequestError::new(StatusCode::BAD_REQUEST, error)
}

async fn submit(
    request: Request<Incoming>,
    session: &Session,
) -> Result<Response<AppBody>, RequestError> {
    let Submission {
        reference,
        page,
        selector,
        selected_text,
        comment,
    } = read_json(request).await?;
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

/// `POST api/settings`: a change to the settings, saved and sent to every
/// page; answers the settings as they are now.
pub(crate) async fn settings(request: Request<Incoming>, session: &Session) -> Response<AppBody> {
    let changed = match read_json::<SettingsChange>(request).await {
        Ok(change) => session.change_settings(change).map_err(|error| {
            tracing::warn!(error = %describe(&error), "could not save the settings");
            RequestError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not save the settings: {}", describe(&error)),
            )
        }),
        Err(error) => Err(error),
    };
    match changed {
        Ok(settings) => {
            // A new name or cursor switch is news for the share.
            session.report().await;
            json_response(StatusCode::OK, &settings)
        }
        Err(error) => error.into(),
    }
}

/// What a page posts to `api/page`: it moved (a single-page app's route,
/// a hash change) or became active.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
struct Visit {
    tab: TabId,
    url: PagePath,
}

/// `POST api/page`: the tab is on this URL now. 204, or 404 for a tab
/// without an open event stream.
pub(crate) async fn page(request: Request<Incoming>, session: &Session) -> Response<AppBody> {
    let visited = match read_json::<Visit>(request).await {
        Ok(Visit { tab, url }) => session.visit(&tab, url).map_err(tab_error),
        Err(error) => Err(error),
    };
    match visited {
        Ok(()) => {
            session.report().await;
            no_content()
        }
        Err(error) => error.into(),
    }
}

fn tab_error(error: TabError) -> RequestError {
    match error {
        TabError::Unknown => RequestError::new(StatusCode::NOT_FOUND, "no such tab"),
        TabError::Full => RequestError::new(StatusCode::TOO_MANY_REQUESTS, "too many tabs"),
    }
}

fn no_content() -> Response<AppBody> {
    empty_response(StatusCode::NO_CONTENT)
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

/// `GET api/events[?tab=<id>&url=<page>]`: the connection state first,
/// then every event, with a `ping` every 15 s. A page that names its tab
/// and URL counts as one of the joiner's tabs while the stream is open.
pub(crate) fn events(session: &Arc<Session>, query: Option<&str>) -> Response<AppBody> {
    let tab = match tab_of(query) {
        Ok(tab) => tab,
        Err(error) => return error.into(),
    };
    let tab = match tab {
        Some((tab, url)) => match session.open_tab(tab.clone(), url) {
            Ok(()) => {
                let reporting = Arc::clone(session);
                tokio::spawn(async move { reporting.report().await });
                Some(OpenTab {
                    session: Arc::clone(session),
                    tab,
                })
            }
            Err(error) => return tab_error(error).into(),
        },
        None => None,
    };
    let receiver = session.subscribe();
    let first = Event::Share {
        connected: session.view().connected,
    };
    let mut keep_alive = interval_at(Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
    keep_alive.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let state = Stream {
        first: Some(first),
        receiver,
        keep_alive,
        _tab: tab,
    };
    let frames = stream::unfold(state, next_frame);
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

/// The tab and URL in an event stream's query, if it names them.
fn tab_of(query: Option<&str>) -> Result<Option<(TabId, PagePath)>, RequestError> {
    let Some(query) = query else {
        return Ok(None);
    };
    let mut tab = None;
    let mut url = None;
    for (name, value) in form_urlencoded::parse(query.as_bytes()) {
        match name.as_ref() {
            "tab" => tab = Some(value.into_owned()),
            "url" => url = Some(value.into_owned()),
            _ => {}
        }
    }
    match (tab, url) {
        (None, None) => Ok(None),
        (Some(tab), Some(url)) => {
            let tab = TabId::try_from(tab).map_err(|error| bad(error.to_string()))?;
            let url = PagePath::try_from(url).map_err(|error| bad(error.to_string()))?;
            Ok(Some((tab, url)))
        }
        (Some(_), None) | (None, Some(_)) => Err(bad("tab and url go together".to_owned())),
    }
}

/// A tab while its event stream is open.
struct OpenTab {
    session: Arc<Session>,
    tab: TabId,
}

impl Drop for OpenTab {
    fn drop(&mut self) {
        self.session.close_tab(&self.tab);
        let session = Arc::clone(&self.session);
        tokio::spawn(async move { session.report().await });
    }
}

struct Stream {
    first: Option<Event>,
    receiver: Receiver<Event>,
    keep_alive: Interval,
    _tab: Option<OpenTab>,
}

async fn next_frame(mut state: Stream) -> Option<(Result<Frame<Bytes>, Infallible>, Stream)> {
    if let Some(event) = state.first.take() {
        return Some((Ok(event_frame(&event)), state));
    }
    loop {
        tokio::select! {
            event = state.receiver.recv() => match event {
                Ok(event) => return Some((Ok(event_frame(&event)), state)),
                Err(RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "event stream fell behind");
                }
                Err(RecvError::Closed) => return None,
            },
            _ = state.keep_alive.tick() => {
                let frame = Frame::data(Bytes::from_static(b"data: {\"type\":\"ping\"}\n\n"));
                return Some((Ok(frame), state));
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
            settings::{KeyBinding, Keys, KeysFields, SettingsChange, Switch},
        },
        join::{
            api::{API_VERSION, Limited429, Queued, Status, Submission, Visit},
            session::{Answer, Event, PageSettings, View},
            tabs::TabId,
        },
        protocol::{
            comment::Author,
            control::{CommentRef, RefusedReason, StatusLine},
            presence::{Colour, PagePath, Person},
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
            Switch::decl(&cfg),
            KeyBinding::decl(&cfg),
            KeysFields::decl(&cfg),
            Keys::decl(&cfg),
            Author::decl(&cfg),
            PagePath::decl(&cfg),
            Colour::decl(&cfg),
            Person::decl(&cfg),
            TabId::decl(&cfg),
            Visit::decl(&cfg),
            PageSettings::decl(&cfg),
            SettingsChange::decl(&cfg),
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
