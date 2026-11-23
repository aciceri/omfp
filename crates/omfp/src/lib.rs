//! The `omfp` command line (D-15): `omfp daemon` runs the daemon (02-daemon) and `omfp daemon restart` restarts it in
//! place ([`restart`]); the other subcommands talk to a running daemon through the protocol (03-protocol) with
//! [`remote::Connection`]; `omfp desktop` (feature `desktop`, M7) opens a window on a daemon, and `omfp daemon --tray`
//! shows the daemon it starts in the system tray; `omfp mcp config` prints the `mcp.json` entry of a daemon's MCP
//! server. The UI is the TypeScript web app in `ui/` (04-ui.md), served by the daemon.

/// `eprintln!` that never panics: a long-lived process (`omfp daemon --tray`, `omfp desktop`) outlives the terminal it
/// was started from, and then every write to stderr fails (DMN-355); its diagnostics are dropped instead.
#[allow(unused_macros)] // Only the `desktop` feature's modules print.
macro_rules! note {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

#[cfg(feature = "desktop")]
pub mod desktop;
pub mod launch;
pub mod mcp;
pub mod remote;
pub mod restart;
pub mod theme;
#[cfg(feature = "desktop")]
pub mod tray;
