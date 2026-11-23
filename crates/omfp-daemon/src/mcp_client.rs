//! The daemon as an MCP client of a workspace's remote servers (DMN-351..DMN-354): `mcp.tools` and `mcp.call`
//! (PRT-220, PRT-221) need no thread and no omp. The daemon reads the server's entry from the workspace's config
//! (`omp_mcp::remote`), resolves its headers as omp does, sends the OAuth login omp stored for the server
//! (`credentials`), refreshing it the way omp does, and speaks Streamable HTTP (keeping the session) or the legacy
//! HTTP+SSE transport (`transport`). stdio servers are refused.

mod credentials;
mod transport;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use omfp_protocol::{McpCallResult, McpTool, McpTransport};
use serde_json::{Map, Value, json};
use tracing::warn;

use self::credentials::{CredentialError, Credentials, Token};
use self::transport::{Failure, Http, Session};
use crate::lock;
use crate::omp_mcp::{self, Remote, Unavailable};

/// How long one operation may take when the server's entry sets no `timeout`.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// omp caps a `!command` header value at ten seconds.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
/// At most this many `tools/list` pages.
const MAX_PAGES: usize = 50;

/// Why the daemon could not use a workspace's MCP server (DMN-354).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum McpError {
    #[error("no MCP server `{0}` is configured in this workspace")]
    NotFound(String),
    #[error("MCP server `{0}` is disabled (`enabled: false`)")]
    Disabled(String),
    #[error("MCP server `{0}` is a stdio server: the daemon calls `http` and `sse` servers only")]
    Stdio(String),
    #[error("MCP server `{0}` has no `url`")]
    NoUrl(String),
    #[error(
        "MCP server `{server}` refused the credentials (HTTP {status}): log in with `/mcp reauth {server}` in a \
         thread of this workspace"
    )]
    Unauthorized { server: String, status: u16 },
    /// The server's JSON-RPC error: an unknown tool, invalid arguments, …
    #[error("MCP server `{server}`: {message} ({code})")]
    Rpc {
        server: String,
        code: i64,
        message: String,
    },
    #[error("MCP server `{server}`: {message}")]
    Transport { server: String, message: String },
    #[error("MCP server `{server}` did not answer within {millis} ms")]
    Timeout { server: String, millis: u128 },
    #[error("MCP server `{server}`: {message}")]
    Login { server: String, message: String },
}

fn unavailable(reason: Unavailable, server: &str) -> McpError {
    let server = server.to_owned();
    match reason {
        Unavailable::NotFound => McpError::NotFound(server),
        Unavailable::Disabled => McpError::Disabled(server),
        Unavailable::Stdio => McpError::Stdio(server),
        Unavailable::NoUrl => McpError::NoUrl(server),
    }
}

/// A Streamable HTTP session kept per workspace and server, for the URL it was opened on.
struct Kept {
    url: String,
    session: Session,
}

struct Inner {
    http: reqwest::Client,
    /// omp's credential store; `None` uses no stored login.
    credentials: Option<Credentials>,
    sessions: Mutex<HashMap<(PathBuf, String), Kept>>,
    /// The output of `!command` header values per workspace, run again after the server refused them.
    commands: Mutex<HashMap<(PathBuf, String), String>>,
    next_id: AtomicU64,
}

/// The daemon's MCP client. Cheap to clone.
#[derive(Clone)]
pub struct McpClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient").finish_non_exhaustive()
    }
}

