//! Integration tests (L2) of the MCP server over HTTP (DMN-155..158, AGT-050..053): a daemon on port 0 with the fake
//! launcher, spoken to as an MCP client of the legacy revision (an `initialize` session, as omp 18.18.6 does) and of
//! the current one (stateless, `_meta` and headers on every request).

mod client;

use std::sync::Arc;

use clap::Parser;
use client::Client;
use omfp_daemon::agent::FakeLauncher;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use tempfile::TempDir;

const CURRENT: &str = "2026-07-28";
const LEGACY: &str = "2025-11-25";

struct Daemon {
    handle: DaemonHandle,
    launcher: FakeLauncher,
    _dir: TempDir,
}

async fn daemon() -> Daemon {
    let dir = tempfile::tempdir().unwrap();
    let workspaces = dir.path().join("workspaces");
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        dir.path().to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        "--allowed-origin",
        "https://omfp.example.ts.net",
    ])
    .unwrap();
    let launcher = FakeLauncher::new();
    let ports = Ports {
        launcher: Arc::new(launcher.clone()),
        git: Arc::new(FakeGit::new()),
        omp_program: "omp".into(),
        omp_version: omfp_core::OMP_VERSION.into(),
        omp_env: Vec::new(),
        warnings: Vec::new(),
        clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
    };
    let handle = start_with(Config::from_cli(cli, None, None).unwrap(), ports)
        .await
        .unwrap();
    Daemon {
        handle,
        launcher,
        _dir: dir,
    }
}

impl Daemon {
    fn url(&self) -> String {
        format!("http://{}/mcp", self.handle.addr())
    }

    fn token(&self) -> &str {
        self.handle.token().expose()
    }

    async fn owner(&self) -> Client {
        Client::hello(self.handle.addr(), self.token(), "test").await
    }

    /// A POST with the token and the headers every MCP client sends.
    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        reqwest::Client::new()
            .post(self.url())
            .bearer_auth(self.token())
            .header("accept", "application/json, text/event-stream")
            .json(body)
    }
}

/// An answer: status, headers and the JSON-RPC message (the `message` event of an SSE stream).
struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    message: Value,
}

async fn answer(request: reqwest::RequestBuilder) -> Answer {
    let response = request.send().await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let text = response.text().await.unwrap();
    let message = if headers
        .get(CONTENT_TYPE)
        .is_some_and(|value| value == "text/event-stream")
    {
        let data = text
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap_or_else(|| panic!("no data in {text:?}"));
        serde_json::from_str(data).unwrap()
    } else if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("not JSON: {text:?}"))
    };
    Answer {
        status,
        headers,
        message,
    }
}

#[allow(clippy::needless_pass_by_value)] // Callers build params inline.
fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// A legacy session, opened with `initialize` as `client`.
struct Session<'a> {
    daemon: &'a Daemon,
    id: String,
}

