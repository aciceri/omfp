//! The port between the agent supervisor and omp (ARC-012): launching a thread's process and forwarding commands.
//! [`OmpLauncher`] is the real implementation over [`crate::omp`]; [`FakeLauncher`] replays recorded fixtures for
//! component tests (TST-014).

mod fake;
mod first_start;

use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, LazyLock, PoisonError, RwLock};
use std::time::Duration;

use omp_rpc::{
    AbortAndRestoreQueueCommand, BashCommand, BtwCommand, BtwResult, CancelSubagentCommand, Client,
    CompactCommand, Event, ExtensionUiResponse, FollowUpCommand, ForkCommand,
    GetAvailableCommandsCommand, GetAvailableCommandsResult, GetAvailableModelsCommand,
    GetAvailableModelsResult, GetAvailableThinkingLevelsCommand, GetAvailableThinkingLevelsResult,
    GetLoginProvidersCommand, GetLoginProvidersResult, GetLogoutAccountsCommand,
    GetLogoutAccountsResult, GetMessagesCommand, GetMessagesResult, GetSessionStatsCommand,
    GetStateCommand, GetSubagentMessagesCommand, GetSubagentsCommand, GetSubagentsResult,
    GetTreeCommand, GoalCommand, HostTool, HostUri, LiveConnectCommand, LiveMuteCommand,
    LiveMuteResult, LiveStartCommand, LiveStartResult, LiveStopCommand, LoginCommand, LoginResult,
    LogoutCommand, NavigateTreeCommand, PredictWordCommand, PredictWordFeedbackCommand,
    PredictWordResult, PromoteQueuedMessageCommand, PromptCommand, RemoveQueuedMessageCommand,
    RpcInbound, SetLabelCommand, SetModelCommand, SetSessionNameCommand, SetThinkingLevelCommand,
    SteerCommand, SteerSubagentCommand,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use fake::{
    FakeCall, FakeLaunch, FakeLauncher, FakeLive, FakeProcess, FakeReply, FakeRun, Gate,
    NO_CREDENTIAL, answer_for, live_end, live_levels, live_phase, live_transcript,
};

use crate::omp::{LaunchSpec, OmpProcess, StartError, Startup};
use crate::titles::{TitleError, TitleRequest, clean_title};
use first_start::{FirstStart, agent_dir};

/// One omp command the daemon forwards (02-daemon, "Commands towards omp"); prompts go through
/// [`AgentProcess::prompt`] because their id matters (PRT-030).
#[derive(Debug, Clone)]
pub enum OmpCall {
    Steer(SteerCommand),
    FollowUp(FollowUpCommand),
    AbortAndRestoreQueue,
    RemoveQueued(RemoveQueuedMessageCommand),
    PromoteQueued(PromoteQueuedMessageCommand),
    SetModel(SetModelCommand),
    SetThinkingLevel(SetThinkingLevelCommand),
    AvailableModels,
    ThinkingLevels,
    Commands,
    Compact(CompactCommand),
    Tree,
    NavigateTree(NavigateTreeCommand),
    Fork(ForkCommand),
    SetLabel(SetLabelCommand),
    Subagents,
    SubagentMessages(GetSubagentMessagesCommand),
    SteerSubagent(SteerSubagentCommand),
    CancelSubagent(CancelSubagentCommand),
    Goal(GoalCommand),
    Btw(BtwCommand),
    Bash(BashCommand),
    State,
    /// The session's cost, tokens and context use (`get_session_stats`, DMN-214).
    SessionStats,
    /// The transcript of the session's current branch (`get_messages`), reloaded after a fork or a navigation
    /// (DMN-111).
    Messages,
    /// Names the session (omp `set_session_name`, DMN-073).
    SetSessionName(SetSessionNameCommand),
    /// Starts a live voice session on the client's peer (DMN-100).
    LiveConnect(LiveConnectCommand),
    /// Starts a live voice session on the daemon machine's microphone and speakers (DMN-109).
    LiveStart(LiveStartCommand),
    /// Stops the live voice session, if any (DMN-102).
    LiveStop,
    /// Sets the live voice mute (DMN-102).
    LiveMute(LiveMuteCommand),
    /// The OAuth providers and whether omp holds a credential for each (`get_login_providers`, DMN-219).
    LoginProviders,
    /// Runs omp's OAuth login of a provider (`login`, DMN-220); the flow arrives as `open_url`, `notify` and `input`
    /// UI requests.
    Login(LoginCommand),
    /// The stored credentials of a provider (`get_logout_accounts`, DMN-223).
    LogoutAccounts(GetLogoutAccountsCommand),
    /// Removes one stored credential (`logout`, DMN-223).
    Logout(LogoutCommand),
    /// The completion of the word ending at the caret (`predict_word`, DMN-360).
    PredictWord(PredictWordCommand),
    /// A shown word completion accepted or typed past (`predict_word_feedback`, DMN-360).
    PredictWordFeedback(PredictWordFeedbackCommand),
}

impl OmpCall {
    /// omp's command name, for errors and logs.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Steer(_) => "steer",
            Self::FollowUp(_) => "follow_up",
            Self::AbortAndRestoreQueue => "abort_and_restore_queue",
            Self::RemoveQueued(_) => "remove_queued_message",
            Self::PromoteQueued(_) => "promote_queued_message",
            Self::SetModel(_) => "set_model",
            Self::SetThinkingLevel(_) => "set_thinking_level",
            Self::AvailableModels => "get_available_models",
            Self::ThinkingLevels => "get_available_thinking_levels",
            Self::Commands => "get_available_commands",
            Self::Compact(_) => "compact",
            Self::Tree => "get_tree",
            Self::NavigateTree(_) => "navigate_tree",
            Self::Fork(_) => "fork",
            Self::SetLabel(_) => "set_label",
            Self::Subagents => "get_subagents",
            Self::SubagentMessages(_) => "get_subagent_messages",
            Self::SteerSubagent(_) => "steer_subagent",
            Self::CancelSubagent(_) => "cancel_subagent",
            Self::Goal(_) => "goal",
            Self::Btw(_) => "btw",
            Self::Bash(_) => "bash",
            Self::State => "get_state",
            Self::SessionStats => "get_session_stats",
            Self::Messages => "get_messages",
            Self::SetSessionName(_) => "set_session_name",
            Self::LiveConnect(_) => "live_connect",
            Self::LiveStart(_) => "live_start",
            Self::LiveStop => "live_stop",
            Self::LiveMute(_) => "live_mute",
            Self::LoginProviders => "get_login_providers",
            Self::Login(_) => "login",
            Self::PredictWord(_) => "predict_word",
            Self::PredictWordFeedback(_) => "predict_word_feedback",
            Self::LogoutAccounts(_) => "get_logout_accounts",
            Self::Logout(_) => "logout",
        }
    }
}

