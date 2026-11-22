//! The two remote MCP transports the daemon speaks (DMN-351): Streamable HTTP (`http`, protocol 2025-11-25, the
//! session revision omp 19.1.0 falls back to when a server lacks 2026-07-28) with its `Mcp-Session-Id` session, and
//! the legacy HTTP+SSE transport (`sse`), one
//! event stream per operation. Requests the server sends the client (`ping`, `roots/list`, …) are answered so it never
//! waits on them: `ping` with `{}`, anything else with "method not found".

use futures::{Stream, StreamExt};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Response, StatusCode, Url};
use serde_json::{Value, json};

/// The MCP revision the daemon offers, omp's `MCP_PROTOCOL_VERSION`.
pub(super) const PROTOCOL_VERSION: &str = "2025-11-25";
const SESSION_HEADER: &str = "mcp-session-id";
const VERSION_HEADER: &str = "mcp-protocol-version";
const EVENT_STREAM: &str = "text/event-stream";

/// Why an exchange with a server failed.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Failure {
    /// 401 or 403: the server refused the credentials.
    Auth(StatusCode),
    /// The server answered with a JSON-RPC error.
    Rpc { code: i64, message: String },
    /// The server no longer knows the Streamable HTTP session (404 to a request carrying its id).
    SessionGone,
    /// HTTP, network or protocol trouble.
    Transport(String),
}

fn transport(error: impl std::fmt::Display) -> Failure {
    Failure::Transport(error.to_string())
}

/// A Streamable HTTP session: the id the server gave at `initialize`, if any, and the revision it chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Session {
    pub id: Option<String>,
    pub protocol: String,
}

/// The configured headers under the generated ones, which win whatever their case (omp's `mergeMCPHeaders`); a
/// configured header that is not valid HTTP is left out.
pub(super) fn headers(configured: &[(String, String)], bearer: Option<&str>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in configured {
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        if name == SESSION_HEADER || name == VERSION_HEADER {
            continue;
        }
        map.insert(name, value);
    }
    if let Some(token) = bearer
        && let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}"))
    {
        map.insert(reqwest::header::AUTHORIZATION, value);
    }
    map
}

fn initialize_params() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": { "name": "omfp", "version": env!("CARGO_PKG_VERSION") },
    })
}

fn request(id: u64, method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn initialized() -> Value {
    json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
}

/// Server-sent events, parsed from a response body (WHATWG `text/event-stream`).
struct Events<S> {
    body: S,
    buffer: Vec<u8>,
    event: String,
    data: Vec<String>,
}

impl<S, B> Events<S>
where
    S: Stream<Item = reqwest::Result<B>> + Unpin,
    B: AsRef<[u8]>,
{
    fn new(body: S) -> Self {
        Self {
            body,
            buffer: Vec::new(),
            event: String::new(),
            data: Vec::new(),
        }
    }

    /// The next event as `(type, data)`; `None` once the stream ended.
    async fn next(&mut self) -> Result<Option<(String, String)>, Failure> {
        loop {
            while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<u8> = self.buffer.drain(..=end).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                let line = String::from_utf8_lossy(&line);
                if line.is_empty() {
                    if self.data.is_empty() {
                        self.event.clear();
                        continue;
                    }
                    let event = std::mem::take(&mut self.event);
                    let data = std::mem::take(&mut self.data).join("\n");
                    let event = if event.is_empty() {
                        "message".to_owned()
                    } else {
                        event
                    };
                    return Ok(Some((event, data)));
                }
                if line.starts_with(':') {
                    continue;
                }
                let (field, value) = line.split_once(':').unwrap_or((&line, ""));
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "event" => value.clone_into(&mut self.event),
                    "data" => self.data.push(value.to_owned()),
                    _ => {}
                }
            }
            match self.body.next().await {
                Some(Ok(chunk)) => self.buffer.extend_from_slice(chunk.as_ref()),
                Some(Err(error)) => return Err(transport(error)),
                None => return Ok(None),
            }
        }
    }
}

/// What one JSON-RPC message from the server means for request `id`.
enum Message {
    /// The answer to `id`.
    Answer(Result<Value, Failure>),
    /// A request of the server, to be answered with this message.
    Request(Value),
    /// A notification or an answer to another request.
    Other,
}