impl McpClient {
    /// A client reading omp's logins from `agent_db` (`<data-dir>/omp-profile/agent/agent.db`, ARC-021).
    #[must_use]
    pub fn new(agent_db: Option<PathBuf>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("omfp/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            inner: Arc::new(Inner {
                http,
                credentials: agent_db.map(Credentials::new),
                sessions: Mutex::new(HashMap::new()),
                commands: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// Every tool of server `server` of the workspace folder `workspace`, in the server's order (PRT-220).
    ///
    /// # Errors
    ///
    /// The server is not callable, refused the credentials, answered an error, or could not be reached.
    pub async fn tools(&self, workspace: &Path, server: &str) -> Result<Vec<McpTool>, McpError> {
        let remote = self.remote(workspace, server).await?;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = cursor.map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
            let page = self
                .operation(workspace, &remote, "tools/list", params)
                .await?;
            tools.extend(
                page.get("tools")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(tool),
            );
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|cursor| !cursor.is_empty())
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    /// Calls tool `tool` of server `server` with `arguments` (PRT-221).
    ///
    /// # Errors
    ///
    /// As [`McpClient::tools`]; a tool that reports a failure is a result with `isError`, not an error.
    pub async fn call(
        &self,
        workspace: &Path,
        server: &str,
        tool: &str,
        arguments: Map<String, Value>,
    ) -> Result<McpCallResult, McpError> {
        let remote = self.remote(workspace, server).await?;
        let result = self
            .operation(
                workspace,
                &remote,
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
            )
            .await?;
        Ok(McpCallResult {
            content: result
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            structured_content: result
                .get("structuredContent")
                .filter(|value| !value.is_null())
                .cloned(),
            is_error: result.get("isError").and_then(Value::as_bool) == Some(true),
        })
    }

    async fn remote(&self, workspace: &Path, server: &str) -> Result<Remote, McpError> {
        let (folder, name) = (workspace.to_path_buf(), server.to_owned());
        tokio::task::spawn_blocking(move || {
            omp_mcp::remote(&folder, &name, &|var| std::env::var(var).ok())
        })
        .await
        .map_err(|error| McpError::Transport {
            server: server.to_owned(),
            message: error.to_string(),
        })?
        .map_err(|reason| unavailable(reason, server))
    }

    /// One JSON-RPC request to the server within its timeout; a refusal of the credentials is retried once with a
    /// refreshed login and `!command` headers run again (omp's `#wireAuthRefresh`).
    async fn operation(
        &self,
        workspace: &Path,
        remote: &Remote,
        method: &str,
        params: Value,
    ) -> Result<Value, McpError> {
        let timeout = remote.timeout.unwrap_or(DEFAULT_TIMEOUT);
        tokio::time::timeout(timeout, self.authorized(workspace, remote, method, &params))
            .await
            .map_err(|_| McpError::Timeout {
                server: remote.name.clone(),
                millis: timeout.as_millis(),
            })?
    }

    async fn authorized(
        &self,
        workspace: &Path,
        remote: &Remote,
        method: &str,
        params: &Value,
    ) -> Result<Value, McpError> {
        let mut configured = self.headers(workspace, remote, false).await;
        let mut token = self.token(remote, &configured, None, false).await?;
        let mut retried = false;
        loop {
            let headers = transport::headers(
                &configured,
                token.as_ref().map(|token| token.access.as_str()),
            );
            match self.send(workspace, remote, &headers, method, params).await {
                Ok(result) => return Ok(result),
                Err(Failure::Auth(_)) if !retried && (token.is_some() || has_commands(remote)) => {
                    retried = true;
                    configured = self.headers(workspace, remote, true).await;
                    token = self
                        .token(remote, &configured, token.as_ref(), true)
                        .await?;
                }
                Err(failure) => return Err(failed(remote, failure)),
            }
        }
    }

    /// Sends one request over the server's transport. A Streamable HTTP session is opened once and kept; when the
    /// server forgot it, it is opened again once (MCP lifecycle, session management).
    async fn send(
        &self,
        workspace: &Path,
        remote: &Remote,
        headers: &reqwest::header::HeaderMap,
        method: &str,
        params: &Value,
    ) -> Result<Value, Failure> {
        if remote.transport == McpTransport::Sse {
            return transport::sse_request(&self.inner.http, &remote.url, headers, method, params)
                .await;
        }
        let http = Http {
            client: &self.inner.http,
            url: &remote.url,
            headers,
        };
        let key = (workspace.to_path_buf(), remote.name.clone());
        for _ in 0..2 {
            let kept = lock(&self.inner.sessions)
                .get(&key)
                .filter(|kept| kept.url == remote.url)
                .map(|kept| kept.session.clone());
            let session = if let Some(session) = kept {
                session
            } else {
                let session = http.initialize().await?;
                lock(&self.inner.sessions).insert(
                    key.clone(),
                    Kept {
                        url: remote.url.clone(),
                        session: session.clone(),
                    },
                );
                session
            };
            let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
            match http.request(&session, id, method, params).await {
                Err(Failure::SessionGone) => {
                    lock(&self.inner.sessions).remove(&key);
                }
                answered => return answered,
            }
        }
        Err(Failure::SessionGone)
    }

    /// The entry's headers with their values resolved as omp's `resolveConfigValue` does: `!command` runs `sh -c` in
    /// the workspace folder (its trimmed output, kept until the server refuses it, `again`), the name of an
    /// environment variable set to a non-empty value gives its value (omp's `envValue || valueConfig`: one set empty
    /// leaves the name as written), anything else is literal; a value that resolves empty is left out.
    /// `headerPolicy: "origin-locked"` keeps every value literal.
    async fn headers(
        &self,
        workspace: &Path,
        remote: &Remote,
        again: bool,
    ) -> Vec<(String, String)> {
        let mut resolved = Vec::with_capacity(remote.headers.len());
        for (name, value) in &remote.headers {
            let value = if remote.literal_headers {
                Some(value.clone())
            } else if let Some(command) = value.strip_prefix('!') {
                self.command(workspace, command.trim(), again).await
            } else {
                Some(
                    std::env::var(value)
                        .ok()
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| value.clone()),
                )
            };
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                resolved.push((name.clone(), value));
            }
        }
        resolved
    }