/// omp admitted a prompt (PRT-030).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAdmission {
    /// The request id; the `prompt_result` ending this prompt carries it.
    pub prompt_id: String,
    /// `false` when omp handled the prompt locally (e.g. `/add-dir`): no `prompt_result` follows.
    pub agent_invoked: bool,
}

/// A running omp process, as seen by the supervisor.
pub trait AgentProcess: Send + Sync {
    /// Sends a prompt and returns once omp admitted it.
    fn prompt(&self, command: PromptCommand) -> Result<PromptAdmission, omp_rpc::Error>;
    /// Sends `call` and returns omp's response data as JSON.
    fn call(&self, call: OmpCall) -> Result<Value, omp_rpc::Error>;
    /// Answers an interactive request (DMN-051).
    fn respond(&self, response: ExtensionUiResponse) -> Result<(), omp_rpc::Error>;
    /// Ends the process now, failing the commands in flight (a running `login`, DMN-222); dropping the last handle
    /// would wait for them.
    fn kill(&self);
}

/// A freshly started process: its handle, setup results and event stream.
pub struct Launched {
    pub process: Arc<dyn AgentProcess>,
    pub startup: Startup,
    pub events: Receiver<Event>,
}

impl std::fmt::Debug for Launched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Launched")
            .field("startup", &self.startup)
            .finish_non_exhaustive()
    }
}

