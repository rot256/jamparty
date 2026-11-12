//! A minimal MCP client over streamable HTTP: `initialize`, the
//! `notifications/initialized` notification, then `tools/call`. Responses
//! come as JSON or as server-sent events; both are read. The session id the
//! server returns goes back on every later request; an unknown session
//! (HTTP 404) starts a new one.

use std::{
    fmt,
    path::PathBuf,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, StatusCode, Uri,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue},
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SESSION_HEADER: &str = "mcp-session-id";
const VERSION_HEADER: &str = "mcp-protocol-version";
/// Every request, including reading its whole response, ends within this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Responses larger than this are refused.
const MAX_RESPONSE: usize = 4 << 20;

/// A failure to talk to the server (not a tool's own failure).
#[derive(Debug)]
pub(crate) struct McpError(pub(crate) String);

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for McpError {}

fn error(text: impl Into<String>) -> McpError {
    McpError(text.into())
}

/// A tool that answered with `isError`: its text content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolFailure(pub(crate) String);

pub(crate) struct McpClient {
    url: Uri,
    /// Read at every request, so a renewed token is picked up.
    token_file: PathBuf,
    http: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    /// The session id, once initialised (`Some(None)`: the server keeps no
    /// sessions).
    session: Mutex<Option<Option<HeaderValue>>>,
    next_id: AtomicU64,
}

#[derive(Serialize)]
struct RpcRequest<'a, P> {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    method: &'a str,
    params: P,
}

#[derive(Deserialize)]
struct RpcMessage<T> {
    id: Option<u64>,
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallToolResult<T> {
    #[serde(default)]
    is_error: bool,
    structured_content: Option<T>,
    #[serde(default)]
    content: Vec<Content>,
}

#[derive(Deserialize)]
struct Content {
    text: Option<String>,
}

/// A result whose contents are not needed (initialize's).
#[derive(Deserialize)]
struct Ignored {}

struct Reply {
    status: StatusCode,
    sse: bool,
    session: Option<HeaderValue>,
    body: Bytes,
}

/// Why a request in a session failed.
enum RpcFailure {
    /// The server does not know the session.
    Expired,
    Failed(McpError),
}

impl From<McpError> for RpcFailure {
    fn from(error: McpError) -> Self {
        Self::Failed(error)
    }
}

impl McpClient {
    pub(crate) fn new(url: Uri, token_file: PathBuf) -> Self {
        let connector = HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        Self {
            url,
            token_file,
            http: Client::builder(TokioExecutor::new()).build(connector),
            session: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    /// Calls tool `name`; the outer error is a transport or protocol
    /// failure, the inner one the tool's own.
    pub(crate) async fn call_tool<A: Serialize, T: DeserializeOwned>(
        &self,
        name: &str,
        arguments: A,
    ) -> Result<Result<T, ToolFailure>, McpError> {
        let params = json!({ "name": name, "arguments": arguments });
        let session = self.session().await?;
        let result: CallToolResult<T> = match self.rpc(session, "tools/call", &params).await {
            Ok(result) => result,
            Err(RpcFailure::Failed(error)) => return Err(error),
            Err(RpcFailure::Expired) => {
                // The server forgot the session (it restarted): start anew once.
                *self.lock_session() = None;
                let session = self.session().await?;
                match self.rpc(session, "tools/call", &params).await {
                    Ok(result) => result,
                    Err(RpcFailure::Failed(error)) => return Err(error),
                    Err(RpcFailure::Expired) => return Err(error("the MCP session expired")),
                }
            }
        };
        if result.is_error {
            let text: Vec<String> = result
                .content
                .into_iter()
                .filter_map(|content| content.text)
                .collect();
            return Ok(Err(ToolFailure(text.join("\n"))));
        }
        result
            .structured_content
            .map(Ok)
            .ok_or_else(|| error(format!("{name} returned no structured content")))
    }

    fn lock_session(&self) -> std::sync::MutexGuard<'_, Option<Option<HeaderValue>>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The current session id, initialising a session if needed.
    async fn session(&self) -> Result<Option<HeaderValue>, McpError> {
        if let Some(session) = self.lock_session().clone() {
            return Ok(session);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let initialize = RpcRequest {
            jsonrpc: "2.0",
            id: Some(id),
            method: "initialize",
            params: json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "nabe", "version": env!("CARGO_PKG_VERSION") },
            }),
        };
        let reply = self.post(None, &initialize).await?;
        check_status(reply.status)?;
        let _: Ignored = parse(id, reply.sse, &reply.body)?;
        let session = reply.session;
        let initialized = RpcRequest {
            jsonrpc: "2.0",
            id: None,
            method: "notifications/initialized",
            params: json!({}),
        };
        let reply = self.post(Some(session.as_ref()), &initialized).await?;
        check_status(reply.status)?;
        *self.lock_session() = Some(session.clone());
        Ok(session)
    }

    async fn rpc<T: DeserializeOwned>(
        &self,
        session: Option<HeaderValue>,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<T, RpcFailure> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: Some(id),
            method,
            params,
        };
        let reply = self.post(Some(session.as_ref()), &request).await?;
        if reply.status == StatusCode::NOT_FOUND && session.is_some() {
            return Err(RpcFailure::Expired);
        }
        check_status(reply.status)?;
        Ok(parse(id, reply.sse, &reply.body)?)
    }