impl<'a> Session<'a> {
    async fn open(daemon: &'a Daemon, client: &str) -> (Self, Value) {
        let answer = answer(daemon.post(&request(
            1,
            "initialize",
            json!({
                "protocolVersion": LEGACY,
                "capabilities": {},
                "clientInfo": { "name": client, "version": "1.0.0" },
            }),
        )))
        .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.message);
        let id = answer.headers["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let session = Self { daemon, id };
        let initialized = session
            .daemon
            .post(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .header("mcp-session-id", &session.id)
            .header("mcp-protocol-version", LEGACY)
            .send()
            .await
            .unwrap();
        assert_eq!(initialized.status(), StatusCode::ACCEPTED);
        (session, answer.message["result"].clone())
    }

    async fn call(&self, id: u64, method: &str, params: Value) -> Answer {
        answer(
            self.daemon
                .post(&request(id, method, params))
                .header("mcp-session-id", &self.id)
                .header("mcp-protocol-version", LEGACY),
        )
        .await
    }

    /// `tools/call` of `name`: its text and whether it is an error.
    async fn tool(&self, name: &str, arguments: Value) -> (String, bool) {
        let answer = self
            .call(
                9,
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.message);
        assert_eq!(answer.headers[CONTENT_TYPE], "text/event-stream");
        let result = &answer.message["result"];
        (
            result["content"][0]["text"].as_str().unwrap().to_owned(),
            result["isError"].as_bool().unwrap(),
        )
    }
}

/// A request of the current revision with its `_meta` and headers.
fn modern(daemon: &Daemon, id: u64, method: &str, mut params: Value) -> reqwest::RequestBuilder {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": CURRENT,
        "io.modelcontextprotocol/clientInfo": { "name": "modern-agent", "version": "2.0.0" },
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let name = params
        .get("name")
        .or_else(|| params.get("uri"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut builder = daemon
        .post(&request(id, method, params))
        .header("mcp-protocol-version", CURRENT)
        .header("mcp-method", method);
    if let Some(name) = name {
        builder = builder.header("mcp-name", name);
    }
    builder
}

// spec: DMN-155, DMN-157, AGT-050, AGT-051, AGT-053, PRT-093, PRT-094
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // One session's whole conversation.
async fn a_legacy_session_lists_and_calls_the_agent_tools_as_the_mcp_principal() {
    let daemon = daemon().await;
    let mut owner = daemon.owner().await;
    let existing = owner
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "existing" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (session, initialized) = Session::open(&daemon, "test-agent").await;
    assert_eq!(initialized["protocolVersion"], LEGACY);
    assert_eq!(initialized["serverInfo"]["name"], "omfp");
    assert!(initialized["capabilities"]["tools"].is_object());
    assert!(initialized["capabilities"]["resources"].is_object());
    assert!(
        initialized["instructions"]
            .as_str()
            .unwrap()
            .contains("omfp_call")
    );
    assert_eq!(session.id.len(), 32);

    let listed = session.call(2, "tools/list", json!({})).await;
    assert_eq!(listed.headers[CONTENT_TYPE], "application/json");
    let tools = listed.message["result"]["tools"]
        .as_array()
        .unwrap()
        .clone();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names[..2], ["omfp_call", "omfp_thread_wait"]);
    for expected in ["omfp_ui_mount", "omfp_component_list"] {
        assert!(names.contains(&expected), "{names:?}");
    }
    for absent in ["omfp_notify", "omfp_open_file"] {
        assert!(!names.contains(&absent), "{names:?}");
    }
    assert_eq!(tools[0]["title"], "omfp call");
    assert_eq!(tools[0]["inputSchema"]["type"], "object");
    assert!(listed.message["result"].get("resultType").is_none());

    let (text, is_error) = session
        .tool("omfp_call", json!({ "method": "threads.list" }))
        .await;
    assert!(!is_error, "{text}");
    let threads: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(threads["threads"][0]["id"], json!(existing));

    let (text, is_error) = session
        .tool(
            "omfp_call",
            json!({ "method": "threads.create", "params": { "workspace": { "new": {} }, "title": "from mcp" } }),
        )
        .await;
    assert!(!is_error, "{text}");
    let created: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        created["provenance"],
        json!({ "createdBy": { "kind": "mcp" }, "client": "test-agent" })
    );

    let (text, is_error) = session
        .tool("omfp_call", json!({ "method": "no.such.method" }))
        .await;
    assert!(is_error);
    assert!(text.contains("omfp://protocol/methods"), "{text}");

    let activity = owner.ok("activity.list", json!({})).await;
    let entry = &activity["entries"][0];
    assert_eq!(entry["threadId"], Value::Null);
    assert_eq!(entry["client"], "test-agent");
    assert_eq!(entry["method"], "threads.create");
    assert_eq!(
        activity["entries"].as_array().unwrap().len(),
        1,
        "{activity}"
    );

    let unknown = session
        .call(3, "tools/call", json!({ "name": "omfp_notify" }))
        .await;
    assert_eq!(unknown.status, StatusCode::OK);
    assert_eq!(unknown.message["error"]["code"], -32602);
    let not_an_object = session
        .call(
            4,
            "tools/call",
            json!({ "name": "omfp_call", "arguments": [] }),
        )
        .await;
    assert_eq!(not_an_object.message["error"]["code"], -32602);
    let ping = session.call(5, "ping", json!({})).await;
    assert_eq!(ping.message["result"], json!({}));
    let missing = session.call(6, "prompts/list", json!({})).await;
    assert_eq!(missing.status, StatusCode::OK);
    assert_eq!(missing.message["error"]["code"], -32601);

    let ended = reqwest::Client::new()
        .delete(daemon.url())
        .bearer_auth(daemon.token())
        .header("mcp-session-id", &session.id)
        .send()
        .await
        .unwrap();
    assert_eq!(ended.status(), StatusCode::OK);
    let gone = session.call(7, "tools/list", json!({})).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

// spec: DMN-157
#[tokio::test(flavor = "multi_thread")]
async fn initialize_answers_a_supported_legacy_version_or_the_newest() {
    let daemon = daemon().await;
    for (requested, answered) in [
        ("2025-06-18", "2025-06-18"),
        ("2025-03-26", "2025-03-26"),
        ("2024-11-05", LEGACY),
        (CURRENT, LEGACY),
    ] {
        let answer = answer(daemon.post(&request(
            1,
            "initialize",
            json!({ "protocolVersion": requested, "capabilities": {}, "clientInfo": { "name": "x" } }),
        )))
        .await;
        assert_eq!(
            answer.message["result"]["protocolVersion"], answered,
            "{requested}"
        );
    }
    let no_session = answer(
        daemon
            .post(&request(2, "tools/list", json!({})))
            .header("mcp-protocol-version", LEGACY),
    )
    .await;
    assert_eq!(no_session.status, StatusCode::BAD_REQUEST);
    assert!(
        no_session.message["error"]["message"]
            .as_str()
            .unwrap()
            .contains("initialize"),
        "{}",
        no_session.message
    );
    let (session, _) = Session::open(&daemon, "x").await;
    let bad_version = answer(
        daemon
            .post(&request(3, "tools/list", json!({})))
            .header("mcp-session-id", &session.id)
            .header("mcp-protocol-version", "1999-01-01"),
    )
    .await;
    assert_eq!(bad_version.status, StatusCode::BAD_REQUEST);
}

// spec: DMN-155, DMN-156, AGT-050, AGT-052
#[tokio::test(flavor = "multi_thread")]
async fn the_current_revision_is_served_statelessly_with_its_metadata() {
    let daemon = daemon().await;

    let discover = answer(modern(&daemon, 1, "server/discover", json!({}))).await;
    assert_eq!(discover.status, StatusCode::OK, "{}", discover.message);
    assert!(discover.headers.get("mcp-session-id").is_none());
    let result = &discover.message["result"];
    assert_eq!(
        result["supportedVersions"],
        json!([CURRENT, "2025-11-25", "2025-06-18", "2025-03-26"])
    );
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["ttlMs"], 3_600_000);
    assert_eq!(result["cacheScope"], "private");
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "omfp"
    );

