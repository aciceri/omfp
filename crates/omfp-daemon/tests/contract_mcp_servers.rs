//! M13 acceptance, L3: the MCP servers of a thread against the pinned omp, the fake LLM and a stub MCP server on
//! loopback (Streamable HTTP, two tools): omp checks its own servers with `/mcp test` (DMN-191) and a reconnect
//! restarts the thread, which connects them again (DMN-192).
//!
//! Spike (07-roadmap, M13): omp 18.18.6 exposes no MCP state over RPC; `/mcp test <name>` over `prompt` is a local
//! command answering one `command_output` frame, `/mcp reconnect` is refused outside the TUI and `/mcp reload` does
//! not reconnect (the stub sees no new `initialize`).

mod client;
mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use clap::Parser;
use client::Client;
use common::{Harness, TIMEOUT};
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omfp_testkit::omp_bin;
use serde_json::{Value, json};
use tempfile::TempDir;

/// A Streamable HTTP MCP server offering the tools `ping` and `pong`, counting `initialize` requests.
struct StubMcp {
    port: u16,
    initializes: Arc<(Mutex<usize>, Condvar)>,
}

impl StubMcp {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let initializes = Arc::new((Mutex::new(0), Condvar::new()));
        let counter = Arc::clone(&initializes);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || answer(stream, &counter));
            }
        });
        Self { port, initializes }
    }

    /// Its URL at `path`: omp skips a server whose endpoint another one already has, so each gets its own path.
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}/{path}", self.port)
    }

    fn initializes(&self) -> usize {
        *self.initializes.0.lock().unwrap()
    }

    /// Waits until at least `count` sessions were initialized.
    fn wait_for(&self, count: usize) {
        let (count_now, changed) = &*self.initializes;
        let deadline = Instant::now() + TIMEOUT;
        let mut guard = count_now.lock().unwrap();
        while *guard < count {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "{} initializes, waited for {count}",
                *guard
            );
            guard = changed.wait_timeout(guard, left).unwrap().0;
        }
    }
}

/// One HTTP request per connection: JSON-RPC requests get JSON answers, notifications `202`, anything but `POST`
/// (the SSE listener) `405`.
fn answer(mut stream: TcpStream, initializes: &(Mutex<usize>, Condvar)) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let empty = |status: &str| {
        format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    };
    if !request_line.starts_with("POST ") {
        let _ = stream.write_all(empty("405 Method Not Allowed").as_bytes());
        return;
    }
    let message: Value = serde_json::from_slice(&body).unwrap_or_default();
    let Some(id) = message.get("id") else {
        let _ = stream.write_all(empty("202 Accepted").as_bytes());
        return;
    };
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let (count, changed) = initializes;
            *count.lock().unwrap() += 1;
            changed.notify_all();
            json!({
                "protocolVersion": message["params"]["protocolVersion"],
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "stub", "version": "1.0.0" },
            })
        }
        "tools/list" => json!({ "tools": [
            { "name": "ping", "description": "Ping", "inputSchema": { "type": "object" } },
            { "name": "pong", "description": "Pong", "inputSchema": { "type": "object" } },
        ] }),
        _ => json!({}),
    };
    let reply = json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
        reply.len()
    );
}

/// The real daemon on the harness's isolated omp.
async fn daemon(harness: &Harness) -> (DaemonHandle, TempDir) {
    let data = tempfile::tempdir().unwrap();
    let omp = omp_bin();
    let workspaces = data.path().join("workspaces");
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        data.path().to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        "--omp-bin",
        &omp,
        "--no-title-generation",
    ])
    .unwrap();
    let config = Config::from_cli(cli, None, None).unwrap();
    let mut ports = Ports::resolve(&config).unwrap();
    ports.omp_env = harness.env.prepare_omfp_profile(data.path()).unwrap();
    (start_with(config, ports).await.unwrap(), data)
}

fn server<'a>(listing: &'a Value, name: &str) -> &'a Value {
    listing["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|server| server["name"] == name)
        .unwrap_or_else(|| panic!("no server {name} in {listing}"))
}