fn message(value: &Value, id: u64) -> Message {
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        let Some(request_id) = value.get("id") else {
            return Message::Other;
        };
        return Message::Request(if method == "ping" {
            json!({ "jsonrpc": "2.0", "id": request_id, "result": {} })
        } else {
            json!({ "jsonrpc": "2.0", "id": request_id, "error": { "code": -32601, "message": "Method not found" } })
        });
    }
    if value.get("id").and_then(Value::as_u64) != Some(id) {
        return Message::Other;
    }
    if let Some(error) = value.get("error") {
        return Message::Answer(Err(Failure::Rpc {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_owned(),
        }));
    }
    Message::Answer(Ok(value.get("result").cloned().unwrap_or(Value::Null)))
}

/// The messages of one JSON body or SSE `data`: a message or a batch of them.
fn messages(text: &str) -> Result<Vec<Value>, Failure> {
    match serde_json::from_str(text).map_err(transport)? {
        Value::Array(batch) => Ok(batch),
        single => Ok(vec![single]),
    }
}

async fn status_failure(response: Response) -> Failure {
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Failure::Auth(status);
    }
    let body = response.text().await.unwrap_or_default();
    let body: String = body.chars().take(300).collect();
    Failure::Transport(format!("HTTP {status}: {}", body.trim()))
}

fn is_event_stream(response: &Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with(EVENT_STREAM))
}

/// A Streamable HTTP endpoint with its headers.
pub(super) struct Http<'a> {
    pub client: &'a Client,
    pub url: &'a str,
    pub headers: &'a HeaderMap,
}

impl Http<'_> {
    async fn post(&self, session: Option<&Session>, body: &Value) -> Result<Response, Failure> {
        let mut builder = self
            .client
            .post(self.url)
            .headers(self.headers.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .json(body);
        if let Some(session) = session {
            if let Some(id) = &session.id {
                builder = builder.header(SESSION_HEADER, id);
            }
            builder = builder.header(VERSION_HEADER, &session.protocol);
        }
        builder.send().await.map_err(transport)
    }

    /// Sends request `id` and reads its answer from the JSON body or the event stream of the response.
    async fn exchange(
        &self,
        session: Option<&Session>,
        id: u64,
        body: &Value,
    ) -> Result<(Value, HeaderMap), Failure> {
        let response = self.post(session, body).await?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND && session.is_some_and(|session| session.id.is_some()) {
            return Err(Failure::SessionGone);
        }
        if !status.is_success() {
            return Err(status_failure(response).await);
        }
        let headers = response.headers().clone();
        if is_event_stream(&response) {
            let mut events = Events::new(response.bytes_stream());
            while let Some((_, data)) = events.next().await? {
                for value in messages(&data)? {
                    match message(&value, id) {
                        Message::Answer(answer) => return answer.map(|value| (value, headers)),
                        Message::Request(reply) => {
                            let _ = self.post(session, &reply).await;
                        }
                        Message::Other => {}
                    }
                }
            }
            return Err(Failure::Transport(
                "the event stream ended without an answer".into(),
            ));
        }
        let text = response.text().await.map_err(transport)?;
        for value in messages(&text)? {
            if let Message::Answer(answer) = message(&value, id) {
                return answer.map(|value| (value, headers));
            }
        }
        Err(Failure::Transport("the response holds no answer".into()))
    }

    /// `initialize`, then `notifications/initialized`: a new session (MCP lifecycle).
    pub(super) async fn initialize(&self) -> Result<Session, Failure> {
        let (result, headers) = self
            .exchange(None, 0, &request(0, "initialize", &initialize_params()))
            .await?;
        let session = Session {
            id: headers
                .get(SESSION_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            protocol: result
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION)
                .to_owned(),
        };
        let response = self.post(Some(&session), &initialized()).await?;
        if !response.status().is_success() {
            return Err(status_failure(response).await);
        }
        Ok(session)
    }

    /// Request `method` in `session`, answered with its `result`.
    pub(super) async fn request(
        &self,
        session: &Session,
        id: u64,
        method: &str,
        params: &Value,
    ) -> Result<Value, Failure> {
        self.exchange(Some(session), id, &request(id, method, params))
            .await
            .map(|(result, _)| result)
    }
}

