/**
 * Visible state of one thread (port of `omfp-core/src/state.rs`), produced by the reducer (ARC-006) and sent to
 * late-joining clients as a snapshot (PRT-021). It is the JSON the Rust `ThreadState` serializes to: snake_case
 * fields, `null` for empty `Option`s, maps as plain objects.
 */

import type { LiveOwner, NotifyRequest, OpenFileRequest, ProcessState, SessionInfo } from "./event.ts";
import type { JsonValue } from "./json.ts";
import type {
  AgentMessage,
  AssistantMessage,
  AutoCompactionEndEvent,
  AutoCompactionStartEvent,
  AutoRetryEndEvent,
  AutoRetryStartEvent,
  AvailableSlashCommand,
  CommandOutputEvent,
  ExtensionError,
  ExtensionUiRequest,
  Goal,
  LivePhase,
  LiveRole,
  ManualCompactionEndEvent,
  ManualCompactionStartEvent,
  NoticeEvent,
  NotifyUiRequest,
  OpenUrlUiRequest,
  PromptResultEvent,
  RpcFrameErrorEvent,
  SubagentLifecyclePayload,
  SubagentProgressPayload,
  TodoPhase,
  WidgetPlacement,
} from "./wire.ts";

/** Everything a client shows about a thread. Built only by `reduce`. */
export interface ThreadState {
  /** Lifecycle of the omp process (last `process` event). */
  process: ProcessState;
  /** Session bound by the last `history` event. */
  session: SessionInfo | null;
  /** `false` from `agent_start` until `session_settled`, or until the process leaves `running`/`starting`. */
  settled: boolean;
  /** Finished messages in order: the last `history`, then every `message_end` (DMN-043). */
  transcript: TranscriptEntry[];
  /** Assistant message being rebuilt from `message_update` deltas; cleared by its `message_end`. */
  streaming: StreamingMessage | null;
  /** Tool runs in order of first appearance. */
  tools: ToolRun[];
  /** Queue chips: always the last `queue_update` (TST-011c). */
  queue: QueueState;
  /** Pending dialog requests (`select`, `confirm`, `input`, `editor`, `ask`) in arrival order (DMN-050..052). */
  interactions: ExtensionUiRequest[];
  /** State set by presentation-only requests (DMN-053). */
  presentation: Presentation;
  /** Slash commands from the last `available_commands_update`. */
  commands: AvailableSlashCommand[];
  /** Outcome of each prompt, keyed by the prompt's request id (`prompt_result`). */
  prompts: Record<string, PromptResultEvent>;
  /** Latest lifecycle and progress per subagent id (PRT-055). */
  subagents: Record<string, Subagent>;
  /** The session's goal from the last `goal_updated` (PRT-056); `null` when omp reported none (or never did). */
  goal: Goal | null;
  /** The todo list: the phases of the transcript's last `todo` result that committed some (PRT-057). */
  todo_phases: TodoPhase[];
  /** Noteworthy frames outside the transcript, shown as inline rows (UI-043). */
  notices: Notice[];
  /**
   * The live voice session (PRT-052): opened by a `live_session` event, closed by `live_end`, a `live_session` without
   * owner, or the end of the process.
   */
  live: LiveSession | null;
  /** Readable message of the failure the last live session ended on (DMN-104); cleared when the next one opens. */
  live_error: string | null;
  /** The last file the agent asked a window to open (`open_file` event, DMN-060); the window it names opens each
   * request once. */
  open_file: OpenFileRequest | null;
  /** The last notification the agent asked a window to show (`notify` event, DMN-125, PRT-065); the window it names
   * shows each request once. */
  notify: NotifyRequest | null;
}

/** A live voice session (PRT-052). */
export interface LiveSession {
  /** The connection that started it. */
  owner: LiveOwner;
  /** The last `live_phase`; `null` until omp reports one. */
  phase: LivePhase | null;
  /** One line per role and turn, in order of first appearance. */
  transcript: LiveLine[];
  /**
   * The transcript's length when the session started: the voice's context holds the messages before it (DMN-105),
   * so a prompt typed from there on is unknown to it.
   */
  context_end: number;
}

/** The text of one realtime turn of one speaker (`live_transcript`). */
export interface LiveLine {
  role: LiveRole;
  turn: number;
  text: string;
  /** omp sent the turn's last frame; later frames of the turn are ignored. */
  final: boolean;
}

/** One finished transcript message. */
export interface TranscriptEntry {
  /** omp's `messageId`; `null` for history messages and frames without one. */
  message_id: string | null;
  message: AgentMessage;
}