/// Whether another omp process owns a session (DMN-033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lease {
    Free,
    Held,
    /// omp could not probe it.
    Unknown,
}

/// Starts omp processes for threads.
pub trait AgentLauncher: Send + Sync {
    /// Starts omp for `spec` with the daemon's host tools for this thread (DMN-060) and its host URI schemes
    /// (DMN-062).
    fn launch(
        &self,
        spec: &LaunchSpec,
        tools: Vec<HostTool>,
        uris: Vec<HostUri>,
    ) -> Result<Launched, StartError>;
    /// Probes the ownership lease of `session_id` without taking it (`omp session lease`, DMN-033).
    fn session_lease(&self, spec: &LaunchSpec, session_id: &str) -> Result<Lease, StartError>;
    /// Generates a thread title from first-message or rolling context with one omp print-mode run (DMN-073).
    fn generate_title(&self, request: &TitleRequest) -> Result<String, TitleError>;
    /// How many models omp started for `spec` can use (DMN-235): the length of its `get_available_models`, the models
    /// of the providers it holds a credential for and of keyless ones; the omp stops before this returns.
    fn usable_models(&self, spec: &LaunchSpec) -> Result<u32, StartError>;
    /// `omp auth export` as `spec` says (program, cwd, environment; DMN-229): omp's export document, its stdout. A
    /// failure carries omp's stderr, never its stdout, which holds secrets.
    fn export_logins(&self, spec: &LaunchSpec) -> Result<String, StartError>;
    /// `omp auth import -` as `spec` says, `document` on its stdin (DMN-230). A failure carries omp's stderr.
    fn import_logins(&self, spec: &LaunchSpec, document: String) -> Result<(), StartError>;
}

/// The real launcher: `omp --mode rpc-ui` through `omp-rpc`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OmpLauncher;

/// The profiles of this process's omp runs, each started alone until one got past its startup (DMN-913).
static FIRST_START: LazyLock<FirstStart> = LazyLock::new(FirstStart::default);

impl AgentLauncher for OmpLauncher {
    fn launch(
        &self,
        spec: &LaunchSpec,
        tools: Vec<HostTool>,
        uris: Vec<HostUri>,
    ) -> Result<Launched, StartError> {
        let (process, startup) = FIRST_START.run(agent_dir(&spec.env), || {
            OmpProcess::start(spec, tools, uris)
        })?;
        let next_request = process.next_request();
        let (client, events) = process.into_parts();
        let events = events.ok_or(StartError::Connect(omp_rpc::Error::Closed))?;
        Ok(Launched {
            process: Arc::new(OmpAgent {
                client,
                next_request: AtomicU64::new(next_request),
                order: RwLock::new(()),
            }),
            startup,
            events,
        })
    }

    fn session_lease(&self, spec: &LaunchSpec, session_id: &str) -> Result<Lease, StartError> {
        std::fs::create_dir_all(&spec.cwd)?;
        let mut command = Command::new(&spec.program);
        command
            .args(["session", "lease", "--json", "--", session_id])
            .current_dir(&spec.cwd);
        for (key, value) in &spec.env {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        FIRST_START.run(agent_dir(&spec.env), || {
            let output = crate::flatpak::on_host(command).output()?;
            if !output.status.success() {
                return Err(StartError::LeaseCheck(
                    String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                ));
            }
            parse_lease(&output.stdout, session_id)
        })
    }

    fn generate_title(&self, request: &TitleRequest) -> Result<String, TitleError> {
        let output = FIRST_START.run(agent_dir(&request.env), || {
            let output = request.command().output()?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let tail: Vec<&str> = stderr.trim().lines().rev().take(5).collect();
                return Err(TitleError::Failed(format!(
                    "{}: {}",
                    output.status,
                    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
                )));
            }
            Ok(output)
        })?;
        clean_title(&String::from_utf8_lossy(&output.stdout)).ok_or(TitleError::Empty)
    }

