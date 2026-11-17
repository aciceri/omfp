//! The MCP server (DMN-155..158, AGT-050..053): the agent tool registry over the Model Context Protocol's Streamable
//! HTTP transport at `/mcp`, for agents the daemon does not run. Dual-era: revision 2026-07-28, stateless, every
//! request carrying its version and client in `_meta` ([`modern`]); and the legacy revisions that open a session with
//! `initialize` ([`Sessions`]). The methods themselves are in [`methods`].

mod methods;

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Map, Value, json};

use super::{AppState, Principal, origin_allowed};
use methods::{Reply, ToolCall};

/// The endpoint's path.
pub(crate) const PATH: &str = "/mcp";

/// The current revision, served statelessly (DMN-156).
pub(crate) const CURRENT: &str = "2026-07-28";

/// The revisions that open a session with `initialize` (DMN-157), newest first.
pub(crate) const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

/// The largest request body (DMN-155).
const MAX_BODY: usize = 4 * 1024 * 1024;

/// Legacy sessions kept at once; one more forgets the least recently used (DMN-157).
const MAX_SESSIONS: usize = 256;

/// How often a running `tools/call` stream sends a comment (DMN-155).
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The client name when an MCP client names none (AGT-051).
const UNNAMED_CLIENT: &str = "mcp";

const SESSION_HEADER: &str = "mcp-session-id";
const VERSION_HEADER: &str = "mcp-protocol-version";
const METHOD_HEADER: &str = "mcp-method";
const NAME_HEADER: &str = "mcp-name";

/// JSON-RPC and MCP error codes.
mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    /// Implementation-defined: authentication, origin, unknown session.
    pub const SERVER_ERROR: i64 = -32000;
    pub const HEADER_MISMATCH: i64 = -32020;
    pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
}

/// A JSON-RPC error and the HTTP status it is answered with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RpcError {
    status: StatusCode,
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn new(status: StatusCode, code: i64, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            data: None,
        }
    }

    /// An error of the method itself: answered `200`, the JSON-RPC body saying what failed.
    pub(crate) fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(StatusCode::OK, code::INVALID_PARAMS, message)
    }

    /// A request the server refuses: answered `400`.
    fn bad_request(code: i64, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    fn header_mismatch(message: impl Into<String>) -> Self {
        Self::bad_request(code::HEADER_MISMATCH, message)
    }

    fn body(&self, id: &Value) -> Value {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(data) = &self.data {
            error["data"] = data.clone();
        }
        json!({ "jsonrpc": "2.0", "id": id, "error": error })
    }

    fn response(&self, id: &Value) -> Response {
        (self.status, json_body(&self.body(id))).into_response()
    }
}

/// The era a request is served in (DMN-156, DMN-157).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Era {
    /// 2026-07-28: results carry `resultType`, the server's identity and caching hints.
    Modern,
    /// A legacy session's revision.
    Legacy,
}

/// One legacy session (DMN-157).
#[derive(Debug, Clone)]
struct Session {
    client: String,
    /// Higher is more recently used.
    used: u64,
}

/// The legacy sessions opened by `initialize` (DMN-157), in memory.
#[derive(Debug, Default)]
pub(crate) struct Sessions {
    inner: Mutex<SessionMap>,
}

#[derive(Debug, Default)]
struct SessionMap {
    sessions: HashMap<String, Session>,
    clock: u64,
}

