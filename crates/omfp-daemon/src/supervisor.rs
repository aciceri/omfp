//! Agent supervision (02-daemon, "Agent supervision"): one actor per thread owns the thread's event log and, while
//! omp runs, its process. Lifecycle operations (start, stop, restart, root sync) are serialized per thread; omp
//! commands are forwarded concurrently (DMN-037); threads proceed in parallel.
//!
//! omp sits behind the [`AgentLauncher`] port (ARC-012). Time is tokio's clock, so component tests drive the idle
//! reaper with paused time.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use omfp_core::{
    LiveOwner, NotifyRequest, OpenFileRequest, ProcessState, SessionInfo, ThreadEvent, ThreadState,
};
use omfp_protocol::RestartReason;
use omp_rpc::{
    FollowUpCommand, GetMessagesResult, LiveLevelsEvent, PromptCommand, RpcAgentEvent,
    RpcNotification, SetModelCommand, SteerCommand,
};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{info, warn};

use crate::agent::{AgentLauncher, AgentProcess, Lease, OmpCall, PromptAdmission};
use crate::conventions;
use crate::event_log::EventLog;
use crate::host_tools;
use crate::omp::{LaunchSpec, startup_events, thread_event};
use crate::paths::DataDir;
use crate::store::{Thread, TitleSource, new_id};
use crate::threads::{ThreadError, Threads};
use crate::workspaces::{WorkspaceError, Workspaces};

mod dialogs;
mod fork;
mod live;
mod login;
mod mcp;
mod models;
mod naming;
mod oauth;
mod restart;
mod roots;

pub use login::{AuthOmp, LOGIN_MODEL};
pub use oauth::OAuthOwner;
pub use restart::Restart;

/// The event log of one thread: omp frames and daemon events folded by the shared reducer (ARC-006).
pub type ThreadLog = EventLog<ThreadEvent, ThreadState>;

/// How long shutdown waits for omp processes to stop (DMN-005).
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// Lines of `omp.log` reported with a crash (DMN-035).
const STDERR_TAIL_LINES: usize = 20;

/// How long the `command_output` of a local command the daemon sent itself (`/mcp test`, a root sync's `/add-dir`)
/// may trail omp's answer to the prompt: omp writes the output first, so it is only in transit through the event pump
/// (DMN-029, DMN-191).
const OUTPUT_WAIT: Duration = Duration::from_secs(10);

/// A message for omp's queues while the agent works (`thread.steer`, `thread.follow_up`, DMN-226).
#[derive(Debug, Clone)]
pub enum QueuedMessage {
    /// Delivered after the current tool calls, before the next model call.
    Steer(SteerCommand),
    /// Delivered once the agent finished.
    FollowUp(FollowUpCommand),
}

/// An omp command that moves the session's leaf: the transcript is read again once it succeeded (DMN-111).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionChange {
    /// `fork`: a new session file too (DMN-072).
    Fork,
    /// `navigate_tree`: the same session file.
    Navigate,
}

/// Tools that never change workspace files, omp's and omfp's own; their end needs no rescan (DMN-023). Unknown tools
/// count as writers. omp has no `fetch` tool: `read` fetches URLs.
const READ_ONLY_TOOLS: &[&str] = &[
    "read",
    "grep",
    "glob",
    "find",
    "ast_grep",
    "web_search",
    "todo",
    "ask",
    "lsp",
    host_tools::NOTIFY,
    host_tools::SET_THREAD_TITLE,
    crate::agent_tools::THREAD_WAIT,
    crate::agent_tools::checks::WIDGET_CHECK,
    crate::agent_tools::checks::VIEWS_ERRORS,
    crate::agent_tools::windows::WINDOWS,
    crate::agent_tools::mounts::COMPONENT_LIST,
];

/// How many `live_levels` a slow subscriber may miss before it skips to the latest (DMN-110).
const LEVELS_BUFFER: usize = 8;

/// What `omfp_open_file` answers when no window shows its thread (DMN-060).
pub const NO_WINDOW: &str = "No omfp window shows this thread, so the file cannot be opened: give the user its path \
                             instead, or ask them to open the thread in omfp and try again.";

/// What `omfp_notify` answers when no window shows its thread (DMN-126).
pub const NOTIFY_NO_WINDOW: &str = "No omfp window shows this thread, so the user cannot be notified: say it in your \
                                    reply instead; they will read it when they open the thread.";

/// Supervisor settings.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// omp executable (DMN-006).
    pub omp_program: PathBuf,
    /// Environment overrides for every omp process (tests isolate omp with them).
    pub omp_env: Vec<(OsString, Option<OsString>)>,
    /// Maximum concurrent omp processes (DMN-038).
    pub max_processes: usize,
    /// Idle, settled, unwatched processes are stopped after this long (DMN-034).
    pub idle_timeout: Duration,
    /// Events kept per thread for catch-up (DMN-041).
    pub event_capacity: usize,
    /// How far a subscriber may fall behind before it must resynchronize (PRT-023).
    pub live_buffer: usize,
    /// omfp's isolated omp agent directory, where the user's `APPEND_SYSTEM.md` lives (ARC-021, DMN-030).
    pub agent_dir: Option<PathBuf>,
    /// The home omp runs with, where it reads the MCP configs of other tools (`~/.claude.json`, …, DMN-190).
    pub home: Option<PathBuf>,
    /// Generate and refresh automatic titles from meaningful new conversation (DMN-073). Tests against the
    /// fake LLM turn it off unless they script the extra model calls.
    pub title_generation: bool,
    /// `<static-dir>/builtin-widgets/` when the daemon serves a bundle with built-in widgets: the `omfp://` host URIs
    /// serve their sources (DMN-062).
    pub builtin_widgets: Option<PathBuf>,
}

impl SupervisorConfig {
    /// Spec defaults: 64 processes, 15 minutes idle timeout, 2000 events kept.
    #[must_use]
    pub fn new(omp_program: impl Into<PathBuf>) -> Self {
        Self {
            omp_program: omp_program.into(),
            omp_env: Vec::new(),
            max_processes: 64,
            idle_timeout: Duration::from_mins(15),
            event_capacity: 2000,
            live_buffer: 4096,
            agent_dir: None,
            home: None,
            title_generation: true,
            builtin_widgets: None,
        }
    }
}

