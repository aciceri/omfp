//! Interactive PTY sessions for widgets and agent-created UI (issue #80).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use base64::Engine;
use omfp_protocol::{TerminalEvent, TerminalInfo, TerminalState, TerminalStatus};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use thiserror::Error;

use crate::event_log::EventLog;
use crate::store::{new_id, now_ms};

/// Maximum raw bytes retained in a terminal snapshot.
pub const SCROLLBACK_BYTES: usize = 1024 * 1024;
/// Maximum chunks retained, bounding allocation overhead when a program flushes one byte at a time.
pub const SCROLLBACK_CHUNKS: usize = 4096;
/// Output events retained for replay and in the live buffer. Each is at most [`READ_CHUNK`], so this cannot retain
/// older raw bytes beyond [`SCROLLBACK_BYTES`] outside the snapshot.
pub const REPLAY_EVENTS: usize = SCROLLBACK_BYTES / READ_CHUNK;
/// Maximum PTY read passed to one output event.
pub const READ_CHUNK: usize = 16 * 1024;

pub(crate) type TerminalLog = EventLog<TerminalEvent, TerminalState>;

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("terminal command is empty")]
    EmptyCommand,
    #[error("terminal dimensions must be positive")]
    InvalidSize,
    #[error("terminal cwd must be a directory inside the workspace: {0}")]
    InvalidCwd(String),
    #[error("terminal {0} was not found")]
    NotFound(String),
    #[error("terminal {0} belongs to another workspace")]
    OtherWorkspace(String),
    #[error("terminal sessions are shutting down")]
    ShuttingDown,
    #[error("terminal data is not valid base64: {0}")]
    InvalidData(String),
    #[error("terminal I/O failed: {0}")]
    Io(String),
}

impl From<TerminalError> for omfp_protocol::ErrorBody {
    fn from(error: TerminalError) -> Self {
        use omfp_protocol::ErrorCode;
        let code = match error {
            TerminalError::NotFound(_) => ErrorCode::NotFound,
            TerminalError::OtherWorkspace(_)
            | TerminalError::EmptyCommand
            | TerminalError::InvalidSize
            | TerminalError::InvalidCwd(_)
            | TerminalError::InvalidData(_) => ErrorCode::BadRequest,
            TerminalError::ShuttingDown | TerminalError::Io(_) => ErrorCode::Internal,
        };
        Self::new(code, error.to_string())
    }
}

struct Control {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send>>,
}

struct Entry {
    workspace_id: String,
    started_by: Option<String>,
    log: Arc<TerminalLog>,
    control: Arc<Control>,
}

#[derive(Debug, Default)]
struct Registry {
    terminals: HashMap<String, Entry>,
    closed: bool,
}

/// All live and recently finished PTY sessions. Sessions are memory-only.
#[derive(Debug, Clone, Default)]
pub struct Terminals {
    registry: Arc<Mutex<Registry>>,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Entry")
            .field("workspace_id", &self.workspace_id)
            .field("started_by", &self.started_by)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Control {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Control(..)")
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn apply(state: &mut TerminalState, event: &TerminalEvent) {
    match event {
        TerminalEvent::Output { data } => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_or(0, |bytes| bytes.len());
            state.scrollback.push(data.clone());
            state.retained_bytes = state.retained_bytes.saturating_add(bytes);
            let mut remove = 0;
            while (state.retained_bytes > SCROLLBACK_BYTES
                || state.scrollback.len() - remove > SCROLLBACK_CHUNKS)
                && remove + 1 < state.scrollback.len()
            {
                let old_bytes = base64::engine::general_purpose::STANDARD
                    .decode(&state.scrollback[remove])
                    .map_or(0, |oldest| oldest.len());
                state.retained_bytes = state.retained_bytes.saturating_sub(old_bytes);
                state.dropped = state.dropped.saturating_add(old_bytes as u64);
                remove += 1;
            }
            state.scrollback.drain(..remove);
        }
        TerminalEvent::Exited { code, ended_at } => {
            state.terminal.status = TerminalStatus::Exited {
                code: *code,
                ended_at: *ended_at,
            };
        }
    }
}

/// Inputs needed to start one daemon-owned PTY session.
#[derive(Debug, Clone, Copy)]
pub struct StartTerminal<'a> {
    pub workspace_id: &'a str,
    pub workspace: &'a Path,
    pub command: &'a str,
    pub cwd: Option<&'a str>,
    pub cols: u16,
    pub rows: u16,
    pub started_by: Option<&'a str>,
}

