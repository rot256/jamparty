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
        session::{CursorError, Epoch, Event, PageSettings, Session, Stamped, View},
        tabs::{TabError, TabId},
    },
    protocol::{
        MAX_LINE_BYTES,
        comment::{Draft, DraftFields},
        control::{AckOutcome, CommentRef},
        presence::{PagePath, Position},
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
    /// The joiner process the revision counts in.
    epoch: Epoch,
    /// The joiner's view revision that has this comment (see `View::rev`).
    rev: u64,
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
    let epoch = session.epoch();
    let queued = |id: CommentId, (pending, rev): (usize, u64)| {
        json_response(
            StatusCode::ACCEPTED,
            &Queued {
                id,
                pending,
                epoch,
                rev,
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
        Ok((settings, rev)) => {
            // A new name or cursor switch is news for the share.
            session.report();
            let saved = Saved {
                epoch: session.epoch(),
                rev,
                settings,
            };
            json_response(StatusCode::OK, &saved)
        }
        Err(error) => error.into(),
    }
}

/// `api/settings`' answer: the settings now, stamped like the `settings`
/// event it sent, so a page applies whichever is newer.
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
struct Saved {
    epoch: Epoch,
    rev: u64,
    #[serde(flatten)]
    #[cfg_attr(test, ts(flatten))]
    settings: PageSettings,
}

/// What a page posts to `api/page`: it moved (a single-page app's route,
/// a hash change) or became active.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
struct Visit {
    tab: TabId,
    /// The tab's number for this report, counting up: an older report that
    /// arrives late changes nothing (0 when absent).
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    seq: Option<u64>,
    /// None: the page cannot say where it is (its URL is too long).
    url: Option<PagePath>,
}

/// `POST api/page`: the tab is on this URL now. 204, or 404 for a tab
/// without an open event stream.
pub(crate) async fn page(request: Request<Incoming>, session: &Session) -> Response<AppBody> {
    let visited = match read_json::<Visit>(request).await {
        Ok(Visit { tab, seq, url }) => session
            .visit(&tab, url, seq.unwrap_or(0))
            .map_err(tab_error),
        Err(error) => Err(error),
    };
    match visited {
        Ok(()) => {
            session.report();
            no_content()
        }
        Err(error) => error.into(),
    }
}

/// What a page posts to `api/cursor`: the pointer moved to `position`
/// (an element and the offset in its box), or (none) left the page.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
struct Pointer {
    tab: TabId,
    /// The number of the tab's report of `url`; older than the joiner's
    /// latest is refused (0 when absent).
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    seq: Option<u64>,
    /// The page the pointer is on, as the tab knows it.
    url: PagePath,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    position: Option<Position>,
}

/// `POST api/cursor`: forwards the pointer to the share. 204; 409 while
/// the cursor switch is off or for a page the joiner does not have for
/// the tab (nothing is sent then), 404 for an unknown tab, 503 without a
/// share.
pub(crate) async fn cursor(request: Request<Incoming>, session: &Session) -> Response<AppBody> {
    let pointer = match read_json::<Pointer>(request).await {
        Ok(pointer) => pointer,
        Err(error) => return error.into(),
    };
    match session
        .cursor(
            &pointer.tab,
            pointer.seq.unwrap_or(0),
            &pointer.url,
            pointer.position,
        )
        .await
    {
        Ok(()) => no_content(),
        Err(CursorError::Off) => {
            RequestError::new(StatusCode::CONFLICT, "the cursor is not shown").into()
        }
        Err(CursorError::Elsewhere) => RequestError::new(
            StatusCode::CONFLICT,
            "the joiner has another page for this tab",
        )
        .into(),
        Err(CursorError::Tab(error)) => tab_error(error).into(),
        Err(CursorError::Unreachable) => RequestError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "the share is not reachable",
        )
        .into(),
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
        Some(TabQuery { tab, url, seq }) => match session.open_tab(tab.clone(), url, seq) {
            Ok(()) => {
                session.report();
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
    let view = session.view();
    let first = Stamped {
        epoch: view.epoch,
        rev: view.rev,
        event: Event::Share {
            connected: view.connected,
        },
    };
    let mut keep_alive = interval_at(Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
    keep_alive.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let state = Stream {
        session: Arc::clone(session),
        first: Some(first),
        receiver,
        keep_alive,
        tab,
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

/// A tab as its event stream's query names it: `tab`, `url` (absent when
/// the page cannot say where it is) and `seq`, the number of the page's
/// report of that url (0 when absent).
struct TabQuery {
    tab: TabId,
    url: Option<PagePath>,
    seq: u64,
}

/// The tab in an event stream's query, if it names one.
fn tab_of(query: Option<&str>) -> Result<Option<TabQuery>, RequestError> {
    let Some(query) = query else {
        return Ok(None);
    };
    let mut tab = None;
    let mut url = None;
    let mut seq = None;
    for (name, value) in form_urlencoded::parse(query.as_bytes()) {
        match name.as_ref() {
            "tab" => tab = Some(value.into_owned()),
            "url" => url = Some(value.into_owned()),
            "seq" => seq = Some(value.into_owned()),
            _ => {}
        }
    }
    let Some(tab) = tab else {
        return match (url, seq) {
            (None, None) => Ok(None),
            (Some(_), _) | (_, Some(_)) => Err(bad("a url or seq needs a tab".to_owned())),
        };
    };
    let tab = TabId::try_from(tab).map_err(|error| bad(error.to_string()))?;
    let url = url
        .map(PagePath::try_from)
        .transpose()
        .map_err(|error| bad(error.to_string()))?;
    let seq = seq
        .map(|seq| seq.parse::<u64>())
        .transpose()
        .map_err(|error| bad(format!("seq: {error}")))?
        .unwrap_or(0);
    Ok(Some(TabQuery { tab, url, seq }))
}

/// A tab while its event stream is open.
struct OpenTab {
    session: Arc<Session>,
    tab: TabId,
}

impl Drop for OpenTab {
    fn drop(&mut self) {
        self.session.close_tab(&self.tab);
        self.session.report();
    }
}

struct Stream {
    session: Arc<Session>,
    first: Option<Stamped>,
    receiver: Receiver<Stamped>,
    keep_alive: Interval,
    tab: Option<OpenTab>,
}

impl Stream {
    /// Cursors go only to tabs on their page; everything else to all.
    fn wants(&self, stamped: &Stamped) -> bool {
        match &stamped.event {
            Event::Cursor { url, .. } => self
                .tab
                .as_ref()
                .is_some_and(|open| self.session.tab_url(&open.tab).as_ref() == Some(url)),
            Event::Share { .. }
            | Event::Reload { .. }
            | Event::Presence { .. }
            | Event::Status { .. }
            | Event::Delivered { .. }
            | Event::Answer(_)
            | Event::Settings(_) => true,
        }
    }
}

async fn next_frame(mut state: Stream) -> Option<(Result<Frame<Bytes>, Infallible>, Stream)> {
    if let Some(event) = state.first.take() {
        return Some((Ok(event_frame(&event)), state));
    }
    loop {
        tokio::select! {
            event = state.receiver.recv() => match event {
                Ok(event) if state.wants(&event) => {
                    return Some((Ok(event_frame(&event)), state));
                }
                Ok(_) => {}
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

fn event_frame(event: &Stamped) -> Frame<Bytes> {
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
            settings::{Corner, KeyBinding, Keys, KeysFields, SettingsChange, Switch},
        },
        join::{
            api::{API_VERSION, Limited429, Pointer, Queued, Saved, Status, Submission, Visit},
            session::{Answer, Epoch, Event, PageSettings, Stamped, View},
            tabs::TabId,
        },
        protocol::{
            comment::Author,
            control::{CommentRef, RefusedReason, StatusLine},
            presence::{Colour, CursorSelector, Fraction, PagePath, Person, Position},
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
            Corner::decl(&cfg),
            KeyBinding::decl(&cfg),
            KeysFields::decl(&cfg),
            Keys::decl(&cfg),
            Author::decl(&cfg),
            PagePath::decl(&cfg),
            Colour::decl(&cfg),
            Person::decl(&cfg),
            TabId::decl(&cfg),
            Visit::decl(&cfg),
            CursorSelector::decl(&cfg),
            Fraction::decl(&cfg),
            Position::decl(&cfg),
            Pointer::decl(&cfg),
            PageSettings::decl(&cfg),
            SettingsChange::decl(&cfg),
            Answer::decl(&cfg),
            View::decl(&cfg),
            Status::decl(&cfg),
            Event::decl(&cfg),
            Epoch::decl(&cfg),
            Stamped::decl(&cfg),
            Saved::decl(&cfg),
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
