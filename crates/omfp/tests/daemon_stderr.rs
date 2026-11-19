//! `omfp daemon` started from a terminal that is closed later (DMN-355): its stdout and stderr then fail every write
//! (`EIO` on a gone terminal, `EPIPE` on a closed pipe, as here). The daemon must keep serving: a log line that cannot
//! be written is dropped, never a panic of the task that logged it.

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use omfp::remote::{Connection, Target};
use omfp_protocol::Empty;
use omfp_protocol::method::ThemeGet;

const DEADLINE: Duration = Duration::from_secs(60);

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// spec: DMN-355
#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_keeps_serving_once_its_terminal_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let theme = dir.path().join("theme.css");
    std::fs::write(&theme, ":root { --omfp-primary: #111111; }\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_omfp"))
        .args([
            "daemon",
            "--bind",
            "127.0.0.1:0",
            "--no-title-generation",
            "--data-dir",
        ])
        .arg(&data)
        .arg("--workspaces-root")
        .arg(dir.path().join("workspaces"))
        .arg("--theme-file")
        .arg(&theme)
        .env("HOME", dir.path())
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let stderr = child.stderr.take().unwrap();
    let daemon = Daemon(child);
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let base = line
        .split_whitespace()
        .find(|word| word.starts_with("http://"))
        .unwrap_or_else(|| panic!("no URL in {line:?}"))
        .trim_end_matches('/')
        .to_owned();
    // The terminal goes away: every later write to stdout or stderr fails.
    drop(stdout);
    drop(stderr);

    // Applying the changed theme file logs a line first (`theme file changed`).
    let changed = ":root { --omfp-primary: #222222; }\n";
    std::fs::write(&theme, changed).unwrap();
    let target = Target {
        url: format!("{}/api/ws", base.replacen("http://", "ws://", 1)),
        token: std::fs::read_to_string(data.join("token"))
            .unwrap()
            .trim()
            .to_owned(),
    };
    let deadline = Instant::now() + DEADLINE;
    loop {
        let mut connection = Connection::open(&target).await.unwrap();
        let theme = connection.call::<ThemeGet>(Empty {}).await.unwrap();
        if theme.css.as_deref() == Some(changed) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the theme file's change never applied: {theme:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    drop(daemon);
}
