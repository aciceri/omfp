//! Visible state of one thread, produced by the reducer (ARC-006) and sent to late-joining clients as a snapshot
//! (PRT-021). Every field is plain data so the state survives a serde round trip unchanged.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::event::{LiveOwner, NotifyRequest, OpenFileRequest, ProcessState, SessionInfo};
use crate::wire::{
    AgentMessage, AssistantMessage, AutoCompactionEndEvent, AutoCompactionStartEvent,
    AutoRetryEndEvent, AutoRetryStartEvent, AvailableSlashCommand, CommandOutputEvent,
    ExtensionError, ExtensionUiRequest, Goal, LivePhase, LiveRole, ManualCompactionEndEvent,
    ManualCompactionStartEvent, NoticeEvent, NotifyUiRequest, OpenUrlUiRequest, PromptResultEvent,
    RpcFrameErrorEvent, SubagentLifecyclePayload, SubagentProgressPayload, TodoPhase,
    WidgetPlacement,
};

/// Everything a client shows about a thread. Built only by [`ThreadState::apply`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadState {
    /// Lifecycle of the omp process (last `Process` event).
    pub process: ProcessState,
    /// Session bound by the last `History` event.
    pub session: Option<SessionInfo>,
    /// `false` from `agent_start` until `session_settled`, or until the process leaves `Running`/`Starting`.
    pub settled: bool,
    /// Finished messages in order: the last `History`, then every `message_end` (DMN-043).
    pub transcript: Vec<TranscriptEntry>,
    /// Assistant message being rebuilt from `message_update` deltas; cleared by its `message_end`.
    pub streaming: Option<StreamingMessage>,
    /// Tool runs in order of first appearance.
    pub tools: Vec<ToolRun>,
    /// Queue chips: always the last `queue_update` (TST-011c).
    pub queue: QueueState,
    /// Pending dialog requests (`select`, `confirm`, `input`, `editor`, `ask`) in arrival order (DMN-050..052).
    pub interactions: Vec<ExtensionUiRequest>,
    /// State set by presentation-only requests (DMN-053).
    pub presentation: Presentation,
    /// Slash commands from the last `available_commands_update`.
    pub commands: Vec<AvailableSlashCommand>,
    /// Outcome of each prompt, keyed by the prompt's request id (`prompt_result`).
    pub prompts: BTreeMap<String, PromptResultEvent>,
    /// Latest lifecycle and progress per subagent id (PRT-055).
    pub subagents: BTreeMap<String, Subagent>,
    /// The session's goal from the last `goal_updated` (PRT-056); `None` when omp reported none (or never did).
    pub goal: Option<Goal>,
    /// The todo list: the phases of the transcript's last `todo` result that committed some (PRT-057).
    pub todo_phases: Vec<TodoPhase>,
    /// Noteworthy frames outside the transcript, shown as inline rows (UI-043), among them the output of local slash
    /// commands and the URLs omp asks to open (PRT-120, UI-345).
    pub notices: Vec<Notice>,
    /// The live voice session (PRT-052): opened by a `live_session` event, closed by `live_end`, a `live_session`
    /// without owner, or the end of the process.
    pub live: Option<LiveSession>,
    /// Readable message of the failure the last live session ended on (DMN-104); cleared when the next one opens.
    pub live_error: Option<String>,
    /// The last file the agent asked a window to open (`open_file` event, DMN-060); the window it names opens each
    /// request once.
    pub open_file: Option<OpenFileRequest>,
    /// The last notification the agent asked a window to show (`notify` event, DMN-125, PRT-065); the window it names
    /// shows each request once.
    pub notify: Option<NotifyRequest>,
}

impl Default for ThreadState {
    fn default() -> Self {
        Self {
            process: ProcessState::default(),
            session: None,
            settled: true,
            transcript: Vec::new(),
            streaming: None,
            tools: Vec::new(),
            queue: QueueState::default(),
            interactions: Vec::new(),
            presentation: Presentation::default(),
            commands: Vec::new(),
            prompts: BTreeMap::new(),
            subagents: BTreeMap::new(),
            goal: None,
            todo_phases: Vec::new(),
            notices: Vec::new(),
            live: None,
            live_error: None,
            open_file: None,
            notify: None,
        }
    }
}

/// A live voice session (PRT-052).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveSession {
    /// The connection that started it.
    pub owner: LiveOwner,
    /// The last `live_phase`; `None` until omp reports one.
    pub phase: Option<LivePhase>,
    /// One line per role and turn, in order of first appearance.
    pub transcript: Vec<LiveLine>,
    /// The transcript's length when the session started: the voice's context holds the messages before it (DMN-105),
    /// so a prompt typed from there on is unknown to it.
    pub context_end: usize,
}