impl Terminals {
    fn lock(&self) -> std::sync::MutexGuard<'_, Registry> {
        lock(&self.registry)
    }

    /// Starts `sh -c <command>` attached to a PTY in the workspace or its relative `cwd`.
    pub fn start(&self, request: StartTerminal<'_>) -> Result<TerminalInfo, TerminalError> {
        let StartTerminal {
            workspace_id,
            workspace,
            command,
            cwd,
            cols,
            rows,
            started_by,
        } = request;
        if command.trim().is_empty() {
            return Err(TerminalError::EmptyCommand);
        }
        if cols == 0 || rows == 0 {
            return Err(TerminalError::InvalidSize);
        }
        let folder = resolve_cwd(workspace, cwd)?;
        let mut registry = self.lock();
        if registry.closed {
            return Err(TerminalError::ShuttingDown);
        }

        let pty = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| TerminalError::Io(error.to_string()))?;
        let mut shell = CommandBuilder::new("sh");
        shell.env("TERM", "xterm-256color");
        shell.env("COLORTERM", "truecolor");
        shell.arg("-c");
        shell.arg(command);
        shell.cwd(&folder);
        let child = pty
            .slave
            .spawn_command(shell)
            .map_err(|error| TerminalError::Io(error.to_string()))?;
        let reader = pty
            .master
            .try_clone_reader()
            .map_err(|error| TerminalError::Io(error.to_string()))?;
        let writer = pty
            .master
            .take_writer()
            .map_err(|error| TerminalError::Io(error.to_string()))?;

        let id = new_id();
        let info = TerminalInfo {
            id: id.clone(),
            workspace_id: workspace_id.to_owned(),
            command: command.to_owned(),
            cwd: folder.to_string_lossy().into_owned(),
            cols,
            rows,
            started_at: now_ms(),
            status: TerminalStatus::Running,
        };
        let log = Arc::new(EventLog::new(
            new_id(),
            TerminalState {
                terminal: info.clone(),
                scrollback: Vec::new(),
                retained_bytes: 0,
                dropped: 0,
            },
            apply,
            REPLAY_EVENTS,
            REPLAY_EVENTS,
        ));
        let control = Arc::new(Control {
            writer: Mutex::new(writer),
            master: Mutex::new(pty.master),
            child: Mutex::new(child),
        });
        let run_id = id.clone();
        let run_control = Arc::clone(&control);
        let run_log = Arc::clone(&log);
        if let Err(error) = thread::Builder::new()
            .name(format!("omfp-terminal-{id}"))
            .spawn(move || run(&run_id, reader, &run_control, &run_log))
        {
            let _ = lock(&control.child).kill();
            return Err(TerminalError::Io(error.to_string()));
        }
        registry.terminals.insert(
            id,
            Entry {
                workspace_id: workspace_id.to_owned(),
                started_by: started_by.map(str::to_owned),
                log,
                control,
            },
        );
        drop(registry);
        Ok(info)
    }

    /// Returns a terminal topic log after checking its workspace.
    pub fn log(
        &self,
        id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Arc<TerminalLog>, TerminalError> {
        let registry = self.lock();
        let entry = registry
            .terminals
            .get(id)
            .ok_or_else(|| TerminalError::NotFound(id.to_owned()))?;
        if let Some(workspace_id) = workspace_id
            && entry.workspace_id != workspace_id
        {
            return Err(TerminalError::OtherWorkspace(id.to_owned()));
        }
        Ok(Arc::clone(&entry.log))
    }

    /// Writes base64-encoded bytes to a terminal.
    pub fn input(&self, id: &str, data: &str) -> Result<(), TerminalError> {
        let control = self
            .lock()
            .terminals
            .get(id)
            .map(|entry| Arc::clone(&entry.control))
            .ok_or_else(|| TerminalError::NotFound(id.to_owned()))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| TerminalError::InvalidData(error.to_string()))?;
        let mut writer = lock(&control.writer);
        writer
            .write_all(&bytes)
            .and_then(|()| writer.flush())
            .map_err(|error| TerminalError::Io(error.to_string()))
    }

    /// Resizes a terminal's PTY.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), TerminalError> {
        if cols == 0 || rows == 0 {
            return Err(TerminalError::InvalidSize);
        }
        let control = self
            .lock()
            .terminals
            .get(id)
            .map(|entry| Arc::clone(&entry.control))
            .ok_or_else(|| TerminalError::NotFound(id.to_owned()))?;
        lock(&control.master)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| TerminalError::Io(error.to_string()))
    }

    /// Sends the PTY child a termination request. A session that already exited is already killed.
    pub fn kill(&self, id: &str) -> Result<(), TerminalError> {
        let (control, log) = self
            .lock()
            .terminals
            .get(id)
            .map(|entry| (Arc::clone(&entry.control), Arc::clone(&entry.log)))
            .ok_or_else(|| TerminalError::NotFound(id.to_owned()))?;
        if log.with_state(|state| state.terminal.status != TerminalStatus::Running) {
            return Ok(());
        }
        match lock(&control.child).kill() {
            Ok(()) => Ok(()),
            Err(_) if log.with_state(|state| state.terminal.status != TerminalStatus::Running) => {
                Ok(())
            }
            Err(error) => Err(TerminalError::Io(error.to_string())),
        }
    }

    /// Kills all sessions in a workspace before its folder is removed.
    pub fn kill_workspace(&self, workspace_id: &str) {
        let controls: Vec<_> = self
            .lock()
            .terminals
            .values()
            .filter(|entry| entry.workspace_id == workspace_id)
            .map(|entry| Arc::clone(&entry.control))
            .collect();
        for control in controls {
            let _ = lock(&control.child).kill();
        }
    }

    /// Forgets all sessions of a removed workspace.
    pub fn forget_workspace(&self, workspace_id: &str) {
        let mut registry = self.lock();
        for (_, entry) in registry
            .terminals
            .extract_if(|_, entry| entry.workspace_id == workspace_id)
        {
            let _ = lock(&entry.control.child).kill();
        }
    }

    /// Kills sessions started by the listed agent threads.
    pub fn kill_started_by(&self, threads: &[String]) {
        let controls: Vec<_> = self
            .lock()
            .terminals
            .values()
            .filter(|entry| {
                entry
                    .started_by
                    .as_ref()
                    .is_some_and(|thread| threads.contains(thread))
            })
            .map(|entry| Arc::clone(&entry.control))
            .collect();
        for control in controls {
            let _ = lock(&control.child).kill();
        }
    }

    /// Kills every session and refuses new starts.
    pub fn shutdown(&self) {
        let controls: Vec<_> = {
            let mut registry = self.lock();
            registry.closed = true;
            registry
                .terminals
                .values()
                .map(|entry| Arc::clone(&entry.control))
                .collect()
        };
        for control in controls {
            let _ = lock(&control.child).kill();
        }
    }
}