    let tools = answer(modern(&daemon, 2, "tools/list", json!({}))).await;
    assert_eq!(tools.message["result"]["tools"][0]["name"], "omfp_call");
    assert_eq!(tools.message["result"]["ttlMs"], 3_600_000);

    let called = answer(modern(
        &daemon,
        3,
        "tools/call",
        json!({ "name": "omfp_call", "arguments": { "method": "workspaces.list" } }),
    ))
    .await;
    assert_eq!(called.headers[CONTENT_TYPE], "text/event-stream");
    let result = &called.message["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(result["resultType"], "complete");
    assert!(result.get("ttlMs").is_none());
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, json!({ "workspaces": [] }));

    let resources = answer(modern(&daemon, 4, "resources/list", json!({}))).await;
    assert_eq!(
        resources.message["result"]["resources"][0]["uri"],
        "omfp://protocol/methods"
    );
    let templates = answer(modern(&daemon, 5, "resources/templates/list", json!({}))).await;
    assert_eq!(
        templates.message["result"]["resourceTemplates"][0]["uriTemplate"],
        "omfp://protocol/methods/{name}"
    );
    let list = answer(modern(
        &daemon,
        6,
        "resources/read",
        json!({ "uri": "omfp://protocol/methods" }),
    ))
    .await;
    let contents = &list.message["result"]["contents"][0];
    assert_eq!(contents["mimeType"], "text/markdown");
    assert!(
        contents["text"]
            .as_str()
            .unwrap()
            .contains("- `threads.list` — ")
    );
    let one = answer(modern(
        &daemon,
        7,
        "resources/read",
        json!({ "uri": "omfp://protocol/methods/threads.create" }),
    ))
    .await;
    let contents = &one.message["result"]["contents"][0];
    assert_eq!(contents["mimeType"], "application/json");
    let schema: Value = serde_json::from_str(contents["text"].as_str().unwrap()).unwrap();
    assert_eq!(schema["method"], "threads.create");
    let unknown = answer(modern(
        &daemon,
        8,
        "resources/read",
        json!({ "uri": "omfp://protocol/methods/subscribe" }),
    ))
    .await;
    assert_eq!(unknown.message["error"]["code"], -32602);

