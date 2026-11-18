//! The daemon the TypeScript client's transport suite runs against (`just test-ts`, TST-030): an in-process daemon on
//! `127.0.0.1:0` with the fake omp launcher and fake git (as in the native L2 tests) and the data directory given as
//! the only argument. Live voice sessions send one `live_levels` frame (a `live_start` session's, DMN-110).
//!
//! It prints `<websocket url> <token>` on one line and serves until SIGINT or SIGTERM.

use std::io::Write as _;
use std::sync::Arc;

use clap::Parser;
use omfp_daemon::agent::{FakeLauncher, FakeLive, live_levels};
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{Ports, start_with};
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = std::env::args()
        .nth(1)
        .ok_or("usage: test_daemon <data-dir>")?;
    let workspaces = format!("{data_dir}/workspaces");
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        &data_dir,
        "--workspaces-root",
        &workspaces,
    ])?;
    let launcher = FakeLauncher::new();
    launcher.script_live(FakeLive::connect(vec![live_levels(0.25, 0.5)]));
    let ports = Ports {
        launcher: Arc::new(launcher),
        git: Arc::new(FakeGit::new()),
        omp_program: "omp".into(),
        omp_version: omfp_core::OMP_VERSION.into(),
        omp_env: Vec::new(),
        // A standalone omp with an empty home: nothing reads the developer's own omp (TST-003).
        user_omp_env: omfp_testkit::absent_standalone_omp(&data_dir),
        warnings: Vec::new(),
        clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
    };
    let daemon = start_with(Config::from_cli(cli, None, None)?, ports).await?;
    let mut stdout = std::io::stdout();
    writeln!(
        stdout,
        "ws://{}/api/ws {}",
        daemon.addr(),
        daemon.token().expose()
    )?;
    stdout.flush()?;

    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    daemon.shutdown().await?;
    Ok(())
}
