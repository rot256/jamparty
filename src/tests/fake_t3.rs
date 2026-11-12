//! A fake T3 MCP endpoint (streamable HTTP, JSON or server-sent events)
//! that records every call; tools and failures as in T3 Code v0.1.9.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response, StatusCode, body::Incoming, server::conn::http1, service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{net::TcpListener, task::JoinHandle};

/// One request the fake received.
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub(crate) method: String,
    /// `tools/call`'s tool name.
    pub(crate) tool: Option<String>,
    pub(crate) arguments: Arguments,
    /// The bearer token presented.
    pub(crate) token: String,
    pub(crate) session: Option<String>,
}

/// The tool arguments the T3 tools take (all optional here).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Arguments {
    pub(crate) thread_id: Option<String>,
    pub(crate) message: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) client_request_id: Option<String>,
}

#[derive(Deserialize)]
struct RpcIn {
    method: String,
    #[serde(default)]
    id: Value,
    #[serde(default)]
    params: Params,
}

#[derive(Default, Deserialize)]
struct Params {
    name: Option<String>,
    #[serde(default)]
    arguments: Arguments,
}

pub(crate) struct FakeThread {
    pub(crate) title: &'static str,
    /// Parent thread and relationship (`subagent` or `fork`).
    pub(crate) parent: Option<(&'static str, &'static str)>,
    pub(crate) archived: bool,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) threads: HashMap<&'static str, FakeThread>,
    /// Threads that refuse messages (`thread_not_sendable`, as archived).
    pub(crate) unsendable: HashSet<String>,
    /// The next this many tool calls fail with HTTP 500.
    pub(crate) http_failures: u32,
    /// Answer as server-sent events instead of JSON.
    pub(crate) sse: bool,
    pub(crate) calls: Vec<Call>,
    sessions: HashSet<String>,
    next_session: u32,
}

pub(crate) struct FakeT3 {
    pub(crate) addr: SocketAddr,
    state: Arc<Mutex<State>>,
    task: JoinHandle<()>,
}

impl Drop for FakeT3 {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeT3 {
    pub(crate) async fn start(state: State) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(state));
        let shared = Arc::clone(&state);
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let state = Arc::clone(&shared);
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        let state = Arc::clone(&state);
                        async move { Ok::<_, hyper::Error>(handle(&state, request).await) }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self { addr, state, task }
    }

    pub(crate) fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The tool calls to `tool`.
    pub(crate) fn calls_to(&self, tool: &str) -> Vec<Call> {
        self.state()
            .calls
            .iter()
            .filter(|call| call.tool.as_deref() == Some(tool))
            .cloned()
            .collect()
    }

    /// Forgets every session, as a restarted T3 does.
    pub(crate) fn forget_sessions(&self) {
        self.state().sessions.clear();
    }
}

fn reply(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::default());
    *response.status_mut() = status;
    response
}

async fn handle(state: &Mutex<State>, request: Request<Incoming>) -> Response<Full<Bytes>> {
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let accept = header("accept").unwrap_or_default();
    if !(accept.contains("application/json") && accept.contains("text/event-stream")) {
        return reply(StatusCode::NOT_ACCEPTABLE);
    }
    if header("content-type").as_deref() != Some("application/json") {
        return reply(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    let Some(token) =
        header("authorization").and_then(|value| value.strip_prefix("Bearer ").map(str::to_owned))
    else {
        return reply(StatusCode::UNAUTHORIZED);
    };
    let session = header("mcp-session-id");
    let body = request.into_body().collect().await.unwrap().to_bytes();
    let RpcIn { method, id, params } = serde_json::from_slice(&body).unwrap();
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    state.calls.push(Call {
        method: method.clone(),
        tool: params.name.clone(),
        arguments: params.arguments.clone(),
        token,
        session: session.clone(),
    });
    if method == "initialize" {
        state.next_session += 1;
        let session = format!("session-{}", state.next_session);
        state.sessions.insert(session.clone());
        let result = json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "T3 Code", "version": "0.1.9" },
        });
        let mut response = rpc(&state, &id, &result);
        response
            .headers_mut()
            .insert("mcp-session-id", session.parse().unwrap());
        return response;
    }
    if !session.is_some_and(|session| state.sessions.contains(&session)) {
        return reply(StatusCode::NOT_FOUND);
    }
    match method.as_str() {
        "notifications/initialized" => reply(StatusCode::ACCEPTED),
        "tools/call" => {
            if state.http_failures > 0 {
                state.http_failures -= 1;
                return reply(StatusCode::INTERNAL_SERVER_ERROR);
            }
            let result = tool(&state, &params);
            rpc(&state, &id, &result)
        }
        _ => reply(StatusCode::BAD_REQUEST),
    }
}

/// A JSON-RPC response with `result`, as JSON or as an event stream (after
/// a progress notification, which clients skip).
fn rpc(state: &State, id: &Value, result: &Value) -> Response<Full<Bytes>> {
    let message = json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string();
    let (body, content_type) = if state.sse {
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": {} });
        (
            format!("data: {note}\n\ndata: {message}\n\n"),
            "text/event-stream",
        )
    } else {
        (message, "application/json")
    };
    let mut response = Response::new(Full::new(Bytes::from(body)));
    response
        .headers_mut()
        .insert("content-type", content_type.parse().unwrap());
    response
}

fn failure(code: &str, message: &str) -> Value {
    let text = json!({ "_tag": "OrchestratorMcpFailure", "code": code, "message": message });
    json!({ "isError": true, "content": [{ "type": "text", "text": text.to_string() }] })
}

fn success(structured: &Value) -> Value {
    json!({
        "isError": false,
        "structuredContent": structured,
        "content": [{ "type": "text", "text": structured.to_string() }],
    })
}

fn tool(state: &State, params: &Params) -> Value {
    let thread_id = params.arguments.thread_id.as_deref().unwrap_or_default();
    let Some(thread) = state.threads.get(thread_id) else {
        return failure(
            "thread_not_found",
            &format!("Thread {thread_id} not found."),
        );
    };
    match params.name.as_deref() {
        Some("t3_thread_read") => {
            let (parent, relationship) = match thread.parent {
                Some((parent, relationship)) => (json!(parent), json!(relationship)),
                None => (Value::Null, Value::Null),
            };
            success(&json!({
                "thread": {
                    "threadId": thread_id,
                    "projectId": "project-1",
                    "title": thread.title,
                    "parentThreadId": parent,
                    "relationshipToParent": relationship,
                    "archived": thread.archived,
                    "status": "idle",
                },
                "recentRuns": [],
                "items": [],
                "nextPosition": null,
                "hasMore": false,
            }))
        }
        Some("t3_thread_send") => {
            if state.unsendable.contains(thread_id) {
                return failure("thread_not_sendable", "Thread is archived.");
            }
            success(&json!({
                "threadId": thread_id,
                "messageId": "message-1",
                "runId": "run-1",
                "status": "running",
                "delivery": "started",
            }))
        }
        _ => failure("invalid_request", "unknown tool"),
    }
}
