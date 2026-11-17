//! The omp process of one thread: launch (DMN-030), session setup (DMN-031) and history load (DMN-032).
//!
//! Everything here is blocking (`omp-rpc` is a blocking client); the agent supervisor calls it from blocking tasks.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use omfp_core::{ProcessState, SessionInfo, ThreadEvent};
use omp_rpc::{
    AgentMessage, Client, ClientOptions, Event, GetMessagesCommand, GetMessagesPageCommand,
    HostTool, HostUri, OpenSessionCommand, OpenSessionResult, ReadyEvent, RpcServerFrame,
    SetAskDialogCommand, SetEventFilterResult, SetSubagentSubscriptionCommand,
    SubagentSubscriptionLevel,
};
use serde::Serialize;
use serde_json::Value;

/// How to start omp for a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    /// omp executable (DMN-006).
    pub program: PathBuf,
    /// Working directory: the thread's workspace folder (DMN-030).
    pub cwd: PathBuf,
    /// The workspace's repositories, passed as `--add-dir` in order (DMN-029).
    pub add_dirs: Vec<PathBuf>,
    /// Directory holding the thread's omp sessions (ARC-020).
    pub session_dir: PathBuf,
    /// A forked thread's first start (DMN-301): the source thread's session file, which omp copies into
    /// `session_dir` as a new session (`--fork`) instead of resuming the directory's newest one (`--continue`).
    pub fork: Option<PathBuf>,
    /// `provider/model` selector.
    pub model: Option<String>,
    /// omp `--approval-mode` value.
    pub approval_mode: Option<String>,
    /// Text passed as `--append-system-prompt`: the daemon's conventions and the user's append prompt (DMN-030).
    pub append_system_prompt: Option<String>,
    /// The daemon's omp extension root, passed as `--extension`: it ships the `widget-builder` agent (DMN-107).
    pub extension: Option<PathBuf>,
    /// File receiving omp's stderr (DMN-036).
    pub stderr_log: PathBuf,
    /// Environment overrides applied last: `Some` sets, `None` removes.
    pub env: Vec<(OsString, Option<OsString>)>,
}

impl LaunchSpec {
    /// Command-line arguments, in order.
    #[must_use]
    pub fn args(&self) -> Vec<OsString> {
        // `--session-dir … --continue` binds the session at launch, so `--add-dir` roots apply to it; a later
        // `open_session` on the same directory is a no-op that reports whether the session was resumed. A fork binds
        // the copy `--fork` writes there, the directory's newest session, the same way (DMN-301).
        let mut args: Vec<OsString> = vec![
            "--mode".into(),
            "rpc-ui".into(),
            "--session-dir".into(),
            self.session_dir.clone().into_os_string(),
        ];
        match &self.fork {
            Some(source) => {
                args.push("--fork".into());
                args.push(source.clone().into_os_string());
            }
            None => args.push("--continue".into()),
        }
        for dir in &self.add_dirs {
            args.push("--add-dir".into());
            args.push(dir.clone().into_os_string());
        }
        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.into());
        }
        if let Some(mode) = &self.approval_mode {
            args.push("--approval-mode".into());
            args.push(mode.into());
        }
        if let Some(text) = &self.append_system_prompt {
            args.push("--append-system-prompt".into());
            args.push(text.into());
        }
        if let Some(dir) = &self.extension {
            args.push("--extension".into());
            args.push(dir.clone().into_os_string());
        }
        args
    }

    /// The process to spawn; stdin/stdout are set by `omp-rpc`.
    pub fn command(&self) -> std::io::Result<Command> {
        if let Some(parent) = self.stderr_log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir_all(&self.cwd)?;
        std::fs::create_dir_all(&self.session_dir)?;
        let stderr = crate::paths::open_private_log(&self.stderr_log)?;
        let mut command = Command::new(&self.program);
        command.args(self.args()).current_dir(&self.cwd);
        for (key, value) in &self.env {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        // On the host when the daemon runs in Flatpak (ENV-064).
        let mut command = crate::flatpak::on_host(command);
        command.stderr(Stdio::from(stderr));
        Ok(command)
    }
}

/// Why omp could not be started or prepared.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("cannot spawn omp: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("omp did not become ready: {0}")]
    Connect(omp_rpc::Error),
    #[error("omp does not support RPC protocol v2 (supported: {0:?})")]
    Protocol(Vec<i64>),
    #[error("omp session setup failed at `{step}`: {error}")]
    Setup {
        step: &'static str,
        error: omp_rpc::Error,
    },
    #[error("omp refused to open the session directory")]
    OpenCancelled,
    #[error("cannot check the omp session lease: {0}")]
    LeaseCheck(String),
}