/// The text of one realtime turn of one speaker (`live_transcript`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveLine {
    pub role: LiveRole,
    pub turn: i64,
    pub text: String,
    /// omp sent the turn's last frame; later frames of the turn are ignored.
    pub r#final: bool,
}

/// One finished transcript message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// omp's `messageId`; `None` for history messages and frames without one.
    pub message_id: Option<String>,
    pub message: AgentMessage,
}

/// The assistant message currently streaming.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamingMessage {
    pub message_id: Option<String>,
    /// Content rebuilt so far. A tool-call block's arguments are complete only after its `toolcall_end`; until then
    /// the block's `partialArgs` key holds the argument JSON received so far.
    pub message: AssistantMessage,
}

/// One tool execution, keyed by omp's `toolCallId`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolRun {
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Option<Value>,
    pub intent: Option<String>,
    pub status: ToolStatus,
    /// Latest `tool_execution_update` partial result.
    pub partial_result: Option<Value>,
    /// Latest `tool_stream_update` projection of the in-flight call (e.g. a diff preview).
    pub preview: Option<Value>,
    /// Final result from `tool_execution_end`.
    pub result: Option<Value>,
    /// The background job the call became (PRT-100): omp's `details.async` frames, kept across
    /// `tool_execution_end` and the `tool_execution_update`s that follow it.
    pub background: Option<BackgroundJob>,
}

/// A tool call running on as an omp background job (`bash` with `async`, or auto-backgrounded), PRT-100.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundJob {
    pub job_id: String,
    pub state: JobState,
    /// Text of the job's latest frame: its output so far, then its final output.
    pub output: Option<String>,
    /// `details.exitCode` of the job's latest frame, once it reports one.
    pub exit_code: Option<i64>,
}

/// `details.async.state` of an omp job frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Progress of a [`ToolRun`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// Announced by a `tool_stream_update` before `tool_execution_start`.
    Pending,
    Running,
    Succeeded,
    Failed,
}

/// Queue chips, as sent by omp's `queue_update`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueState {
    pub steering: Vec<String>,
    pub follow_up: Vec<String>,
    pub live_steered: i64,
}

/// State set by presentation-only extension UI requests (DMN-053).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Presentation {
    /// `setStatus` texts by status key; a request without text removes the key.
    pub statuses: BTreeMap<String, String>,
    /// `setWidget` lines by widget key; a request without lines removes the key.
    pub widgets: BTreeMap<String, Widget>,
    /// Last `setTitle`.
    pub title: Option<String>,
    /// Last `set_editor_text`.
    pub editor_text: Option<String>,
    /// Last `open_url`.
    pub open_url: Option<OpenUrlUiRequest>,
    /// Every `notify`, in order.
    pub notifications: Vec<NotifyUiRequest>,
}

/// Lines of one `setWidget` widget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Widget {
    pub lines: Vec<String>,
    pub placement: Option<WidgetPlacement>,
}

/// What omp last reported about one subagent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Subagent {
    pub lifecycle: Option<SubagentLifecyclePayload>,
    pub progress: Option<SubagentProgressPayload>,
}

/// A noteworthy frame outside the transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    /// Number of transcript entries when the frame arrived: the row goes after that many messages.
    pub position: usize,
    pub frame: NoticeFrame,
}

/// The frames kept as [`Notice`]s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum NoticeFrame {
    AutoCompactionStart(AutoCompactionStartEvent),
    AutoCompactionEnd(AutoCompactionEndEvent),
    ManualCompactionStart(ManualCompactionStartEvent),
    ManualCompactionEnd(ManualCompactionEndEvent),
    AutoRetryStart(AutoRetryStartEvent),
    AutoRetryEnd(AutoRetryEndEvent),
    Notice(NoticeEvent),
    ExtensionError(ExtensionError),
    /// Output of a local slash command (`/dirs`, `/mcp reauth …`, background ones arriving later), PRT-120.
    CommandOutput(CommandOutputEvent),
    /// An `open_url` request (`login`): omp asks the client to open a URL, PRT-120.
    OpenUrl(OpenUrlUiRequest),
    RpcFrameError(RpcFrameErrorEvent),
    /// A frame the reducer cannot interpret (DMN-042), as received.
    Unknown(Value),
}
