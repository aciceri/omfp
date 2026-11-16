//! Integration test (L2) of `omfp mcp config` (DMN-159): the real binary finds the local daemon of a data directory
//! and prints the `mcp.json` entry of its MCP server, the token read from `$OMFP_TOKEN` (AGT-054).

use std::sync::Arc;

use clap::Parser as _;
use omfp_daemon::agent::FakeLauncher;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{Ports, start_with};
use serde_json::{Value, json};

// spec: DMN-159, AGT-054, DMN-007
#[tokio::test(flavor = "multi_thread")]
async fn mcp_config_prints_the_entry_of_the_local_daemon() {
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
        warnings: Vec::new(),
        clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
    };
    let daemon = start_with(Config::from_cli(cli, None, None).unwrap(), ports)
        .await
        .unwrap();

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_omfp"))
        .args(["mcp", "config", "--data-dir", dir.path().to_str().unwrap()])
        .env_remove("XDG_DATA_HOME")
        .output()
        .await
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        printed,
        json!({ "mcpServers": { "omfp": {
            "type": "http",
            "url": format!("http://{}/mcp", daemon.addr()),
            "headers": { "Authorization": "Bearer ${OMFP_TOKEN}" },
            "timeout": 0,
        } } })
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(daemon.token().expose()), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "export OMFP_TOKEN=\"$(cat '{}')\"",
            dir.path().join("token").display()
        )),
        "{stderr}"
    );

    let literal = tokio::process::Command::new(env!("CARGO_BIN_EXE_omfp"))
        .args([
            "mcp",
            "config",
            "--literal-token",
            "--data-dir",
            dir.path().to_str().unwrap(),
        ])
        .env_remove("XDG_DATA_HOME")
        .output()
        .await
        .unwrap();
    let printed: Value = serde_json::from_slice(&literal.stdout).unwrap();
    assert_eq!(
        printed["mcpServers"]["omfp"]["headers"]["Authorization"],
        format!("Bearer {}", daemon.token().expose())
    );
}