/// What session setup found.
#[derive(Debug, Clone, PartialEq)]
pub struct Startup {
    pub ready: ReadyEvent,
    pub session: OpenSessionResult,
    /// Transcript of the resumed session (empty for a fresh one).
    pub history: Vec<AgentMessage>,
}

/// `set_event_filter` with delta message updates. The generated `SetEventFilterCommand` of omp-rpc 18.22.0 lacks
/// `messageUpdates`, so the daemon sends its own frame; the response type is the generated one.
#[derive(Debug, Serialize)]
struct SetEventFilterDelta {
    events: Option<Vec<String>>,
    #[serde(rename = "messageUpdates")]
    message_updates: &'static str,
}

impl omp_rpc::Command for SetEventFilterDelta {
    const NAME: &'static str = "set_event_filter";
    const TIMEOUT_MS: Option<u64> = None;
    type Output = SetEventFilterResult;

    fn decode(data: Option<Value>) -> Result<Self::Output, serde_json::Error> {
        serde_json::from_value(data.unwrap_or_else(|| Value::Object(serde_json::Map::new())))
    }
}

/// A running omp process connected over RPC.
pub struct OmpProcess {
    client: Arc<Client>,
    events: Option<Receiver<Event>>,
    next_request: u64,
}

impl std::fmt::Debug for OmpProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OmpProcess")
            .field("pid", &self.client.pid())
            .finish_non_exhaustive()
    }
}

/// Messages per `get_messages_page` call while loading history.
const HISTORY_PAGE: i64 = 200;

impl OmpProcess {
    /// Spawns omp with the host tools (DMN-060) and host URI schemes (DMN-062), waits for `ready`, and runs the
    /// session setup sequence (DMN-031, DMN-032).
    pub fn start(
        spec: &LaunchSpec,
        tools: Vec<HostTool>,
        uris: Vec<HostUri>,
    ) -> Result<(Self, Startup), StartError> {
        // `omp-rpc` numbers requests `req_1`, `req_2`, … and does not expose the numbers; the daemon needs the id of
        // each prompt (the `id` of its `prompt_result`, PRT-030), so it counts every request made on the client:
        // the protocol negotiation, the host tool registration and the host URI registration happen at connect.
        let mut requests: u64 = 1 + u64::from(!tools.is_empty()) + u64::from(!uris.is_empty());
        let options = ClientOptions {
            tools,
            uris,
            ..ClientOptions::default()
        };
        let (client, events) =
            Client::spawn(spec.command()?, options).map_err(StartError::Connect)?;
        let ready = client.ready().clone();
        let supported = ready
            .supported_protocol_versions
            .clone()
            .unwrap_or_default();
        if !supported.contains(&2) || client.protocol_version() != 2 {
            return Err(StartError::Protocol(supported));
        }

        let setup = |step: &'static str| move |error| StartError::Setup { step, error };
        let session = client
            .call(&OpenSessionCommand {
                session_dir: spec.session_dir.to_string_lossy().into_owned(),
                provider: None,
                model_id: None,
            })
            .map_err(setup("open_session"))?;
        if session.cancelled {
            return Err(StartError::OpenCancelled);
        }
        client
            .call(&SetEventFilterDelta {
                events: None,
                message_updates: "delta",
            })
            .map_err(setup("set_event_filter"))?;
        client
            .call(&SetSubagentSubscriptionCommand {
                level: SubagentSubscriptionLevel::Progress,
            })
            .map_err(setup("set_subagent_subscription"))?;
        client
            .call(&SetAskDialogCommand { enabled: true })
            .map_err(setup("set_ask_dialog"))?;
        requests += 4;
        let history = load_history(&client, &mut requests).map_err(setup("get_messages"))?;

        Ok((
            Self {
                client: Arc::new(client),
                events: Some(events),
                next_request: requests + 1,
            },
            Startup {
                ready,
                session,
                history,
            },
        ))
    }

    /// Shared handle for commands; `omp-rpc` allows concurrent calls.
    #[must_use]
    pub fn client(&self) -> &Arc<Client> {
        &self.client
    }

    /// Frames that are not command responses; can be taken once.
    pub fn take_events(&mut self) -> Option<Receiver<Event>> {
        self.events.take()
    }