impl Sessions {
    /// Opens a session for `client` and returns its id: 128 random bits, hex encoded.
    fn open(&self, client: String) -> Result<String, RpcError> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| {
            RpcError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                code::SERVER_ERROR,
                format!("no randomness for a session id: {error}"),
            )
        })?;
        let id = hex::encode(bytes);
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if map.sessions.len() >= MAX_SESSIONS
            && let Some(oldest) = map
                .sessions
                .iter()
                .min_by_key(|(_, session)| session.used)
                .map(|(id, _)| id.clone())
        {
            map.sessions.remove(&oldest);
        }
        map.clock += 1;
        let used = map.clock;
        map.sessions.insert(id.clone(), Session { client, used });
        Ok(id)
    }

    /// The client of session `id`, marking it used; `None` when there is no such session.
    fn client(&self, id: &str) -> Option<String> {
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        map.clock += 1;
        let used = map.clock;
        let session = map.sessions.get_mut(id)?;
        session.used = used;
        Some(session.client.clone())
    }

    /// Ends session `id`; whether it existed.
    fn end(&self, id: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sessions
            .remove(id)
            .is_some()
    }
}

/// `POST /mcp` (DMN-155).
pub(crate) async fn post(State(state): State<Arc<AppState>>, request: Request) -> Response {
    if let Some(refused) = refusal(&state, request.headers()) {
        return refused;
    }
    let (parts, body) = request.into_parts();
    let headers = parts.headers;
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return RpcError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            code::INVALID_REQUEST,
            format!("a request body is at most {MAX_BODY} bytes"),
        )
        .response(&Value::Null);
    };
    let message: Value = match serde_json::from_slice(&bytes) {
        Ok(message) => message,
        Err(error) => {
            return RpcError::bad_request(code::PARSE_ERROR, format!("not JSON: {error}"))
                .response(&Value::Null);
        }
    };
    let request = match JsonRpc::parse(message) {
        Ok(request) => request,
        Err(error) => return error.response(&Value::Null),
    };
    let Some(id) = request.id.clone() else {
        // A notification (`notifications/initialized`, `notifications/cancelled`): nothing to answer.
        return StatusCode::ACCEPTED.into_response();
    };
    match serve(&state, &headers, request) {
        Ok((reply, session)) => {
            let mut response = match reply {
                Reply::Value(result) => {
                    (StatusCode::OK, json_body(&result_body(&id, &result))).into_response()
                }
                Reply::Tool(call) => tool_stream(id, call),
            };
            if let Some(session) = session
                && let Ok(value) = HeaderValue::from_str(&session)
            {
                response.headers_mut().insert(SESSION_HEADER, value);
            }
            response
        }
        Err(error) => error.response(&id),
    }
}

/// `DELETE /mcp` (DMN-157): ends the legacy session the request names.
pub(crate) async fn delete(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(refused) = refusal(&state, &headers) {
        return refused;
    }
    let Some(id) = header_str(&headers, SESSION_HEADER) else {
        return RpcError::bad_request(
            code::INVALID_REQUEST,
            "DELETE ends a session: send its Mcp-Session-Id",
        )
        .response(&Value::Null);
    };
    if state.mcp_sessions.end(id) {
        StatusCode::OK.into_response()
    } else {
        unknown_session().response(&Value::Null)
    }
}

/// The origin check, then the bearer token (DMN-158): the refusal, if any.
fn refusal(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let origin = header_str(headers, header::ORIGIN.as_str());
    let host = header_str(headers, header::HOST.as_str());
    if !origin_allowed(origin, host, &state.allowed_origins) {
        tracing::warn!(
            origin,
            host,
            "refused an MCP request from a foreign origin (DMN-158)"
        );
        return Some(
            RpcError::new(
                StatusCode::FORBIDDEN,
                code::SERVER_ERROR,
                "origin not allowed",
            )
            .response(&Value::Null),
        );
    }
    let presented = header_str(headers, header::AUTHORIZATION.as_str())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    if presented.is_some_and(|token| state.token.matches(token)) {
        return None;
    }
    let mut refused = RpcError::new(
        StatusCode::UNAUTHORIZED,
        code::SERVER_ERROR,
        "unauthorized: send `Authorization: Bearer <token>` with the omfp daemon's token (`omfp mcp config` prints \
         the client configuration, which reads the token from $OMFP_TOKEN: is it set where the client runs?)",
    )
    .response(&Value::Null);
    refused
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    Some(refused)
}

