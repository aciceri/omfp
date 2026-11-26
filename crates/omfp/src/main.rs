//! The `omfp` binary (D-15): `omfp daemon` runs the daemon (02-daemon) and `omfp daemon restart` restarts it in place
//! (DMN-391), `omfp theme set` sets the UI theme (UI-091), `omfp mcp config` prints the MCP client configuration
//! (DMN-159), `omfp desktop` opens a desktop window (feature `desktop`, M7).

use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

/// jemalloc's background threads give the memory the daemon frees back to the system (DMN-366): glibc's malloc keeps
/// it in its arenas, so the daemon would stay at the size of its largest moment (a long conversation read, then
/// forgotten, DMN-365).
#[cfg(target_os = "linux")]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// omfp: many omp agent threads in parallel, from the browser or the desktop.
#[derive(Debug, Parser)]
#[command(name = "omfp")]
struct Omfp {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Runs the daemon: owns threads, workspaces and omp processes, serves the UI; `omfp daemon restart` restarts the
    /// running one in place.
    Daemon(DaemonArgs),
    /// The UI theme of a daemon (UI-091).
    #[command(subcommand)]
    Theme(omfp::theme::Command),
    /// The daemon's MCP server, for agents outside omfp (DMN-155).
    #[command(subcommand)]
    Mcp(omfp::mcp::Command),
    /// Opens a desktop window on the local daemon, starting it in the background if needed (D-15, D-16), or with
    /// `--connect` on another daemon.
    #[cfg(feature = "desktop")]
    Desktop(omfp::desktop::Args),
}

#[derive(Debug, clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
struct DaemonArgs {
    #[command(subcommand)]
    command: Option<DaemonCommand>,
    // Boxed, or every `Command` would be as large as the daemon's options (clippy's `large_enum_variant`).
    #[command(flatten)]
    cli: Box<omfp_daemon::config::Cli>,
    /// Shows the daemon in the system tray (a status notifier item) with its threads, a menu to open windows and to quit
    /// it; `omfp desktop` passes it to the daemon it starts (DMN-128).
    #[cfg(feature = "desktop")]
    #[arg(long)]
    tray: bool,
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    /// Restarts the running daemon of the data directory in place as this omfp executable (DMN-391): it stops as on
    /// SIGTERM, marking the turns it cuts, and becomes this program with its own arguments, environment and address,
    /// so open windows reconnect. Also from the shell of one of its threads.
    Restart(omfp::restart::Args),
}

fn run_daemon(args: DaemonArgs) -> anyhow::Result<()> {
    // Linux: the status notifier item on a GLib thread of the daemon (DMN-128).
    #[cfg(all(feature = "desktop", target_os = "linux"))]
    if args.tray {
        return omfp_daemon::run_with(*args.cli, omfp::tray::Tray::start);
    }
    // macOS: AppKit's status item needs the main thread; the daemon runs on another (DMN-171).
    #[cfg(all(feature = "desktop", target_os = "macos"))]
    if args.tray {
        return omfp::tray::run_daemon(*args.cli);
    }
    omfp_daemon::run(*args.cli)
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    // The release `omfp.app` runs its executable without arguments (ENV-063).
    #[cfg(all(feature = "desktop", target_os = "macos"))]
    let args = omfp::launch::args(args, std::env::current_exe().ok().as_deref());
    let matches = Omfp::command()
        .version(omfp_daemon::version_string())
        .get_matches_from(args);
    let omfp = Omfp::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    match omfp.command {
        Command::Daemon(DaemonArgs {
            command: Some(DaemonCommand::Restart(args)),
            ..
        }) => report("omfp daemon restart", omfp::restart::run(&args).map(Some)),
        Command::Daemon(args) => report("omfp daemon", run_daemon(args).map(|()| None)),
        Command::Theme(command) => {
            report("omfp theme", block_on(omfp::theme::run(command)).map(Some))
        }
        Command::Mcp(command) => report("omfp mcp", omfp::mcp::run(&command).map(Some)),
        #[cfg(feature = "desktop")]
        Command::Desktop(args) => match omfp::desktop::run(&args) {
            Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            Err(error) => report("omfp desktop", Err(error)),
        },
    }
}

/// Runs a client subcommand on a single-threaded runtime.
fn block_on<T>(future: impl Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future)
}

fn report(command: &str, result: anyhow::Result<Option<String>>) -> ExitCode {
    match result {
        Ok(message) => {
            if let Some(message) = message {
                println!("{message}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{command}: {error:#}");
            ExitCode::FAILURE
        }
    }
}
