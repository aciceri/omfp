//! The daemon's log and its restart in place on the real binary: `omfp daemon` appends its log to
//! `<data-dir>/daemon.log` whoever started it, each line once (DMN-390), and `omfp daemon restart` makes the running
//! daemon the restart command's executable: the same process on the same address, a new start (DMN-391).

use std::io::Read as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use omfp_daemon::DaemonInfo;
use omfp_daemon::paths::{create_private_dir, open_private_log};
use rustix::process::{Pid, Signal, kill_process};

const DEADLINE: Duration = Duration::from_secs(60);

/// `omfp <args>` with the temporary directory as `HOME` and none of the variables that pick a data directory.
fn omfp(home: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_omfp"));
    command
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("OMFP_DATA_DIR")
        .stdin(Stdio::null());
    command
}

/// `omfp daemon` on a random port with its data directory `<home>/data`.
fn daemon(home: &Path) -> (Command, PathBuf) {
    let data = home.join("data");
    let workspaces = home.join("workspaces");
    let command = omfp(
        home,
        &[
            "daemon",
            "--bind",
            "127.0.0.1:0",
            "--no-title-generation",
            "--data-dir",
            data.to_str().unwrap(),
            "--workspaces-root",
            workspaces.to_str().unwrap(),
        ],
    );
    (command, data)
}

/// Waits until `check` holds, polling.
fn until<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn running(data: &Path) -> DaemonInfo {
    until("no daemon started", || omfp_daemon::discover(data).unwrap())
}

fn log(data: &Path) -> String {
    std::fs::read_to_string(data.join("daemon.log")).unwrap_or_default()
}

/// The line the daemon's subscriber logs once it listens, as opposed to what `omfp daemon` prints on stdout.
const LISTENING: &str = "omfp_daemon::daemon: daemon listening";

struct Stop(rustix::process::Pid);

impl Drop for Stop {
    fn drop(&mut self) {
        let _ = kill_process(self.0, Signal::KILL);
    }
}

// spec: DMN-391, DMN-390
#[test]
fn the_daemon_restarts_in_place_and_logs_each_line_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut command, data) = daemon(dir.path());
    create_private_dir(&data).unwrap();
    let output = open_private_log(&data.join("daemon.log")).unwrap();
    // Started as `omfp desktop` starts it (ARC-003): its own process group, stdout and stderr appended to daemon.log.
    let mut child = command
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .process_group(0)
        .spawn()
        .unwrap();
    let _stop = Stop(Pid::from_child(&child));
    let first = running(&data);
    assert_eq!(first.pid, child.id());

    let restart = omfp(
        dir.path(),
        &["daemon", "restart", "--data-dir", data.to_str().unwrap()],
    )
    .output()
    .unwrap();
    let stdout = String::from_utf8_lossy(&restart.stdout);
    assert!(
        restart.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&restart.stderr)
    );
    let second = omfp_daemon::discover(&data)
        .unwrap()
        .expect("the restarted daemon");
    // The same process, on the same port although the first one was random, from a new start.
    assert_eq!(second.pid, first.pid);
    assert_eq!(second.addr, first.addr);
    assert_ne!(second.started, first.started);
    assert!(
        stdout.contains(&format!(
            "pid {}, listening on http://{}/",
            first.pid, first.addr
        )),
        "{stdout}"
    );

    // What `omfp daemon` prints once it listens follows its log line: once both starts printed it, every line is in.
    let text = until("the restarted daemon never listened", || {
        let text = log(&data);
        (text.matches("omfp daemon listening on").count() == 2).then_some(text)
    });
    assert_eq!(text.matches(LISTENING).count(), 2, "{text}");
    assert_eq!(text.matches("restart requested").count(), 1, "{text}");
    assert!(!text.contains('\u{1b}'), "no colours in daemon.log: {text}");
    assert_eq!(child.try_wait().unwrap(), None);
}

// spec: DMN-390
#[test]
fn a_daemon_started_anywhere_appends_its_log_to_daemon_log() {
    let dir = tempfile::tempdir().unwrap();
    let (mut command, data) = daemon(dir.path());
    // Started from a terminal, by an autostart or a service manager: stderr is not daemon.log.
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stop = Stop(Pid::from_child(&child));
    let mut stderr = child.stderr.take().unwrap();
    running(&data);
    until("daemon.log has no line", || {
        log(&data).contains(LISTENING).then_some(())
    });
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(data.join("daemon.log"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    kill_process(stop.0, Signal::TERM).unwrap();
    let mut written = String::new();
    stderr.read_to_string(&mut written).unwrap();
    assert!(child.wait().unwrap().success());
    // Both got every line.
    let text = log(&data);
    for line in [LISTENING, "daemon stopped"] {
        assert_eq!(text.matches(line).count(), 1, "{text}");
        assert_eq!(written.matches(line).count(), 1, "{written}");
    }
}
