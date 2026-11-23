//! Integration tests (L2) of the MCP server methods (M13) over a real WebSocket, on the fake launcher: the configured
//! list (DMN-190), the checks by the thread's omp (DMN-191), the reconnect by restart (DMN-192), the refusal of `/mcp`
//! commands writing the user config (DMN-350) and the daemon's own calls to a workspace's server (PRT-220, PRT-221).

mod client;

use std::path::{Path, PathBuf};

use client::{Client, TestDaemon};
use omfp_daemon::agent::FakeCall;
use omfp_daemon::config::Config;
use omp_rpc::{AgentStartEvent, RpcAgentEvent, RpcNotification};
use serde_json::{Value, json};

/// A daemon whose omp runs with `HOME` at `<dir>/home`.
async fn daemon() -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::loopback(dir.path(), &[]);
    let home = dir.path().join("home");
    TestDaemon::launch(dir, config, |ports| {
        ports.omp_env = vec![("HOME".into(), Some(home.into()))];
    })
    .await
}

impl TestDaemon {
    fn profile_agent(&self) -> PathBuf {
        self.dir.path().join("omp-profile/agent")
    }

    /// The `/mcp test …` prompts omp received, over every launch.
    fn checks(&self) -> Vec<String> {
        self.launcher
            .launches()
            .iter()
            .flat_map(|launch| launch.process.calls())
            .filter_map(|call| match call {
                FakeCall::Prompt(prompt) if prompt.message.starts_with("/mcp ") => {
                    Some(prompt.message)
                }
                _ => None,
            })
            .collect()
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A thread in a new workspace whose folder holds a project config; omfp's profile holds a user config too, which
/// belongs to no workspace.
async fn thread_with_servers(
    daemon: &TestDaemon,
    client: &mut Client,
) -> (String, String, PathBuf) {
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "mcp" }),
        )
        .await;
    let workspace_id = thread["workspaceId"].as_str().unwrap().to_owned();
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    let folder = PathBuf::from(index["state"]["workspaces"][0]["path"].as_str().unwrap());
    write(
        &folder.join(".omp/mcp.json"),
        r#"{"mcpServers":{"good":{"type":"http","url":"http://127.0.0.1:9/mcp?token=x"},"broken":{"command":"/nonexistent/server","args":["--stdio"]},"off":{"command":"off","enabled":false}}}"#,
    );
    write(
        &folder.join(".cursor/mcp.json"),
        r#"{"mcpServers":{"imported":{"command":"cursor-server"}}}"#,
    );
    write(
        &daemon.profile_agent().join("mcp.json"),
        r#"{"mcpServers":{"mine":{"command":"my-server"}}}"#,
    );
    (
        thread["id"].as_str().unwrap().to_owned(),
        workspace_id,
        folder,
    )
}