    /// Through [`Self::launch`], which starts it alone on a fresh profile (DMN-913).
    fn usable_models(&self, spec: &LaunchSpec) -> Result<u32, StartError> {
        let launched = self.launch(spec, Vec::new(), Vec::new())?;
        let listed = launched
            .process
            .call(OmpCall::AvailableModels)
            .and_then(|value| {
                serde_json::from_value::<GetAvailableModelsResult>(value)
                    .map_err(omp_rpc::Error::Json)
            });
        launched.process.kill();
        let listed = listed.map_err(|error| StartError::Setup {
            step: "get_available_models",
            error,
        })?;
        Ok(u32::try_from(listed.models.len()).unwrap_or(u32::MAX))
    }

    fn export_logins(&self, spec: &LaunchSpec) -> Result<String, StartError> {
        FIRST_START.run(agent_dir(&spec.env), || export_logins(spec, AUTH_DEADLINE))
    }

    fn import_logins(&self, spec: &LaunchSpec, document: String) -> Result<(), StartError> {
        FIRST_START.run(agent_dir(&spec.env), || {
            import_logins(spec, document, AUTH_DEADLINE)
        })
    }
}

/// How long a one-shot `omp auth …` may run before it is killed (DMN-229, DMN-230).
const AUTH_DEADLINE: Duration = Duration::from_mins(1);

/// How many of omp's last stderr lines a failed `omp auth …` reports.
const AUTH_STDERR_LINES: usize = 20;

/// `omp auth export` within `deadline`.
fn export_logins(spec: &LaunchSpec, deadline: Duration) -> Result<String, StartError> {
    let output = run_auth(spec, "auth export", &["auth", "export"], None, deadline)?;
    String::from_utf8(output.stdout).map_err(|_| StartError::Auth {
        command: "auth export",
        stderr: "its output is not UTF-8".to_owned(),
    })
}

/// `omp auth import -` with `document` on its stdin, within `deadline`.
fn import_logins(
    spec: &LaunchSpec,
    document: String,
    deadline: Duration,
) -> Result<(), StartError> {
    run_auth(
        spec,
        "auth import",
        &["auth", "import", "-"],
        Some(document),
        deadline,
    )
    .map(drop)
}

