/**
 * The subset of omp's RPC wire types (omp-rpc `wire.rs`, omp 19.1.0) the reducer reads or keeps, in their JSON form.
 *
 * omp-rpc has two kinds of records. Closed structs (events, UI requests) decode their declared fields and drop every
 * other key: the reducer keeps them through the decoders below, so they serialize like Rust. Open records (messages,
 * content blocks) keep unknown keys: the reducer keeps them as received, so their types end in an index signature.
 */

import {
  arrayOf,
  bool,
  type Decoder,
  fail,
  int,
  json,
  num,
  object,
  oneOf,
  optional,
  record,
  str,
  withDefault,
} from "./decode.ts";
import { isObject, type JsonObject, type JsonValue } from "./json.ts";

// ---------------------------------------------------------------------------------------------------------------------
// Messages and content blocks (open records)

export type StopReason = "stop" | "length" | "toolUse" | "error" | "aborted";

export interface TextContent {
  type: "text";
  text?: string;
  textSignature?: string;
  [key: string]: unknown;
}

export interface ThinkingContent {
  type: "thinking";
  thinking?: string;
  thinkingSignature?: string;
  itemId?: string;
  narration?: boolean;
  [key: string]: unknown;
}

export interface RedactedThinkingContent {
  type: "redactedThinking";
  data?: string;
  [key: string]: unknown;
}

export interface FallbackContent {
  type: "fallback";
  [key: string]: unknown;
}

export interface AnthropicServerToolContent {
  type: "anthropicServerTool";
  [key: string]: unknown;
}

export interface ImageContent {
  type: "image";
  data?: string;
  mimeType?: string;
  url?: string;
  [key: string]: unknown;
}

/** A tool-call block. While it streams, `partialArgs` holds the argument JSON received so far. */
export interface ToolCall {
  type: "toolCall";
  id?: string;
  name?: string;
  arguments?: JsonObject;
  intent?: string;
  partialArgs?: string;
  [key: string]: unknown;
}

export type AssistantContent =
  | TextContent
  | ThinkingContent
  | RedactedThinkingContent
  | FallbackContent
  | AnthropicServerToolContent
  | ImageContent
  | ToolCall;

export type UserContent = TextContent | ImageContent;

export interface AssistantMessage {
  role?: "assistant";
  content?: AssistantContent[];
  api?: string;
  provider?: string;
  model?: string;
  usage?: JsonObject;
  stopReason?: StopReason;
  timestamp?: number;
  responseId?: string;
  errorMessage?: string;
  [key: string]: unknown;
}

export interface UserMessage {
  role: "user";
  content?: string | UserContent[];
  timestamp?: number;
  [key: string]: unknown;
}

export interface ToolResultMessage {
  role: "toolResult";
  toolCallId?: string;
  toolName?: string;
  content?: UserContent[];
  isError?: boolean;
  timestamp?: number;
  [key: string]: unknown;
}

const OTHER_MESSAGE_ROLES = [
  "developer",
  "bashExecution",
  "pythonExecution",
  "custom",
  "hookMessage",
  "branchSummary",
  "compactionSummary",
  "fileMention",
] as const;

/** Roles whose records the reducer only stores. */
export type OtherMessageRole = (typeof OTHER_MESSAGE_ROLES)[number];

export interface OtherMessage {
  role: OtherMessageRole;
  [key: string]: unknown;
}

/** A transcript message, discriminated by `role`. */
export type AgentMessage = UserMessage | (AssistantMessage & { role: "assistant" }) | ToolResultMessage | OtherMessage;

const AGENT_MESSAGE_ROLES: readonly string[] = ["user", "assistant", "toolResult", ...OTHER_MESSAGE_ROLES];

const ASSISTANT_CONTENT_TYPES: readonly string[] = [
  "text",
  "thinking",
  "redactedThinking",
  "fallback",
  "anthropicServerTool",
  "image",
  "toolCall",
];