/// Why a supervisor operation failed.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("at most {0} omp processes may run at once; stop another thread first")]
    LimitReached(usize),
    #[error(
        "omp session {session_id} is open in another omp process (e.g. in a terminal); close it there first"
    )]
    SessionLeased { session_id: String },
    /// omp could not start; `stderr_tail` holds the last lines of its stderr log (DMN-212).
    #[error("omp failed to start: {message}")]
    Start {
        message: String,
        stderr_tail: String,
    },
    #[error("omp `{command}` failed: {message}")]
    Omp {
        command: String,
        message: String,
        /// omp's own code (DMN-071).
        code: Option<String>,
    },
    #[error("the omp process is gone: {0}")]
    ProcessGone(String),
    #[error("interaction {0} is not pending (already answered or cancelled)")]
    AlreadyResolved(String),
    #[error("{0}")]
    InvalidResponse(String),
    #[error("thread.live_connect needs a non-empty sdpOffer")]
    EmptyOffer,
    #[error("a live session is already running on this thread (started by {client})")]
    LiveActive { client: String },
    #[error("no live session is running on this thread")]
    NoLiveSession,
    #[error(
        "the thread is working: a restart would abort it; wait until it is idle (or stop it), then reconnect"
    )]
    Busy,
    #[error(
        "the thread is working: moving it to another workspace restarts its omp; wait until it is idle (or stop it), then move it"
    )]
    BusyMove,
    /// A live voice failure, readable (DMN-104); `omp` is omp's own message.
    #[error("{message}")]
    Live { message: String, omp: String },
    #[error("the daemon is shutting down")]
    ShuttingDown,
    #[error(transparent)]
    Thread(#[from] ThreadError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T, E = SupervisorError> = std::result::Result<T, E>;

impl SupervisorError {
    /// Whether the thread's omp process could not be started, so it never ran for this request (DMN-211).
    #[must_use]
    pub fn is_start_failure(&self) -> bool {
        matches!(
            self,
            Self::LimitReached(_) | Self::SessionLeased { .. } | Self::Start { .. }
        )
    }
}

impl From<omp_rpc::Error> for SupervisorError {
    fn from(error: omp_rpc::Error) -> Self {
        match error {
            omp_rpc::Error::Command {
                command,
                error,
                code,
            } => Self::Omp {
                command,
                message: error,
                code,
            },
            omp_rpc::Error::Timeout { command } => Self::Omp {
                command,
                message: "no response in time".into(),
                code: None,
            },
            other @ (omp_rpc::Error::Closed
            | omp_rpc::Error::Io(_)
            | omp_rpc::Error::Protocol(_)) => Self::ProcessGone(other.to_string()),
            other => Self::Omp {
                command: String::new(),
                message: other.to_string(),
                code: None,
            },
        }
    }
}

/// What the thread list shows about a thread's process (`ThreadSummary`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadStatus {
    pub process: ProcessState,
    pub settled: bool,
    pub needs_attention: bool,
    /// Why the thread's omp should restart while background work holds the restart back (DMN-204, DMN-228).
    pub restart_pending: Vec<RestartReason>,
}

impl Default for ThreadStatus {
    fn default() -> Self {
        Self {
            process: ProcessState::Stopped,
            settled: true,
            needs_attention: false,
            restart_pending: Vec::new(),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[allow(clippy::trivially_copy_pass_by_ref)] // `EventLog` takes `fn(&mut S, &E)`.
fn reduce(state: &mut ThreadState, event: &ThreadEvent) {
    state.apply(event);
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| SupervisorError::Internal(error.to_string()))
}

/// A running process of a thread.
struct Live {
    process: Arc<dyn AgentProcess>,
    generation: u64,
    /// Becomes `true` once every event of the process has been pushed.
    ended: watch::Receiver<bool>,
}

/// The live voice session of a thread, from `thread.live_connect` or `thread.live_start` until it ends (DMN-101).
struct VoiceSlot {
    /// Tells a session from the next one on the same thread.
    id: u64,
    owner: LiveOwner,
}

/// A thread's prompts as `omfp_thread_wait` follows them (DMN-152).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastPrompt {
    /// The id of the last prompt omp admitted with agent work, until the thread's next process starts.
    pub id: Option<String>,
    /// Prompts on their way to omp, not admitted yet: each may become the last one.
    pub sending: usize,
}

/// A prompt on its way to omp, counted in [`LastPrompt::sending`] until it is admitted, fails or is dropped (DMN-152).
struct Sending<'a>(&'a watch::Sender<LastPrompt>);

impl<'a> Sending<'a> {
    fn new(last: &'a watch::Sender<LastPrompt>) -> Self {
        last.send_modify(|last| last.sending += 1);
        Self(last)
    }

    /// omp admitted the prompt with agent work: it is the last prompt before it stops counting as on its way.
    fn admitted(self, id: String) {
        self.0.send_modify(|last| last.id = Some(id));
    }
}

impl Drop for Sending<'_> {
    fn drop(&mut self) {
        self.0.send_modify(|last| last.sending -= 1);
    }
}

/// One thread: its log, its process and who watches it.
struct Actor {
    id: String,
    log: Arc<ThreadLog>,
    /// Serializes lifecycle operations (DMN-037).
    lifecycle: tokio::sync::Mutex<()>,
    live: Mutex<Option<Live>>,
    /// Makes check-forward-record of an answer atomic (DMN-051).
    answers: Mutex<()>,
    /// Connections watching the thread (PRT-025), one entry per subscription, the one the user last used on it last
    /// (DMN-060).
    watchers: Mutex<Vec<u64>>,
    last_activity: Mutex<Instant>,
    /// A root sync was asked while the agent was busy (omp refuses `/add-dir` while streaming); it runs once the
    /// session settles (DMN-029).
    roots_pending: AtomicBool,
    /// Conversation-driven title cadence and pending topic confirmation (DMN-073).
    title_policy: tokio::sync::Mutex<crate::titles::RollingTitle>,
    /// Serializes session renames; each sender reads the latest stored title after taking it.
    title_sync: tokio::sync::Mutex<()>,
    /// The running title generation, aborted when the thread is forgotten.
    title_task: Mutex<Option<JoinHandle<()>>>,
    /// The live voice session (DMN-101).
    voice: Mutex<Option<VoiceSlot>>,
    /// omp's `live_levels`, which bypass the log (DMN-110).
    levels: broadcast::Sender<LiveLevelsEvent>,
    /// The last prompt omp admitted with agent work and the prompts on their way to omp (DMN-152).
    last_prompt: watch::Sender<LastPrompt>,
    /// Serializes the daemon's own local commands (`/mcp test`, root sync), whose `command_output` frames carry no id
    /// (DMN-029, DMN-191).
    local_commands: tokio::sync::Mutex<()>,
    /// The daemon's `/mcp test` in flight: its `command_output` goes here, not into the transcript (DMN-200).
    mcp_check: Mutex<Option<mcp::PendingCheck>>,
    /// The daemon's root sync command (`/add-dir`, `/remove-dir`) in flight: its `command_output` goes here, not into
    /// the transcript (DMN-029).
    root_sync: Mutex<Option<roots::PendingRoot>>,
    /// The workspace folder the running omp read its MCP configs from and their fingerprint then (DMN-201).
    mcp_configs: Mutex<Option<(PathBuf, crate::omp_mcp::ConfigFingerprint)>>,
    /// The restart the daemon owes the thread's omp, and why (DMN-203, DMN-204, DMN-228).
    restart: Mutex<restart::DueRestart>,
    /// Held (shared) while a message is on its way to omp (DMN-226), so a restart never cuts it, nor the answer of
    /// the prompt that asked for it (`/mcp add`, DMN-203, DMN-227).
    prompting: tokio::sync::RwLock<()>,
    /// The source session file a forked thread's first start copies (`--fork`, DMN-301), taken by that start.
    fork_source: Mutex<Option<PathBuf>>,
    /// The model the stored session restores, read from its file when omp could not start (DMN-338); cleared when omp
    /// starts.
    session_model: Mutex<Option<String>>,
    /// The OAuth sign-ins the running omp started that wait for their loopback callback (DMN-340); cleared when omp
    /// starts or stops.
    oauth: Mutex<crate::oauth_relay::Flows>,
}

