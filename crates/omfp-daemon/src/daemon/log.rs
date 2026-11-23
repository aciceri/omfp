//! The log of `omfp daemon` (DMN-390): on stderr, and appended to `<data-dir>/daemon.log` whoever started the daemon
//! (a terminal, `omfp desktop`, a session's autostart, a service manager), each line once.

use std::fs::{File, Metadata};
use std::io::IsTerminal as _;
use std::os::fd::AsFd as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::sync::Arc;

use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, fmt};

use crate::paths::open_private_log;

/// Installs the log subscriber of the process: every line goes to stderr (coloured only on a terminal) and to `log`,
/// without colours, unless stderr already is that file (`omfp desktop` starts the daemon with its output there,
/// ARC-003).
pub(super) fn init(log: &Path) -> std::io::Result<()> {
    let stderr = std::io::stderr();
    let stderr_file = stderr
        .as_fd()
        .try_clone_to_owned()
        .map(File::from)
        .and_then(|file| file.metadata())
        .ok();
    let file = open_unless_stderr(log, stderr_file.as_ref())?;
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(stderr.is_terminal())
                // A started-from-a-terminal daemon outlives its terminal (DMN-355): a line that cannot be written is
                // dropped. Left on, tracing-subscriber reports the failed write with `eprintln!`, which panics the
                // task that logged.
                .log_internal_errors(false),
        )
        .with(file.map(|file| {
            fmt::layer()
                .with_writer(Arc::new(file))
                .with_ansi(false)
                .log_internal_errors(false)
        }))
        .init();
    Ok(())
}

/// `path` opened for appending (`0600`, DMN-180), or `None` when `stderr` (its metadata) is that file already.
fn open_unless_stderr(path: &Path, stderr: Option<&Metadata>) -> std::io::Result<Option<File>> {
    let file = open_private_log(path)?;
    let opened = file.metadata()?;
    let same =
        stderr.is_some_and(|stderr| stderr.dev() == opened.dev() && stderr.ino() == opened.ino());
    Ok((!same).then_some(file))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    // spec: DMN-390, DMN-180
    #[test]
    fn the_log_file_is_appended_privately_unless_stderr_is_it() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("daemon.log");
        let terminal = File::create(dir.path().join("elsewhere"))
            .unwrap()
            .metadata()
            .unwrap();

        let mut first = open_unless_stderr(&log, Some(&terminal))
            .unwrap()
            .expect("stderr is elsewhere: the log gets the lines too");
        first.write_all(b"first\n").unwrap();
        assert_eq!(
            std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mut second = open_unless_stderr(&log, None)
            .unwrap()
            .expect("no stderr: the log gets the lines");
        second.write_all(b"second\n").unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "first\nsecond\n");

        // `omfp desktop` started the daemon with stderr appended to daemon.log: no second copy of each line.
        let redirected = std::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .unwrap()
            .metadata()
            .unwrap();
        assert!(
            open_unless_stderr(&log, Some(&redirected))
                .unwrap()
                .is_none()
        );
    }
}