/** `value` as an open record when its `key` is one of `tags` (its other keys decode leniently), else `undefined`. */
function openRecord<T>(value: unknown, key: string, tags: readonly string[]): T | undefined {
  const tag = isObject(value) ? value[key] : undefined;
  return typeof tag === "string" && tags.includes(tag) ? (value as T) : undefined;
}

/** An `AgentMessage`: any object whose `role` is known. */
export const agentMessage: Decoder<AgentMessage> = (value) =>
  openRecord<AgentMessage>(value, "role", AGENT_MESSAGE_ROLES) ?? fail();

/** `value` as a content block when its `type` is known, else `undefined`. */
export function assistantContent(value: unknown): AssistantContent | undefined {
  return openRecord(value, "type", ASSISTANT_CONTENT_TYPES);
}

/** The `content` of an assistant message when it decodes (every block known), else `undefined`. */
export function assistantContentList(value: unknown): AssistantContent[] | undefined {
  return Array.isArray(value) && value.every((item) => assistantContent(item) !== undefined) ? value : undefined;
}

// ---------------------------------------------------------------------------------------------------------------------
// Streaming updates of one assistant message (`assistantMessageEvent`, open records)

const ASSISTANT_MESSAGE_EVENT_TYPES = [
  "start",
  "text_start",
  "text_delta",
  "text_end",
  "thinking_start",
  "thinking_delta",
  "thinking_end",
  "image_end",
  "toolcall_start",
  "toolcall_delta",
  "toolcall_end",
  "done",
  "error",
] as const;

export type AssistantMessageEventType = (typeof ASSISTANT_MESSAGE_EVENT_TYPES)[number];

export interface AssistantMessageEvent {
  type: AssistantMessageEventType;
  contentIndex?: number;
  delta?: string;
  [key: string]: unknown;
}

export const assistantMessageEvent: Decoder<AssistantMessageEvent> = (value) =>
  openRecord<AssistantMessageEvent>(value, "type", ASSISTANT_MESSAGE_EVENT_TYPES) ?? fail();

// ---------------------------------------------------------------------------------------------------------------------
// Closed structs the reducer keeps

export type PromptStatus = "completed" | "aborted" | "error";

export interface PromptError {
  message: string;
  retryable: boolean;
  provider?: string;
  model?: string;
  httpStatus?: number;
  code?: "cost_cap";
}

/** Terminal outcome of one accepted `prompt`, keyed by request `id`. */
export interface PromptResultEvent {
  agentInvoked: boolean;
  status: PromptStatus;
  sessionSettled: boolean;
  id?: string;
  error?: PromptError;
}

const promptError = record<PromptError>({
  message: str,
  retryable: bool,
  provider: optional(str),
  model: optional(str),
  httpStatus: optional(int),
  code: optional(oneOf(["cost_cap"])),
});

export const promptResultEvent = record<PromptResultEvent>({
  agentInvoked: bool,
  status: oneOf(["completed", "aborted", "error"]),
  sessionSettled: bool,
  id: optional(str),
  error: optional(promptError),
});

export type SlashCommandSource = "builtin" | "skill" | "extension" | "custom" | "mcp_prompt" | "file";

export interface SlashSubcommand {
  name: string;
  description?: string;
  usage?: string;
}

export interface AvailableSlashCommand {
  name: string;
  source: SlashCommandSource;
  aliases?: string[];
  description?: string;
  input?: { hint?: string };
  subcommands?: SlashSubcommand[];
}

export const availableSlashCommand = record<AvailableSlashCommand>({
  name: str,
  source: oneOf(["builtin", "skill", "extension", "custom", "mcp_prompt", "file"]),
  aliases: optional(arrayOf(str)),
  description: optional(str),
  input: optional(record<{ hint?: string }>({ hint: optional(str) })),
  subcommands: optional(
    arrayOf(record<SlashSubcommand>({ name: str, description: optional(str), usage: optional(str) })),
  ),
});