/// Runs `omp <args>` as `spec` says (program, cwd, environment overrides), on the host (DMN-229, DMN-230), as the
/// leader of its own process group, `input` written to its stdin beside the reads of its output so neither blocks.
/// Once omp exits, or at `deadline` when it has not, whatever is left of its group is killed — a descendant holding
/// its pipes would block the reads — and only then is omp reaped and its pipes read to their end; past the deadline
/// the call fails. A failure carries the tail of its stderr, never its stdout (an export's holds secrets); a non-zero
/// exit wins over a failed write (omp may exit without reading its input).
fn run_auth(
    spec: &LaunchSpec,
    command: &'static str,
    args: &[&str],
    input: Option<String>,
    deadline: Duration,
) -> Result<Output, StartError> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;

    use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};

    let mut process = Command::new(&spec.program);
    process.args(args).current_dir(&spec.cwd);
    for (key, value) in &spec.env {
        match value {
            Some(value) => process.env(key, value),
            None => process.env_remove(key),
        };
    }
    let mut process = crate::flatpak::on_host(process);
    process
        .process_group(0)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = process.spawn()?;
    let pid = i32::try_from(child.id())
        .ok()
        .and_then(Pid::from_raw)
        .ok_or_else(|| std::io::Error::other("omp has no process id"))?;
    let writer = child.stdin.take().zip(input).map(|(mut stdin, input)| {
        // Dropping `stdin` at the end ends omp's input.
        std::thread::spawn(move || stdin.write_all(input.as_bytes()))
    });
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                pipe.read_to_end(&mut bytes)?;
            }
            Ok::<_, std::io::Error>(bytes)
        })
    };
    let stdout = drain(child.stdout.take().map(|pipe| Box::new(pipe) as _));
    let stderr = drain(child.stderr.take().map(|pipe| Box::new(pipe) as _));
    // Waits for omp's exit without reaping it: until it is reaped its pid names its group, so the kill below cannot
    // reach a process that reused the number.
    let (exited, exit) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while matches!(
            waitid(
                WaitId::Pid(pid),
                WaitIdOptions::EXITED | WaitIdOptions::NOWAIT
            ),
            Err(rustix::io::Errno::INTR)
        ) {}
        let _ = exited.send(());
    });
    let finished = exit.recv_timeout(deadline).is_ok();
    match kill_process_group(pid, Signal::KILL) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => {}
        Err(error) => tracing::warn!(%error, "cannot stop omp {command}"),
    }
    let status = child.wait();
    let joined = |thread: std::thread::JoinHandle<std::io::Result<Vec<u8>>>| {
        thread
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("its reader panicked")))
    };
    let (stdout, stderr) = (joined(stdout), joined(stderr));
    let written = writer.map(std::thread::JoinHandle::join);
    if !finished {
        return Err(StartError::Auth {
            command,
            stderr: format!(
                "omp {command} did not finish within {} s",
                deadline.as_secs_f64()
            ),
        });
    }
    let output = Output {
        status: status?,
        stdout: stdout?,
        stderr: stderr?,
    };
    auth_outcome(command, &output)?;
    match written {
        None | Some(Ok(Ok(()))) => Ok(output),
        Some(Ok(Err(error))) => Err(error.into()),
        Some(Err(_)) => Err(StartError::Auth {
            command,
            stderr: "writing its input failed".to_owned(),
        }),
    }
}

/// A failed `omp <command>`: the tail of its stderr, never its stdout (an export's holds secrets).
fn auth_outcome(command: &'static str, output: &Output) -> Result<(), StartError> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.trim().lines().collect();
    let tail = lines[lines.len().saturating_sub(AUTH_STDERR_LINES)..].join("\n");
    Err(StartError::Auth {
        command,
        stderr: if tail.is_empty() {
            output.status.to_string()
        } else {
            tail
        },
    })
}

/// `omp session lease --json` output.
#[derive(Debug, Deserialize)]
struct LeaseReport {
    #[serde(default)]
    held: Vec<String>,
    #[serde(default)]
    unknown: Vec<String>,
}

fn parse_lease(stdout: &[u8], session_id: &str) -> Result<Lease, StartError> {
    let report: LeaseReport = serde_json::from_slice(stdout).map_err(|error| {
        StartError::LeaseCheck(format!(
            "unexpected `omp session lease` output ({error}): {}",
            String::from_utf8_lossy(stdout).trim()
        ))
    })?;
    Ok(if report.held.iter().any(|id| id == session_id) {
        Lease::Held
    } else if report.unknown.iter().any(|id| id == session_id) {
        Lease::Unknown
    } else {
        Lease::Free
    })
}

/// omp's `compact` with the deadline omp-rpc 19.2.0 gives `navigate_tree`, whose branch summary is the same kind of
/// model call (DMN-910): the generated `CompactCommand` declares none, so the client's 30 s default would fail a
/// manual compaction's call while omp still summarizes.
#[derive(Serialize)]
#[serde(transparent)]
struct ManualCompact<'a>(&'a CompactCommand);

impl omp_rpc::Command for ManualCompact<'_> {
    const NAME: &'static str = <CompactCommand as omp_rpc::Command>::NAME;
    const TIMEOUT_MS: Option<u64> = <NavigateTreeCommand as omp_rpc::Command>::TIMEOUT_MS;
    type Output = <CompactCommand as omp_rpc::Command>::Output;

    fn decode(data: Option<Value>) -> Result<Self::Output, serde_json::Error> {
        <CompactCommand as omp_rpc::Command>::decode(data)
    }
}

