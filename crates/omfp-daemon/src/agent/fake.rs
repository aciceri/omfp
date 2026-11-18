//! A scripted [`AgentLauncher`] for component tests (L1, L2). Each launch replays a recorded thread fixture
//! (`fixtures/threads/<omp version>/*.jsonl`, the format of TST-012), so the fake only sends frames omp really sent
//! (TST-014): the frames before the first turn at startup, then one segment per prompt and per answered interaction.

mod live;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use omfp_core::ThreadEvent;
use omp_rpc::{
    AgentMessage, CommandOutputEvent, Event, ExtensionUiResponse, HostTool, HostUri,
    OpenSessionResult, PromptCommand, ReadyEvent, RpcAgentEvent, RpcNotification,
};
use serde_json::{Value, json};
use tokio::sync::watch;

pub use live::{
    FakeLive, NO_CREDENTIAL, answer_for, end as live_end, levels as live_levels,
    phase as live_phase, transcript as live_transcript,
};

use super::{AgentLauncher, AgentProcess, Launched, Lease, OmpCall, PromptAdmission};
use crate::omp::{LaunchSpec, StartError, Startup};
use crate::store::new_id;
use crate::titles::{TitleError, TitleRequest};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What one fake process plays: its startup and the frames it sends, in segments.
#[derive(Debug, Clone, PartialEq)]
pub struct FakeRun {
    startup: Startup,
    /// Segment 0 is sent at launch; each prompt or answer sends the next one.
    segments: VecDeque<Vec<RpcNotification>>,
}

impl FakeRun {
    /// A fresh session without history or frames.
    #[must_use]
    pub fn fresh() -> Self {
        Self {
            startup: Startup {
                ready: default_ready(),
                session: OpenSessionResult {
                    cancelled: false,
                    resumed: false,
                    session_id: new_id(),
                    session_file: None,
                },
                history: Vec::new(),
            },
            segments: VecDeque::new(),
        }
    }

    /// The same run resuming a session with `messages` as its history.
    #[must_use]
    pub fn with_history(mut self, messages: Vec<AgentMessage>) -> Self {
        self.startup.session.resumed = true;
        self.startup.history = messages;
        self
    }

    /// Reads a fixture file.
    pub fn from_fixture(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::from_jsonl(&text).map_err(std::io::Error::other)
    }

    /// Parses fixture lines (`omfp_core::ThreadEvent` as JSON): `history` gives the startup, `process`, `live_session`,
    /// `open_file` and `notify` events are the daemon's own and skipped, omp frames are split into segments at each turn
    /// start (`agent_start` after the startup or after `session_settled`) and after each `interaction_resolved`.
    pub fn from_jsonl(text: &str) -> Result<Self, String> {
        let mut run = Self::fresh();
        run.segments.push_back(Vec::new());
        let mut ready = None;
        let mut turn_open = false;
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let event: ThreadEvent = serde_json::from_str(line)
                .map_err(|error| format!("fixture line {}: {error}", number + 1))?;
            match event {
                ThreadEvent::History { session, messages } => {
                    run.startup.session = OpenSessionResult {
                        cancelled: false,
                        resumed: session.resumed,
                        session_id: session.session_id,
                        session_file: session.session_file,
                    };
                    run.startup.history = messages;
                }
                ThreadEvent::Omp { frame } => {
                    match &frame {
                        RpcNotification::Ready(event) if ready.is_none() => {
                            ready = Some(event.clone());
                        }
                        RpcNotification::RpcAgentEvent(RpcAgentEvent::AgentStart(_))
                            if !turn_open =>
                        {
                            turn_open = true;
                            run.segments.push_back(Vec::new());
                        }
                        RpcNotification::SessionSettled(_) => turn_open = false,
                        _ => {}
                    }
                    if let Some(segment) = run.segments.back_mut() {
                        segment.push(frame);
                    }
                }
                ThreadEvent::InteractionResolved { .. } => run.segments.push_back(Vec::new()),
                ThreadEvent::Process { .. }
                | ThreadEvent::LiveSession { .. }
                | ThreadEvent::OpenFile(_)
                | ThreadEvent::Notify(_)
                | ThreadEvent::Unknown { .. } => {}
            }
        }
        if let Some(ready) = ready {
            run.startup.ready = ready;
        }
        Ok(run)
    }

    /// The startup the launch reports.
    #[must_use]
    pub fn startup(&self) -> &Startup {
        &self.startup
    }
}