impl Actor {
    fn new(id: &str, config: &SupervisorConfig) -> Self {
        Self {
            id: id.to_owned(),
            log: Arc::new(EventLog::new(
                new_id(),
                ThreadState::default(),
                reduce,
                config.event_capacity,
                config.live_buffer,
            )),
            lifecycle: tokio::sync::Mutex::new(()),
            live: Mutex::new(None),
            answers: Mutex::new(()),
            watchers: Mutex::new(Vec::new()),
            last_activity: Mutex::new(Instant::now()),
            roots_pending: AtomicBool::new(false),
            title_policy: tokio::sync::Mutex::new(crate::titles::RollingTitle::default()),
            title_sync: tokio::sync::Mutex::new(()),
            title_task: Mutex::new(None),
            voice: Mutex::new(None),
            levels: broadcast::channel(LEVELS_BUFFER).0,
            last_prompt: watch::Sender::new(LastPrompt::default()),
            local_commands: tokio::sync::Mutex::new(()),
            mcp_check: Mutex::new(None),
            root_sync: Mutex::new(None),
            mcp_configs: Mutex::new(None),
            restart: Mutex::new(restart::DueRestart::default()),
            prompting: tokio::sync::RwLock::new(()),
            fork_source: Mutex::new(None),
            session_model: Mutex::new(None),
            oauth: Mutex::new(crate::oauth_relay::Flows::default()),
        }
    }

    fn process(&self) -> Option<Arc<dyn AgentProcess>> {
        lock(&self.live)
            .as_ref()
            .map(|live| Arc::clone(&live.process))
    }

    fn status(&self) -> ThreadStatus {
        let restart_pending = lock(&self.restart).pending();
        self.log.with_state(|state| ThreadStatus {
            process: state.process.clone(),
            settled: state.settled,
            needs_attention: !state.interactions.is_empty(),
            restart_pending,
        })
    }

    fn touch(&self) {
        *lock(&self.last_activity) = Instant::now();
    }

    /// Running, settled, without pending requests and unwatched since at least `timeout` (DMN-034).
    fn is_idle(&self, timeout: Duration, now: Instant) -> bool {
        lock(&self.live).is_some()
            && lock(&self.watchers).is_empty()
            && self.log.with_state(|state| {
                state.process == ProcessState::Running
                    && state.settled
                    && state.interactions.is_empty()
            })
            && now.saturating_duration_since(*lock(&self.last_activity)) >= timeout
            && lock(&self.voice).is_none()
    }

    /// Frees the live session slot if it still holds session `id`; `true` when it did.
    fn release_voice(&self, id: u64) -> bool {
        let mut voice = lock(&self.voice);
        if voice.as_ref().is_some_and(|slot| slot.id == id) {
            voice.take();
            true
        } else {
            false
        }
    }
}

struct Inner {
    launcher: Arc<dyn AgentLauncher>,
    threads: Arc<Threads>,
    workspaces: Arc<Workspaces>,
    data: DataDir,
    config: SupervisorConfig,
    actors: Mutex<HashMap<String, Arc<Actor>>>,
    /// Processes running or starting (DMN-038).
    running: AtomicUsize,
    generations: AtomicU64,
    /// Numbers live sessions (DMN-101).
    voice_ids: AtomicU64,
    /// Ids of threads whose [`ThreadStatus`] changed.
    changes: broadcast::Sender<String>,
    /// Ids of threads that finished a tool execution (a rescan trigger, DMN-023).
    tool_ends: broadcast::Sender<String>,
    /// Clients watching each workspace (subscribers of `workspace/<id>`), which polls it (DMN-023).
    workspace_watchers: Mutex<HashMap<String, usize>>,
    /// Backs the `omfp://` URIs of the workspace of every thread launched from now on (DMN-108, DMN-199, AGT-081);
    /// set by the server once its services exist, dropped at shutdown (the services hold the supervisor too).
    widget_host: Mutex<Option<Arc<dyn host_tools::WidgetHost>>>,
    /// Backs the agent tools of every thread launched from now on (AGT-040, DMN-151); set and dropped like
    /// `widget_host`.
    agent_host: Mutex<Option<Arc<dyn host_tools::AgentHost>>>,
    shutting_down: AtomicBool,
    /// The running `auth.login` and the numbers telling logins apart (DMN-222).
    login: Mutex<Option<login::Running>>,
    logins: AtomicU64,
    /// What the daemon knows of the models omp can use (DMN-235..DMN-237).
    models: models::UsableModels,
}

/// The agent supervisor. Cheap to clone.
#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("config", &self.inner.config)
            .finish_non_exhaustive()
    }
}

/// Counts a client watching a thread while alive (PRT-025).
pub struct WatchGuard {
    actor: Arc<Actor>,
    connection: u64,
}

impl std::fmt::Debug for WatchGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("WatchGuard").field(&self.actor.id).finish()
    }
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        let mut watchers = lock(&self.actor.watchers);
        if let Some(index) = watchers
            .iter()
            .rposition(|watcher| *watcher == self.connection)
        {
            watchers.remove(index);
        }
        drop(watchers);
        // The idle timeout counts from the moment the last client left.
        self.actor.touch();
    }
}

/// Counts a client watching a workspace while alive (DMN-023).
pub struct WorkspaceWatchGuard {
    inner: Arc<Inner>,
    id: String,
}

impl std::fmt::Debug for WorkspaceWatchGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("WorkspaceWatchGuard")
            .field(&self.id)
            .finish()
    }
}

impl Drop for WorkspaceWatchGuard {
    fn drop(&mut self) {
        let mut watchers = lock(&self.inner.workspace_watchers);
        if let Some(count) = watchers.get_mut(&self.id) {
            *count -= 1;
            if *count == 0 {
                watchers.remove(&self.id);
            }
        }
    }
}