/// [`AgentProcess`] over a connected `omp-rpc` client; dropping the last handle closes omp.
///
/// `omp-rpc` numbers requests itself and does not expose the numbers, yet a prompt's id is the id of its
/// `prompt_result` (PRT-030). Every request goes through this type, which mirrors the client's counter: commands run
/// concurrently (DMN-037) and only bump the mirror, while a prompt waits until no command is in flight, so the
/// mirror equals the client's counter when the prompt takes its number.
struct OmpAgent {
    client: Arc<Client>,
    next_request: AtomicU64,
    order: RwLock<()>,
}

impl OmpAgent {
    /// Runs `command` and returns its output as omp sent it, re-wrapped by `data` where `omp-rpc` unwraps omp's
    /// response (e.g. `{models}` decoded to the bare list): the JSON is the command's wire `*Result`.
    fn run_as<C, D>(
        &self,
        command: &C,
        data: impl FnOnce(C::Output) -> D,
    ) -> Result<Value, omp_rpc::Error>
    where
        C: omp_rpc::Command,
        D: Serialize,
    {
        let _shared = self.order.read().unwrap_or_else(PoisonError::into_inner);
        self.next_request.fetch_add(1, Ordering::SeqCst);
        let output = self.client.call(command)?;
        serde_json::to_value(data(output)).map_err(omp_rpc::Error::Json)
    }

    /// Runs a command whose output is omp's response data unchanged.
    fn run<C>(&self, command: &C) -> Result<Value, omp_rpc::Error>
    where
        C: omp_rpc::Command,
        C::Output: Serialize,
    {
        self.run_as(command, std::convert::identity)
    }
}

impl AgentProcess for OmpAgent {
    fn prompt(&self, command: PromptCommand) -> Result<PromptAdmission, omp_rpc::Error> {
        let _exclusive = self.order.write().unwrap_or_else(PoisonError::into_inner);
        let number = self.next_request.fetch_add(1, Ordering::SeqCst);
        let ack = self.client.call(&command)?;
        Ok(PromptAdmission {
            prompt_id: format!("req_{number}"),
            agent_invoked: ack.agent_invoked.unwrap_or(true),
        })
    }

    fn call(&self, call: OmpCall) -> Result<Value, omp_rpc::Error> {
        match &call {
            OmpCall::Steer(command) => self.run(command),
            OmpCall::FollowUp(command) => self.run(command),
            OmpCall::AbortAndRestoreQueue => self.run(&AbortAndRestoreQueueCommand {}),
            OmpCall::RemoveQueued(command) => self.run(command),
            OmpCall::PromoteQueued(command) => self.run(command),
            OmpCall::SetModel(command) => self.run(command),
            OmpCall::SetThinkingLevel(command) => self.run(command),
            OmpCall::AvailableModels => self.run_as(&GetAvailableModelsCommand {}, |models| {
                GetAvailableModelsResult { models }
            }),
            OmpCall::ThinkingLevels => self
                .run_as(&GetAvailableThinkingLevelsCommand {}, |levels| {
                    GetAvailableThinkingLevelsResult { levels }
                }),
            OmpCall::Commands => self.run_as(&GetAvailableCommandsCommand {}, |commands| {
                GetAvailableCommandsResult { commands }
            }),
            OmpCall::Compact(command) => self.run(&ManualCompact(command)),
            OmpCall::Tree => self.run(&GetTreeCommand {}),
            OmpCall::NavigateTree(command) => self.run(command),
            OmpCall::Fork(command) => self.run(command),
            OmpCall::SetLabel(command) => self.run(command),
            OmpCall::Subagents => self.run_as(&GetSubagentsCommand {}, |subagents| {
                GetSubagentsResult { subagents }
            }),
            OmpCall::SubagentMessages(command) => self.run(command),
            OmpCall::SteerSubagent(command) => self.run(command),
            OmpCall::CancelSubagent(command) => self.run(command),
            OmpCall::Goal(command) => self.run(command),
            OmpCall::Btw(command) => self.run_as(command, |record| BtwResult { record }),
            OmpCall::Bash(command) => self.run(command),
            OmpCall::State => self.run(&GetStateCommand {}),
            OmpCall::SessionStats => self.run(&GetSessionStatsCommand {}),
            OmpCall::Messages => self.run_as(&GetMessagesCommand {}, |messages| {
                GetMessagesResult { messages }
            }),
            OmpCall::SetSessionName(command) => self.run(command),
            OmpCall::LiveConnect(command) => self.run(command),
            OmpCall::LiveStart(command) => self.run_as(command, |voice| LiveStartResult { voice }),
            OmpCall::LiveStop => self.run(&LiveStopCommand {}),
            OmpCall::LiveMute(command) => self.run_as(command, |muted| LiveMuteResult { muted }),
            OmpCall::LoginProviders => self.run_as(&GetLoginProvidersCommand {}, |providers| {
                GetLoginProvidersResult { providers }
            }),
            OmpCall::Login(command) => {
                self.run_as(command, |provider_id| LoginResult { provider_id })
            }
            OmpCall::LogoutAccounts(command) => {
                self.run_as(command, |accounts| GetLogoutAccountsResult { accounts })
            }
            OmpCall::Logout(command) => self.run(command),
            OmpCall::PredictWord(command) => {
                self.run_as(command, |suffix| PredictWordResult { suffix })
            }
            OmpCall::PredictWordFeedback(command) => self.run(command),
        }
    }