    /// POSTs one message; `session` is `None` before initialisation and
    /// `Some(id)` after.
    async fn post<P: Serialize>(
        &self,
        session: Option<Option<&HeaderValue>>,
        message: &RpcRequest<'_, P>,
    ) -> Result<Reply, McpError> {
        let token = std::fs::read_to_string(&self.token_file).map_err(|e| {
            error(format!(
                "could not read the token file {}: {e}",
                self.token_file.display()
            ))
        })?;
        let authorization = HeaderValue::try_from(format!("Bearer {}", token.trim()))
            .map_err(|_| error("the token file holds characters a header cannot carry"))?;
        let body = serde_json::to_vec(message).map_err(|e| error(e.to_string()))?;
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(self.url.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .header(AUTHORIZATION, authorization);
        if let Some(session) = session {
            request = request.header(VERSION_HEADER, PROTOCOL_VERSION);
            if let Some(id) = session {
                request = request.header(SESSION_HEADER, id);
            }
        }
        let request = request
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| error(e.to_string()))?;
        let exchange = async {
            let response = self
                .http
                .request(request)
                .await
                .map_err(|e| error(format!("could not reach {}: {e}", self.url)))?;
            let status = response.status();
            let sse = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.trim_start().starts_with("text/event-stream"));
            let session = response.headers().get(SESSION_HEADER).cloned();
            let body = Limited::new(response.into_body(), MAX_RESPONSE)
                .collect()
                .await
                .map_err(|e| error(format!("reading the response: {e}")))?
                .to_bytes();
            Ok(Reply {
                status,
                sse,
                session,
                body,
            })
        };
        tokio::time::timeout(REQUEST_TIMEOUT, exchange)
            .await
            .map_err(|_| error(format!("no answer within {}s", REQUEST_TIMEOUT.as_secs())))?
    }
}

fn check_status(status: StatusCode) -> Result<(), McpError> {
    match status {
        StatusCode::OK | StatusCode::ACCEPTED => Ok(()),
        StatusCode::UNAUTHORIZED => Err(error("HTTP 401: the token was refused")),
        other => Err(error(format!("HTTP {other}"))),
    }
}

/// The result of request `id` from a JSON body or an event stream (whose
/// other messages are skipped).
fn parse<T: DeserializeOwned>(id: u64, sse: bool, body: &[u8]) -> Result<T, McpError> {
    let messages: Vec<Vec<u8>> = if sse {
        events(body)
    } else {
        vec![body.to_vec()]
    };
    for data in messages {
        let Ok(message) = serde_json::from_slice::<RpcMessage<serde_json::Value>>(&data) else {
            // Notifications and requests from the server have other shapes.
            continue;
        };
        if message.id != Some(id) {
            continue;
        }
        if let Some(RpcError { code, message }) = message.error {
            return Err(error(format!("MCP error {code}: {message}")));
        }
        let result = message
            .result
            .ok_or_else(|| error("an MCP response without a result"))?;
        return serde_json::from_value(result)
            .map_err(|e| error(format!("an unexpected MCP result: {e}")));
    }
    Err(error("no response to the request"))
}

/// The data of each server-sent event in `body`.
fn events(body: &[u8]) -> Vec<Vec<u8>> {
    let text = String::from_utf8_lossy(body);
    let mut events = Vec::new();
    let mut data: Option<String> = None;
    for line in text.lines() {
        if line.is_empty() {
            events.extend(data.take().map(String::into_bytes));
        } else if let Some(value) = line.strip_prefix("data:") {
            let value = value.strip_prefix(' ').unwrap_or(value);
            match &mut data {
                Some(data) => {
                    data.push('\n');
                    data.push_str(value);
                }
                None => data = Some(value.to_owned()),
            }
        }
    }
    events.extend(data.map(String::into_bytes));
    events
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use crate::adapters::mcp::{events, parse};

    #[derive(Debug, Deserialize, PartialEq)]
    struct Answer {
        ok: bool,
    }

    #[test]
    fn reads_json_and_event_streams() {
        let json = br#"{"jsonrpc":"2.0","id":3,"result":{"ok":true}}"#;
        assert_eq!(
            parse::<Answer>(3, false, json).unwrap(),
            Answer { ok: true }
        );
        let sse = b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n\n: comment\ndata: {\"jsonrpc\":\"2.0\",\"id\":4,\n data: \"result\":{\"ok\":true}}\n\n";
        assert_eq!(events(sse).len(), 2);
        assert!(
            parse::<Answer>(4, true, sse).is_err(),
            "a line not starting with data: is not data"
        );
        let sse = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"x\",\"params\":{}}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":4,\ndata: \"result\":{\"ok\":true}}\n\n";
        assert_eq!(parse::<Answer>(4, true, sse).unwrap(), Answer { ok: true });
        let error = br#"{"jsonrpc":"2.0","id":5,"error":{"code":-32602,"message":"bad"}}"#;
        let message = parse::<Answer>(5, false, error).unwrap_err().to_string();
        assert!(message.contains("bad"), "{message}");
        assert!(parse::<Answer>(6, false, json).is_err());
    }
}