impl Supervisor {
    #[must_use]
    pub fn new(
        launcher: Arc<dyn AgentLauncher>,
        threads: Arc<Threads>,
        workspaces: Arc<Workspaces>,
        data: DataDir,
        config: SupervisorConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                launcher,
                threads,
                workspaces,
                data,
                config,
                actors: Mutex::new(HashMap::new()),
                running: AtomicUsize::new(0),
                generations: AtomicU64::new(0),
                voice_ids: AtomicU64::new(0),
                changes: broadcast::channel(256).0,
                tool_ends: broadcast::channel(256).0,
                workspace_watchers: Mutex::new(HashMap::new()),
                widget_host: Mutex::new(None),
                agent_host: Mutex::new(None),
                shutting_down: AtomicBool::new(false),
                login: Mutex::new(None),
                models: models::UsableModels::default(),
                logins: AtomicU64::new(0),
            }),
        }
    }

    /// Gives the threads launched from now on the `omfp://` URIs of their workspace (DMN-108, DMN-199, AGT-081).
    pub fn set_widget_host(&self, host: Arc<dyn host_tools::WidgetHost>) {
        *lock(&self.inner.widget_host) = Some(host);
    }

    /// Gives the threads launched from now on the agent tools of the registry (AGT-040, DMN-151).
    pub fn set_agent_host(&self, host: Arc<dyn host_tools::AgentHost>) {
        *lock(&self.inner.agent_host) = Some(host);
    }

    /// Ids of threads whose agent finished a tool execution (`tool_execution_end`).
    #[must_use]
    pub fn subscribe_tool_ends(&self) -> broadcast::Receiver<String> {
        self.inner.tool_ends.subscribe()
    }

    /// Ids of threads whose process state, settledness or pending requests changed.
    #[must_use]
    pub fn subscribe_changes(&self) -> broadcast::Receiver<String> {
        self.inner.changes.subscribe()
    }

    /// The event log of a thread (PRT-020..022).
    pub async fn thread_log(&self, id: &str) -> Result<Arc<ThreadLog>> {
        Ok(Arc::clone(&self.actor(id).await?.log))
    }

    /// Marks the thread as watched by connection `connection` until the guard drops (PRT-025); it becomes the
    /// window the user last used on the thread (DMN-060).
    pub async fn watch(&self, id: &str, connection: u64) -> Result<WatchGuard> {
        let actor = self.actor(id).await?;
        lock(&actor.watchers).push(connection);
        Ok(WatchGuard { actor, connection })
    }

    /// Connection `connection` sent thread `id` something on the user's behalf: if it watches the thread, it becomes
    /// the window the user last used on it (DMN-060).
    pub fn used(&self, id: &str, connection: u64) {
        if let Some(actor) = self.lookup(id) {
            let mut watchers = lock(&actor.watchers);
            if let Some(index) = watchers.iter().rposition(|watcher| *watcher == connection) {
                let watcher = watchers.remove(index);
                watchers.push(watcher);
            }
        }
    }

    /// Number of clients watching the thread (PRT-025).
    #[must_use]
    pub fn watchers(&self, id: &str) -> usize {
        self.lookup(id)
            .map_or(0, |actor| lock(&actor.watchers).len())
    }

    /// Asks the window the user last used on thread `id` to open `path` at `line` (DMN-060): an `open_file` event
    /// naming its connection, which the window acts on. Without a window watching the thread the error tells the
    /// agent what to do instead.
    pub fn open_file(&self, id: &str, path: &str, line: Option<u64>) -> Result<(), String> {
        let (actor, connection) = self.last_window(id).ok_or(NO_WINDOW)?;
        self.push(
            &actor,
            ThreadEvent::OpenFile(OpenFileRequest {
                id: new_id(),
                connection,
                path: path.to_owned(),
                line,
            }),
        );
        Ok(())
    }

    /// Asks the window the user last used on thread `id` to notify the user (DMN-125): a `notify` event naming its
    /// connection, which the window shows as a notice about the thread (UI-215). Without a window watching the thread
    /// the error tells the agent what to do instead (DMN-126).
    pub fn notify(&self, id: &str, title: &str, body: &str) -> Result<(), String> {
        let (actor, connection) = self.last_window(id).ok_or(NOTIFY_NO_WINDOW)?;
        self.push(
            &actor,
            ThreadEvent::Notify(NotifyRequest {
                id: new_id(),
                connection,
                title: title.to_owned(),
                body: body.to_owned(),
            }),
        );
        Ok(())
    }

    /// The thread's actor and the connection of the window the user last used on it, if any watches it (DMN-060).
    fn last_window(&self, id: &str) -> Option<(Arc<Actor>, u64)> {
        let actor = self.lookup(id)?;
        let connection = lock(&actor.watchers).last().copied()?;
        Some((actor, connection))
    }

    /// Whether the thread has a live omp process.
    #[must_use]
    pub fn is_running(&self, id: &str) -> bool {
        self.lookup(id)
            .is_some_and(|actor| lock(&actor.live).is_some())
    }

    /// Marks the workspace as watched until the guard drops (DMN-023).
    #[must_use]
    pub fn watch_workspace(&self, id: &str) -> WorkspaceWatchGuard {
        *lock(&self.inner.workspace_watchers)
            .entry(id.to_owned())
            .or_default() += 1;
        WorkspaceWatchGuard {
            inner: Arc::clone(&self.inner),
            id: id.to_owned(),
        }
    }

    /// Number of clients watching the workspace (DMN-023).
    #[must_use]
    pub fn workspace_watchers(&self, id: &str) -> usize {
        lock(&self.inner.workspace_watchers)
            .get(id)
            .copied()
            .unwrap_or(0)
    }

    /// What the thread list shows about the thread.
    #[must_use]
    pub fn status(&self, id: &str) -> ThreadStatus {
        self.lookup(id)
            .map_or_else(ThreadStatus::default, |actor| actor.status())
    }

    /// The thread's last prompt and the prompts on their way to it, as they change (DMN-152).
    pub async fn last_prompt(&self, id: &str) -> Result<watch::Receiver<LastPrompt>> {
        Ok(self.actor(id).await?.last_prompt.subscribe())
    }

    /// The model the thread's stored session restores, known from its session file after omp could not start
    /// (DMN-338); `None` while omp runs or before a failed start.
    #[must_use]
    pub fn session_model(&self, id: &str) -> Option<String> {
        self.lookup(id)
            .and_then(|actor| lock(&actor.session_model).clone())
    }

    /// Starts omp for the thread if it is not running.
    pub async fn start(&self, id: &str) -> Result<()> {
        let actor = self.actor(id).await?;
        self.ensure_running(&actor).await.map(drop)
    }

    /// `thread.set_model` (DMN-339): a running omp switches with `set_model`, then the thread stores the choice. A
    /// thread whose omp does not run stores it first and starts omp with it (`--model`, DMN-030), resuming the same
    /// session: the change never needs the session's saved model, which may be one omp cannot restore. Nothing else
    /// ever changes the model.
    pub async fn set_model(&self, id: &str, command: SetModelCommand) -> Result<()> {
        let actor = self.actor(id).await?;
        let model = format!("{}/{}", command.provider, command.model_id);
        if actor.process().is_none() {
            let lifecycle = actor.lifecycle.lock().await;
            if actor.process().is_none() {
                self.store_model(id, model).await?;
                return self.start_locked(&actor).await.map(drop);
            }
            drop(lifecycle);
        }
        self.call(id, OmpCall::SetModel(command)).await?;
        self.store_model(id, model).await
    }

    async fn store_model(&self, id: &str, model: String) -> Result<()> {
        let threads = Arc::clone(&self.inner.threads);
        let id = id.to_owned();
        blocking(move || threads.set_model(&id, Some(&model))).await??;
        Ok(())
    }

    /// Stops the thread's omp process, if any. Cancellation-safe ([`Self::detached`]).
    pub async fn stop(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.detached(|supervisor| async move { supervisor.stop_now(&id).await })
            .await
    }

    async fn stop_now(&self, id: &str) -> Result<()> {
        let actor = self.actor(id).await?;
        let _lifecycle = actor.lifecycle.lock().await;
        self.stop_locked(&actor).await
    }

    /// Stops the thread, then every thread of its spawned tree (AGT-071, DMN-162) in creation order, reading the tree
    /// again after each pass until a pass finds no member left, so threads spawned meanwhile stop too. Every member is
    /// tried; the first error comes back after that. Returns the stopped threads in the order they were stopped.
    /// Cancellation-safe ([`Self::detached`]): an agent stopping its own tree loses its call with its process.
    pub async fn stop_tree(&self, id: &str) -> Result<Vec<String>> {
        let id = id.to_owned();
        self.detached(|supervisor| async move { supervisor.stop_tree_now(&id).await })
            .await
    }

    async fn stop_tree_now(&self, id: &str) -> Result<Vec<String>> {
        self.stop_now(id).await?;
        let mut stopped = vec![id.to_owned()];
        let mut failed = None;
        loop {
            let threads = Arc::clone(&self.inner.threads);
            let root = id.to_owned();
            let tree = blocking(move || threads.spawned(&root)).await??;
            let pending: Vec<String> = tree
                .into_iter()
                .filter(|member| !stopped.contains(member))
                .collect();
            if pending.is_empty() {
                break;
            }
            for member in pending {
                if let Err(error) = self.stop_now(&member).await {
                    warn!(thread = %member, %error, "cascade stop failed");
                    failed.get_or_insert(error);
                }
                stopped.push(member);
            }
        }
        failed.map_or(Ok(stopped), Err)
    }

    /// Runs a lifecycle operation as a task of its own that the caller only awaits, so dropping the caller cannot
    /// interrupt it half-way: an agent stopping or restarting its own thread through a host tool loses that call when
    /// its omp process dies, and an operation cut after taking the process would never report it stopped nor release
    /// its slot of the process limit (DMN-038).
    async fn detached<T, F>(&self, operation: impl FnOnce(Self) -> F) -> Result<T>
    where
        T: Send + 'static,
        F: Future<Output = Result<T>> + Send + 'static,
    {
        tokio::spawn(operation(self.clone()))
            .await
            .map_err(|error| SupervisorError::Internal(error.to_string()))?
    }

    /// Deletes the thread (DMN-039): its records first, so no omp can start for it any more (every start reads them
    /// under the thread's lifecycle lock), then its process, waiting for a start under way, then its directory. Once it
    /// returns no omp runs for the thread and nothing writes its directory again. Until then its workspace cannot be
    /// removed (DMN-028): a start under way still creates its folder. The hold names the workspace the records name
    /// when they go, which a move (DMN-283) may change meanwhile. Cancellation-safe ([`Self::detached`]).
    pub async fn delete(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.detached(|supervisor| async move {
            let workspaces = Arc::clone(&supervisor.inner.workspaces);
            let _hold = loop {
                let threads = Arc::clone(&supervisor.inner.threads);
                let thread_id = id.clone();
                let workspace_id = blocking(move || threads.get(&thread_id))
                    .await??
                    .workspace_id;
                let hold = workspaces.hold_removal(&workspace_id);
                let threads = Arc::clone(&supervisor.inner.threads);
                let thread_id = id.clone();
                if blocking(move || threads.delete_record(&thread_id, &workspace_id)).await?? {
                    break hold;
                }
            };
            supervisor.forget(&id).await?;
            let threads = Arc::clone(&supervisor.inner.threads);
            blocking(move || threads.remove_dir(&id)).await??;
            Ok(())
        })
        .await
    }

    /// Stops the thread's process and forgets its actor (the thread's records are gone).
    async fn forget(&self, id: &str) -> Result<()> {
        let Some(actor) = self.lookup(id) else {
            return Ok(());
        };
        {
            let _lifecycle = actor.lifecycle.lock().await;
            self.stop_locked(&actor).await?;
        }
        if let Some(task) = lock(&actor.title_task).take() {
            task.abort();
        }
        lock(&self.inner.actors).remove(id);
        Ok(())
    }

    /// Sends a prompt, starting omp first when needed (DMN-070); returns at admission (PRT-030).
    /// Admitted agent work names or reconsiders the thread's automatic title (DMN-073).
    pub async fn prompt(&self, id: &str, command: PromptCommand) -> Result<PromptAdmission> {
        let actor = self.actor(id).await?;
        let message = command.message.clone();
        let sending = Sending::new(&actor.last_prompt);
        let admission = self
            .deliver(&actor, move |process| process.prompt(command))
            .await?;
        if admission.agent_invoked {
            sending.admitted(admission.prompt_id.clone());
            self.consider_title(&actor, message).await;
        }
        Ok(admission)
    }

    /// Puts a message in omp's steering or follow-up queue with a prompt's bookkeeping (DMN-226):
    /// omp started when needed, root sync first, automatic-title consideration from admitted text (DMN-073).
    pub async fn queue(&self, id: &str, message: QueuedMessage) -> Result<()> {
        let actor = self.actor(id).await?;
        let (text, call) = match message {
            QueuedMessage::Steer(command) => (command.message.clone(), OmpCall::Steer(command)),
            QueuedMessage::FollowUp(command) => {
                (command.message.clone(), OmpCall::FollowUp(command))
            }
        };
        self.deliver(&actor, move |process| process.call(call))
            .await?;
        self.consider_title(&actor, text).await;
        Ok(())
    }

    /// What every message of the user goes through (DMN-226): omp started when needed (DMN-070), a root change made
    /// before it reaches omp first (DMN-029), and the send holds `prompting` so a reload never cuts it (DMN-203).
    async fn deliver<T: Send + 'static>(
        &self,
        actor: &Arc<Actor>,
        send: impl FnOnce(&dyn AgentProcess) -> Result<T, omp_rpc::Error> + Send + 'static,
    ) -> Result<T> {
        let process = self.ensure_running(actor).await?;
        self.roots_before_prompt(actor).await;
        actor.touch();
        let sending = actor.prompting.read().await;
        let sent = blocking(move || send(process.as_ref())).await;
        drop(sending);
        let sent = sent??;
        actor.touch();
        Ok(sent)
    }

    /// A root change made before the prompt reaches omp first (DMN-029): a sync still pending runs now if the session
    /// settled, and one in flight (it holds the lifecycle lock) finishes before the prompt is sent.
    async fn roots_before_prompt(&self, actor: &Actor) {
        if actor.roots_pending.load(Ordering::SeqCst) && actor.status().settled {
            if let Err(error) = self.sync_roots(&actor.id).await {
                warn!(thread = %actor.id, %error, "root sync before the prompt failed");
            }
        } else {
            drop(actor.lifecycle.lock().await);
        }
    }

    /// Forwards an omp command, starting omp first when needed (DMN-070). Commands do not wait for each other
    /// (DMN-037); omp's error codes are kept (DMN-071).
    pub async fn call(&self, id: &str, call: OmpCall) -> Result<Value> {
        let actor = self.actor(id).await?;
        let process = self.ensure_running(&actor).await?;
        actor.touch();
        let change = match call {
            OmpCall::Fork(_) => Some(SessionChange::Fork),
            OmpCall::NavigateTree(_) => Some(SessionChange::Navigate),
            _ => None,
        };
        let forwarded = Arc::clone(&process);
        let result = blocking(move || forwarded.call(call)).await??;
        actor.touch();
        // `cancelled` (an extension refused) or `aborted` (the summary stopped): nothing moved.
        let moved = result.get("cancelled").and_then(Value::as_bool) != Some(true)
            && result.get("aborted").and_then(Value::as_bool) != Some(true);
        if let Some(change) = change.filter(|_| moved) {
            self.reload_branch(&actor, process, change).await?;
        }
        Ok(result)
    }

    /// Forwards a read-only omp command only when the thread's omp runs, `None` otherwise: it never starts omp and is
    /// no activity of the thread (DMN-214).
    pub async fn call_if_running(&self, id: &str, call: OmpCall) -> Result<Option<Value>> {
        let Some(process) = self.lookup(id).and_then(|actor| actor.process()) else {
            return Ok(None);
        };
        Ok(Some(blocking(move || process.call(call)).await??))
    }

    /// A fork moves the process to a new session file, whose id is stored (DMN-072); a fork or a navigation changes
    /// the branch the transcript shows, which omp announces with no event: the daemon reads the session and its
    /// messages again and records them as a `history` event (DMN-111).
    async fn reload_branch(
        &self,
        actor: &Actor,
        process: Arc<dyn AgentProcess>,
        change: SessionChange,
    ) -> Result<()> {
        let state_process = Arc::clone(&process);
        let state = blocking(move || state_process.call(OmpCall::State)).await??;
        let Some(session_id) = state.get("sessionId").and_then(Value::as_str) else {
            return Err(SupervisorError::Internal(
                "get_state returned no sessionId".into(),
            ));
        };
        let session = SessionInfo {
            session_id: session_id.to_owned(),
            session_file: state
                .get("sessionFile")
                .and_then(Value::as_str)
                .map(str::to_owned),
            resumed: true,
        };
        if change == SessionChange::Fork {
            let threads = Arc::clone(&self.inner.threads);
            let (id, session_id) = (actor.id.clone(), session.session_id.clone());
            blocking(move || threads.set_omp_session_id(&id, &session_id)).await??;
        }
        let messages = blocking(move || process.call(OmpCall::Messages)).await??;
        let messages: GetMessagesResult = serde_json::from_value(messages)
            .map_err(|error| SupervisorError::Internal(format!("get_messages: {error}")))?;
        self.push(
            actor,
            ThreadEvent::History {
                session,
                messages: messages.messages,
            },
        );
        Ok(())
    }

    /// Brings the live process's roots in line with the repositories discovered in its workspace (DMN-029):
    /// `/add-dir` for new ones, `/remove-dir` for roots that disappeared, through `prompt` (local commands). omp
    /// refuses them while the agent streams, so a busy thread syncs once its session settles. A process still starting
    /// is waited for (it holds the lifecycle lock), since its flags come from a scan made before. A stopped thread
    /// needs nothing: its next launch passes the roots.
    pub async fn sync_roots(&self, id: &str) -> Result<()> {
        let Some(actor) = self.lookup(id) else {
            return Ok(());
        };
        let _lifecycle = actor.lifecycle.lock().await;
        let Some(process) = actor.process() else {
            actor.roots_pending.store(false, Ordering::SeqCst);
            return Ok(());
        };
        if !actor.status().settled {
            actor.roots_pending.store(true, Ordering::SeqCst);
            return Ok(());
        }
        actor.roots_pending.store(false, Ordering::SeqCst);
        let threads = Arc::clone(&self.inner.threads);
        let workspaces = Arc::clone(&self.inner.workspaces);
        let thread_id = actor.id.clone();
        let (desired, recorded) = blocking(move || {
            let thread = threads.get(&thread_id)?;
            let desired = root_paths(&workspaces.discovered(&thread.workspace_id)?);
            Ok::<_, SupervisorError>((desired, threads.roots(&thread_id)?))
        })
        .await??;
        self.apply_roots(&actor, process, desired, &recorded).await
    }

    /// Sends `/add-dir` for `desired` roots not `recorded` and `/remove-dir` for recorded ones no longer desired,
    /// then records `desired`. Housekeeping, not the thread's doing: it is no activity of the thread and its output
    /// stays out of the transcript (DMN-029).
    async fn apply_roots(
        &self,
        actor: &Actor,
        process: Arc<dyn AgentProcess>,
        desired: Vec<PathBuf>,
        recorded: &[PathBuf],
    ) -> Result<()> {
        let added = desired
            .iter()
            .filter(|root| !recorded.contains(root))
            .map(|root| roots::RootCommand::Add(root.clone()));
        let removed = recorded
            .iter()
            .filter(|root| !desired.contains(root))
            .map(|root| roots::RootCommand::Remove(root.clone()));
        let commands: Vec<roots::RootCommand> = added.chain(removed).collect();
        if !commands.is_empty() {
            let _one_at_a_time = actor.local_commands.lock().await;
            for command in commands {
                Self::root_command(actor, &process, command).await?;
            }
        }
        let threads = Arc::clone(&self.inner.threads);
        let thread_id = actor.id.clone();
        blocking(move || threads.set_roots(&thread_id, &desired)).await??;
        Ok(())
    }

    /// Stops every process (DMN-005): closing the `omp-rpc` client closes omp's stdin, then terminates its process
    /// group. Gives up waiting after [`SHUTDOWN_GRACE`]. Later starts are refused.
    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        lock(&self.inner.widget_host).take();
        lock(&self.inner.agent_host).take();
        self.cancel_login(None);
        let actors: Vec<Arc<Actor>> = lock(&self.inner.actors).values().cloned().collect();
        let stops = actors.iter().map(|actor| async move {
            let _lifecycle = actor.lifecycle.lock().await;
            if let Err(error) = self.stop_locked(actor).await {
                warn!(thread = %actor.id, %error, "stopping omp failed");
            }
        });
        if tokio::time::timeout(SHUTDOWN_GRACE, futures::future::join_all(stops))
            .await
            .is_err()
        {
            warn!("omp processes did not stop within {SHUTDOWN_GRACE:?}");
        }
    }

    /// Stops idle threads every so often (DMN-034) until the supervisor is dropped or shut down.
    #[must_use]
    pub fn spawn_reaper(&self) -> JoinHandle<()> {
        let inner = Arc::downgrade(&self.inner);
        let period = (self.inner.config.idle_timeout / 4)
            .clamp(Duration::from_secs(1), Duration::from_mins(1));
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let Some(supervisor) = Weak::upgrade(&inner).map(|inner| Self { inner }) else {
                    break;
                };
                if supervisor.inner.shutting_down.load(Ordering::SeqCst) {
                    break;
                }
                supervisor.reap_idle().await;
            }
        })
    }

    async fn reap_idle(&self) {
        let timeout = self.inner.config.idle_timeout;
        let actors: Vec<Arc<Actor>> = lock(&self.inner.actors).values().cloned().collect();
        for actor in actors {
            if !actor.is_idle(timeout, Instant::now()) {
                continue;
            }
            let Ok(_lifecycle) = actor.lifecycle.try_lock() else {
                continue;
            };
            if actor.is_idle(timeout, Instant::now()) {
                info!(thread = %actor.id, "stopping idle omp process");
                if let Err(error) = self.stop_locked(&actor).await {
                    warn!(thread = %actor.id, %error, "stopping idle omp failed");
                }
            }
        }
    }

    fn lookup(&self, id: &str) -> Option<Arc<Actor>> {
        lock(&self.inner.actors).get(id).cloned()
    }

    async fn actor(&self, id: &str) -> Result<Arc<Actor>> {
        if let Some(actor) = self.lookup(id) {
            return Ok(actor);
        }
        let threads = Arc::clone(&self.inner.threads);
        let thread_id = id.to_owned();
        blocking(move || threads.get(&thread_id)).await??;
        Ok(Arc::clone(
            lock(&self.inner.actors)
                .entry(id.to_owned())
                .or_insert_with(|| Arc::new(Actor::new(id, &self.inner.config))),
        ))
    }

    fn push(&self, actor: &Actor, event: ThreadEvent) {
        // Up to ten a second for the spectrum only: they bypass the log, the state and catch-up (DMN-110).
        if let ThreadEvent::Omp {
            frame: RpcNotification::LiveLevels(levels),
        } = event
        {
            let _ = actor.levels.send(levels);
            return;
        }
        // Read-only tools cannot change files: skip the `git status` of a full scan after them (DMN-023).
        let tool_end = matches!(
            &event,
            ThreadEvent::Omp {
                frame: RpcNotification::RpcAgentEvent(RpcAgentEvent::ToolExecutionEnd(end))
            } if !READ_ONLY_TOOLS.contains(&end.tool_name.as_str())
        );
        if let ThreadEvent::Omp {
            frame: RpcNotification::CommandOutput(output),
        } = &event
        {
            // The daemon's own root sync and checks answer the daemon, not the user (DMN-029, DMN-200).
            if Self::take_root_output(actor, &output.text)
                || Self::take_check_output(actor, &output.text)
            {
                return;
            }
            if let Some(reason) = crate::omp_mcp::restart_reason(&output.text) {
                lock(&actor.restart).want(reason);
            }
        }
        Self::track_oauth(actor, &event);
        if matches!(
            &event,
            ThreadEvent::Omp {
                frame: RpcNotification::LiveEnd(_)
            }
        ) {
            // omp ended the session: the thread may start another (DMN-101).
            lock(&actor.voice).take();
        }
        let before = actor.status();
        actor.log.push(event);
        actor.touch();
        let after = actor.status();
        if after != before {
            let _ = self.inner.changes.send(actor.id.clone());
        }
        if tool_end {
            let _ = self.inner.tool_ends.send(actor.id.clone());
        }
        self.restart_when_free(actor);
        if after.settled && !before.settled && actor.roots_pending.load(Ordering::SeqCst) {
            let supervisor = self.clone();
            let id = actor.id.clone();
            tokio::spawn(async move {
                if let Err(error) = supervisor.sync_roots(&id).await {
                    warn!(thread = %id, %error, "root sync failed");
                }
            });
        }
    }

    async fn ensure_running(&self, actor: &Arc<Actor>) -> Result<Arc<dyn AgentProcess>> {
        if let Some(process) = actor.process() {
            return Ok(process);
        }
        let _lifecycle = actor.lifecycle.lock().await;
        if let Some(process) = actor.process() {
            return Ok(process);
        }
        self.start_locked(actor).await
    }

    /// Starts omp; the caller holds the lifecycle lock.
    async fn start_locked(&self, actor: &Arc<Actor>) -> Result<Arc<dyn AgentProcess>> {
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return Err(SupervisorError::ShuttingDown);
        }
        let max = self.inner.config.max_processes;
        self.inner
            .running
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |running| {
                (running < max).then_some(running + 1)
            })
            .map_err(|_| SupervisorError::LimitReached(max))?;
        let started = self.launch(actor).await;
        if started.is_err() {
            self.inner.running.fetch_sub(1, Ordering::SeqCst);
        }
        started
    }

    /// How to start the thread's omp (DMN-030), from a fresh scan of its workspace (DMN-023); with the roots its
    /// session had and the ones it gets now.
    async fn launch_spec(
        &self,
        actor: &Actor,
    ) -> Result<(Thread, LaunchSpec, Vec<PathBuf>, Vec<PathBuf>)> {
        let id = actor.id.clone();
        let threads = Arc::clone(&self.inner.threads);
        let workspaces = Arc::clone(&self.inner.workspaces);
        let (thread, ws, repos, recorded) = blocking(move || {
            let thread = threads.get(&id)?;
            let (view, _) = workspaces.rescan(&thread.workspace_id)?;
            let recorded = threads.roots(&id)?;
            Ok::<_, SupervisorError>((thread, view.workspace, view.repos, recorded))
        })
        .await??;
        let add_dirs = root_paths(&repos);
        let data = &self.inner.data;
        let spec = LaunchSpec {
            program: self.inner.config.omp_program.clone(),
            cwd: ws.path.clone(),
            add_dirs: add_dirs.clone(),
            session_dir: thread.session_dir.clone(),
            // Only a thread without a session of its own yet starts from a copy (DMN-301).
            fork: lock(&actor.fork_source)
                .take()
                .filter(|_| thread.omp_session_id.is_none()),
            model: thread.model.clone(),
            approval_mode: thread.approval_mode.clone(),
            append_system_prompt: Some(conventions::append_system_prompt(
                &ws.name,
                &ws.id,
                &ws.path,
                self.inner.config.agent_dir.as_deref(),
                &data.widgets(),
                &actor.id,
            )),
            extension: Some(data.omp_extension()),
            stderr_log: data.thread_log(&actor.id),
            env: self.inner.config.omp_env.clone(),
        };
        Ok((thread, spec, recorded, add_dirs))
    }

    async fn launch(&self, actor: &Arc<Actor>) -> Result<Arc<dyn AgentProcess>> {
        let (thread, spec, mut recorded, add_dirs) = self.launch_spec(actor).await?;
        // Taken before omp reads the files: a change while it starts reloads it once more (DMN-201). The new process
        // loads everything a due restart was for (DMN-203).
        lock(&actor.restart).clear();
        lock(&actor.oauth).clear();
        self.loading_mcp_configs(actor, spec.cwd.clone()).await;
        self.push(
            actor,
            ThreadEvent::Process {
                state: ProcessState::Starting,
            },
        );

        self.check_lease(actor, &thread, &spec).await?;

        let agent_tools = lock(&self.inner.agent_host)
            .clone()
            .map(|host| host.tools(&actor.id, &thread.workspace_id))
            .unwrap_or_default();
        let tools = host_tools::thread_tools(
            self.file_opener(&actor.id),
            self.notifier(&actor.id),
            self.title_setter(&actor.id),
            agent_tools,
        );
        let check_host = lock(&self.inner.widget_host)
            .clone()
            .map(|host| (thread.workspace_id.clone(), host));
        let uris =
            match host_tools::host_uris(self.inner.config.builtin_widgets.clone(), check_host) {
                Ok(uris) => uris,
                Err(error) => {
                    return Err(self
                        .failed_start(actor, &thread, error.to_string(), String::new())
                        .await);
                }
            };
        let launcher = Arc::clone(&self.inner.launcher);
        let launch_spec = spec.clone();
        let launched = match blocking(move || launcher.launch(&launch_spec, tools, uris)).await? {
            Ok(launched) => launched,
            Err(error) => {
                let path = spec.stderr_log.clone();
                let tail = blocking(move || stderr_tail(&path, STDERR_TAIL_LINES))
                    .await
                    .unwrap_or_default();
                return Err(self
                    .failed_start(actor, &thread, error.to_string(), tail)
                    .await);
            }
        };

        // A new process starts a new epoch: positions of the old one cannot be replayed (PRT-022).
        actor.log.new_epoch(new_id());
        // omp numbers prompts per process (PRT-030, DMN-152).
        actor.last_prompt.send_modify(|last| last.id = None);
        // omp's own history follows; it knows the session's model (`thread.state`).
        lock(&actor.session_model).take();
        for event in startup_events(&launched.startup) {
            self.push(actor, event);
        }
        let threads = Arc::clone(&self.inner.threads);
        let (id, session_id) = (
            actor.id.clone(),
            launched.startup.session.session_id.clone(),
        );
        if let Err(error) = blocking(move || threads.set_omp_session_id(&id, &session_id)).await? {
            warn!(thread = %actor.id, %error, "cannot store the omp session id");
        }

        let generation = self.inner.generations.fetch_add(1, Ordering::SeqCst);
        let (ended_tx, ended) = watch::channel(false);
        let process = launched.process;
        *lock(&actor.live) = Some(Live {
            process: Arc::clone(&process),
            generation,
            ended,
        });
        self.spawn_pump(Arc::clone(actor), launched.events, generation, ended_tx);
        info!(thread = %actor.id, "omp started");
        self.started_with(&spec);
        // The flags added today's roots; the session still has the ones it saved before (S-10).
        recorded.extend(add_dirs.iter().cloned());
        if let Err(error) = self
            .apply_roots(actor, Arc::clone(&process), add_dirs, &recorded)
            .await
        {
            warn!(thread = %actor.id, %error, "cannot remove stale roots");
        }
        Ok(process)
    }

    /// A stored session another omp process owns fails the start (DMN-033); its transcript stays readable (DMN-337).
    async fn check_lease(&self, actor: &Actor, thread: &Thread, spec: &LaunchSpec) -> Result<()> {
        let Some(session_id) = thread.omp_session_id.clone() else {
            return Ok(());
        };
        let launcher = Arc::clone(&self.inner.launcher);
        let probe = spec.clone();
        let lease = blocking(move || launcher.session_lease(&probe, &session_id)).await?;
        match lease {
            Ok(Lease::Held) => {
                let error = SupervisorError::SessionLeased {
                    session_id: thread.omp_session_id.clone().unwrap_or_default(),
                };
                self.history_from_file(actor, thread).await;
                self.fail(actor, error.to_string(), String::new());
                Err(error)
            }
            Ok(Lease::Free) => Ok(()),
            Ok(Lease::Unknown) => {
                warn!(thread = %actor.id, "omp could not probe the session lease; starting anyway");
                Ok(())
            }
            Err(error) => Err(self
                .failed_start(actor, thread, error.to_string(), String::new())
                .await),
        }
    }

    /// What the thread's `omfp_open_file` host tool calls (DMN-060): [`Self::open_file`] on this thread. It holds the
    /// supervisor weakly (the tools live as long as the process it owns).
    fn file_opener(&self, id: &str) -> host_tools::OpenFile {
        let inner = Arc::downgrade(&self.inner);
        let id = id.to_owned();
        Arc::new(move |path: &str, line: Option<u64>| {
            let inner = Weak::upgrade(&inner).ok_or("the daemon is shutting down")?;
            Self { inner }.open_file(&id, path, line)
        })
    }

    /// What the thread's `omfp_notify` host tool calls (DMN-125): [`Self::notify`] on this thread, holding the
    /// supervisor weakly like [`Self::file_opener`].
    fn notifier(&self, id: &str) -> host_tools::Notify {
        let inner = Arc::downgrade(&self.inner);
        let id = id.to_owned();
        Arc::new(move |title: &str, body: &str| {
            let inner = Weak::upgrade(&inner).ok_or("the daemon is shutting down")?;
            Self { inner }.notify(&id, title, body)
        })
    }

    /// What the thread's `set_thread_title` host tool calls (DMN-060): stores a set title, refreshes the index and
    /// names the omp session. It runs on an `omp-rpc` handler thread, so the store call blocks there and the session
    /// rename goes to the runtime; it holds the supervisor weakly (the tools live as long as the process it owns).
    fn title_setter(&self, id: &str) -> host_tools::SetTitle {
        let inner = Arc::downgrade(&self.inner);
        let runtime = tokio::runtime::Handle::current();
        let id = id.to_owned();
        Arc::new(move |title: &str| {
            let inner = Weak::upgrade(&inner).ok_or("the daemon is shutting down")?;
            inner
                .threads
                .rename(&id, title, TitleSource::Set)
                .map_err(|error| error.to_string())?;
            let _ = inner.changes.send(id.clone());
            let supervisor = Self { inner };
            let id = id.clone();
            runtime.spawn(async move { supervisor.name_session(&id).await });
            Ok(())
        })
    }

    /// Fails the thread whose omp could not start and returns the error carrying omp's stderr tail (DMN-212); omp's
    /// refusal without a usable model may contradict the known count of usable models (DMN-237). The transcript stays
    /// readable from the session file (DMN-337).
    async fn failed_start(
        &self,
        actor: &Actor,
        thread: &Thread,
        message: String,
        stderr_tail: String,
    ) -> SupervisorError {
        self.failed_with(&stderr_tail);
        self.history_from_file(actor, thread).await;
        self.fail(actor, message.clone(), stderr_tail.clone());
        SupervisorError::Start {
            message,
            stderr_tail,
        }
    }

    /// A thread whose omp could not start shows its stored session's current branch, read from the session file
    /// without omp, as the `history` event omp's start would have given (DMN-337), and knows the model that session
    /// restores (DMN-338). Without a session file nothing changes; an unreadable one is only logged.
    async fn history_from_file(&self, actor: &Actor, thread: &Thread) {
        let dir = thread.session_dir.clone();
        let session_id = thread.omp_session_id.clone();
        let read = blocking(move || crate::session_file::read(&dir, session_id.as_deref()))
            .await
            .and_then(|read| read.map_err(|error| SupervisorError::Internal(error.to_string())));
        match read {
            Ok(Some(read)) => {
                *lock(&actor.session_model) = read.model;
                self.push(
                    actor,
                    ThreadEvent::History {
                        session: read.session,
                        messages: read.messages,
                    },
                );
            }
            Ok(None) => {}
            Err(error) => warn!(thread = %actor.id, %error, "cannot read the session file"),
        }
    }

    fn fail(&self, actor: &Actor, message: String, stderr_tail: String) {
        self.push(
            actor,
            ThreadEvent::Process {
                state: ProcessState::Failed {
                    message,
                    exit_code: None,
                    stderr_tail,
                },
            },
        );
    }

    /// Forwards omp's frames into the log: a thread reads the blocking receiver into a channel, a task pushes them
    /// (DMN-040). When the stream ends without a requested stop, the thread fails (DMN-035).
    fn spawn_pump(
        &self,
        actor: Arc<Actor>,
        events: std::sync::mpsc::Receiver<omp_rpc::Event>,
        generation: u64,
        ended: watch::Sender<bool>,
    ) {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            for event in events {
                if sender.send(thread_event(event)).is_err() {
                    break;
                }
            }
        });
        let supervisor = self.clone();
        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                supervisor.push(&actor, event);
            }
            // A live session ends with its process (DMN-103).
            lock(&actor.voice).take();
            supervisor.process_ended(&actor, generation).await;
            let _ = ended.send(true);
        });
    }

    async fn process_ended(&self, actor: &Actor, generation: u64) {
        let crashed = {
            let mut live = lock(&actor.live);
            if live
                .as_ref()
                .is_some_and(|live| live.generation == generation)
            {
                live.take()
            } else {
                None
            }
        };
        let Some(crashed) = crashed else {
            return;
        };
        self.inner.running.fetch_sub(1, Ordering::SeqCst);
        // Its OAuth listeners died with it: another process may take their ports (DMN-340).
        lock(&actor.oauth).clear();
        let path = self.inner.data.thread_log(&actor.id);
        let process = crashed.process;
        let tail = blocking(move || {
            drop(process);
            stderr_tail(&path, STDERR_TAIL_LINES)
        })
        .await
        .unwrap_or_default();
        warn!(thread = %actor.id, "omp exited unexpectedly");
        self.fail(actor, "omp exited unexpectedly".into(), tail);
    }

    /// Stops the process; the caller holds the lifecycle lock.
    async fn stop_locked(&self, actor: &Actor) -> Result<()> {
        let Some(Live {
            process, mut ended, ..
        }) = lock(&actor.live).take()
        else {
            return Ok(());
        };
        blocking(move || drop(process)).await?;
        let _ = ended.wait_for(|ended| *ended).await;
        self.inner.running.fetch_sub(1, Ordering::SeqCst);
        // The next process loads everything anew (DMN-203).
        lock(&actor.mcp_configs).take();
        lock(&actor.restart).clear();
        lock(&actor.oauth).clear();
        self.push(
            actor,
            ThreadEvent::Process {
                state: ProcessState::Stopped,
            },
        );
        info!(thread = %actor.id, "omp stopped");
        Ok(())
    }
}

/// The `--add-dir` roots for a workspace's repositories: every one but the folder itself, by its path inside the
/// workspace (DMN-029).
fn root_paths(repos: &[crate::workspaces::Discovered]) -> Vec<PathBuf> {
    repos
        .iter()
        .filter(|repo| !repo.is_root)
        .map(|repo| repo.path.clone())
        .collect()
}

/// The last `lines` lines of omp's stderr log (at most its last 16 KiB).
fn stderr_tail(path: &Path, lines: usize) -> String {
    const MAX_BYTES: u64 = 16 * 1024;
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let length = file.metadata().map_or(0, |metadata| metadata.len());
    if file
        .seek(SeekFrom::Start(length.saturating_sub(MAX_BYTES)))
        .is_err()
    {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests;