/// A decoded JSON-RPC request or notification.
#[derive(Debug, Clone, PartialEq)]
struct JsonRpc {
    /// `None` for a notification.
    id: Option<Value>,
    method: String,
    params: Map<String, Value>,
}

impl JsonRpc {
    /// One JSON-RPC 2.0 request or notification; a batch, a response or anything else is `-32600` (DMN-155).
    fn parse(message: Value) -> Result<Self, RpcError> {
        let invalid = |why: &str| {
            RpcError::bad_request(
                code::INVALID_REQUEST,
                format!("invalid JSON-RPC request: {why}"),
            )
        };
        let Value::Object(mut message) = message else {
            return Err(invalid("send one JSON-RPC object per request (no batches)"));
        };
        if message.get("jsonrpc") != Some(&json!("2.0")) {
            return Err(invalid("`jsonrpc` must be \"2.0\""));
        }
        let Some(Value::String(method)) = message.remove("method") else {
            return Err(invalid(
                "no `method` (a client sends requests and notifications only)",
            ));
        };
        let id = match message.remove("id") {
            None => None,
            Some(id @ (Value::String(_) | Value::Number(_))) => Some(id),
            Some(_) => return Err(invalid("`id` must be a string or a number")),
        };
        let params = match message.remove("params") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(params)) => params,
            Some(_) => return Err(invalid("`params` must be an object")),
        };
        Ok(Self { id, method, params })
    }

    /// `params._meta[key]`.
    fn meta(&self, key: &str) -> Option<&Value> {
        self.params.get("_meta")?.get(key)
    }
}

/// Serves a request in its era: the reply, and the id of a session `initialize` opened.
fn serve(
    state: &AppState,
    headers: &HeaderMap,
    request: JsonRpc,
) -> Result<(Reply, Option<String>), RpcError> {
    if request.method == "initialize" {
        let (result, client) = methods::initialize(&request.params);
        let session = state.mcp_sessions.open(client)?;
        return Ok((Reply::Value(result), Some(session)));
    }
    let (era, client) = if let Some(session) = header_str(headers, SESSION_HEADER) {
        legacy(state, headers, session)?
    } else {
        (Era::Modern, modern(headers, &request)?)
    };
    let principal = Principal::Mcp { client };
    let reply = methods::call(
        &state.services,
        principal,
        era,
        &request.method,
        request.params,
    )?;
    Ok((reply, None))
}

/// A request of legacy session `session` (DMN-157): its client.
fn legacy(state: &AppState, headers: &HeaderMap, session: &str) -> Result<(Era, String), RpcError> {
    if let Some(version) = header_str(headers, VERSION_HEADER)
        && version != CURRENT
        && !LEGACY.contains(&version)
    {
        return Err(RpcError::bad_request(
            code::INVALID_REQUEST,
            format!(
                "unsupported MCP-Protocol-Version `{version}`; supported: {}",
                supported().join(", ")
            ),
        ));
    }
    let client = state
        .mcp_sessions
        .client(session)
        .ok_or_else(unknown_session)?;
    Ok((Era::Legacy, client))
}