fn resolve_cwd(workspace: &Path, cwd: Option<&str>) -> Result<PathBuf, TerminalError> {
    let Some(cwd) = cwd else {
        return Ok(workspace.to_owned());
    };
    let path = Path::new(cwd);
    if path.is_absolute()
        || path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(TerminalError::InvalidCwd(cwd.to_owned()));
    }
    let folder = workspace.join(path);
    if !folder.is_dir() {
        return Err(TerminalError::InvalidCwd(cwd.to_owned()));
    }
    Ok(folder)
}

fn run(id: &str, mut reader: Box<dyn Read + Send>, control: &Control, log: &TerminalLog) {
    let mut buffer = vec![0; READ_CHUNK];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                let data = base64::engine::general_purpose::STANDARD.encode(&buffer[..size]);
                log.push(TerminalEvent::Output { data });
            }
            Err(error) => {
                tracing::debug!(terminal = %id, %error, "PTY reader closed");
                break;
            }
        }
    }
    let status = lock(&control.child).wait();
    let code = status.ok().map(|status| status.exit_code().cast_signed());
    log.push(TerminalEvent::Exited {
        code,
        ended_at: now_ms(),
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    // spec: TRM-100
    #[test]
    fn rejects_parent_directory_cwd() {
        let error = resolve_cwd(Path::new("/tmp"), Some("../etc")).unwrap_err();
        assert!(matches!(error, TerminalError::InvalidCwd(_)));
    }

    // spec: TRM-103
    #[test]
    fn applies_output_and_bounds_scrollback() {
        let info = TerminalInfo {
            id: "t".into(),
            workspace_id: "w".into(),
            command: "cat".into(),
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            started_at: 0,
            status: TerminalStatus::Running,
        };
        let mut state = TerminalState {
            terminal: info,
            scrollback: Vec::new(),
            retained_bytes: 0,
            dropped: 0,
        };
        let first = base64::engine::general_purpose::STANDARD.encode(vec![b'x'; SCROLLBACK_BYTES]);
        let second = base64::engine::general_purpose::STANDARD.encode(b"tail");
        apply(&mut state, &TerminalEvent::Output { data: first });
        apply(
            &mut state,
            &TerminalEvent::Output {
                data: second.clone(),
            },
        );
        assert_eq!(state.scrollback, vec![second]);
        assert_eq!(state.retained_bytes, 4);
        assert_eq!(state.dropped, SCROLLBACK_BYTES as u64);
        let byte = base64::engine::general_purpose::STANDARD.encode(b"x");
        for _ in 0..SCROLLBACK_CHUNKS {
            apply(&mut state, &TerminalEvent::Output { data: byte.clone() });
        }
        assert_eq!(state.scrollback.len(), SCROLLBACK_CHUNKS);
        assert_eq!(state.dropped, SCROLLBACK_BYTES as u64 + 4);
    }

    fn output(log: &TerminalLog) -> String {
        log.with_state(|state| {
            state
                .scrollback
                .iter()
                .filter_map(|chunk| base64::engine::general_purpose::STANDARD.decode(chunk).ok())
                .flat_map(std::iter::IntoIterator::into_iter)
                .map(char::from)
                .collect()
        })
    }

    // spec: TRM-100, TRM-101, TRM-102, TRM-103, TRM-105
    #[tokio::test(flavor = "multi_thread")]
    async fn runs_an_interactive_command_without_recording_input_in_scrollback() {
        let workspace = tempfile::tempdir().unwrap();
        let terminals = Terminals::default();
        let terminal = terminals
            .start(StartTerminal {
                workspace_id: "workspace",
                workspace: workspace.path(),
                command: "printf 'env:%s:%s\\n' \"$TERM\" \"$COLORTERM\"; stty -echo; printf 'ready\\n'; IFS= read -r secret; printf 'accepted:%s\\n' \"${#secret}\"",
                cwd: None,
                cols: 80,
                rows: 24,
                started_by: Some("thread"),
            })
            .unwrap();
        let log = terminals.log(&terminal.id, Some("workspace")).unwrap();
        let mut live = log.subscribe(None).live;
        tokio::time::timeout(Duration::from_secs(10), async {
            while !output(&log).contains("ready") {
                live.recv().await.unwrap();
            }
        })
        .await
        .expect("terminal did not become ready");

        terminals.resize(&terminal.id, 100, 30).unwrap();
        let input = base64::engine::general_purpose::STANDARD.encode("secret\r");
        terminals.input(&terminal.id, &input).unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while log.with_state(|state| state.terminal.status == TerminalStatus::Running) {
                live.recv().await.unwrap();
            }
        })
        .await
        .expect("terminal did not exit");

        let output = output(&log);
        assert!(output.contains("env:xterm-256color:truecolor"));
        assert!(output.contains("accepted:6"));
        assert!(!output.contains("secret"));
    }
}
