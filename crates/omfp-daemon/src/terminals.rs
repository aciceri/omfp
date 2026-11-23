//! Interactive PTY sessions for widgets and agent-created UI (issue #80).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::Engine;
use omfp_protocol::{
    TerminalCursor, TerminalEvent, TerminalInfo, TerminalScreen, TerminalState, TerminalStatus,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use thiserror::Error;
use tokio::sync::watch;

use crate::event_log::EventLog;
use crate::lock;
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
/// Largest terminal dimensions accepted; bounds the emulator's cell grid per session.
pub const MAX_COLS: u16 = 400;
pub const MAX_ROWS: u16 = 200;
/// Longest one-call wait for a screen revision.
pub const MAX_SCREEN_WAIT_MS: u32 = 30_000;

pub(crate) type TerminalLog = EventLog<TerminalEvent, TerminalState>;

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("terminal command is empty")]
    EmptyCommand,
    #[error("terminal dimensions must be between 1x1 and {MAX_COLS}x{MAX_ROWS}")]
    InvalidSize,
    #[error("terminal screen wait must not exceed {MAX_SCREEN_WAIT_MS} ms")]
    InvalidWait,
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
            | TerminalError::InvalidWait
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

struct Emulator {
    parser: vt100::Parser,
    revision: u64,
    status: TerminalStatus,
}