export type AgentSource = "bundled" | "user" | "project";
export type SubagentLifecycleStatus = "started" | "completed" | "failed" | "aborted";

export interface SubagentLifecyclePayload {
  id: string;
  agent: string;
  agentSource: AgentSource;
  status: SubagentLifecycleStatus;
  index: number;
  description?: string;
  sessionFile?: string;
  parentToolCallId?: string;
  detached?: boolean;
}

export interface SubagentProgressPayload {
  index: number;
  agent: string;
  agentSource: AgentSource;
  task: string;
  /** Raw `AgentProgress` record; its `id` is the subagent id. */
  progress: JsonObject;
  parentToolCallId?: string;
  assignment?: string;
  sessionFile?: string;
  detached?: boolean;
}

const agentSource = oneOf<AgentSource>(["bundled", "user", "project"]);

export const subagentLifecyclePayload = record<SubagentLifecyclePayload>({
  id: str,
  agent: str,
  agentSource,
  status: oneOf(["started", "completed", "failed", "aborted"]),
  index: int,
  description: optional(str),
  sessionFile: optional(str),
  parentToolCallId: optional(str),
  detached: optional(bool),
});

export const subagentProgressPayload = record<SubagentProgressPayload>({
  index: int,
  agent: str,
  agentSource,
  task: str,
  progress: object,
  parentToolCallId: optional(str),
  assignment: optional(str),
  sessionFile: optional(str),
  detached: optional(bool),
});

export type GoalStatus = "active" | "paused" | "budget-limited" | "complete" | "dropped";

/** A tracked goal: its objective, lifecycle status, and resource accounting. */
export interface Goal {
  id: string;
  objective: string;
  status: GoalStatus;
  tokensUsed: number;
  timeUsedSeconds: number;
  createdAt: number;
  updatedAt: number;
  tokenBudget?: number;
}

export const goal = record<Goal>({
  id: str,
  objective: str,
  status: oneOf(["active", "paused", "budget-limited", "complete", "dropped"]),
  tokensUsed: int,
  timeUsedSeconds: num,
  createdAt: int,
  updatedAt: int,
  tokenBudget: optional(int),
});

/** Session goal mode; `mode` is `"exiting"` while a completed goal unwinds. */
export interface GoalModeState {
  enabled: boolean;
  mode: "active" | "exiting";
  goal: Goal;
  reason?: "completed";
}

export interface GoalUpdatedEvent {
  goal: Goal | null;
  state?: GoalModeState;
}

export const goalUpdatedEvent = record<GoalUpdatedEvent>({
  // Required, `null` when the session has no goal.
  goal: (value) => (value === null ? null : goal(value)),
  state: optional(
    record<GoalModeState>({
      enabled: bool,
      mode: oneOf(["active", "exiting"]),
      goal,
      reason: optional(oneOf(["completed"])),
    }),
  ),
});

export type TodoStatus = "pending" | "in_progress" | "completed" | "abandoned";

export interface TodoItem {
  content: string;
  status: TodoStatus;
  details?: string;
  notes?: string[];
}

export interface TodoPhase {
  name: string;
  tasks: TodoItem[];
}

export const todoPhase = record<TodoPhase>({
  name: str,
  tasks: arrayOf(
    record<TodoItem>({
      content: str,
      status: oneOf(["pending", "in_progress", "completed", "abandoned"]),
      details: optional(str),
      notes: optional(arrayOf(str)),
    }),
  ),
});

// ---------------------------------------------------------------------------------------------------------------------
// Frames kept as notices

export type AutoCompactionReason = "threshold" | "overflow" | "idle" | "incomplete";
export type AutoCompactionAction = "context-full" | "remote" | "handoff" | "shake" | "snapcompact";
export type NotifyType = "info" | "warning" | "error";