/** The assistant message currently streaming. */
export interface StreamingMessage {
  message_id: string | null;
  /**
   * Content rebuilt so far. A tool-call block's arguments are complete only after its `toolcall_end`; until then the
   * block's `partialArgs` key holds the argument JSON received so far.
   */
  message: AssistantMessage;
}

/** Progress of a {@link ToolRun}; `pending` is announced by a `tool_stream_update` before `tool_execution_start`. */
export type ToolStatus = "pending" | "running" | "succeeded" | "failed";

/** One tool execution, keyed by omp's `toolCallId`. */
export interface ToolRun {
  tool_call_id: string;
  tool_name: string;
  args: JsonValue | null;
  intent: string | null;
  status: ToolStatus;
  /** Latest `tool_execution_update` partial result. */
  partial_result: JsonValue | null;
  /** Latest `tool_stream_update` projection of the in-flight call (e.g. a diff preview). */
  preview: JsonValue | null;
  /** Final result from `tool_execution_end`. */
  result: JsonValue | null;
  /** The background job the call became (PRT-100): omp's `details.async` frames, kept across `tool_execution_end`
   * and the `tool_execution_update`s that follow it. */
  background: BackgroundJob | null;
}

/** `details.async.state` of an omp job frame. */
export type JobState = "running" | "completed" | "failed" | "cancelled";

/** A tool call running on as an omp background job (`bash` with `async`, or auto-backgrounded), PRT-100. */
export interface BackgroundJob {
  job_id: string;
  state: JobState;
  /** Text of the job's latest frame: its output so far, then its final output. */
  output: string | null;
  /** `details.exitCode` of the job's latest frame, once it reports one. */
  exit_code: number | null;
}

/** Queue chips, as sent by omp's `queue_update`. */
export interface QueueState {
  steering: string[];
  follow_up: string[];
  live_steered: number;
}

/** State set by presentation-only extension UI requests (DMN-053). */
export interface Presentation {
  /** `setStatus` texts by status key; a request without text removes the key. */
  statuses: Record<string, string>;
  /** `setWidget` lines by widget key; a request without lines removes the key. */
  widgets: Record<string, Widget>;
  /** Last `setTitle`. */
  title: string | null;
  /** Last `set_editor_text`. */
  editor_text: string | null;
  /** Last `open_url`. */
  open_url: OpenUrlUiRequest | null;
  /** Every `notify`, in order. */
  notifications: NotifyUiRequest[];
}

/** Lines of one `setWidget` widget. */
export interface Widget {
  lines: string[];
  placement: WidgetPlacement | null;
}

/** What omp last reported about one subagent. */
export interface Subagent {
  lifecycle: SubagentLifecyclePayload | null;
  progress: SubagentProgressPayload | null;
}

/** A noteworthy frame outside the transcript. */
export interface Notice {
  /** Number of transcript entries when the frame arrived: the row goes after that many messages. */
  position: number;
  frame: NoticeFrame;
}

/** The frames kept as {@link Notice}s; `unknown` holds a frame the reducer cannot interpret (DMN-042), as received. */
export type NoticeFrame =
  | { type: "auto_compaction_start"; data: AutoCompactionStartEvent }
  | { type: "auto_compaction_end"; data: AutoCompactionEndEvent }
  | { type: "manual_compaction_start"; data: ManualCompactionStartEvent }
  | { type: "manual_compaction_end"; data: ManualCompactionEndEvent }
  | { type: "auto_retry_start"; data: AutoRetryStartEvent }
  | { type: "auto_retry_end"; data: AutoRetryEndEvent }
  | { type: "notice"; data: NoticeEvent }
  | { type: "extension_error"; data: ExtensionError }
  /** Output of a local slash command (`/dirs`, `/mcp reauth …`, background ones arriving later), PRT-120. */
  | { type: "command_output"; data: CommandOutputEvent }
  /** An `open_url` request (`login`): omp asks the client to open a URL, PRT-120. */
  | { type: "open_url"; data: OpenUrlUiRequest }
  | { type: "rpc_frame_error"; data: RpcFrameErrorEvent }
  | { type: "unknown"; data: JsonValue };

/** A new thread: stopped, settled and empty. */
export function emptyThreadState(): ThreadState {
  return {
    process: { state: "stopped" },
    session: null,
    settled: true,
    transcript: [],
    streaming: null,
    tools: [],
    queue: { steering: [], follow_up: [], live_steered: 0 },
    interactions: [],
    presentation: {
      statuses: {},
      widgets: {},
      title: null,
      editor_text: null,
      open_url: null,
      notifications: [],
    },
    commands: [],
    prompts: {},
    subagents: {},
    goal: null,
    todo_phases: [],
    notices: [],
    live: null,
    live_error: null,
    open_file: null,
    notify: null,
  };
}