struct Entry {
    workspace_id: String,
    started_by: Option<String>,
    log: Arc<TerminalLog>,
    control: Arc<Control>,
    emulator: Arc<Mutex<Emulator>>,
    changes: watch::Sender<u64>,
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
        let (parser_rows, parser_cols) = terminal_size(cols, rows)?;
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
        let emulator = Arc::new(Mutex::new(Emulator {
            parser: vt100::Parser::new(parser_rows, parser_cols, 0),
            revision: 0,
            status: TerminalStatus::Running,
        }));
        let (changes, _) = watch::channel(0);
        spawn_reader(&id, reader, &control, &log, &emulator, &changes)?;
        registry.terminals.insert(
            id,
            Entry {
                workspace_id: workspace_id.to_owned(),
                started_by: started_by.map(str::to_owned),
                log,
                control,
                emulator,
                changes,
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

    /// Returns the current plain-text screen, waiting for a revision change when requested.
    pub async fn screen(
        &self,
        id: &str,
        after_revision: Option<u64>,
        wait_ms: Option<u32>,
    ) -> Result<TerminalScreen, TerminalError> {
        let wait_ms = wait_ms.unwrap_or_default();
        if wait_ms > MAX_SCREEN_WAIT_MS {
            return Err(TerminalError::InvalidWait);
        }
        let (emulator, mut changes) = {
            let registry = self.lock();
            let entry = registry
                .terminals
                .get(id)
                .ok_or_else(|| TerminalError::NotFound(id.to_owned()))?;
            (Arc::clone(&entry.emulator), entry.changes.subscribe())
        };
        let (revision, running) = {
            let emulator = lock(&emulator);
            (
                emulator.revision,
                emulator.status == TerminalStatus::Running,
            )
        };
        if wait_ms > 0 && running && after_revision == Some(revision) {
            let changed = async {
                loop {
                    if changes.changed().await.is_err() || lock(&emulator).revision != revision {
                        break;
                    }
                }
            };
            let _ = tokio::time::timeout(Duration::from_millis(u64::from(wait_ms)), changed).await;
        }
        Ok(screen_snapshot(&emulator))
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
        let (parser_rows, parser_cols) = terminal_size(cols, rows)?;
        let (control, emulator, changes) = self
            .lock()
            .terminals
            .get(id)
            .map(|entry| {
                (
                    Arc::clone(&entry.control),
                    Arc::clone(&entry.emulator),
                    entry.changes.clone(),
                )
            })
            .ok_or_else(|| TerminalError::NotFound(id.to_owned()))?;
        lock(&control.master)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| TerminalError::Io(error.to_string()))?;
        let revision = {
            let mut emulator = lock(&emulator);
            emulator.parser.set_size(parser_rows, parser_cols);
            emulator.revision = emulator.revision.saturating_add(1);
            emulator.revision
        };
        changes.send_replace(revision);
        Ok(())
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

fn screen_snapshot(emulator: &Mutex<Emulator>) -> TerminalScreen {
    let emulator = lock(emulator);
    let screen = emulator.parser.screen();
    let (rows, cols) = screen.size();
    let (row, column) = screen.cursor_position();
    TerminalScreen {
        revision: emulator.revision,
        cols: cols.get(),
        rows: rows.get(),
        contents: screen.contents(),
        cursor: TerminalCursor {
            row,
            column,
            visible: !screen.hide_cursor(),
        },
        alternate_screen: screen.alternate_screen(),
        status: emulator.status.clone(),
    }
}

fn terminal_size(cols: u16, rows: u16) -> Result<(NonZeroU16, NonZeroU16), TerminalError> {
    if cols > MAX_COLS || rows > MAX_ROWS {
        return Err(TerminalError::InvalidSize);
    }
    let Some(rows) = NonZeroU16::new(rows) else {
        return Err(TerminalError::InvalidSize);
    };
    let Some(cols) = NonZeroU16::new(cols) else {
        return Err(TerminalError::InvalidSize);
    };
    Ok((rows, cols))
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

fn spawn_reader(
    id: &str,
    reader: Box<dyn Read + Send>,
    control: &Arc<Control>,
    log: &Arc<TerminalLog>,
    emulator: &Arc<Mutex<Emulator>>,
    changes: &watch::Sender<u64>,
) -> Result<(), TerminalError> {
    let run_id = id.to_owned();
    let run_control = Arc::clone(control);
    let run_log = Arc::clone(log);
    let run_emulator = Arc::clone(emulator);
    let run_changes = changes.clone();
    if let Err(error) = thread::Builder::new()
        .name(format!("omfp-terminal-{id}"))
        .spawn(move || {
            run(
                &run_id,
                reader,
                &run_control,
                &run_log,
                &run_emulator,
                &run_changes,
            );
        })
    {
        let _ = lock(&control.child).kill();
        return Err(TerminalError::Io(error.to_string()));
    }
    Ok(())
}

fn run(
    id: &str,
    mut reader: Box<dyn Read + Send>,
    control: &Control,
    log: &TerminalLog,
    emulator: &Mutex<Emulator>,
    changes: &watch::Sender<u64>,
) {
    let mut buffer = vec![0; READ_CHUNK];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                let revision = {
                    let mut emulator = lock(emulator);
                    emulator.parser.process(&buffer[..size]);
                    emulator.revision = emulator.revision.saturating_add(1);
                    emulator.revision
                };
                let data = base64::engine::general_purpose::STANDARD.encode(&buffer[..size]);
                log.push(TerminalEvent::Output { data });
                changes.send_replace(revision);
            }
            Err(error) => {
                tracing::debug!(terminal = %id, %error, "PTY reader closed");
                break;
            }
        }
    }
    let status = lock(&control.child).wait();
    let code = status.ok().map(|status| status.exit_code().cast_signed());
    let ended_at = now_ms();
    let revision = {
        let mut emulator = lock(emulator);
        emulator.status = TerminalStatus::Exited { code, ended_at };
        emulator.revision = emulator.revision.saturating_add(1);
        emulator.revision
    };
    log.push(TerminalEvent::Exited { code, ended_at });
    changes.send_replace(revision);
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

    // spec: TRM-100, TRM-106
    #[test]
    fn bounds_the_emulator_grid() {
        assert!(matches!(
            terminal_size(MAX_COLS + 1, 1),
            Err(TerminalError::InvalidSize)
        ));
        assert!(matches!(
            terminal_size(1, MAX_ROWS + 1),
            Err(TerminalError::InvalidSize)
        ));
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