export interface CompactionResult {
  summary: string;
  firstKeptEntryId: string;
  tokensBefore: number;
  shortSummary?: string;
  details?: JsonValue;
  preserveData?: JsonObject;
  providerReplayThroughEntryId?: string;
}

export interface AutoCompactionStartEvent {
  reason: AutoCompactionReason;
  action: AutoCompactionAction;
}

export interface AutoCompactionEndEvent {
  action: AutoCompactionAction;
  aborted: boolean;
  willRetry: boolean;
  result?: CompactionResult;
  errorMessage?: string;
  skipped?: boolean;
}

export interface ManualCompactionStartEvent {
  mode?: string;
}

export interface ManualCompactionEndEvent {
  aborted: boolean;
  result?: CompactionResult;
  skipped?: boolean;
  errorMessage?: string;
}

export interface AutoRetryStartEvent {
  attempt: number;
  maxAttempts: number;
  delayMs: number;
  errorMessage: string;
  errorId?: number;
}

export interface AutoRetryEndEvent {
  success: boolean;
  attempt: number;
  finalError?: string;
  retryErrors: JsonObject[];
}

export interface NoticeEvent {
  level: NotifyType;
  message: string;
  source?: string;
}

export interface ExtensionError {
  extensionPath: string;
  event: string;
  error: string;
}

/** An event could not fit within the transport limits and was dropped. */
export interface RpcFrameErrorEvent {
  error: string;
  originalType?: string;
}

/** Output of a builtin slash command. */
export interface CommandOutputEvent {
  text: string;
}

const compactionAction = oneOf<AutoCompactionAction>(["context-full", "remote", "handoff", "shake", "snapcompact"]);

const compactionResult = record<CompactionResult>({
  summary: str,
  firstKeptEntryId: str,
  tokensBefore: int,
  shortSummary: optional(str),
  details: optional(json),
  preserveData: optional(object),
  providerReplayThroughEntryId: optional(str),
});

export const autoCompactionStartEvent = record<AutoCompactionStartEvent>({
  reason: oneOf(["threshold", "overflow", "idle", "incomplete"]),
  action: compactionAction,
});

export const autoCompactionEndEvent = record<AutoCompactionEndEvent>({
  action: compactionAction,
  aborted: bool,
  willRetry: bool,
  result: optional(compactionResult),
  errorMessage: optional(str),
  skipped: optional(bool),
});

export const manualCompactionStartEvent = record<ManualCompactionStartEvent>({ mode: optional(str) });

export const manualCompactionEndEvent = record<ManualCompactionEndEvent>({
  aborted: bool,
  result: optional(compactionResult),
  skipped: optional(bool),
  errorMessage: optional(str),
});

export const autoRetryStartEvent = record<AutoRetryStartEvent>({
  attempt: int,
  maxAttempts: int,
  delayMs: int,
  errorMessage: str,
  errorId: optional(int),
});

export const autoRetryEndEvent = record<AutoRetryEndEvent>({
  success: bool,
  attempt: int,
  finalError: optional(str),
  retryErrors: withDefault(arrayOf(object), []),
});

export const notifyType = oneOf<NotifyType>(["info", "warning", "error"]);

export const noticeEvent = record<NoticeEvent>({ level: notifyType, message: str, source: optional(str) });

export const extensionError = record<ExtensionError>({ extensionPath: str, event: str, error: str });

export const rpcFrameErrorEvent = record<RpcFrameErrorEvent>({ error: str, originalType: optional(str) });

export const commandOutputEvent = record<CommandOutputEvent>({ text: str });

// ---------------------------------------------------------------------------------------------------------------------
// Extension UI requests (closed structs, discriminated by `method`)

export type WidgetPlacement = "aboveEditor" | "belowEditor";

export interface SelectOptionDetail {
  description?: string;
  [key: string]: unknown;
}

export type SelectionMarker = "radio" | "checkbox";