// spec: PRT-111, PRT-112, DMN-191, DMN-192
#[tokio::test(flavor = "multi_thread")]
async fn omp_checks_the_thread_servers_and_a_reconnect_connects_them_again() {
    let harness = Harness::new(Vec::new()).await;
    let stub = StubMcp::start();
    let (daemon, _data) = daemon(&harness).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "mcp" }),
        )
        .await;
    let id = thread["id"].as_str().unwrap().to_owned();
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    let folder =
        std::path::PathBuf::from(index["state"]["workspaces"][0]["path"].as_str().unwrap());
    std::fs::create_dir_all(folder.join(".omp")).unwrap();
    std::fs::write(
        folder.join(".omp/mcp.json"),
        json!({ "mcpServers": {
            "stub": { "type": "http", "url": stub.url("stub") },
            "broken": { "command": "/nonexistent/mcp-server" },
        } })
        .to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(folder.join(".cursor")).unwrap();
    std::fs::write(
        folder.join(".cursor/mcp.json"),
        json!({ "mcpServers": { "imported": { "type": "http", "url": stub.url("imported") } } })
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        harness.env.agent_dir().join("mcp.json"),
        json!({ "mcpServers": { "mine": { "type": "http", "url": stub.url("mine") } } })
            .to_string(),
    )
    .unwrap();

    client.ok("thread.state", json!({ "threadId": id })).await;
    let checked = client
        .ok("thread.mcp_servers", json!({ "threadId": id }))
        .await;

    assert_eq!(checked["running"], true, "{checked}");
    let stub_server = server(&checked, "stub");
    assert_eq!(
        (&stub_server["state"], &stub_server["tools"]),
        (&json!("connected"), &json!(2)),
        "{checked}"
    );
    let broken = server(&checked, "broken");
    assert_eq!(broken["state"], "failed", "{checked}");
    assert!(
        broken["error"]
            .as_str()
            .unwrap()
            .contains("/nonexistent/mcp-server"),
        "{checked}"
    );
    let mine = server(&checked, "mine");
    assert_eq!(
        (&mine["state"], &mine["source"]["scope"]),
        (&json!("connected"), &json!("user")),
        "{checked}"
    );
    assert_eq!(server(&checked, "imported")["state"], "configured");

    // omp connected the stub at startup (`stub`, `mine`, `imported`) and twice more for the checks.
    stub.wait_for(5);
    let before = stub.initializes();
    let reconnected = client
        .ok(
            "thread.mcp_reconnect",
            json!({ "threadId": id, "name": "stub" }),
        )
        .await;

    assert_eq!(server(&reconnected, "stub")["state"], "connected");
    assert_eq!(server(&reconnected, "broken")["state"], "configured");
    // The restarted omp connects its servers again (3) and checks `stub` (1).
    stub.wait_for(before + 4);
    harness.assert_no_stray_network();
}

// spec: DMN-201, DMN-203
#[tokio::test(flavor = "multi_thread")]
async fn a_server_added_to_the_config_reaches_the_running_thread_without_a_manual_restart() {
    let harness = Harness::new(Vec::new()).await;
    let stub = StubMcp::start();
    let (daemon, _data) = daemon(&harness).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "mcp" }),
        )
        .await;
    let id = thread["id"].as_str().unwrap().to_owned();
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    let folder =
        std::path::PathBuf::from(index["state"]["workspaces"][0]["path"].as_str().unwrap());
    client.ok("thread.state", json!({ "threadId": id })).await;
    let listing = client
        .ok("thread.mcp_servers", json!({ "threadId": id }))
        .await;
    assert_eq!(
        (&listing["running"], &listing["servers"]),
        (&json!(true), &json!([]))
    );
    assert_eq!(stub.initializes(), 0);

    std::fs::create_dir_all(folder.join(".omp")).unwrap();
    std::fs::write(
        folder.join(".omp/mcp.json"),
        json!({ "mcpServers": { "late": { "type": "http", "url": stub.url("late") } } })
            .to_string(),
    )
    .unwrap();

    // The workspace poll sees the change and restarts the idle thread, whose new omp connects the server.
    stub.wait_for(1);
    harness.assert_no_stray_network();
}