fn states(listing: &Value) -> Vec<(String, String)> {
    listing["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|server| {
            (
                server["name"].as_str().unwrap().to_owned(),
                server["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(name, state)| ((*name).to_owned(), (*state).to_owned()))
        .collect()
}

// spec: PRT-110, PRT-113, DMN-190
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_lists_its_configured_servers_without_starting_omp() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (_, workspace_id, folder) = thread_with_servers(&daemon, &mut client).await;

    let listing = client
        .ok(
            "workspaces.mcp_servers",
            json!({ "workspaceId": workspace_id }),
        )
        .await;

    assert_eq!(
        states(&listing),
        pairs(&[
            ("good", "configured"),
            ("broken", "configured"),
            ("off", "disabled"),
            ("imported", "configured"),
        ])
    );
    assert_eq!(listing["threadId"], Value::Null);
    assert_eq!(listing["running"], false);
    let project = folder.join(".omp/mcp.json");
    assert_eq!(listing["projectConfig"], project.to_str().unwrap());
    assert_eq!(listing.get("userConfig"), None);
    let good = &listing["servers"][0];
    assert_eq!(
        (&good["transport"], &good["target"], &good["checkable"]),
        (
            &json!("http"),
            &json!("http://127.0.0.1:9/mcp"),
            &json!(true)
        )
    );
    assert_eq!(
        good["source"],
        json!({
            "path": project.to_str().unwrap(),
            "workspacePath": ".omp/mcp.json",
            "tool": "omp",
        })
    );
    assert_eq!(
        listing["servers"][1]["target"],
        "/nonexistent/server --stdio"
    );
    assert_eq!(listing["servers"][3]["source"]["tool"], "cursor");
    assert_eq!(listing["servers"][3]["checkable"], false);
    assert_eq!(listing["servers"].as_array().unwrap().len(), 4);
    assert_eq!(daemon.launcher.launches().len(), 0);
}

// spec: PRT-111, PRT-114, DMN-191
#[tokio::test(flavor = "multi_thread")]
async fn a_running_thread_checks_its_servers_with_omp() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (thread, _, _) = thread_with_servers(&daemon, &mut client).await;

    let stopped = client
        .ok("thread.mcp_servers", json!({ "threadId": thread }))
        .await;
    assert_eq!(stopped["running"], false);
    assert_eq!(stopped["servers"][0]["state"], "configured");
    assert_eq!(
        daemon.launcher.launches().len(),
        0,
        "listing does not start omp"
    );

    daemon.launcher.local_output(
        "/mcp test good",
        &["Server \"good\" connected (2 tools).\n  - ping\n  - pong"],
    );
    daemon.launcher.local_output(
        "/mcp test broken",
        &["Connection to \"broken\" failed: ENOENT: no such file or directory, posix_spawn '/nonexistent/server'"],
    );
    client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    let checked = client
        .ok("thread.mcp_servers", json!({ "threadId": thread }))
        .await;

    assert_eq!(checked["running"], true);
    assert_eq!(checked["threadId"], thread.as_str());
    assert_eq!(
        states(&checked),
        pairs(&[
            ("good", "connected"),
            ("broken", "failed"),
            ("off", "disabled"),
            ("imported", "configured"),
        ])
    );
    assert_eq!(checked["servers"][0]["tools"], 2);
    assert_eq!(
        checked["servers"][1]["error"],
        "ENOENT: no such file or directory, posix_spawn '/nonexistent/server'"
    );
    assert_eq!(
        daemon.checks(),
        ["/mcp test good", "/mcp test broken"],
        "omp checks only the enabled servers of its own files"
    );
}

// spec: PRT-112, DMN-192
#[tokio::test(flavor = "multi_thread")]
async fn reconnecting_restarts_an_idle_thread_and_checks_the_server_again() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (thread, _, _) = thread_with_servers(&daemon, &mut client).await;
    daemon
        .launcher
        .local_output("/mcp test good", &["Server \"good\" connected (3 tools)."]);
    client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;

    let listing = client
        .ok(
            "thread.mcp_reconnect",
            json!({ "threadId": thread, "name": "good" }),
        )
        .await;

    assert_eq!(daemon.launcher.launches().len(), 2, "omp restarted");
    assert!(daemon.launcher.process(0).is_closed());
    assert_eq!(listing["servers"][0]["state"], "connected");
    assert_eq!(listing["servers"][0]["tools"], 3);
    assert_eq!(listing["servers"][1]["state"], "configured");
    assert_eq!(daemon.checks(), ["/mcp test good"]);
}

// spec: PRT-112, DMN-192
#[tokio::test(flavor = "multi_thread")]
async fn a_working_thread_is_not_restarted_to_reconnect() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (thread, _, _) = thread_with_servers(&daemon, &mut client).await;
    let topic = format!("thread/{thread}");
    client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    client.ok("subscribe", json!({ "topic": topic })).await;
    let event: AgentStartEvent = serde_json::from_value(json!({ "type": "agent_start" })).unwrap();
    daemon
        .launcher
        .process(0)
        .emit(vec![RpcNotification::RpcAgentEvent(
            RpcAgentEvent::AgentStart(event),
        )]);
    loop {
        let event = client.event(&topic).await;
        if event["event"].to_string().contains("agent_start") {
            break;
        }
    }

    let refused = client
        .call("thread.mcp_reconnect", json!({ "threadId": thread }))
        .await
        .unwrap_err();

    assert_eq!(refused["code"], "conflict", "{refused}");
    assert_eq!(daemon.launcher.launches().len(), 1);
}