/// Checks a stateless request of the current revision (DMN-156): its client.
fn modern(headers: &HeaderMap, request: &JsonRpc) -> Result<String, RpcError> {
    let Some(version) = request
        .meta("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
    else {
        if header_str(headers, VERSION_HEADER).is_none_or(|version| LEGACY.contains(&version)) {
            return Err(RpcError::bad_request(
                code::INVALID_REQUEST,
                format!(
                    "no Mcp-Session-Id: send `initialize` first, or use protocol version {CURRENT} with \
                     `_meta[\"io.modelcontextprotocol/protocolVersion\"]` on every request"
                ),
            ));
        }
        return Err(RpcError::bad_request(
            code::INVALID_PARAMS,
            "missing `_meta[\"io.modelcontextprotocol/protocolVersion\"]`",
        ));
    };
    if version != CURRENT {
        if LEGACY.contains(&version) {
            return Err(RpcError::bad_request(
                code::INVALID_REQUEST,
                format!(
                    "protocol version {version} needs a session: send `initialize` first, then its Mcp-Session-Id"
                ),
            ));
        }
        let mut error = RpcError::bad_request(
            code::UNSUPPORTED_PROTOCOL_VERSION,
            "Unsupported protocol version",
        );
        error.data = Some(json!({ "supported": supported(), "requested": version }));
        return Err(error);
    }
    if !request
        .meta("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(Value::is_object)
    {
        return Err(RpcError::bad_request(
            code::INVALID_PARAMS,
            "missing `_meta[\"io.modelcontextprotocol/clientCapabilities\"]`",
        ));
    }
    let header = |name| header_str(headers, name);
    expect_header("MCP-Protocol-Version", header(VERSION_HEADER), version)?;
    expect_header("Mcp-Method", header(METHOD_HEADER), &request.method)?;
    let name_field = match request.method.as_str() {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    };
    if let Some(field) = name_field {
        let name = request
            .params
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default();
        let presented = match header(NAME_HEADER).map(decode_header) {
            Some(None) => {
                return Err(RpcError::header_mismatch(
                    "Header mismatch: Mcp-Name is not valid `=?base64?…?=`",
                ));
            }
            presented => presented.flatten(),
        };
        expect_header("Mcp-Name", presented.as_deref(), name)?;
    }
    Ok(client_name(
        request.meta("io.modelcontextprotocol/clientInfo"),
    ))
}

/// The client `info` (`clientInfo`) names, else [`UNNAMED_CLIENT`] (AGT-051).
fn client_name(info: Option<&Value>) -> String {
    info.and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(UNNAMED_CLIENT)
        .to_owned()
}

/// `HeaderMismatch` unless header `label` was `presented` as exactly `expected`.
fn expect_header(label: &str, presented: Option<&str>, expected: &str) -> Result<(), RpcError> {
    match presented {
        Some(value) if value == expected => Ok(()),
        Some(value) => Err(RpcError::header_mismatch(format!(
            "Header mismatch: {label} header value '{value}' does not match body value '{expected}'"
        ))),
        None => Err(RpcError::header_mismatch(format!(
            "Header mismatch: missing {label}"
        ))),
    }
}

/// A header value as sent, or decoded from the `=?base64?…?=` sentinel; `None` for an invalid encoding.
fn decode_header(value: &str) -> Option<String> {
    match value
        .strip_prefix("=?base64?")
        .and_then(|rest| rest.strip_suffix("?="))
    {
        Some(encoded) => String::from_utf8(BASE64.decode(encoded).ok()?).ok(),
        None => Some(value.to_owned()),
    }
}

/// Every supported version, current first.
fn supported() -> Vec<&'static str> {
    std::iter::once(CURRENT)
        .chain(LEGACY.iter().copied())
        .collect()
}

fn unknown_session() -> RpcError {
    RpcError::new(
        StatusCode::NOT_FOUND,
        code::SERVER_ERROR,
        "unknown or ended MCP session: send `initialize` again",
    )
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn result_body(id: &Value, result: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn json_body(value: &Value) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

/// `tools/call`'s SSE stream (DMN-155): a comment every [`KEEP_ALIVE`] while the tool runs, then the response as one
/// `message` event. Dropping the stream (the client closed it) drops the tool's body: the call is cancelled.
fn tool_stream(id: Value, call: ToolCall) -> Response {
    let stream = futures::stream::once(async move {
        let result = call.run().await;
        Ok::<_, Infallible>(
            Event::default()
                .event("message")
                .data(result_body(&id, &result).to_string()),
        )
    });
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE).text(""))
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

#[cfg(test)]
mod tests;
