//! Restarting a daemon in place (DMN-391..DMN-394): `omfp daemon restart` asks the running daemon of a data
//! directory, through a request file and `SIGUSR2`, to stop and replace itself with another `omfp` executable
//! (`exec`): the same process, environment, working directory and arguments, so the restart depends neither on the
//! caller (a shell of one of the daemon's threads dies with the stop) nor on who started the daemon (a terminal,
//! `omfp desktop`, a service manager watching its pid).

use std::ffi::OsString;
use std::fs::Permissions;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use rustix::process::{Signal, kill_process};
use serde::{Deserialize, Serialize};

use super::{DaemonInfo, POLL, discover, process_alive, restarting};
use crate::paths::{DataDir, replace_file};

/// The restart request: which executable the daemon becomes (DMN-391).
const REQUEST_FILE: &str = "daemon.restart";

/// How long `omfp daemon restart` and a client waiting through a restart (DMN-392) wait for the restarted daemon:
/// the stop takes at most 10 s (DMN-005), the start an omp version probe (DMN-001).
pub const RESTART_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    program: PathBuf,
}

/// `omfp daemon restart` (DMN-391): asks the daemon running for `data_dir` to restart in place as `program` and
/// waits, at most `deadline`, until the restarted daemon listens; returns what its `daemon.json` says.
pub fn restart(data_dir: &Path, program: &Path, deadline: Duration) -> anyhow::Result<DaemonInfo> {
    let Some(running) = discover(data_dir)? else {
        if restarting(data_dir)? {
            bail!(
                "the omfp daemon of {} is restarting already",
                data_dir.display()
            );
        }
        bail!("no omfp daemon is running for {}", data_dir.display());
    };
    // Only a daemon that writes `started` catches the signal: to an older one it is fatal (SIGUSR2's default).
    let Some(started) = running.started else {
        bail!(
            "the omfp daemon of {} (pid {}) is older than `omfp daemon restart`: stop it and start it again",
            data_dir.display(),
            running.pid
        );
    };
    // A Flatpak's daemon shares the data directory with the host, but its pid names another process here.
    let pid = running.local_pid().with_context(|| {
        format!(
            "the omfp daemon of {} (pid {}) runs in another PID namespace (a Flatpak sandbox, a container), where \
             this command cannot reach it: stop it and start it again",
            data_dir.display(),
            running.pid
        )
    })?;
    let request = serde_json::to_vec(&Request {
        program: program.to_owned(),
    })?;
    replace_file(
        &data_dir.join(REQUEST_FILE),
        &request,
        Some(Permissions::from_mode(0o600)),
    )
    .context("cannot write the restart request")?;
    kill_process(pid, Signal::USR2)
        .with_context(|| format!("cannot signal the omfp daemon (pid {})", running.pid))?;
    let log = DataDir::new(data_dir).daemon_log();
    let until = Instant::now() + deadline;
    loop {
        if let Some(info) = discover(data_dir)?
            && info.started != Some(started)
        {
            return Ok(info);
        }
        if !process_alive(pid) {
            bail!(
                "the omfp daemon (pid {}) exited while restarting, see {}",
                running.pid,
                log.display()
            );
        }
        if Instant::now() > until {
            bail!(
                "the omfp daemon (pid {}) did not restart in {} s, see {}",
                running.pid,
                deadline.as_secs(),
                log.display()
            );
        }
        std::thread::sleep(POLL);
    }
}

/// The executable the pending restart request names (DMN-394): an absolute path to an executable file. The request
/// is removed, valid or not.
pub(super) fn take_request(data_dir: &Path) -> anyhow::Result<PathBuf> {
    let path = data_dir.join(REQUEST_FILE);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    std::fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
    let Request { program } = serde_json::from_str(&text)
        .with_context(|| format!("{} is no restart request", path.display()))?;
    if !program.is_absolute() {
        bail!("{} is not an absolute path", program.display());
    }
    let metadata =
        std::fs::metadata(&program).with_context(|| format!("cannot run {}", program.display()))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        bail!("{} is not an executable file", program.display());
    }
    Ok(program)
}

