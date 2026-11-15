//! Integration tests (L2) of agent-readable terminal screens (TRM-106) over a real WebSocket: a daemon on port 0,
//! a real PTY in a temporary workspace, and two native protocol clients.

mod client;

use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use clap::Parser;
use client::Client;
use omfp_daemon::agent::FakeLauncher;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use serde_json::{Value, json};
use tempfile::TempDir;

struct TestDaemon {
    handle: DaemonHandle,
    _dir: TempDir,
}

fn config(dir: &Path) -> Config {
    let workspaces = dir.join("workspaces");
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        dir.to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
    ])
    .unwrap();
    Config::from_cli(cli, None, None).unwrap()
}

async fn daemon() -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let ports = Ports {
        launcher: Arc::new(FakeLauncher::new()),
        git: Arc::new(FakeGit::new()),
        omp_program: "omp".into(),
        omp_version: omfp_core::OMP_VERSION.into(),
        omp_env: Vec::new(),
        warnings: Vec::new(),
    };
    let handle = start_with(config(dir.path()), ports).await.unwrap();
    TestDaemon { handle, _dir: dir }
}

impl TestDaemon {
    async fn client(&self, name: &str) -> Client {
        Client::hello(self.handle.addr(), self.handle.token().expose(), name).await
    }
}

async fn workspace(client: &mut Client) -> String {
    client.ok("workspaces.create", json!({})).await["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn screen(client: &mut Client, terminal_id: &str, after_revision: u64) -> Value {
    client
        .ok(
            "terminals.screen",
            json!({
                "terminalId": terminal_id,
                "afterRevision": after_revision,
                "waitMs": 10_000,
            }),
        )
        .await
}

// spec: TRM-106
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_reads_and_waits_for_a_full_screen_terminal_without_echoed_input() {
    let daemon = daemon().await;
    let mut controller = daemon.client("controller").await;
    let workspace_id = workspace(&mut controller).await;
    let command = concat!(
        "stty -echo; ",
        r#"printf '\033[?1049h\033[2J\033[HMenu\033[2;1H> first\033[3;1Hsecond'; "#,
        "IFS= read -r secret; ",
        r#"printf '\033[2J\033[Haccepted:%s' "${#secret}""#,
    );
    let terminal = controller
        .ok(
            "terminals.start",
            json!({
                "workspaceId": workspace_id,
                "command": command,
                "cols": 40,
                "rows": 8,
            }),
        )
        .await;
    let terminal_id = terminal["id"].as_str().unwrap().to_owned();

    let first = screen(&mut controller, &terminal_id, 0).await;
    assert_eq!(first["contents"], "Menu\n> first\nsecond");
    assert_eq!(
        first["cursor"],
        json!({ "row": 2, "column": 6, "visible": true })
    );
    assert_eq!(first["alternateScreen"], true);
    assert_eq!(first["cols"], 40);
    assert_eq!(first["rows"], 8);
    let first_revision = first["revision"].as_u64().unwrap();

    controller
        .ok(
            "terminals.resize",
            json!({ "terminalId": terminal_id, "cols": 50, "rows": 10 }),
        )
        .await;
    let resized = screen(&mut controller, &terminal_id, first_revision).await;
    assert_eq!(resized["cols"], 50);
    assert_eq!(resized["rows"], 10);
    let resized_revision = resized["revision"].as_u64().unwrap();
    assert!(resized_revision > first_revision);
    let invalid_wait = controller
        .call(
            "terminals.screen",
            json!({
                "terminalId": terminal_id,
                "afterRevision": resized_revision,
                "waitMs": 30_001,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(invalid_wait["code"], "bad_request");

    let mut observer = daemon.client("observer").await;
    let wait_id = terminal_id.clone();
    let waiting =
        tokio::spawn(async move { screen(&mut observer, &wait_id, resized_revision).await });
    let data = base64::engine::general_purpose::STANDARD.encode("secret\r");
    controller
        .ok(
            "terminals.input",
            json!({ "terminalId": terminal_id, "data": data }),
        )
        .await;
    let second = waiting.await.unwrap();

    assert!(second["revision"].as_u64().unwrap() > resized_revision);
    assert_eq!(second["contents"], "accepted:6");
    assert!(!second["contents"].as_str().unwrap().contains("secret"));
    daemon.handle.shutdown().await.unwrap();
}