    let missing = answer(modern(&daemon, 9, "prompts/list", json!({}))).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.message["error"]["code"], -32601);
}

// spec: AGT-052, DMN-198
#[tokio::test(flavor = "multi_thread")]
async fn the_docs_are_resources_and_a_threads_workspace_uris_are_not() {
    let daemon = daemon().await;
    let resources = answer(modern(&daemon, 1, "resources/list", json!({}))).await;
    let listed: Vec<&str> = resources.message["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|resource| resource["uri"].as_str())
        .collect();
    for uri in [
        "omfp://docs",
        "omfp://docs/widgets",
        "omfp://docs/shell",
        "omfp://keymap",
    ] {
        assert!(listed.contains(&uri), "{uri}: {listed:?}");
    }
    let docs = answer(modern(
        &daemon,
        2,
        "resources/read",
        json!({ "uri": "omfp://docs/widgets" }),
    ))
    .await;
    let contents = &docs.message["result"]["contents"][0];
    assert_eq!(contents["mimeType"], "text/markdown");
    assert!(
        contents["text"]
            .as_str()
            .unwrap()
            .starts_with("# omfp widgets\n")
    );
    // A thread's workspace URIs are no resources: their tools are (`omfp_widget_check`, `omfp_windows`).
    for (id, uri) in [(3, "omfp://windows"), (4, "omfp://widgets/check/tests")] {
        let refused = answer(modern(&daemon, id, "resources/read", json!({ "uri": uri }))).await;
        assert_eq!(refused.message["error"]["code"], -32602, "{uri}");
    }
}

// spec: DMN-156
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // Every header check in turn.
async fn a_request_of_the_current_revision_must_carry_matching_metadata_and_headers() {
    let daemon = daemon().await;
    let call = json!({ "name": "omfp_call", "arguments": { "method": "workspaces.list" } });

    let unsupported = answer(
        daemon
            .post(&request(
                1,
                "tools/list",
                json!({ "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2099-01-01",
                    "io.modelcontextprotocol/clientCapabilities": {},
                } }),
            ))
            .header("mcp-protocol-version", "2099-01-01")
            .header("mcp-method", "tools/list"),
    )
    .await;
    assert_eq!(unsupported.status, StatusCode::BAD_REQUEST);
    assert_eq!(unsupported.message["error"]["code"], -32022);
    assert_eq!(
        unsupported.message["error"]["data"]["requested"],
        "2099-01-01"
    );
    assert_eq!(
        unsupported.message["error"]["data"]["supported"][0],
        CURRENT
    );

    let no_capabilities = answer(
        daemon
            .post(&request(
                2,
                "tools/list",
                json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": CURRENT } }),
            ))
            .header("mcp-protocol-version", CURRENT)
            .header("mcp-method", "tools/list"),
    )
    .await;
    assert_eq!(no_capabilities.status, StatusCode::BAD_REQUEST);
    assert_eq!(no_capabilities.message["error"]["code"], -32602);

    let mut params = call.clone();
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": CURRENT,
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    for (headers, missing) in [
        (
            vec![("mcp-method", "tools/call"), ("mcp-name", "omfp_call")],
            "MCP-Protocol-Version",
        ),
        (
            vec![("mcp-protocol-version", CURRENT), ("mcp-name", "omfp_call")],
            "Mcp-Method",
        ),
        (
            vec![
                ("mcp-protocol-version", CURRENT),
                ("mcp-method", "tools/call"),
            ],
            "Mcp-Name",
        ),
        (
            vec![
                ("mcp-protocol-version", CURRENT),
                ("mcp-method", "tools/list"),
                ("mcp-name", "omfp_call"),
            ],
            "Mcp-Method",
        ),
        (
            vec![
                ("mcp-protocol-version", CURRENT),
                ("mcp-method", "tools/call"),
                ("mcp-name", "omfp_thread_wait"),
            ],
            "Mcp-Name",
        ),
    ] {
        let mut builder = daemon.post(&request(3, "tools/call", params.clone()));
        for (name, value) in &headers {
            builder = builder.header(*name, HeaderValue::from_str(value).unwrap());
        }
        let refused = answer(builder).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{headers:?}");
        assert_eq!(refused.message["error"]["code"], -32020, "{headers:?}");
        assert!(
            refused.message["error"]["message"]
                .as_str()
                .unwrap()
                .contains(missing),
            "{headers:?}: {}",
            refused.message
        );
    }

    let encoded = answer(
        daemon
            .post(&request(4, "tools/call", params))
            .header("mcp-protocol-version", CURRENT)
            .header("mcp-method", "tools/call")
            .header("mcp-name", "=?base64?b21mcF9jYWxs?="),
    )
    .await;
    assert_eq!(encoded.status, StatusCode::OK, "{}", encoded.message);
    assert_eq!(encoded.message["result"]["isError"], false);
}