// spec: DMN-350
#[tokio::test(flavor = "multi_thread")]
async fn mcp_commands_that_would_write_the_user_config_are_refused_before_omp() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (thread, _, _) = thread_with_servers(&daemon, &mut client).await;

    let refused = client
        .call(
            "thread.prompt",
            json!({ "threadId": thread, "text": "/mcp add gh --scope user --url https://gh.example/mcp" }),
        )
        .await
        .unwrap_err();
    let imported = client
        .call(
            "thread.prompt",
            json!({ "threadId": thread, "text": "/mcp disable imported" }),
        )
        .await
        .unwrap_err();
    client
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": "/mcp disable good" }),
        )
        .await;

    assert_eq!(refused["code"], "bad_request", "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("MCP servers belong to the workspace")
    );
    assert_eq!(imported["code"], "bad_request", "{imported}");
    assert_eq!(daemon.checks(), ["/mcp disable good"]);
}

/// A Streamable HTTP server with one tool, `echo`, answering every request as JSON.
async fn echo_server() -> String {
    use axum::Json;
    use axum::response::IntoResponse;
    async fn mcp(Json(body): Json<Value>) -> axum::response::Response {
        let Some(id) = body.get("id").cloned() else {
            return axum::http::StatusCode::ACCEPTED.into_response();
        };
        let result = match body["method"].as_str() {
            Some("initialize") => {
                json!({ "protocolVersion": "2025-11-25", "capabilities": { "tools": {} }, "serverInfo": { "name": "echo" } })
            }
            Some("tools/list") => {
                json!({ "tools": [{ "name": "echo", "inputSchema": { "type": "object" } }] })
            }
            _ => {
                json!({ "content": [{ "type": "text", "text": body["params"]["arguments"]["text"] }] })
            }
        };
        Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let app = axum::Router::new().route("/mcp", axum::routing::post(mcp));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

// spec: PRT-220, PRT-221
#[tokio::test(flavor = "multi_thread")]
async fn a_widget_lists_and_calls_a_workspace_server_without_starting_omp() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (_, workspace_id, folder) = thread_with_servers(&daemon, &mut client).await;
    let url = echo_server().await;
    write(
        &folder.join(".omp/mcp.json"),
        &json!({ "mcpServers": {
            "echo": { "type": "http", "url": url },
            "broken": { "command": "/nonexistent/server" },
        }})
        .to_string(),
    );

    let tools = client
        .ok(
            "mcp.tools",
            json!({ "workspaceId": workspace_id, "server": "echo" }),
        )
        .await;
    let called = client
        .ok(
            "mcp.call",
            json!({ "workspaceId": workspace_id, "server": "echo", "tool": "echo", "arguments": { "text": "hi" } }),
        )
        .await;
    let stdio = client
        .call(
            "mcp.tools",
            json!({ "workspaceId": workspace_id, "server": "broken" }),
        )
        .await
        .unwrap_err();
    let user_level = client
        .call(
            "mcp.tools",
            json!({ "workspaceId": workspace_id, "server": "mine" }),
        )
        .await
        .unwrap_err();

    assert_eq!(
        tools,
        json!({ "workspaceId": workspace_id, "server": "echo", "tools": [{ "name": "echo", "inputSchema": { "type": "object" } }] })
    );
    assert_eq!(
        called,
        json!({ "content": [{ "type": "text", "text": "hi" }], "isError": false })
    );
    assert_eq!(stdio["code"], "bad_request", "{stdio}");
    assert_eq!(user_level["code"], "not_found", "{user_level}");
    assert_eq!(daemon.launcher.launches().len(), 0, "no omp was started");
}