    async fn command(&self, workspace: &Path, command: &str, again: bool) -> Option<String> {
        let key = (workspace.to_path_buf(), command.to_owned());
        if again {
            lock(&self.inner.commands).remove(&key);
        } else if let Some(output) = lock(&self.inner.commands).get(&key) {
            return Some(output.clone());
        }
        // omp runs `/bin/sh -c`; on the host when the daemon runs in Flatpak, like jobs (ENV-064).
        let mut shell = std::process::Command::new("sh");
        shell.arg("-c").arg(command).current_dir(workspace);
        let run = tokio::process::Command::from(crate::flatpak::on_host(shell))
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = match tokio::time::timeout(COMMAND_TIMEOUT, run).await {
            Ok(Ok(output)) if output.status.success() => output,
            Ok(Ok(output)) => {
                warn!(status = %output.status, "an MCP header command failed");
                return None;
            }
            Ok(Err(error)) => {
                warn!(%error, "cannot run an MCP header command");
                return None;
            }
            Err(_) => {
                warn!("an MCP header command timed out");
                return None;
            }
        };
        let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if text.is_empty() {
            return None;
        }
        lock(&self.inner.commands).insert(key, text.clone());
        Some(text)
    }

    /// omp's stored login for the server, unless the entry's own `Authorization` header stands in for it (then only
    /// its `auth.credentialId` counts).
    async fn token(
        &self,
        remote: &Remote,
        configured: &[(String, String)],
        observed: Option<&Token>,
        force: bool,
    ) -> Result<Option<Token>, McpError> {
        let Some(credentials) = &self.inner.credentials else {
            return Ok(None);
        };
        let explicit_only = configured
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"));
        credentials
            .token(&self.inner.http, remote, explicit_only, observed, force)
            .await
            .map_err(|error: CredentialError| McpError::Login {
                server: remote.name.clone(),
                message: error.to_string(),
            })
    }
}

fn has_commands(remote: &Remote) -> bool {
    !remote.literal_headers
        && remote
            .headers
            .iter()
            .any(|(_, value)| value.starts_with('!'))
}

fn failed(remote: &Remote, failure: Failure) -> McpError {
    let server = remote.name.clone();
    match failure {
        Failure::Auth(status) => McpError::Unauthorized {
            server,
            status: status.as_u16(),
        },
        Failure::Rpc { code, message } => McpError::Rpc {
            server,
            code,
            message,
        },
        Failure::SessionGone => McpError::Transport {
            server,
            message: "the server forgot the session twice".into(),
        },
        Failure::Transport(message) => McpError::Transport { server, message },
    }
}

