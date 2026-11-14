//! The port between the agent supervisor and omp (ARC-012): launching a thread's process and forwarding commands.
//! [`OmpLauncher`] is the real implementation over [`crate::omp`]; [`FakeLauncher`] replays recorded fixtures for
//! component tests (TST-014).

mod fake;

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, PoisonError, RwLock};

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
    LogoutCommand, NavigateTreeCommand, PromoteQueuedMessageCommand, PromptCommand,
    RemoveQueuedMessageCommand, RpcInbound, SetLabelCommand, SetModelCommand,
    SetSessionNameCommand, SetThinkingLevelCommand, SteerCommand, SteerSubagentCommand,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use fake::{
    FakeCall, FakeLaunch, FakeLauncher, FakeLive, FakeProcess, FakeReply, FakeRun, Gate,
    NO_CREDENTIAL, answer_for, live_end, live_levels, live_phase, live_transcript,
};

use crate::omp::{LaunchSpec, OmpProcess, StartError, Startup};
use crate::titles::{TitleError, TitleRequest, clean_title};

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
}

/// The real launcher: `omp --mode rpc-ui` through `omp-rpc`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OmpLauncher;

impl AgentLauncher for OmpLauncher {
    fn launch(
        &self,
        spec: &LaunchSpec,
        tools: Vec<HostTool>,
        uris: Vec<HostUri>,
    ) -> Result<Launched, StartError> {
        let (process, startup) = OmpProcess::start(spec, tools, uris)?;
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
        let output = crate::flatpak::on_host(command).output()?;
        if !output.status.success() {
            return Err(StartError::LeaseCheck(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        parse_lease(&output.stdout, session_id)
    }

    fn generate_title(&self, request: &TitleRequest) -> Result<String, TitleError> {
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
        clean_title(&String::from_utf8_lossy(&output.stdout)).ok_or(TitleError::Empty)
    }

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
            OmpCall::Compact(command) => self.run(command),
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
}