fn default_ready() -> ReadyEvent {
    ReadyEvent {
        protocol_version: Some(2),
        supported_protocol_versions: Some(vec![1, 2]),
        max_frame_bytes: None,
        max_reassembled_frame_bytes: None,
        supports_bash_exclude_from_context: Some(true),
    }
}

/// A scripted answer to a command.
#[derive(Debug, Clone, PartialEq)]
pub enum FakeReply {
    Ok(Value),
    /// omp's `success: false`, with its `code`.
    Fail {
        message: String,
        code: Option<String>,
    },
}

/// A command a fake process received.
#[derive(Debug, Clone)]
pub enum FakeCall {
    Prompt(PromptCommand),
    Call(OmpCall),
    Respond(ExtensionUiResponse),
}

/// Holds calls of one command until opened.
#[derive(Debug, Clone)]
pub struct Gate {
    arrived: watch::Sender<usize>,
    open: Arc<(Mutex<bool>, Condvar)>,
}

impl Gate {
    fn new() -> Self {
        Self {
            arrived: watch::Sender::new(0),
            open: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    /// Waits until `count` calls are held at the gate.
    pub async fn arrived(&self, count: usize) {
        let mut arrived = self.arrived.subscribe();
        let _ = arrived.wait_for(|arrived| *arrived >= count).await;
    }

    /// Lets every held and later call through.
    pub fn open(&self) {
        let (open, changed) = &*self.open;
        *lock(open) = true;
        changed.notify_all();
    }

    fn pass(&self) {
        self.arrived.send_modify(|arrived| *arrived += 1);
        let (open, changed) = &*self.open;
        let mut guard = lock(open);
        while !*guard {
            guard = changed.wait(guard).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

#[derive(Debug, Default)]
struct Script {
    runs: VecDeque<FakeRun>,
    runs_by_cwd: HashMap<PathBuf, VecDeque<FakeRun>>,
    replies: HashMap<String, FakeReply>,
    gates: HashMap<String, Gate>,
    held: HashSet<String>,
    launches: Vec<FakeLaunch>,
    /// Answers of the next title generations (DMN-073); without one a generation fails.
    titles: VecDeque<Result<String, String>>,
    title_requests: Vec<TitleRequest>,
    /// How every later `live_connect` or `live_start` goes (TST-015, TST-051).
    live: FakeLive,
    /// The `command_output` texts of local commands by message (e.g. `/mcp test fs`), sent before the answer as omp
    /// does (DMN-191).
    outputs: HashMap<String, Vec<String>>,
    /// The stderr of the next launches that fail as omp exiting at startup does (DMN-211).
    failures: VecDeque<String>,
    /// The stderr of every later thread launch without `--model`, which fails as omp does when it cannot restore the
    /// session's saved model (`SessionModelRestoreError`, DMN-337).
    saved_model_failure: Option<String>,
    /// What every later check of the usable models answers (DMN-235); `None`: one model, as a configured omp has.
    models: Option<u32>,
    /// The specs of every check of the usable models so far, oldest first.
    model_checks: Vec<LaunchSpec>,
    /// What every later `omp auth export` answers (DMN-229): its document, or the stderr it fails with; `None`: a
    /// document without credentials.
    export: Option<Result<String, String>>,
    /// The stderr every later `omp auth import` fails with (DMN-230); `None`: it imports.
    import_failure: Option<String>,
    /// The specs of every `omp auth export` so far, oldest first.
    exports: Vec<LaunchSpec>,
    /// The bytes of the store each `omp auth export` so far found in its agent directory (`PI_CODING_AGENT_DIR`'s
    /// `agent.db`), oldest first; `None` without one.
    export_stores: Vec<Option<Vec<u8>>>,
    /// The spec and the document of every `omp auth import` so far, oldest first.
    imports: Vec<(LaunchSpec, String)>,
}

/// The fake launcher. Without queued runs, every launch plays [`FakeRun::fresh`].
#[derive(Debug, Clone, Default)]
pub struct FakeLauncher {
    script: Arc<Mutex<Script>>,
}

/// One launch, as the fake saw it.
#[derive(Debug, Clone)]
pub struct FakeLaunch {
    pub spec: LaunchSpec,
    /// Names of the host tools registered at launch (DMN-060).
    pub tools: Vec<String>,
    /// Schemes of the host URIs registered at launch (DMN-062).
    pub uris: Vec<String>,
    pub process: FakeProcess,
}

impl FakeLauncher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues the run the next launch plays.
    pub fn push_run(&self, run: FakeRun) {
        lock(&self.script).runs.push_back(run);
    }

    /// Queues a run for the next launch in `cwd`, unaffected by transient launches elsewhere.
    pub fn push_run_for(&self, cwd: PathBuf, run: FakeRun) {
        lock(&self.script)
            .runs_by_cwd
            .entry(cwd)
            .or_default()
            .push_back(run);
    }

    /// Answers every later `command` (omp name, e.g. `get_state`) with `reply`; default: the smallest response data
    /// of the command's wire type (see [`default_data`]).
    pub fn reply(&self, command: &str, reply: FakeReply) {
        lock(&self.script).replies.insert(command.to_owned(), reply);
    }

    /// Holds every later `command` until the returned gate opens.
    #[must_use]
    pub fn gate(&self, command: &str) -> Gate {
        lock(&self.script)
            .gates
            .entry(command.to_owned())
            .or_insert_with(Gate::new)
            .clone()
    }

    /// Reports `session_id` as owned by another omp process (DMN-033).
    pub fn hold_lease(&self, session_id: &str) {
        lock(&self.script).held.insert(session_id.to_owned());
    }

    /// Every launch so far, oldest first.
    #[must_use]
    pub fn launches(&self) -> Vec<FakeLaunch> {
        lock(&self.script).launches.clone()
    }

    /// The process of launch `index`.
    #[must_use]
    pub fn process(&self, index: usize) -> FakeProcess {
        lock(&self.script).launches[index].process.clone()
    }

    /// Queues the answer of the next title generation: the title, or the error omp reported.
    pub fn push_title(&self, title: Result<&str, &str>) {
        lock(&self.script)
            .titles
            .push_back(title.map(str::to_owned).map_err(str::to_owned));
    }

    /// Holds every later title generation until the returned gate opens.
    #[must_use]
    pub fn gate_titles(&self) -> Gate {
        self.gate(TITLE_GATE)
    }

    /// Holds every later `omp auth import` (DMN-230), before it stores anything, until the returned gate opens.
    #[must_use]
    pub fn gate_imports(&self) -> Gate {
        self.gate(IMPORT_GATE)
    }

    /// Holds every later launch, before the process exists, until the returned gate opens.
    #[must_use]
    pub fn gate_launches(&self) -> Gate {
        self.gate(LAUNCH_GATE)
    }

    /// Holds every later close of a process by the daemon (dropping it, as `omp-rpc` kills omp) until the returned
    /// gate opens.
    #[must_use]
    pub fn gate_closes(&self) -> Gate {
        self.gate(CLOSE_GATE)
    }

    /// Every title generation so far, oldest first.
    #[must_use]
    pub fn title_requests(&self) -> Vec<TitleRequest> {
        lock(&self.script).title_requests.clone()
    }

    /// Plays `live` for every later `live_connect` or `live_start` without a scripted [`FakeReply`]; by default a
    /// call connects without further frames ([`FakeLive::default`]).
    pub fn script_live(&self, live: FakeLive) {
        lock(&self.script).live = live;
    }

    /// Answers every later local prompt `message` (a slash command, e.g. `/mcp test fs`) with `command_output` frames
    /// of `texts`, sent before the prompt's answer as omp does. Unscripted, `/add-dir` and `/remove-dir` answer with
    /// omp's one line and other local commands with nothing.
    pub fn local_output(&self, message: &str, texts: &[&str]) {
        lock(&self.script).outputs.insert(
            message.to_owned(),
            texts.iter().map(|text| (*text).to_owned()).collect(),
        );
    }

    /// Fails the next launch (or check of the usable models, which starts an omp too) as omp exiting at startup does
    /// (`omp did not become ready: server closed`), after writing `stderr` to its stderr log.
    pub fn fail_next_launch(&self, stderr: &str) {
        lock(&self.script).failures.push_back(stderr.to_owned());
    }

    /// Fails every later launch without an explicit model (`--model`, DMN-030) as omp exiting at startup does, after
    /// writing `stderr` to its stderr log: omp cannot restore the session's saved model, and only an explicit choice
    /// overrides it (DMN-339). Checks of the usable models are no such launch.
    pub fn fail_launches_without_model(&self, stderr: &str) {
        lock(&self.script).saved_model_failure = Some(stderr.to_owned());
    }

    /// Answers every later check of the usable models (DMN-235) with `count`; by default the fake's omp has one model.
    pub fn set_usable_models(&self, count: u32) {
        lock(&self.script).models = Some(count);
    }

    /// The spec of every check of the usable models so far, oldest first; a check is no launch.
    #[must_use]
    pub fn model_checks(&self) -> Vec<LaunchSpec> {
        lock(&self.script).model_checks.clone()
    }

    /// Answers every later `omp auth export` (DMN-229) with `export`: its document on stdout, or the stderr it fails
    /// with; by default a document without credentials.
    pub fn set_export(&self, export: Result<&str, &str>) {
        lock(&self.script).export = Some(export.map(str::to_owned).map_err(str::to_owned));
    }

    /// Fails every later `omp auth import` (DMN-230) with `stderr`.
    pub fn fail_imports(&self, stderr: &str) {
        lock(&self.script).import_failure = Some(stderr.to_owned());
    }

    /// The spec of every `omp auth export` so far, oldest first.
    #[must_use]
    pub fn exports(&self) -> Vec<LaunchSpec> {
        lock(&self.script).exports.clone()
    }

    /// The bytes of the store each `omp auth export` so far found in its agent directory, oldest first (DMN-229).
    #[must_use]
    pub fn export_stores(&self) -> Vec<Option<Vec<u8>>> {
        lock(&self.script).export_stores.clone()
    }

    /// The spec and the stdin document of every `omp auth import` so far, oldest first.
    #[must_use]
    pub fn imports(&self) -> Vec<(LaunchSpec, String)> {
        lock(&self.script).imports.clone()
    }
}

/// Gate key of title generations (not an omp command).
const TITLE_GATE: &str = "<title>";

/// Gate key of launches (not an omp command).
const LAUNCH_GATE: &str = "<launch>";

/// Gate key of process closes (not an omp command).
const CLOSE_GATE: &str = "<close>";

/// Gate key of `omp auth import` runs (not an RPC command).
const IMPORT_GATE: &str = "<auth import>";

impl AgentLauncher for FakeLauncher {
    fn launch(
        &self,
        spec: &LaunchSpec,
        tools: Vec<HostTool>,
        uris: Vec<HostUri>,
    ) -> Result<Launched, StartError> {
        let gate = lock(&self.script).gates.get(LAUNCH_GATE).cloned();
        if let Some(gate) = gate {
            gate.pass();
        }
        let mut script = lock(&self.script);
        if let Some(stderr) = script.failures.pop_front().or_else(|| {
            spec.model
                .is_none()
                .then(|| script.saved_model_failure.clone())
                .flatten()
        }) {
            std::fs::write(&spec.stderr_log, stderr)?;
            return Err(StartError::Connect(omp_rpc::Error::Closed));
        }
        let mut run = script
            .runs_by_cwd
            .get_mut(&spec.cwd)
            .and_then(VecDeque::pop_front)
            .or_else(|| script.runs.pop_front())
            .unwrap_or_else(FakeRun::fresh);
        let (sender, events) = channel();
        let startup_frames = run.segments.pop_front().unwrap_or_default();
        let process = FakeProcess {
            shared: Arc::new(Shared {
                sender: Mutex::new(Some(sender)),
                segments: Mutex::new(run.segments),
                calls: Mutex::new(Vec::new()),
                session_id: run.startup.session.session_id.clone(),
                script: Arc::clone(&self.script),
                live: Mutex::new(None),
            }),
        };
        process.emit(startup_frames);
        script.launches.push(FakeLaunch {
            spec: spec.clone(),
            tools: tools
                .iter()
                .map(|tool| tool.definition().name.clone())
                .collect(),
            uris: uris.iter().map(|uri| uri.scheme().to_owned()).collect(),
            process: process.clone(),
        });
        Ok(Launched {
            process: Arc::new(FakeAgent(process)),
            startup: run.startup,
            events,
        })
    }

    fn session_lease(&self, _spec: &LaunchSpec, session_id: &str) -> Result<Lease, StartError> {
        Ok(if lock(&self.script).held.contains(session_id) {
            Lease::Held
        } else {
            Lease::Free
        })
    }

    fn generate_title(&self, request: &TitleRequest) -> Result<String, TitleError> {
        let gate = {
            let mut script = lock(&self.script);
            script.title_requests.push(request.clone());
            script.gates.get(TITLE_GATE).cloned()
        };
        if let Some(gate) = gate {
            gate.pass();
        }
        match lock(&self.script).titles.pop_front() {
            Some(Ok(title)) => Ok(title),
            Some(Err(message)) => Err(TitleError::Failed(message)),
            None => Err(TitleError::Failed("no title scripted".into())),
        }
    }

    /// Fails like [`AgentLauncher::launch`] when a failure is queued; else the scripted count.
    fn usable_models(&self, spec: &LaunchSpec) -> Result<u32, StartError> {
        let mut script = lock(&self.script);
        if let Some(stderr) = script.failures.pop_front() {
            std::fs::write(&spec.stderr_log, stderr)?;
            return Err(StartError::Connect(omp_rpc::Error::Closed));
        }
        script.model_checks.push(spec.clone());
        Ok(script.models.unwrap_or(1))
    }

    fn export_logins(&self, spec: &LaunchSpec) -> Result<String, StartError> {
        let store = spec
            .env
            .iter()
            .rev()
            .find(|(key, _)| key == "PI_CODING_AGENT_DIR")
            .and_then(|(_, dir)| dir.as_ref())
            .and_then(|dir| std::fs::read(Path::new(dir).join("agent.db")).ok());
        let mut script = lock(&self.script);
        script.exports.push(spec.clone());
        script.export_stores.push(store);
        match script.export.clone() {
            Some(Ok(document)) => Ok(document),
            Some(Err(stderr)) => Err(StartError::Auth {
                command: "auth export",
                stderr,
            }),
            None => Ok(r#"{"version":1,"credentials":[]}"#.to_owned()),
        }
    }

    fn import_logins(&self, spec: &LaunchSpec, document: String) -> Result<(), StartError> {
        let gate = lock(&self.script).gates.get(IMPORT_GATE).cloned();
        if let Some(gate) = gate {
            gate.pass();
        }
        let mut script = lock(&self.script);
        script.imports.push((spec.clone(), document));
        match script.import_failure.clone() {
            Some(stderr) => Err(StartError::Auth {
                command: "auth import",
                stderr,
            }),
            None => Ok(()),
        }
    }
}

#[derive(Debug)]
struct Shared {
    sender: Mutex<Option<Sender<Event>>>,
    segments: Mutex<VecDeque<Vec<RpcNotification>>>,
    calls: Mutex<Vec<FakeCall>>,
    /// The session the process reports in `get_state`.
    session_id: String,
    script: Arc<Mutex<Script>>,
    /// The live voice session (TST-015).
    live: Mutex<live::LiveSlot>,
}

/// Test handle on one fake process.
#[derive(Debug, Clone)]
pub struct FakeProcess {
    shared: Arc<Shared>,
}

impl FakeProcess {
    /// Every command received so far.
    #[must_use]
    pub fn calls(&self) -> Vec<FakeCall> {
        lock(&self.shared.calls).clone()
    }

    /// The process exits on its own (crash): its event stream ends.
    pub fn exit(&self) {
        lock(&self.shared.sender).take();
    }

    /// Whether the event stream ended (exit, or the daemon closed the process).
    #[must_use]
    pub fn is_closed(&self) -> bool {
        lock(&self.shared.sender).is_none()
    }

    /// Sends frames now, as omp would.
    pub fn emit(&self, frames: Vec<RpcNotification>) {
        if let Some(sender) = &*lock(&self.shared.sender) {
            for frame in frames {
                let _ = sender.send(Event::Notification(frame));
            }
        }
    }

    fn play_next(&self) {
        let next = lock(&self.shared.segments).pop_front();
        if let Some(frames) = next {
            self.emit(frames);
        }
    }

    fn next_prompt_id(&self) -> Option<String> {
        lock(&self.shared.segments)
            .iter()
            .flatten()
            .find_map(|frame| match frame {
                RpcNotification::PromptResult(result) => result.id.clone(),
                _ => None,
            })
    }

    fn record(&self, call: FakeCall) {
        lock(&self.shared.calls).push(call);
    }

    fn scripted(&self, command: &str) -> (Option<FakeReply>, Option<Gate>) {
        let script = lock(&self.shared.script);
        (
            script.replies.get(command).cloned(),
            script.gates.get(command).cloned(),
        )
    }
}

/// The [`AgentProcess`] handed to the daemon; dropping it closes the process like `omp-rpc` does.
struct FakeAgent(FakeProcess);

impl Drop for FakeAgent {
    fn drop(&mut self) {
        if let (_, Some(gate)) = self.0.scripted(CLOSE_GATE) {
            gate.pass();
        }
        self.0.exit();
    }
}

/// omp's `command_output` for an unscripted local command: `/add-dir` and `/remove-dir` always answer with one
/// (DMN-029), other commands with none.
fn default_output(message: &str) -> Vec<String> {
    if let Some(path) = message.strip_prefix("/add-dir ") {
        vec![format!("Added {path}.")]
    } else if let Some(path) = message.strip_prefix("/remove-dir ") {
        vec![format!("Removed {path}.")]
    } else {
        Vec::new()
    }
}

impl AgentProcess for FakeAgent {
    fn prompt(&self, command: PromptCommand) -> Result<PromptAdmission, omp_rpc::Error> {
        let process = &self.0;
        let local = command.message.starts_with('/');
        let message = command.message.clone();
        process.record(FakeCall::Prompt(command));
        if let (_, Some(gate)) = process.scripted("prompt") {
            gate.pass();
        }
        if process.is_closed() {
            return Err(omp_rpc::Error::Closed);
        }
        if local {
            let texts = lock(&process.shared.script)
                .outputs
                .get(message.as_str())
                .cloned()
                .unwrap_or_else(|| default_output(&message));
            process.emit(
                texts
                    .into_iter()
                    .map(|text| RpcNotification::CommandOutput(CommandOutputEvent { text }))
                    .collect(),
            );
            return Ok(PromptAdmission {
                prompt_id: format!("req_{}", new_id()),
                agent_invoked: false,
            });
        }
        let prompt_id = process
            .next_prompt_id()
            .unwrap_or_else(|| format!("req_{}", new_id()));
        process.play_next();
        Ok(PromptAdmission {
            prompt_id,
            agent_invoked: true,
        })
    }

    fn call(&self, call: OmpCall) -> Result<Value, omp_rpc::Error> {
        let process = &self.0;
        let command = call.name();
        process.record(FakeCall::Call(call.clone()));
        let (reply, gate) = process.scripted(command);
        if let Some(gate) = gate {
            gate.pass();
        }
        if process.is_closed() {
            return Err(omp_rpc::Error::Closed);
        }
        if let Some(reply) = reply {
            return match reply {
                FakeReply::Ok(value) => Ok(value),
                FakeReply::Fail { message, code } => Err(omp_rpc::Error::Command {
                    command: command.to_owned(),
                    error: message,
                    code,
                }),
            };
        }
        match &call {
            OmpCall::LiveConnect(connect) => {
                let script = lock(&process.shared.script).live.clone();
                process.live_connect(connect, script)
            }
            OmpCall::LiveStart(start) => {
                let script = lock(&process.shared.script).live.clone();
                process.live_start(start, script)
            }
            OmpCall::LiveMute(mute) => process.live_mute(mute),
            OmpCall::LiveStop => Ok(process.live_stop()),
            OmpCall::Login(login) => Ok(json!({ "providerId": login.provider_id })),
            _ => Ok(default_data(command, &process.shared.session_id)),
        }
    }

    fn respond(&self, response: ExtensionUiResponse) -> Result<(), omp_rpc::Error> {
        let process = &self.0;
        process.record(FakeCall::Respond(response));
        if process.is_closed() {
            return Err(omp_rpc::Error::Closed);
        }
        process.play_next();
        Ok(())
    }

    fn kill(&self) {
        self.0.exit();
    }
}

/// omp's response data to `command` when the test scripted none: the smallest value of the command's wire type
/// (`GetAvailableModelsResult` for `get_available_models`, …), `null` for commands without data.
fn default_data(command: &str, session_id: &str) -> Value {
    match command {
        "get_state" => json!({ "sessionId": session_id }),
        "get_session_stats" => json!({
            "sessionId": session_id, "userMessages": 0, "assistantMessages": 0, "toolCalls": 0, "toolResults": 0,
            "totalMessages": 0, "premiumRequests": 0, "cost": 0,
            "tokens": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0, "reasoning": 0 }
        }),
        "get_messages" => json!({ "messages": [] }),
        "abort_and_restore_queue" => json!({ "steering": [], "followUp": [] }),
        "remove_queued_message" => json!({ "removed": false }),
        "promote_queued_message" => json!({ "promoted": false }),
        "get_available_models" => json!({ "models": [] }),
        "get_available_thinking_levels" => json!({ "levels": ["off"] }),
        "get_available_commands" => json!({ "commands": [] }),
        "compact" => json!({ "summary": "", "firstKeptEntryId": "", "tokensBefore": 0 }),
        "get_tree" => json!({ "tree": [], "leafId": null }),
        "navigate_tree" | "fork" => json!({ "cancelled": false }),
        "get_subagents" => json!({ "subagents": [] }),
        "get_subagent_messages" => json!({
            "sessionFile": "", "fromByte": 0, "nextByte": 0, "reset": false, "entries": [], "messages": []
        }),
        "cancel_subagent" => json!(false),
        "goal" => json!({ "goal": null, "state": null }),
        "btw" => json!({ "record": {
            "question": "", "answer": "", "status": "complete", "createdAt": 0, "updatedAt": 0, "id": "btw",
            "leafId": null
        }}),
        "bash" => json!({
            "output": "", "cancelled": false, "truncated": false, "totalLines": 0, "totalBytes": 0,
            "outputLines": 0, "outputBytes": 0, "exitCode": 0
        }),
        "get_login_providers" => json!({ "providers": [] }),
        "get_logout_accounts" => json!({ "accounts": [] }),
        "logout" => json!({}),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> FakeRun {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/threads")
            .join(omfp_core::OMP_VERSION)
            .join(format!("{name}.jsonl"));
        FakeRun::from_fixture(&path).unwrap()
    }

    fn frame_types(frames: &[RpcNotification]) -> Vec<String> {
        frames
            .iter()
            .map(|frame| {
                serde_json::to_value(frame).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    // spec: TST-014
    #[test]
    fn a_fixture_splits_into_startup_turn_and_answer_segments() {
        let run = fixture("approval_granted");

        assert_eq!(run.segments.len(), 3);
        assert_eq!(frame_types(&run.segments[0])[0], "ready");
        assert_eq!(frame_types(&run.segments[1])[0], "agent_start");
        assert!(frame_types(&run.segments[1]).ends_with(&["extension_ui_request".to_owned()]));
        assert!(frame_types(&run.segments[2]).ends_with(&["session_settled".to_owned()]));
        assert_eq!(
            run.startup.ready.supports_bash_exclude_from_context,
            Some(true)
        );
    }

    // spec: TST-014
    #[test]
    fn the_fake_replays_only_recorded_frames_in_order() {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/threads")
                .join(omfp_core::OMP_VERSION)
                .join("text_reply.jsonl"),
        )
        .unwrap();
        let recorded: Vec<RpcNotification> = text
            .lines()
            .filter_map(|line| match serde_json::from_str(line).unwrap() {
                ThreadEvent::Omp { frame } => Some(frame),
                _ => None,
            })
            .collect();
        let launcher = FakeLauncher::new();
        launcher.push_run(FakeRun::from_jsonl(&text).unwrap());
        let spec = LaunchSpec {
            program: "omp".into(),
            cwd: "/home".into(),
            add_dirs: Vec::new(),
            session_dir: "/sessions".into(),
            fork: None,
            model: None,
            approval_mode: None,
            append_system_prompt: None,
            extension: None,
            stderr_log: "/omp.log".into(),
            env: Vec::new(),
        };

        let launched = launcher.launch(&spec, Vec::new(), Vec::new()).unwrap();
        let admission = launched
            .process
            .prompt(PromptCommand {
                message: "say hi".into(),
                images: None,
                streaming_behavior: None,
            })
            .unwrap();
        drop(launched.process);
        let replayed: Vec<RpcNotification> = launched
            .events
            .iter()
            .map(|event| match event {
                Event::Notification(frame) => frame,
                other => panic!("unexpected {other:?}"),
            })
            .collect();

        assert_eq!(replayed, recorded);
        assert_eq!(admission.prompt_id, "req_7");
        assert!(launcher.process(0).is_closed());
    }
}