/// One tool of a `tools/list` page; an entry without a name is skipped.
fn tool(value: &Value) -> Option<McpTool> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(McpTool {
        name: text("name")?,
        title: text("title"),
        description: text("description"),
        input_schema: value
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" })),
        output_schema: value.get("outputSchema").cloned(),
        annotations: value.get("annotations").cloned(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::convert::Infallible;

    use axum::extract::State;
    use axum::http::{HeaderMap as Headers, StatusCode};
    use axum::response::sse::{Event, Sse};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use futures::StreamExt;
    use omfp_protocol::{ErrorBody, ErrorCode};
    use rusqlite::Connection;
    use tokio::sync::mpsc;

    use super::*;
    use crate::store::now_ms;

    /// `(method, Authorization, Mcp-Session-Id, MCP-Protocol-Version)` of one request.
    type Request = (String, Option<String>, Option<String>, Option<String>);

    /// What the fake server saw and how it behaves.
    #[derive(Default)]
    struct Seen {
        initialized: u32,
        sessions: HashSet<String>,
        requests: Vec<Request>,
        /// Replies the client sent to the server's own requests.
        replies: Vec<Value>,
        /// Only this bearer token is accepted.
        token: Option<String>,
        /// The next request forgets every session.
        forget: bool,
        /// Answers come as an event stream after a `ping` of the server and a notification.
        stream: bool,
        /// The legacy transport's event stream.
        sse: Option<mpsc::UnboundedSender<Value>>,
    }

    type Fake = Arc<Mutex<Seen>>;

    fn header(headers: &Headers, name: &str) -> Option<String> {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    /// The answer of the fake server to request `body`, `None` to never answer.
    fn answer(body: &Value) -> Option<Value> {
        let id = body["id"].clone();
        let params = &body["params"];
        let error = |code: i64, message: &str| {
            Some(
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
            )
        };
        let result = match body["method"].as_str() {
            Some("initialize") => json!({
                "protocolVersion": transport::PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake", "version": "1" },
            }),
            Some("tools/list") if params["cursor"].is_null() => json!({
                "tools": [
                    { "name": "echo", "description": "Echoes", "inputSchema": { "type": "object" } },
                    { "title": "nameless" },
                ],
                "nextCursor": "2",
            }),
            Some("tools/list") => json!({
                "tools": [{ "name": "fail", "annotations": { "readOnlyHint": true } }],
            }),
            Some("tools/call") => match params["name"].as_str() {
                Some("echo") => json!({
                    "content": [{ "type": "text", "text": params["arguments"]["text"] }],
                    "structuredContent": params["arguments"],
                }),
                Some("fail") => {
                    json!({ "content": [{ "type": "text", "text": "no" }], "isError": true })
                }
                Some("hang") => return None,
                _ => return error(-32602, "Unknown tool"),
            },
            _ => return error(-32601, "Method not found"),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    async fn streamable(
        State(fake): State<Fake>,
        headers: Headers,
        Json(body): Json<Value>,
    ) -> Response {
        let method = body["method"].as_str().unwrap_or_default().to_owned();
        let session = header(&headers, "mcp-session-id");
        let answered = {
            let mut seen = fake.lock().unwrap();
            seen.requests.push((
                method.clone(),
                header(&headers, "authorization"),
                session.clone(),
                header(&headers, "mcp-protocol-version"),
            ));
            if let Some(token) = &seen.token
                && header(&headers, "authorization") != Some(format!("Bearer {token}"))
            {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            if body.get("method").is_none() {
                seen.replies.push(body);
                return StatusCode::ACCEPTED.into_response();
            }
            if method == "initialize" {
                seen.initialized += 1;
                let id = format!("s{}", seen.initialized);
                seen.sessions.insert(id.clone());
                return ([("mcp-session-id", id)], Json(answer(&body))).into_response();
            }
            if seen.forget {
                seen.forget = false;
                seen.sessions.clear();
            }
            if !session.is_some_and(|session| seen.sessions.contains(&session)) {
                return StatusCode::NOT_FOUND.into_response();
            }
            if method == "notifications/initialized" {
                return StatusCode::ACCEPTED.into_response();
            }
            answer(&body).map(|answer| (answer, seen.stream))
        };
        let Some((answer, stream)) = answered else {
            return futures::future::pending::<Response>().await;
        };
        if !stream {
            return Json(answer).into_response();
        }
        let ping = json!({ "jsonrpc": "2.0", "id": "p1", "method": "ping" });
        let progress = json!({ "jsonrpc": "2.0", "method": "notifications/progress" });
        let body = format!(
            ": hello\n\nevent: message\ndata: {ping}\n\ndata: {progress}\n\ndata: {answer}\n\n"
        );
        ([("content-type", "text/event-stream")], body).into_response()
    }

    async fn legacy_stream(State(fake): State<Fake>) -> Response {
        let (sender, receiver) = mpsc::unbounded_channel::<Value>();
        fake.lock().unwrap().sse = Some(sender);
        let endpoint = futures::stream::once(async {
            Ok::<_, Infallible>(Event::default().event("endpoint").data("/messages?sid=1"))
        });
        let messages = futures::stream::unfold(receiver, |mut receiver| async move {
            let message = receiver.recv().await?;
            Some((Ok(Event::default().data(message.to_string())), receiver))
        });
        Sse::new(endpoint.chain(messages)).into_response()
    }

    async fn legacy_post(State(fake): State<Fake>, Json(body): Json<Value>) -> StatusCode {
        let mut seen = fake.lock().unwrap();
        let method = body["method"].as_str().unwrap_or_default().to_owned();
        seen.requests.push((method, None, None, None));
        if body.get("id").is_some()
            && let (Some(answer), Some(sse)) = (answer(&body), &seen.sse)
        {
            let _ = sse.send(answer);
        }
        StatusCode::ACCEPTED
    }

    struct Setup {
        _dir: tempfile::TempDir,
        workspace: PathBuf,
        db: PathBuf,
        base: String,
        fake: Fake,
    }

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        base
    }

    async fn setup() -> Setup {
        let fake = Fake::default();
        let base = serve(
            Router::new()
                .route("/mcp", post(streamable))
                .route("/sse", get(legacy_stream))
                .route("/messages", post(legacy_post))
                .with_state(Arc::clone(&fake)),
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(workspace.join(".omp")).unwrap();
        let mcp = format!("{base}/mcp");
        let config = json!({ "mcpServers": {
            "fake": { "type": "http", "url": mcp },
            "legacy": { "type": "sse", "url": format!("{base}/sse") },
            "keyed": { "type": "http", "url": mcp, "headers": { "Authorization": "!printf 'Bearer from-command'" } },
            "slow": { "type": "http", "url": mcp, "timeout": 300 },
            "local": { "command": "fs" },
            "off": { "type": "http", "url": mcp, "enabled": false },
        }});
        std::fs::write(workspace.join(".omp/mcp.json"), config.to_string()).unwrap();
        let db = dir.path().join("agent.db");
        Connection::open(&db)
            .unwrap()
            .execute_batch(credentials::tests::SCHEMA)
            .unwrap();
        Setup {
            _dir: dir,
            workspace,
            db,
            base,
            fake,
        }
    }

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    // spec: DMN-351, PRT-220, PRT-221
    #[tokio::test]
    async fn tools_are_listed_across_pages_and_called_in_one_kept_session() {
        let s = setup().await;
        let client = McpClient::new(Some(s.db.clone()));

        let tools = client.tools(&s.workspace, "fake").await.unwrap();
        let echoed = client
            .call(&s.workspace, "fake", "echo", args(&json!({ "text": "hi" })))
            .await
            .unwrap();
        let failed = client
            .call(&s.workspace, "fake", "fail", Map::new())
            .await
            .unwrap();

        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["echo", "fail"]
        );
        assert_eq!(tools[0].description.as_deref(), Some("Echoes"));
        assert_eq!(tools[1].input_schema, json!({ "type": "object" }));
        assert_eq!(tools[1].annotations, Some(json!({ "readOnlyHint": true })));
        assert_eq!(
            echoed,
            McpCallResult {
                content: vec![json!({ "type": "text", "text": "hi" })],
                structured_content: Some(json!({ "text": "hi" })),
                is_error: false,
            }
        );
        assert!(failed.is_error);
        let seen = s.fake.lock().unwrap();
        assert_eq!(seen.initialized, 1, "one session for every request");
        let later: Vec<&Request> = seen.requests.iter().skip(1).collect();
        assert!(
            later.iter().all(|(_, _, session, version)| {
                session.as_deref() == Some("s1")
                    && version.as_deref() == Some(transport::PROTOCOL_VERSION)
            }),
            "{later:?}"
        );
    }

    // spec: DMN-351
    #[tokio::test]
    async fn an_event_stream_answer_is_read_and_the_server_ping_answered() {
        let s = setup().await;
        s.fake.lock().unwrap().stream = true;
        let client = McpClient::new(None);

        let result = client
            .call(
                &s.workspace,
                "fake",
                "echo",
                args(&json!({ "text": "streamed" })),
            )
            .await
            .unwrap();

        assert_eq!(
            result.content,
            vec![json!({ "type": "text", "text": "streamed" })]
        );
        assert_eq!(
            s.fake.lock().unwrap().replies,
            vec![json!({ "jsonrpc": "2.0", "id": "p1", "result": {} })]
        );
    }

    // spec: DMN-351
    #[tokio::test]
    async fn a_forgotten_session_is_opened_again_once() {
        let s = setup().await;
        let client = McpClient::new(None);
        client.tools(&s.workspace, "fake").await.unwrap();
        s.fake.lock().unwrap().forget = true;

        client
            .call(&s.workspace, "fake", "fail", Map::new())
            .await
            .unwrap();

        assert_eq!(s.fake.lock().unwrap().initialized, 2);
    }

    // spec: DMN-351
    #[tokio::test]
    async fn the_legacy_sse_transport_posts_to_the_announced_endpoint() {
        let s = setup().await;
        let client = McpClient::new(None);

        let result = client
            .call(
                &s.workspace,
                "legacy",
                "echo",
                args(&json!({ "text": "old" })),
            )
            .await
            .unwrap();

        assert_eq!(
            result.content,
            vec![json!({ "type": "text", "text": "old" })]
        );
        let methods: Vec<String> = s
            .fake
            .lock()
            .unwrap()
            .requests
            .iter()
            .map(|request| request.0.clone())
            .collect();
        assert_eq!(
            methods,
            vec!["initialize", "notifications/initialized", "tools/call"]
        );
    }

    // spec: DMN-352, DMN-353
    #[tokio::test]
    async fn omp_stored_login_is_sent_and_renewed_once_the_server_refuses_it() {
        let s = setup().await;
        let token_url = format!(
            "{}/token",
            serve(Router::new().route(
                "/token",
                post(|| async {
                    Json(json!({ "access_token": "renewed", "refresh_token": "r2", "expires_in": 3600 }))
                }),
            ))
            .await
        );
        let provider = format!("mcp_oauth:profile:default:{}/mcp", s.base);
        let stored = json!({ "access": "stale", "refresh": "r1", "expires": now_ms() + 3_600_000, "tokenUrl": token_url });
        Connection::open(&s.db)
            .unwrap()
            .execute(
                "INSERT INTO auth_credentials (provider, credential_type, data) VALUES (?1, 'oauth', ?2)",
                rusqlite::params![provider, stored.to_string()],
            )
            .unwrap();
        s.fake.lock().unwrap().token = Some("renewed".into());
        let client = McpClient::new(Some(s.db.clone()));

        client.tools(&s.workspace, "fake").await.unwrap();

        let auths: Vec<Option<String>> = s
            .fake
            .lock()
            .unwrap()
            .requests
            .iter()
            .map(|request| request.1.clone())
            .collect();
        assert_eq!(auths[0].as_deref(), Some("Bearer stale"));
        assert!(
            auths[1..]
                .iter()
                .all(|auth| auth.as_deref() == Some("Bearer renewed")),
            "{auths:?}"
        );
        let data: String = Connection::open(&s.db)
            .unwrap()
            .query_row("SELECT data FROM auth_credentials", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&data).unwrap()["refresh"],
            "r2"
        );
    }

    // spec: DMN-351, DMN-354
    #[tokio::test]
    async fn a_refusal_without_a_login_asks_for_reauth_and_header_commands_are_run() {
        let s = setup().await;
        s.fake.lock().unwrap().token = Some("from-command".into());
        let client = McpClient::new(Some(s.db.clone()));

        let refused = client.tools(&s.workspace, "fake").await;
        client.tools(&s.workspace, "keyed").await.unwrap();

        let refusal = McpError::Unauthorized {
            server: "fake".into(),
            status: 401,
        };
        assert_eq!(refused, Err(refusal.clone()));
        let body = ErrorBody::from(refusal);
        assert_eq!(body.code, ErrorCode::Unauthorized);
        assert_eq!(body.details.unwrap()["reauth"], "/mcp reauth fake");
    }

    // spec: DMN-353
    #[tokio::test]
    async fn a_refresh_outliving_the_operation_timeout_is_still_saved_for_omp() {
        let s = setup().await;
        let release = Arc::new(tokio::sync::Notify::new());
        let gate = Arc::clone(&release);
        let token_url = format!(
            "{}/token",
            serve(Router::new().route(
                "/token",
                post(move || {
                    let gate = Arc::clone(&gate);
                    async move {
                        gate.notified().await;
                        Json(json!({ "access_token": "late", "refresh_token": "r2", "expires_in": 3600 }))
                    }
                }),
            ))
            .await
        );
        let provider = format!("mcp_oauth:profile:default:{}/mcp", s.base);
        let stored =
            json!({ "access": "old", "refresh": "r1", "expires": now_ms(), "tokenUrl": token_url });
        Connection::open(&s.db)
            .unwrap()
            .execute(
                "INSERT INTO auth_credentials (provider, credential_type, data) VALUES (?1, 'oauth', ?2)",
                rusqlite::params![provider, stored.to_string()],
            )
            .unwrap();
        let client = McpClient::new(Some(s.db.clone()));

        // `slow` times out after 300 ms while the token endpoint still holds its answer.
        let timed_out = client.tools(&s.workspace, "slow").await;
        assert!(
            matches!(timed_out, Err(McpError::Timeout { .. })),
            "{timed_out:?}"
        );
        release.notify_one();

        // The exchange goes on without its caller: the rotated token lands in omp's row and the lease is given back.
        let saved = |db: &Path| -> (Value, i64) {
            let conn = Connection::open(db).unwrap();
            let data: String = conn
                .query_row("SELECT data FROM auth_credentials", [], |row| row.get(0))
                .unwrap();
            let leases = conn
                .query_row(
                    "SELECT COUNT(*) FROM auth_credential_refresh_leases",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            (serde_json::from_str(&data).unwrap(), leases)
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let (data, leases) = saved(&s.db);
            if data["refresh"] == "r2" && leases == 0 {
                assert_eq!(data["access"], "late");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{data} {leases}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    // spec: DMN-354
    #[tokio::test]
    async fn unusable_servers_and_server_errors_become_protocol_errors() {
        let s = setup().await;
        let client = McpClient::new(None);
        let code = |error: &McpError| ErrorBody::from(error.clone()).code;

        let missing = client.tools(&s.workspace, "missing").await.unwrap_err();
        let local = client.tools(&s.workspace, "local").await.unwrap_err();
        let off = client.tools(&s.workspace, "off").await.unwrap_err();
        let unknown = client
            .call(&s.workspace, "fake", "nope", Map::new())
            .await
            .unwrap_err();
        let slow = client
            .call(&s.workspace, "slow", "hang", Map::new())
            .await
            .unwrap_err();

        assert_eq!(missing, McpError::NotFound("missing".into()));
        assert_eq!(code(&missing), ErrorCode::NotFound);
        assert_eq!(local, McpError::Stdio("local".into()));
        assert_eq!(code(&local), ErrorCode::BadRequest);
        assert_eq!(off, McpError::Disabled("off".into()));
        assert_eq!(
            unknown,
            McpError::Rpc {
                server: "fake".into(),
                code: -32602,
                message: "Unknown tool".into()
            }
        );
        assert_eq!(ErrorBody::from(unknown).details.unwrap()["mcpCode"], -32602);
        assert_eq!(
            slow,
            McpError::Timeout {
                server: "slow".into(),
                millis: 300
            }
        );
        assert_eq!(code(&slow), ErrorCode::Internal);
    }
}