export interface SelectUiRequest {
  id: string;
  title: string;
  options: string[];
  optionDetails?: SelectOptionDetail[];
  /** Index into `options` to focus first. */
  initialIndex?: number;
  /** Marker style for the first `markableCount` options. */
  selectionMarker?: SelectionMarker;
  /** For `checkbox`: indices into `options` currently checked; authoritative. */
  checkedIndices?: number[];
  /** Leading options that get a marker; trailing ones are actions. All options when absent and `selectionMarker` set. */
  markableCount?: number;
  timeout?: number;
}

export interface ConfirmUiRequest {
  id: string;
  title: string;
  message: string;
  timeout?: number;
}

export interface InputUiRequest {
  id: string;
  title: string;
  placeholder?: string;
  timeout?: number;
  secret?: boolean;
}

export interface EditorUiRequest {
  id: string;
  title: string;
  prefill?: string;
  promptStyle?: boolean;
}

export interface AskOption {
  label: string;
  description?: string;
  preview?: string;
}

/** One question of an `ask` request; hosts always offer free text besides `options`. */
export interface AskQuestion {
  id: string;
  question: string;
  options: AskOption[];
  header?: string;
  multi: boolean;
  /** Index into `options` of the recommended choice. */
  recommended?: number;
}

export interface AskUiRequest {
  id: string;
  questions: AskQuestion[];
  timeout?: number;
}

/** Answer to one `ask` question: exact option labels, plus optional free text. */
export interface AskAnswer {
  id: string;
  selectedOptions: string[];
  customInput?: string;
}

/** The HTML `<form>` of one `ui.formDialog` call: the `ask` tool's form, or an extension's. */
export interface FormUiRequest {
  id: string;
  html: string;
  timeout?: number;
}

/** A `form` answer: the submitted values by control `name`, as an HTML form submits them. */
export type FormAnswerData = Record<string, string | number | string[]>;

export interface CancelUiRequest {
  id: string;
  targetId: string;
}

export interface NotifyUiRequest {
  id: string;
  message: string;
  notifyType?: NotifyType;
}

export interface SetStatusUiRequest {
  id: string;
  statusKey: string;
  statusText?: string;
}

export interface SetWidgetUiRequest {
  id: string;
  widgetKey: string;
  widgetLines?: string[];
  widgetPlacement?: WidgetPlacement;
}

export interface SetTitleUiRequest {
  id: string;
  title: string;
}

export interface SetEditorTextUiRequest {
  id: string;
  text: string;
}

export interface OpenUrlUiRequest {
  id: string;
  url: string;
  launchUrl?: string;
  instructions?: string;
}

/** A UI request as omp sends it: the payload plus its `type` and `method` tags. */
export type UiRequest<M extends string, T> = T & { type: "extension_ui_request"; method: M };

/** Extension UI request, discriminated by `method`. */
export type ExtensionUiRequest =
  | UiRequest<"select", SelectUiRequest>
  | UiRequest<"confirm", ConfirmUiRequest>
  | UiRequest<"input", InputUiRequest>
  | UiRequest<"editor", EditorUiRequest>
  | UiRequest<"ask", AskUiRequest>
  | UiRequest<"form", FormUiRequest>
  | UiRequest<"cancel", CancelUiRequest>
  | UiRequest<"notify", NotifyUiRequest>
  | UiRequest<"setStatus", SetStatusUiRequest>
  | UiRequest<"setWidget", SetWidgetUiRequest>
  | UiRequest<"setTitle", SetTitleUiRequest>
  | UiRequest<"set_editor_text", SetEditorTextUiRequest>
  | UiRequest<"open_url", OpenUrlUiRequest>;

// An open record: kept as received, an invalid `description` included.
const selectOptionDetail: Decoder<SelectOptionDetail> = (value) =>
  isObject(value) ? (value as SelectOptionDetail) : fail();

const askOption = record<AskOption>({ label: str, description: optional(str), preview: optional(str) });