/// One operation over the legacy HTTP+SSE transport: open the event stream, learn the message endpoint, initialize,
/// send `method` and read its answer from the stream, then close it.
pub(super) async fn sse_request(
    client: &Client,
    url: &str,
    headers: &HeaderMap,
    method: &str,
    params: &Value,
) -> Result<Value, Failure> {
    let response = client
        .get(url)
        .headers(headers.clone())
        .header(ACCEPT, EVENT_STREAM)
        .send()
        .await
        .map_err(transport)?;
    if !response.status().is_success() {
        return Err(status_failure(response).await);
    }
    let mut events = Events::new(response.bytes_stream());
    let endpoint = loop {
        match events.next().await? {
            Some((event, data)) if event == "endpoint" => {
                break Url::parse(url)
                    .and_then(|base| base.join(data.trim()))
                    .map_err(transport)?;
            }
            Some(_) => {}
            None => {
                return Err(Failure::Transport(
                    "the event stream sent no endpoint".into(),
                ));
            }
        }
    };
    let post = |body: Value| {
        let builder = client
            .post(endpoint.clone())
            .headers(headers.clone())
            .header(CONTENT_TYPE, "application/json")
            .json(&body);
        async move {
            let response = builder.send().await.map_err(transport)?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(status_failure(response).await)
            }
        }
    };
    let answer = async |events: &mut Events<_>, id: u64| -> Result<Value, Failure> {
        while let Some((event, data)) = events.next().await? {
            if event != "message" {
                continue;
            }
            for value in messages(&data)? {
                match message(&value, id) {
                    Message::Answer(answer) => return answer,
                    Message::Request(reply) => post(reply).await?,
                    Message::Other => {}
                }
            }
        }
        Err(Failure::Transport(
            "the event stream ended without an answer".into(),
        ))
    };
    post(request(1, "initialize", &initialize_params())).await?;
    answer(&mut events, 1).await?;
    post(initialized()).await?;
    post(request(2, method, params)).await?;
    answer(&mut events, 2).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(parts: &[&str]) -> impl Stream<Item = reqwest::Result<Vec<u8>>> + Unpin {
        futures::stream::iter(
            parts
                .iter()
                .map(|part| Ok(part.as_bytes().to_vec()))
                .collect::<Vec<_>>(),
        )
    }

    // spec: DMN-351
    #[tokio::test]
    async fn events_are_parsed_across_chunks_with_crlf_comments_and_multiline_data() {
        let mut events = Events::new(chunks(&[
            ": keep-alive\r\n\r\nevent: endpoint\r\ndata: /messages?session=1\r\n\r\n",
            "data: {\"a\":\ndata: 1}\n",
            "\nevent: ping\n\n",
            "data: tail",
        ]));
        assert_eq!(
            events.next().await.unwrap(),
            Some(("endpoint".to_owned(), "/messages?session=1".to_owned()))
        );
        assert_eq!(
            events.next().await.unwrap(),
            Some(("message".to_owned(), "{\"a\":\n1}".to_owned()))
        );
        // An event without data dispatches nothing; an unterminated one is dropped at the end.
        assert_eq!(events.next().await.unwrap(), None);
    }

    // spec: DMN-351
    #[test]
    fn generated_headers_win_over_configured_ones_whatever_their_case() {
        let map = headers(
            &[
                ("authorization".into(), "Basic x".into()),
                ("X-Team".into(), "core".into()),
                ("Mcp-Session-Id".into(), "forged".into()),
                ("bad header".into(), "x".into()),
            ],
            Some("tok"),
        );
        assert_eq!(map.get("Authorization").unwrap(), "Bearer tok");
        assert_eq!(map.get("x-team").unwrap(), "core");
        assert_eq!(map.get(SESSION_HEADER), None);
        assert_eq!(map.len(), 2);
        assert_eq!(
            headers(&[("authorization".into(), "Basic x".into())], None)
                .get("authorization")
                .unwrap(),
            "Basic x"
        );
    }

    // spec: DMN-351
    #[test]
    fn server_requests_are_answered_and_errors_read() {
        assert!(matches!(
            message(&json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" }), 1),
            Message::Request(reply) if reply == json!({ "jsonrpc": "2.0", "id": 7, "result": {} })
        ));
        assert!(matches!(
            message(&json!({ "jsonrpc": "2.0", "id": "r", "method": "roots/list" }), 1),
            Message::Request(reply) if reply["error"]["code"] == -32601
        ));
        assert!(matches!(
            message(
                &json!({ "jsonrpc": "2.0", "method": "notifications/progress" }),
                1
            ),
            Message::Other
        ));
        assert!(matches!(
            message(&json!({ "jsonrpc": "2.0", "id": 2, "result": {} }), 1),
            Message::Other
        ));
        assert!(matches!(
            message(&json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32602, "message": "Unknown tool" } }), 1),
            Message::Answer(Err(Failure::Rpc { code: -32602, message })) if message == "Unknown tool"
        ));
    }
}
