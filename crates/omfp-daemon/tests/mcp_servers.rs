//! Integration tests (L2) of the MCP server methods (M13) over a real WebSocket, on the fake launcher: the configured
//! list (DMN-190), the checks by the thread's omp (DMN-191) and the reconnect by restart (DMN-192).

mod client;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use client::Client;
use omfp_daemon::agent::{FakeCall, FakeLauncher};
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omp_rpc::{AgentStartEvent, RpcAgentEvent, RpcNotification};
use serde_json::{Value, json};
use tempfile::TempDir;

struct TestDaemon {
    handle: DaemonHandle,
    launcher: FakeLauncher,
    dir: TempDir,
}

impl TestDaemon {
    async fn start() -> Self {
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
        ])
        .unwrap();
        let launcher = FakeLauncher::new();
        let home = dir.path().join("home");
        let ports = Ports {
            launcher: Arc::new(launcher.clone()),
            git: Arc::new(FakeGit::new()),
            omp_program: "omp".into(),
            omp_version: omfp_core::OMP_VERSION.into(),
            omp_env: vec![("HOME".into(), Some(home.into()))],
            warnings: Vec::new(),
            clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
        };
        let handle = start_with(Config::from_cli(cli, None, None).unwrap(), ports)
            .await
            .unwrap();
        Self {
            handle,
            launcher,
            dir,
        }
    }

    async fn client(&self) -> Client {
        Client::hello(self.handle.addr(), self.handle.token().expose(), "test").await
    }

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

/// A thread in a new workspace whose folder holds a project config; the user config adds one server.
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
    let daemon = TestDaemon::start().await;
    let mut client = daemon.client().await;
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
            ("mine", "configured"),
            ("imported", "configured"),
        ])
    );
    assert_eq!(listing["threadId"], Value::Null);
    assert_eq!(listing["running"], false);
    let project = folder.join(".omp/mcp.json");
    assert_eq!(listing["projectConfig"], project.to_str().unwrap());
    let user = daemon.profile_agent().join("mcp.json");
    assert_eq!(listing["userConfig"], user.to_str().unwrap());
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
            "scope": "project",
            "tool": "omp",
        })
    );
    assert_eq!(
        listing["servers"][1]["target"],
        "/nonexistent/server --stdio"
    );
    assert_eq!(listing["servers"][3]["source"]["scope"], "user");
    assert_eq!(listing["servers"][3]["source"].get("workspacePath"), None);
    assert_eq!(listing["servers"][4]["source"]["tool"], "cursor");
    assert_eq!(listing["servers"][4]["checkable"], false);
    assert_eq!(daemon.launcher.launches().len(), 0);
}

// spec: PRT-111, PRT-114, DMN-191
#[tokio::test(flavor = "multi_thread")]
async fn a_running_thread_checks_its_servers_with_omp() {
    let daemon = TestDaemon::start().await;
    let mut client = daemon.client().await;
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
    daemon.launcher.local_output(
        "/mcp test mine",
        &["Server \"mine\" not found. Run /mcp list to see configured servers."],
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
            ("mine", "configured"),
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
        ["/mcp test good", "/mcp test broken", "/mcp test mine"],
        "omp checks only the enabled servers of its own files"
    );
}

// spec: PRT-112, DMN-192
#[tokio::test(flavor = "multi_thread")]
async fn reconnecting_restarts_an_idle_thread_and_checks_the_server_again() {
    let daemon = TestDaemon::start().await;
    let mut client = daemon.client().await;
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
    let daemon = TestDaemon::start().await;
    let mut client = daemon.client().await;
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