export const askQuestion = record<AskQuestion>({
  id: str,
  question: str,
  options: arrayOf(askOption),
  header: optional(str),
  multi: withDefault(bool, false),
  recommended: optional(int),
});

export const askUiRequest = record<AskUiRequest>({ id: str, questions: arrayOf(askQuestion), timeout: optional(int) });

export const notifyUiRequest = record<NotifyUiRequest>({ id: str, message: str, notifyType: optional(notifyType) });

export const openUrlUiRequest = record<OpenUrlUiRequest>({
  id: str,
  url: str,
  launchUrl: optional(str),
  instructions: optional(str),
});

function tagged<M extends string, T>(method: M, payload: Decoder<T>): Decoder<UiRequest<M, T>> {
  return (value) => ({ ...payload(value), type: "extension_ui_request", method });
}

const UI_REQUESTS: { [M in ExtensionUiRequest["method"]]: Decoder<Extract<ExtensionUiRequest, { method: M }>> } = {
  select: tagged(
    "select",
    record<SelectUiRequest>({
      id: str,
      title: str,
      options: arrayOf(str),
      optionDetails: optional(arrayOf(selectOptionDetail)),
      initialIndex: optional(int),
      selectionMarker: optional(oneOf(["radio", "checkbox"])),
      checkedIndices: optional(arrayOf(int)),
      markableCount: optional(int),
      timeout: optional(int),
    }),
  ),
  confirm: tagged("confirm", record<ConfirmUiRequest>({ id: str, title: str, message: str, timeout: optional(int) })),
  input: tagged(
    "input",
    record<InputUiRequest>({
      id: str,
      title: str,
      placeholder: optional(str),
      timeout: optional(int),
      secret: optional(bool),
    }),
  ),
  editor: tagged(
    "editor",
    record<EditorUiRequest>({ id: str, title: str, prefill: optional(str), promptStyle: optional(bool) }),
  ),
  ask: tagged("ask", askUiRequest),
  form: tagged("form", record<FormUiRequest>({ id: str, html: str, timeout: optional(int) })),
  cancel: tagged("cancel", record<CancelUiRequest>({ id: str, targetId: str })),
  notify: tagged("notify", notifyUiRequest),
  setStatus: tagged("setStatus", record<SetStatusUiRequest>({ id: str, statusKey: str, statusText: optional(str) })),
  setWidget: tagged(
    "setWidget",
    record<SetWidgetUiRequest>({
      id: str,
      widgetKey: str,
      widgetLines: optional(arrayOf(str)),
      widgetPlacement: optional(oneOf(["aboveEditor", "belowEditor"])),
    }),
  ),
  setTitle: tagged("setTitle", record<SetTitleUiRequest>({ id: str, title: str })),
  set_editor_text: tagged("set_editor_text", record<SetEditorTextUiRequest>({ id: str, text: str })),
  open_url: tagged("open_url", openUrlUiRequest),
};

/** An `extension_ui_request` frame, dispatched on `method`. */
export const extensionUiRequest: Decoder<ExtensionUiRequest> = (value) => {
  const method = isObject(value) ? value.method : undefined;
  return typeof method === "string" && Object.hasOwn(UI_REQUESTS, method)
    ? UI_REQUESTS[method as ExtensionUiRequest["method"]](value)
    : fail();
};

// ---------------------------------------------------------------------------------------------------------------------
// Frames

/**
 * A frame from omp that is not a command response, as JSON. Known frames are objects discriminated by `type`; a frame
 * of unknown type (any JSON value) becomes an `unknown` notice.
 */
export type RpcNotification = JsonValue;

// ---------------------------------------------------------------------------------------------------------------------
// Live voice frames (`live_phase`, `live_transcript`, `live_end`; `live_levels` changes no state)

export type LivePhase = "connecting" | "listening" | "working" | "speaking" | "muted" | "error";
export type LiveRole = "user" | "assistant";