    fn respond(&self, response: ExtensionUiResponse) -> Result<(), omp_rpc::Error> {
        self.client.send(&RpcInbound::ExtensionUiResponse(response))
    }

    fn kill(&self) {
        // omp-rpc spawns omp as the leader of its own process group; its pid is the group id. The reader then sees
        // the end of omp's output and fails the pending calls; dropping the client reaps what is left.
        let Some(pid) = self.client.pid().and_then(|pid| i32::try_from(pid).ok()) else {
            return;
        };
        if let Some(group) = rustix::process::Pid::from_raw(pid)
            && let Err(error) =
                rustix::process::kill_process_group(group, rustix::process::Signal::TERM)
        {
            tracing::warn!(pid, %error, "cannot stop omp");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-229, DMN-230
    #[test]
    fn a_failed_omp_auth_reports_its_stderr_tail_never_its_stdout() {
        use std::os::unix::process::ExitStatusExt;
        let stderr = (1..=25)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let output = Output {
            status: std::process::ExitStatus::from_raw(256),
            stdout: br#"{"credentials":[{"key":"sk-secret"}]}"#.to_vec(),
            stderr: stderr.into_bytes(),
        };
        let error = auth_outcome("auth export", &output)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("sk-secret"), "{error}");
        assert!(
            error.starts_with("omp auth export failed: line 6\n"),
            "{error}"
        );
        assert!(error.ends_with("line 25"), "{error}");
        let silent = Output {
            stderr: Vec::new(),
            ..output.clone()
        };
        let error = auth_outcome("auth import", &silent)
            .unwrap_err()
            .to_string();
        assert_eq!(error, "omp auth import failed: exit status: 1");
        let ok = Output {
            status: std::process::ExitStatus::from_raw(0),
            ..output
        };
        assert!(auth_outcome("auth export", &ok).is_ok());
    }

    // spec: DMN-033
    #[test]
    fn lease_report_names_held_unknown_and_free_sessions() {
        let report = br#"{"held":["s1"],"unknown":["s2"]}"#;
        assert_eq!(parse_lease(report, "s1").unwrap(), Lease::Held);
        assert_eq!(parse_lease(report, "s2").unwrap(), Lease::Unknown);
        assert_eq!(parse_lease(report, "s3").unwrap(), Lease::Free);
        assert!(matches!(
            parse_lease(b"not json", "s1"),
            Err(StartError::LeaseCheck(_))
        ));
    }

    // spec: DMN-910
    #[test]
    fn a_manual_compaction_is_omps_compact_with_a_branch_summarys_deadline() {
        use omp_rpc::Command as _;
        let command = CompactCommand {
            custom_instructions: Some("keep the API decisions".into()),
        };
        assert_eq!(
            omp_rpc::encode_command("req_7", &ManualCompact(&command)).unwrap(),
            omp_rpc::encode_command("req_7", &command).unwrap()
        );
        assert_eq!(CompactCommand::TIMEOUT_MS, None);
        assert_eq!(ManualCompact::TIMEOUT_MS, Some(600_000));
    }

    /// A `LaunchSpec` whose omp is `sh` running the script `body`: `sh auth …` runs `<dir>/auth`, which is read, never
    /// executed (no `ETXTBSY` while other tests fork).
    fn script(dir: &std::path::Path, body: &str) -> LaunchSpec {
        std::fs::write(dir.join("auth"), format!("{body}\n")).unwrap();
        LaunchSpec {
            program: "sh".into(),
            cwd: dir.to_owned(),
            add_dirs: Vec::new(),
            session_dir: dir.join("sessions"),
            fork: None,
            model: None,
            approval_mode: None,
            append_system_prompt: None,
            extension: None,
            stderr_log: dir.join("omp.log"),
            env: Vec::new(),
        }
    }

    /// A FIFO in `dir` nobody ever writes: opening it to read blocks for good, without `sleep`.
    fn never(dir: &std::path::Path) -> std::path::PathBuf {
        let fifo = dir.join("never");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        fifo
    }

    /// `call`'s result, failing the test when it has none within 30 s instead of hanging it.
    fn bounded<T: Send + 'static>(call: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || done.send(call()));
        result
            .recv_timeout(Duration::from_secs(30))
            .expect("the call returns")
    }