    /// The number `omp-rpc` gives the next request on this client (`req_<n>`), as long as every request so far went
    /// through [`OmpProcess::start`].
    #[must_use]
    pub fn next_request(&self) -> u64 {
        self.next_request
    }

    /// The shared client and, unless already taken, the event receiver.
    #[must_use]
    pub fn into_parts(self) -> (Arc<Client>, Option<Receiver<Event>>) {
        (self.client, self.events)
    }
}

/// The session transcript, paged like `Client::get_messages` (protocol v2), counting the requests made: a page
/// request that finds the session busy or the cursor stale falls back to one `get_messages`.
fn load_history(client: &Client, requests: &mut u64) -> Result<Vec<AgentMessage>, omp_rpc::Error> {
    let mut messages = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        *requests += 1;
        let page = match client.call(&GetMessagesPageCommand {
            cursor: cursor.clone(),
            limit: Some(HISTORY_PAGE),
        }) {
            Ok(page) => page,
            Err(omp_rpc::Error::Command { code, .. })
                if matches!(code.as_deref(), Some("session_busy" | "stale_cursor")) =>
            {
                *requests += 1;
                return client.call(&GetMessagesCommand {});
            }
            Err(error) => return Err(error),
        };
        messages.extend(page.messages);
        match page.next_cursor {
            Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
            _ => break,
        }
    }
    Ok(messages)
}

/// Fields omp 18.22.0 can send as fractional numbers although `omp-rpc` types them as integers, by frame `type`:
/// `auto_retry_start.delayMs` is a jittered backoff (`turn-recovery.ts`), e.g. `406.547…`.
const FRACTIONAL_INTEGER_FIELDS: &[(&str, &[&str])] = &[("auto_retry_start", &["delayMs"])];

/// The reducer input for a frame from the event receiver (DMN-040, DMN-042).
///
/// A frame `omp-rpc` could not decode only because of a fractional number in one of
/// [`FRACTIONAL_INTEGER_FIELDS`] is decoded again with that number rounded.
#[must_use]
pub fn thread_event(event: Event) -> ThreadEvent {
    match event {
        Event::Notification(frame) => ThreadEvent::Omp { frame },
        Event::UnmatchedResponse(response) => ThreadEvent::Unknown {
            raw: serde_json::to_value(response).unwrap_or(Value::Null),
        },
        Event::Undecodable { raw, .. } => match round_integer_fields(&raw)
            .and_then(|rounded| RpcServerFrame::from_value(rounded).ok())
        {
            Some(RpcServerFrame::RpcNotification(frame)) => ThreadEvent::Omp { frame },
            _ => ThreadEvent::Unknown { raw },
        },
    }
}

/// A copy of `raw` with the fractional numbers of its [`FRACTIONAL_INTEGER_FIELDS`] rounded; `None` when there are none.
fn round_integer_fields(raw: &Value) -> Option<Value> {
    let kind = raw.get("type")?.as_str()?;
    let (_, fields) = FRACTIONAL_INTEGER_FIELDS
        .iter()
        .find(|(name, _)| *name == kind)?;
    let mut rounded = raw.clone();
    let object = rounded.as_object_mut()?;
    let mut changed = false;
    for field in *fields {
        if let Some(value) = object.get_mut(*field)
            && let Some(number) = value.as_f64()
            && value.as_i64().is_none()
            && number.is_finite()
        {
            #[allow(clippy::cast_possible_truncation)]
            let integer = number.round() as i64;
            *value = Value::from(integer);
            changed = true;
        }
    }
    changed.then_some(rounded)
}