export interface LivePhaseEvent {
  phase: LivePhase;
}

/** Accumulated text of one realtime turn; replaces earlier frames with the same `role` and `turn`. */
export interface LiveTranscriptEvent {
  role: LiveRole;
  turn: number;
  text: string;
  final: boolean;
}

/** Sent exactly once when a live session ends; `error` carries the failure cause. */
export interface LiveEndEvent {
  error?: string;
}

export const livePhase = oneOf<LivePhase>(["connecting", "listening", "working", "speaking", "muted", "error"]);

export const livePhaseEvent = record<LivePhaseEvent>({ phase: livePhase });

export const liveTranscriptEvent = record<LiveTranscriptEvent>({
  role: oneOf<LiveRole>(["user", "assistant"]),
  turn: int,
  text: str,
  final: bool,
});

export const liveEndEvent = record<LiveEndEvent>({ error: optional(str) });

/** `message_start` and `message_end`; `messageId` is shared by the start, updates, and end of one message. */
export interface MessageEvent {
  message: AgentMessage;
  messageId?: string;
}

export interface MessageUpdateEvent {
  message: AgentMessage;
  assistantMessageEvent: AssistantMessageEvent;
  messageId?: string;
}

export interface ToolExecutionStartEvent {
  toolCallId: string;
  toolName: string;
  args?: JsonValue;
  intent?: string;
}

export interface ToolExecutionUpdateEvent {
  toolCallId: string;
  toolName: string;
  args?: JsonValue;
  partialResult?: JsonValue;
}

export interface ToolStreamUpdateEvent {
  toolCallId: string;
  toolName: string;
  update?: JsonValue;
}

export interface ToolExecutionEndEvent {
  toolCallId: string;
  toolName: string;
  result?: JsonValue;
  isError?: boolean;
}

/** Coalesced snapshot of the displayable steering/follow-up queue, sent whenever it changes. */
export interface QueueUpdateEvent {
  steering: string[];
  followUp: string[];
  /** Leading `steering` entries already sent into the streaming response. */
  liveSteered: number;
}

export interface AvailableCommandsUpdateEvent {
  commands: AvailableSlashCommand[];
}

export interface SubagentLifecycleEvent {
  payload: SubagentLifecyclePayload;
}

export interface SubagentProgressEvent {
  payload: SubagentProgressPayload;
}

export const messageEvent = record<MessageEvent>({ message: agentMessage, messageId: optional(str) });

export const messageUpdateEvent = record<MessageUpdateEvent>({
  message: agentMessage,
  assistantMessageEvent,
  messageId: optional(str),
});

export const toolExecutionStartEvent = record<ToolExecutionStartEvent>({
  toolCallId: str,
  toolName: str,
  args: optional(json),
  intent: optional(str),
});

export const toolExecutionUpdateEvent = record<ToolExecutionUpdateEvent>({
  toolCallId: str,
  toolName: str,
  args: optional(json),
  partialResult: optional(json),
});

export const toolStreamUpdateEvent = record<ToolStreamUpdateEvent>({
  toolCallId: str,
  toolName: str,
  update: optional(json),
});

export const toolExecutionEndEvent = record<ToolExecutionEndEvent>({
  toolCallId: str,
  toolName: str,
  result: optional(json),
  isError: optional(bool),
});

export const queueUpdateEvent = record<QueueUpdateEvent>({
  steering: arrayOf(str),
  followUp: arrayOf(str),
  liveSteered: withDefault(int, 0),
});

export const availableCommandsUpdateEvent = record<AvailableCommandsUpdateEvent>({
  commands: arrayOf(availableSlashCommand),
});

export const subagentLifecycleEvent = record<SubagentLifecycleEvent>({ payload: subagentLifecyclePayload });

export const subagentProgressEvent = record<SubagentProgressEvent>({ payload: subagentProgressPayload });
