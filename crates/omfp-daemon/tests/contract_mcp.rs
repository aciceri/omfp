//! M10 contract test (L3): an omp session **outside omfp** — the pinned omp in an isolated environment, launched by
//! the test, not by the daemon — configured with an `mcp.json` pointing at the daemon's MCP server (AGT-054), calls
//! `omfp_call` through MCP (AGT-050): what it creates records the MCP provenance and omp's client name (AGT-051), and
//! the activity log keeps its action without a thread (AGT-053).

mod client;
mod common;

use std::sync::Arc;

use clap::Parser;
use client::Client;
use common::{Harness, Recorder, TIMEOUT, prompt, start};
use fake_llm::{Reply, Step, ToolCall};
use omfp_daemon::agent::FakeLauncher;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omp_rpc::PromptStatus;
use serde_json::{Value, json};
use tempfile::TempDir;

/// omp 19.1.0's MCP `clientInfo.name`.
const OMP_CLIENT: &str = "omp-coding-agent";

/// A daemon on the fake launcher: the omp under test is the only real one.
async fn daemon() -> (DaemonHandle, TempDir) {
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
    let ports = Ports {
        launcher: Arc::new(FakeLauncher::new()),
        git: Arc::new(FakeGit::new()),
        omp_program: "omp".into(),
        omp_version: omfp_core::OMP_VERSION.into(),
        omp_env: Vec::new(),
        user_omp_env: omfp_testkit::absent_standalone_omp(&dir),
        warnings: Vec::new(),
        clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
    };
    let handle = start_with(Config::from_cli(cli, None, None).unwrap(), ports)
        .await
        .unwrap();
    (handle, dir)
}

/// The text content of the tool result answering `call_id` in a chat-completions request.
fn tool_result(request: &Value, call_id: &str) -> String {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == call_id)
        .map_or_else(
            || panic!("no result for {call_id}: {request:#}"),
            |message| message["content"].as_str().unwrap_or_default().to_owned(),
        )
}

/// omp 19.1 mounts MCP tools as bash tool commands: the model runs one with its JSON arguments (`-j`) in a `bash` call.
fn command_call(id: &str, tool: &str, arguments: &Value) -> ToolCall {
    let json = arguments.to_string();
    assert!(!json.contains('\''), "single-quoted for the shell: {json}");
    ToolCall::new(
        id,
        "bash",
        json!({ "command": format!("{tool} -j '{json}'") }),
    )
}

// spec: AGT-050, AGT-051, AGT-053, AGT-054, DMN-155, DMN-157, DMN-158
#[tokio::test(flavor = "multi_thread")]
async fn an_omp_outside_omfp_calls_omfp_call_through_mcp() {
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![
            command_call(
                "create",
                "mcp__omfp_call",
                &json!({ "method": "threads.create", "params": { "workspace": { "new": {} }, "title": "made over mcp" } }),
            ),
            command_call(
                "list",
                "mcp__omfp_call",
                &json!({ "method": "threads.list" }),
            ),
        ])),
        Step::reply(Reply::text("MCP-DONE")),
    ])
    .await;
    let (daemon, _data) = daemon().await;
    // What `omfp mcp config` prints (DMN-159), in the isolated omp's user-level mcp.json: the token comes from
    // `$OMFP_TOKEN`, which omp expands in `mcp.json` (AGT-054).
    let mcp_json = json!({
        "mcpServers": {
            "omfp": {
                "type": "http",
                "url": format!("http://{}/mcp", daemon.addr()),
                "headers": { "Authorization": "Bearer ${OMFP_TOKEN}" },
                "timeout": 0,
            }
        }
    });
    std::fs::write(
        harness.env.agent_dir().join("mcp.json"),
        serde_json::to_string_pretty(&mcp_json).unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(harness.workspace()).unwrap();
    let mut spec = harness.spec(&[]);
    // Wait for the MCP connection before the first turn instead of omp's 250 ms window.
    spec.env
        .push(("OMP_MCP_STARTUP_TIMEOUT_MS".into(), Some("0".into())));
    spec.env
        .push(("OMFP_TOKEN".into(), Some(daemon.token().expose().into())));

    let stderr = harness.thread.path().join("omp.log");
    tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        let turn = process
            .client()
            .prompt_and_wait(&prompt("Use omfp over MCP"), TIMEOUT)
            .unwrap_or_else(|error| {
                panic!(
                    "{error}\nomp stderr:\n{}",
                    std::fs::read_to_string(&stderr).unwrap_or_default()
                )
            });
        assert_eq!(turn.result.unwrap().status, PromptStatus::Completed);
        recorder.wait_until("session_settled", |events| {
            Recorder::count(events, "session_settled") >= 1
        });
    })
    .await
    .unwrap();

    let requests = harness.fake.requests();
    let system = common::system_prompt(&requests[0]);
    for command in ["mcp__omfp_call", "mcp__omfp_thread_wait"] {
        assert!(system.contains(command), "{command} not offered:\n{system}");
    }
    assert!(
        system.contains("omfp_thread_wait` waits for a thread"),
        "the server's instructions:\n{system}"
    );
    let last = requests.last().unwrap();
    let created: Value = serde_json::from_str(&tool_result(last, "create"))
        .unwrap_or_else(|_| panic!("{}", tool_result(last, "create")));
    assert_eq!(created["title"], "made over mcp");
    assert_eq!(
        created["provenance"],
        json!({ "createdBy": { "kind": "mcp" }, "client": OMP_CLIENT })
    );
    let listed = tool_result(last, "list");
    assert!(listed.contains("\"threads\""), "{listed}");

    let mut owner = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let threads = owner.ok("threads.list", json!({})).await;
    assert_eq!(threads["threads"][0]["id"], created["id"]);
    let activity = owner.ok("activity.list", json!({})).await;
    assert_eq!(
        activity["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| (
                entry["threadId"].clone(),
                entry["client"].clone(),
                entry["method"].clone()
            ))
            .collect::<Vec<_>>(),
        [(Value::Null, json!(OMP_CLIENT), json!("threads.create"))]
    );
    assert!(harness.fake.is_exhausted_cleanly());
    harness.assert_no_stray_network();
}