/// The arguments of the restarted daemon (DMN-391): this one's (`args`, without the program), with a random port
/// (`configured`'s port 0) pinned to the address it got (`bound`), so the windows open on it reconnect.
pub(super) fn exec_args(
    args: impl IntoIterator<Item = OsString>,
    configured: SocketAddr,
    bound: SocketAddr,
) -> Vec<OsString> {
    let mut args = args.into_iter();
    if configured.port() != 0 {
        return args.collect();
    }
    let mut kept = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--bind" {
            args.next();
        } else if !arg.to_str().is_some_and(|arg| arg.starts_with("--bind=")) {
            kept.push(arg);
        }
    }
    kept.extend(["--bind".into(), bound.to_string().into()]);
    kept
}

/// Replaces this process with `program` (DMN-391); returns only when that fails.
pub(super) fn exec(program: &Path, args: &[OsString]) -> std::io::Error {
    Command::new(program).args(args).exec()
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;

    fn strings(args: &[OsString]) -> Vec<&str> {
        args.iter().map(|arg| arg.to_str().unwrap()).collect()
    }

    // spec: DMN-391
    #[test]
    fn the_restarted_daemon_keeps_the_arguments_and_a_random_port() {
        let args = |line: &[&str]| line.iter().map(OsString::from).collect::<Vec<_>>();
        let fixed: SocketAddr = "127.0.0.1:7420".parse().unwrap();
        let random: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let bound: SocketAddr = "127.0.0.1:41234".parse().unwrap();
        let line = args(&["daemon", "--tray", "--data-dir", "/d"]);
        assert_eq!(
            strings(&exec_args(line.clone(), fixed, fixed)),
            ["daemon", "--tray", "--data-dir", "/d"]
        );
        // A random port, from `--bind` (either spelling) or the config file, becomes the one the daemon got.
        for line in [
            args(&["daemon", "--bind", "127.0.0.1:0", "--tray"]),
            args(&["daemon", "--bind=127.0.0.1:0", "--tray"]),
            args(&["daemon", "--tray"]),
        ] {
            assert_eq!(
                strings(&exec_args(line, random, bound)),
                ["daemon", "--tray", "--bind", "127.0.0.1:41234"]
            );
        }
    }

    // spec: DMN-394
    #[test]
    fn only_a_request_naming_an_executable_restarts_and_it_is_used_once() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let request = |program: &Path| {
            let text = serde_json::to_vec(&Request {
                program: program.to_owned(),
            })
            .unwrap();
            std::fs::write(dir.path().join(REQUEST_FILE), text).unwrap();
        };
        let error = |dir: &Path| take_request(dir).unwrap_err().to_string();

        assert!(error(dir.path()).contains("cannot read"));
        let program = dir.path().join("omfp");
        File::create(&program).unwrap();
        request(&program);
        assert!(error(dir.path()).contains("not an executable file"));
        assert!(!dir.path().join(REQUEST_FILE).exists());
        request(Path::new("omfp"));
        assert!(error(dir.path()).contains("not an absolute path"));
        std::fs::write(dir.path().join(REQUEST_FILE), "{").unwrap();
        assert!(error(dir.path()).contains("no restart request"));

        std::fs::set_permissions(&program, Permissions::from_mode(0o755)).unwrap();
        request(&program);
        assert_eq!(take_request(dir.path()).unwrap(), program);
        assert!(error(dir.path()).contains("cannot read"));
    }

    // spec: DMN-391
    #[test]
    fn a_restart_needs_a_running_daemon_that_restarts_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let program = Path::new("/bin/omfp");
        let error = |dir: &Path| {
            restart(dir, program, Duration::ZERO)
                .unwrap_err()
                .to_string()
        };
        assert!(error(dir.path()).contains("no omfp daemon is running"));

        // A daemon of an older omfp holds the lock: its daemon.json has no `started`.
        let lock = File::create(dir.path().join("daemon.lock")).unwrap();
        lock.lock().unwrap();
        let older = format!(r#"{{"addr":"127.0.0.1:9","pid":{}}}"#, std::process::id());
        std::fs::write(dir.path().join("daemon.json"), older).unwrap();
        assert!(error(dir.path()).contains("older than `omfp daemon restart`"));
        assert!(!dir.path().join(REQUEST_FILE).exists());

        // A daemon in a Flatpak sandbox shares the data directory, but its pid is not one of this namespace's.
        let mut gone = Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        let sandboxed = format!(
            r#"{{"addr":"127.0.0.1:9","pid":{},"started":1,"pid_namespace":"pid:[1]"}}"#,
            gone.id()
        );
        std::fs::write(dir.path().join("daemon.json"), sandboxed).unwrap();
        assert!(error(dir.path()).contains("another PID namespace"));
        assert!(!dir.path().join(REQUEST_FILE).exists());
    }
}