// spec: DMN-158, DMN-155
#[tokio::test(flavor = "multi_thread")]
async fn requests_without_the_token_or_from_a_foreign_origin_are_refused() {
    let daemon = daemon().await;
    let list = request(1, "tools/list", json!({}));
    let http = reqwest::Client::new();

    let anonymous = http.post(daemon.url()).json(&list).send().await.unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.headers()["www-authenticate"], "Bearer");
    let body: Value = anonymous.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("omfp mcp config"),
        "{body}"
    );

    for token in ["wrong", &daemon.token()[1..]] {
        let refused = http
            .post(daemon.url())
            .bearer_auth(token)
            .json(&list)
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED, "{token}");
    }
    let not_bearer = http
        .post(daemon.url())
        .header("authorization", daemon.token())
        .json(&list)
        .send()
        .await
        .unwrap();
    assert_eq!(not_bearer.status(), StatusCode::UNAUTHORIZED);

    let foreign = daemon
        .post(&list)
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
    let allowed = answer(
        daemon
            .post(&request(
                2,
                "initialize",
                json!({ "protocolVersion": LEGACY }),
            ))
            .header("origin", "https://omfp.example.ts.net"),
    )
    .await;
    assert_eq!(allowed.status, StatusCode::OK);

    let get = http
        .get(daemon.url())
        .bearer_auth(daemon.token())
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::METHOD_NOT_ALLOWED);
    let not_json = answer(daemon.post(&json!(null)).body("{")).await;
    assert_eq!(not_json.status, StatusCode::BAD_REQUEST);
    assert_eq!(not_json.message["error"]["code"], -32700);
}

// spec: DMN-221
#[tokio::test(flavor = "multi_thread")]
async fn an_agents_login_shows_its_flow_in_the_most_recently_focused_window() {
    use omfp_core::wire::{ExtensionUiRequest, OpenUrlUiRequest, RpcNotification};
    let daemon = daemon().await;
    daemon.launcher.reply(
        "get_login_providers",
        omfp_daemon::agent::FakeReply::Ok(json!({ "providers": [
            { "id": "anthropic", "name": "Anthropic (Claude Pro/Max)", "available": true, "authenticated": false }
        ]})),
    );
    let (session, _) = Session::open(&daemon, "test-agent").await;
    let login = json!({ "method": "auth.login", "params": { "providerId": "anthropic" } });

    let (text, is_error) = session.tool("omfp_call", login.clone()).await;
    assert!(is_error, "{text}");
    assert!(text.contains("no omfp window is open"), "{text}");

    let mut unfocused = daemon.owner().await;
    unfocused
        .ok(
            "widgets.report",
            json!({ "workspaceId": null, "views": null, "widgets": [], "window": { "focused": false } }),
        )
        .await;
    let mut window = daemon.owner().await;
    window
        .ok(
            "widgets.report",
            json!({ "workspaceId": null, "views": null, "widgets": [], "window": { "focused": true } }),
        )
        .await;
    let launcher = daemon.launcher.clone();
    let gate = launcher.gate("login");
    let call = tokio::spawn(async move {
        let (session, _) = Session::open(&daemon, "test-agent").await;
        session.tool("omfp_call", login).await
    });
    gate.arrived(1).await;
    // Without a window the first call started no omp; this login's `open_url` goes to the focused window.
    let omp = launcher.process(0);
    omp.emit(vec![RpcNotification::ExtensionUiRequest(
        ExtensionUiRequest::OpenUrl(OpenUrlUiRequest {
            id: "u1".into(),
            url: "https://claude.ai/oauth/authorize".into(),
            launch_url: None,
            instructions: None,
        }),
    )]);
    let frame = window.frame().await;
    assert_eq!(frame["type"], "window_request", "{frame}");
    assert_eq!(frame["request"]["kind"], "login_url");
    assert_eq!(frame["request"]["url"], "https://claude.ai/oauth/authorize");
    gate.open();
    let (text, is_error) = call.await.unwrap();
    assert!(!is_error, "{text}");
    assert!(text.contains("Anthropic (Claude Pro/Max)"), "{text}");
}
