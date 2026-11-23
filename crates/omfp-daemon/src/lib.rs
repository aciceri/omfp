//! The omfp daemon (`omfp daemon`), see docs/specs/02-daemon.md.
//!
//! [`start`] runs a daemon in-process (DMN-004); [`run`] is everything `omfp daemon` does; [`discover`] finds the
//! daemon running for a data directory (DMN-007); [`restart`] restarts it in place (DMN-391).

pub mod agent;
pub mod agent_tools;
pub mod config;
pub mod config_file;
pub mod conventions;
mod daemon;
pub mod event_log;
pub mod flatpak;
pub mod fs_complete;
pub mod git;
pub mod host_tools;
pub mod jobs;
pub mod live_context;
pub mod mcp_client;
pub mod names;
pub mod oauth_relay;
pub mod omp;
pub mod omp_binary;
pub mod omp_mcp;
pub mod omp_profile;
pub mod paths;
pub mod search;
pub mod server;
pub mod session_file;
pub mod store;
pub mod supervisor;
pub mod terminals;
#[cfg(test)]
mod test_support;
pub mod theme;
pub mod threads;
pub mod titles;
pub mod token;
pub mod voice_log;
pub mod watch;
pub mod widgets;
pub mod workspaces;

pub use daemon::{
    DaemonHandle, DaemonInfo, Ports, RESTART_DEADLINE, StopRequest, discover,
    discover_through_restart, lock_held, restart, restarting, run, run_with, start, start_with,
};

/// Locks `mutex`, recovering the guard of a poisoned one.
pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The serde name of a unit variant (`tokyo-night`, `running`, …): the protocol's, the config file's and the store's
/// text of an enum; empty for a value that does not serialize to a string.
pub(crate) fn wire_name(value: impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

/// The unit variant serde names `name`; `None` for an unknown name.
pub(crate) fn from_wire_name<T: serde::de::DeserializeOwned>(name: &str) -> Option<T> {
    T::deserialize(serde::de::value::StrDeserializer::<serde::de::value::Error>::new(name)).ok()
}

/// Version string reported by `omfp --version`: daemon version plus the omp release it speaks to.
#[must_use]
pub fn version_string() -> String {
    format!(
        "{} (omp {}, protocol {})",
        env!("CARGO_PKG_VERSION"),
        omfp_core::OMP_VERSION,
        omfp_protocol::PROTOCOL_VERSION
    )
}
