//! Integration tests (L2) of `omfp theme set`: the real binary against an in-process daemon on port 0 with a temporary
//! data directory, the fake launcher and fake git.

use std::path::Path;
use std::process::Output;
use std::sync::Arc;

use clap::Parser as _;
use omfp::remote::{Connection, Target};
use omfp_daemon::agent::FakeLauncher;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omfp_protocol::method::ThemeGet;
use omfp_protocol::{Empty, Theme};
use tempfile::TempDir;

const CSS: &str = ":root { --omfp-primary: #6750a4; }\n";

struct TestDaemon {
    handle: DaemonHandle,
    dir: TempDir,
}

async fn daemon() -> TestDaemon {
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
    let handle = start_with(Config::from_cli(cli, None, None).unwrap(), ports)
        .await
        .unwrap();
    TestDaemon { handle, dir }
}

impl TestDaemon {
    fn target(&self) -> Target {
        Target {
            url: format!("ws://{}/api/ws", self.handle.addr()),
            token: self.handle.token().expose().to_owned(),
        }
    }

    async fn theme(&self) -> Theme {
        let mut connection = Connection::open(&self.target()).await.unwrap();
        connection.call::<ThemeGet>(Empty {}).await.unwrap()
    }
}

fn theme_file(dir: &Path) -> std::path::PathBuf {
    let file = dir.join("theme.css");
    std::fs::write(&file, CSS).unwrap();
    file
}

async fn omfp(args: &[&str]) -> Output {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_omfp"))
        .args(args)
        .env_remove("XDG_DATA_HOME")
        .output()
        .await
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// spec: UI-091, DMN-007
#[tokio::test(flavor = "multi_thread")]
async fn theme_set_finds_the_local_daemon_of_the_data_dir() {
    let daemon = daemon().await;
    let scratch = tempfile::tempdir().unwrap();
    let file = theme_file(scratch.path());

    let output = omfp(&[
        "theme",
        "set",
        file.to_str().unwrap(),
        "--data-dir",
        daemon.dir.path().to_str().unwrap(),
    ])
    .await;

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        daemon.theme().await,
        Theme {
            css: Some(CSS.into())
        }
    );
}

// spec: UI-091
#[tokio::test(flavor = "multi_thread")]
async fn theme_set_talks_to_the_daemon_given_by_url_and_token() {
    let daemon = daemon().await;
    let scratch = tempfile::tempdir().unwrap();
    let file = theme_file(scratch.path());
    let url = format!("http://{}", daemon.handle.addr());

    let output = omfp(&[
        "theme",
        "set",
        file.to_str().unwrap(),
        "--daemon",
        &url,
        "--token",
        daemon.handle.token().expose(),
        // No daemon and no token here: --daemon and --token are all it needs.
        "--data-dir",
        scratch.path().to_str().unwrap(),
    ])
    .await;

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        daemon.theme().await,
        Theme {
            css: Some(CSS.into())
        }
    );
}

// spec: UI-091, PRT-002
#[tokio::test(flavor = "multi_thread")]
async fn theme_set_reports_a_wrong_token_and_changes_nothing() {
    let daemon = daemon().await;
    let scratch = tempfile::tempdir().unwrap();
    let file = theme_file(scratch.path());
    let url = format!("ws://{}/api/ws", daemon.handle.addr());

    let output = omfp(&[
        "theme",
        "set",
        file.to_str().unwrap(),
        "--daemon",
        &url,
        "--token",
        "not-the-token",
    ])
    .await;

    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.starts_with("omfp theme: system.hello failed:")
            && message.contains("(unauthorized)"),
        "{message}"
    );
    assert_eq!(daemon.theme().await, Theme { css: None });
}

// spec: UI-091, DMN-007
#[tokio::test(flavor = "multi_thread")]
async fn theme_set_without_a_running_daemon_says_so() {
    let scratch = tempfile::tempdir().unwrap();
    let file = theme_file(scratch.path());

    let output = omfp(&[
        "theme",
        "set",
        file.to_str().unwrap(),
        "--data-dir",
        scratch.path().to_str().unwrap(),
    ])
    .await;

    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.contains("no daemon is running for") && message.contains("omfp daemon"),
        "{message}"
    );
}

// spec: UI-091
#[tokio::test(flavor = "multi_thread")]
async fn theme_set_reports_an_unreadable_file() {
    let daemon = daemon().await;
    let missing = daemon.dir.path().join("missing.css");

    let output = omfp(&[
        "theme",
        "set",
        missing.to_str().unwrap(),
        "--data-dir",
        daemon.dir.path().to_str().unwrap(),
    ])
    .await;

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot read"),
        "{}",
        stderr(&output)
    );
    assert_eq!(daemon.theme().await, Theme { css: None });
}