    /// A document larger than a pipe's buffer.
    fn large_document() -> String {
        format!(r#"{{"version":1,"pad":"{}"}}"#, "x".repeat(256 * 1024))
    }

    // spec: DMN-230
    #[test]
    fn an_import_refused_before_reading_its_input_reports_omps_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let spec = script(dir.path(), "echo refused >&2\nexit 1");
        let error = import_logins(&spec, large_document(), AUTH_DEADLINE).unwrap_err();
        assert_eq!(error.to_string(), "omp auth import failed: refused");
    }

    // spec: DMN-230
    #[test]
    fn an_import_reading_a_large_document_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let spec = script(dir.path(), "cat >/dev/null");
        import_logins(&spec, large_document(), AUTH_DEADLINE).unwrap();
    }

    // spec: DMN-229, DMN-230
    #[test]
    fn an_omp_auth_past_its_deadline_is_killed_with_its_group() {
        let dir = tempfile::tempdir().unwrap();
        let never = never(dir.path());
        // A hung omp that never reads its input, with a descendant holding its output pipes: the call returns only once
        // the whole group is gone, since it reads the pipes to their end.
        let spec = script(
            dir.path(),
            &format!("cat '{0}' &\ncat '{0}'", never.display()),
        );
        let deadline = Duration::from_millis(200);
        let import = spec.clone();
        let error =
            bounded(move || import_logins(&import, large_document(), deadline)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "omp auth import failed: omp auth import did not finish within 0.2 s"
        );
        let error = bounded(move || export_logins(&spec, deadline)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "omp auth export failed: omp auth export did not finish within 0.2 s"
        );
    }

    // spec: DMN-229, DMN-230
    #[test]
    fn a_descendant_holding_the_pipes_of_an_exited_omp_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let never = never(dir.path());
        let spec = script(
            dir.path(),
            &format!("cat '{}' &\nprintf '{{}}'", never.display()),
        );
        let exported = bounded(move || export_logins(&spec, AUTH_DEADLINE)).unwrap();
        assert_eq!(exported, "{}");
    }
}