/// Reducer inputs announcing a started process: its history (DMN-032), then `Running`.
#[must_use]
pub fn startup_events(startup: &Startup) -> Vec<ThreadEvent> {
    vec![
        ThreadEvent::History {
            session: SessionInfo {
                session_id: startup.session.session_id.clone(),
                session_file: startup.session.session_file.clone(),
                resumed: startup.session.resumed,
            },
            messages: startup.history.clone(),
        },
        ThreadEvent::Process {
            state: ProcessState::Running,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> LaunchSpec {
        LaunchSpec {
            program: "omp".into(),
            cwd: "/ws/checkout".into(),
            add_dirs: vec![
                "/ws/checkout/frontend".into(),
                "/ws/checkout/backend".into(),
            ],
            session_dir: "/data/threads/t1/sessions".into(),
            fork: None,
            model: Some("anthropic/claude".into()),
            approval_mode: Some("always-ask".into()),
            append_system_prompt: Some("line one\nline two".into()),
            extension: Some("/data/omp".into()),
            stderr_log: "/data/threads/t1/omp.log".into(),
            env: vec![],
        }
    }

    // spec: DMN-030, DMN-107
    #[test]
    fn args_run_rpc_ui_with_one_add_dir_per_repository_in_order() {
        let args: Vec<String> = spec()
            .args()
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--mode",
                "rpc-ui",
                "--session-dir",
                "/data/threads/t1/sessions",
                "--continue",
                "--add-dir",
                "/ws/checkout/frontend",
                "--add-dir",
                "/ws/checkout/backend",
                "--model",
                "anthropic/claude",
                "--approval-mode",
                "always-ask",
                "--append-system-prompt",
                "line one\nline two",
                "--extension",
                "/data/omp"
            ]
        );
    }

    // spec: DMN-030
    #[test]
    fn args_omit_unset_model_approval_mode_append_prompt_and_extension() {
        let mut spec = spec();
        spec.add_dirs.clear();
        spec.model = None;
        spec.approval_mode = None;
        spec.append_system_prompt = None;
        spec.extension = None;
        let args: Vec<String> = spec
            .args()
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--mode",
                "rpc-ui",
                "--session-dir",
                "/data/threads/t1/sessions",
                "--continue"
            ]
        );
    }

    // spec: DMN-301
    #[test]
    fn a_forked_thread_starts_with_fork_of_the_source_session_instead_of_continue() {
        let mut spec = spec();
        spec.add_dirs.truncate(1);
        spec.fork = Some("/data/threads/t0/sessions/2026-11-01T10-00-00-000Z_s0.jsonl".into());
        let args: Vec<String> = spec
            .args()
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args[..8],
            [
                "--mode",
                "rpc-ui",
                "--session-dir",
                "/data/threads/t1/sessions",
                "--fork",
                "/data/threads/t0/sessions/2026-11-01T10-00-00-000Z_s0.jsonl",
                "--add-dir",
                "/ws/checkout/frontend",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--continue"), "{args:?}");
    }

    // spec: DMN-030, DMN-036
    #[test]
    fn command_runs_in_the_workspace_folder_and_creates_its_directories() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = spec();
        spec.cwd = root.path().join("workspace");
        spec.session_dir = root.path().join("sessions");
        spec.stderr_log = root.path().join("logs/omp.log");
        spec.env = vec![("KEEP".into(), Some("1".into())), ("DROP".into(), None)];

        let command = spec.command().unwrap();

        assert_eq!(command.get_current_dir(), Some(spec.cwd.as_path()));
        assert!(spec.cwd.is_dir() && spec.session_dir.is_dir() && spec.stderr_log.is_file());
        let envs: Vec<_> = command.get_envs().collect();
        assert!(envs.contains(&("KEEP".as_ref(), Some("1".as_ref()))));
        assert!(envs.contains(&("DROP".as_ref(), None)));
    }

    // spec: DMN-040, DMN-042, UI-043
    #[test]
    fn a_fractional_retry_delay_is_rounded_into_a_typed_frame() {
        // As omp 18.22.0 sends it: `delayMs` is a jittered backoff, typed `i64` by omp-rpc.
        let raw = serde_json::json!({
            "type": "auto_retry_start",
            "attempt": 1,
            "maxAttempts": 3,
            "delayMs": 406.547_312_909_617_4,
            "errorMessage": "500 Internal Server Error",
        });
        assert!(RpcServerFrame::from_value(raw.clone()).is_err());

        let event = thread_event(Event::Undecodable {
            raw: raw.clone(),
            error: "invalid type: floating point".into(),
        });

        let ThreadEvent::Omp { frame } = event else {
            panic!("expected a typed frame, got {event:?}");
        };
        let mut expected = raw;
        expected["delayMs"] = 407.into();
        assert_eq!(serde_json::to_value(frame).unwrap(), expected);
    }

    // spec: DMN-042
    #[test]
    fn frames_still_undecodable_after_rounding_stay_unknown() {
        for raw in [
            serde_json::json!({ "type": "auto_retry_start", "delayMs": 1.5 }),
            serde_json::json!({ "type": "turn_end", "delayMs": 1.5 }),
        ] {
            let event = thread_event(Event::Undecodable {
                raw: raw.clone(),
                error: String::new(),
            });
            assert_eq!(event, ThreadEvent::Unknown { raw });
        }
    }
}
