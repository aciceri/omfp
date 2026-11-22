/**
 * The thread reducer (port of `omfp-core/src/reducer.rs`; ARC-006, D-04): a total, pure function from (state, event)
 * to state, shared with the daemon's Rust reducer. It never mutates its inputs and never throws on a JSON-decoded
 * event; unchanged parts of the state are shared with the previous state.
 */

import { arrayOf, type Decoder, decodeOrUndefined, Invalid, oneOf } from "./decode.ts";
import {
  liveOwner,
  notifyRequest,
  openFileRequest,
  processState,
  resolution,
  sessionInfo,
  type ThreadEvent,
} from "./event.ts";
import { isObject, type JsonValue, withKey, withoutKey } from "./json.ts";
import { liveErrorMessage } from "./live.ts";
import type {
  BackgroundJob,
  JobState,
  LiveLine,
  LiveSession,
  Notice,
  NoticeFrame,
  StreamingMessage,
  ThreadState,
  ToolRun,
  ToolStatus,
} from "./state.ts";
import { emptyThreadState } from "./state.ts";
import { applyStreamEvent, newStreamingMessage } from "./stream.ts";
import {
  type AgentMessage,
  agentMessage,
  autoCompactionEndEvent,
  autoCompactionStartEvent,
  autoRetryEndEvent,
  autoRetryStartEvent,
  availableCommandsUpdateEvent,
  commandOutputEvent,
  type ExtensionUiRequest,
  extensionError,
  extensionUiRequest,
  goalUpdatedEvent,
  liveEndEvent,
  livePhaseEvent,
  liveTranscriptEvent,
  manualCompactionEndEvent,
  manualCompactionStartEvent,
  messageEvent,
  messageUpdateEvent,
  noticeEvent,
  promptResultEvent,
  queueUpdateEvent,
  rpcFrameErrorEvent,
  subagentLifecycleEvent,
  subagentProgressEvent,
  type TodoPhase,
  todoPhase,
  toolExecutionEndEvent,
  toolExecutionStartEvent,
  toolExecutionUpdateEvent,
  toolStreamUpdateEvent,
} from "./wire.ts";

/** Frames omp-rpc knows that do not change the visible state. */
const IGNORED_FRAMES: Record<string, true> = {
  ready: true,
  subagent_event: true,
  live_levels: true,
  dictation_state: true,
  dictation_transcript: true,
  dictation_end: true,
  btw_delta: true,
  btw_record: true,
  session_info_update: true,
  config_update: true,
  // `agent_end.messages` can be compacted: the transcript comes from `message_end` only (DMN-043).
  agent_end: true,
  turn_start: true,
  turn_end: true,
  cache_warming_start: true,
  cache_warming_end: true,
  retry_fallback_applied: true,
  retry_fallback_succeeded: true,
  model_changed: true,
  config_warnings_changed: true,
  advisor_cost_changed: true,
  advisor_yielded: true,
  ttsr_triggered: true,
  todo_reminder: true,
  todo_auto_clear: true,
  irc_message: true,
  thinking_level_changed: true,
};

/** `open_url` notices come from the `extension_ui_request` frame. */
type NoticeType = Exclude<NoticeFrame["type"], "unknown" | "open_url">;

/** Frames kept as notices, with the decoder of their payload. */
const NOTICE_FRAMES: { [T in NoticeType]: Decoder<Extract<NoticeFrame, { type: T }>["data"]> } = {
  auto_compaction_start: autoCompactionStartEvent,
  auto_compaction_end: autoCompactionEndEvent,
  manual_compaction_start: manualCompactionStartEvent,
  manual_compaction_end: manualCompactionEndEvent,
  auto_retry_start: autoRetryStartEvent,
  auto_retry_end: autoRetryEndEvent,
  notice: noticeEvent,
  extension_error: extensionError,
  rpc_frame_error: rpcFrameErrorEvent,
  command_output: commandOutputEvent,
};

/** The id of a dialog request (`select`, `confirm`, `input`, `editor`, `ask`, `form`); `undefined` for other methods. */
export function dialogId(request: ExtensionUiRequest): string | undefined {
  switch (request.method) {
    case "select":
    case "confirm":
    case "input":
    case "editor":
    case "ask":
    case "form":
      return request.id;
    default:
      return undefined;
  }
}

function notice(state: ThreadState, frame: NoticeFrame): ThreadState {
  const entry: Notice = { position: state.transcript.length, frame };
  return { ...state, notices: [...state.notices, entry] };
}

/** Removes the pending dialog(s) with omp id `id`. */
function closeInteraction(state: ThreadState, id: string): ThreadState {
  const interactions = state.interactions.filter((request) => dialogId(request) !== id);
  return interactions.length === state.interactions.length ? state : { ...state, interactions };
}

const jobState = oneOf<JobState>(["running", "completed", "failed", "cancelled"]);

/** The background job of an omp tool result or partial result (`{ content, details: { async: { state, jobId },
 * exitCode? } }`, PRT-100); `null` without a job frame or with an unknown state. */
function jobFrame(result: unknown): BackgroundJob | null {
  const details = isObject(result) ? result.details : undefined;
  const frame = isObject(details) ? details.async : undefined;
  const state = isObject(frame) ? decodeOrUndefined(jobState, frame.state) : undefined;
  if (!isObject(details) || !isObject(frame) || state === undefined || typeof frame.jobId !== "string") {
    return null;
  }
  const content = isObject(result) && Array.isArray(result.content) ? result.content : [];
  const output = content
    .map((block) => (isObject(block) && block.type === "text" && typeof block.text === "string" ? block.text : ""))
    .join("");
  return {
    job_id: frame.jobId,
    state,
    output: output === "" ? null : output,
    exit_code: typeof details.exitCode === "number" && Number.isInteger(details.exitCode) ? details.exitCode : null,
  };
}

/** A job as its start result reports it: without output. */
function startedJob(job: BackgroundJob | null): BackgroundJob | null {
  return job === null ? null : { ...job, output: null };
}

/** `state` with the run `toolCallId` changed by `update`; a new run starts with `toolName` and `status`. */
function updateTool(
  state: ThreadState,
  toolCallId: string,
  toolName: string,
  status: ToolStatus,
  update: (run: ToolRun) => ToolRun,
): ThreadState {
  const index = state.tools.findIndex((run) => run.tool_call_id === toolCallId);
  const existing = state.tools[index];
  if (existing !== undefined) {
    const tools = [...state.tools];
    tools[index] = update(existing);
    return { ...state, tools };
  }
  const run: ToolRun = {
    tool_call_id: toolCallId,
    tool_name: toolName,
    args: null,
    intent: null,
    status,
    partial_result: null,
    preview: null,
    result: null,
    background: null,
  };
  return { ...state, tools: [...state.tools, update(run)] };
}

function applyUiRequest(state: ThreadState, request: ExtensionUiRequest): ThreadState {
  const presentation = state.presentation;
  switch (request.method) {
    case "select":
    case "confirm":
    case "input":
    case "editor":
    case "ask":
    case "form":
      return { ...state, interactions: [...state.interactions, request] };
    case "cancel":
      return closeInteraction(state, request.targetId);
    case "notify": {
      const { type: _type, method: _method, ...notify } = request;
      return { ...state, presentation: { ...presentation, notifications: [...presentation.notifications, notify] } };
    }
    case "setStatus": {
      const statuses =
        request.statusText === undefined
          ? withoutKey(presentation.statuses, request.statusKey)
          : withKey(presentation.statuses, request.statusKey, request.statusText);
      return { ...state, presentation: { ...presentation, statuses } };
    }
    case "setWidget": {
      const widgets =
        request.widgetLines === undefined
          ? withoutKey(presentation.widgets, request.widgetKey)
          : withKey(presentation.widgets, request.widgetKey, {
              lines: request.widgetLines,
              placement: request.widgetPlacement ?? null,
            });
      return { ...state, presentation: { ...presentation, widgets } };
    }
    case "setTitle":
      return { ...state, presentation: { ...presentation, title: request.title } };
    case "set_editor_text":
      return { ...state, presentation: { ...presentation, editor_text: request.text } };
    case "open_url": {
      const { type: _type, method: _method, ...open } = request;
      return notice({ ...state, presentation: { ...presentation, open_url: open } }, { type: "open_url", data: open });
    }
  }
}

function applyMessageUpdate(state: ThreadState, frame: Record<string, unknown>): ThreadState {
  const { assistantMessageEvent: event, messageId } = messageUpdateEvent(frame);
  if (event.type === "done" || event.type === "error") {
    return state;
  }
  const id = messageId ?? null;
  const current: StreamingMessage =
    state.streaming !== null && state.streaming.message_id === id ? state.streaming : newStreamingMessage(id);
  return { ...state, streaming: applyStreamEvent(current, event) };
}

function endMessage(state: ThreadState, frame: Record<string, unknown>): ThreadState {
  const { message, messageId } = messageEvent(frame);
  const entry = { message_id: messageId ?? null, message };
  const index = messageId === undefined ? -1 : state.transcript.findIndex((item) => item.message_id === messageId);
  const transcript = [...state.transcript];
  if (index === -1) {
    transcript.push(entry);
  } else {
    transcript[index] = entry;
  }
  const todo_phases = committedTodoPhases(message) ?? state.todo_phases;
  // Records injected mid-stream (IRC, advisor cards) end without ending the streaming reply.
  return { ...state, transcript, todo_phases, streaming: message.role === "assistant" ? null : state.streaming };
}

/**
 * The phases a successful, state-changing `todo` result committed (omp's `committedTodoPhases`): `undefined` for other
 * messages, failed calls, `view` reads and phases that do not decode. A task of an older omp's `blocked` status (gone
 * since 18.19) reads as `pending`, as omp restores it.
 */
function committedTodoPhases(message: AgentMessage): TodoPhase[] | undefined {
  if (message.role !== "toolResult" || message.toolName !== "todo" || message.isError === true) {
    return undefined;
  }
  const details = message.details;
  if (!isObject(details) || details.op === "view") {
    return undefined;
  }
  return decodeOrUndefined(arrayOf(todoPhase), withoutBlocked(details.phases));
}

/** `phases` with every task's legacy `blocked` status as `pending`; anything else as is. */
function withoutBlocked(phases: unknown): unknown {
  if (!Array.isArray(phases)) return phases;
  return phases.map((phase) => {
    if (!isObject(phase) || !Array.isArray(phase.tasks)) return phase;
    const tasks = phase.tasks.map((task) =>
      isObject(task) && task.status === "blocked" ? { ...task, status: "pending" } : task,
    );
    return { ...phase, tasks };
  });
}

/** One line per role and turn; a later frame replaces the text until the final one (PRT-052). */
function liveTranscript(live: LiveSession, line: LiveLine): LiveSession {
  const index = live.transcript.findIndex((known) => known.role === line.role && known.turn === line.turn);
  if (index === -1) {
    return { ...live, transcript: [...live.transcript, line] };
  }
  if (live.transcript[index]?.final === true) {
    return live;
  }
  return { ...live, transcript: live.transcript.map((known, at) => (at === index ? line : known)) };
}

/** Applies a frame of known `type`; throws {@link Invalid} when it does not decode. */
function applyFrame(state: ThreadState, type: string, frame: Record<string, unknown>): ThreadState {
  switch (type) {
    case "agent_start":
      return { ...state, settled: false };
    case "session_settled":
      return { ...state, settled: true };
    case "prompt_result": {
      const result = promptResultEvent(frame);
      return result.id === undefined ? state : { ...state, prompts: withKey(state.prompts, result.id, result) };
    }
    case "extension_ui_request":
      return applyUiRequest(state, extensionUiRequest(frame));
    case "available_commands_update":
      return { ...state, commands: availableCommandsUpdateEvent(frame).commands };
    case "subagent_lifecycle": {
      const { payload } = subagentLifecycleEvent(frame);
      const progress = Object.hasOwn(state.subagents, payload.id)
        ? (state.subagents[payload.id]?.progress ?? null)
        : null;
      return { ...state, subagents: withKey(state.subagents, payload.id, { lifecycle: payload, progress }) };
    }
    case "subagent_progress": {
      const { payload } = subagentProgressEvent(frame);
      // The subagent id lives in the raw `AgentProgress` record.
      const id = payload.progress.id;
      if (typeof id !== "string") {
        return state;
      }
      const lifecycle = Object.hasOwn(state.subagents, id) ? (state.subagents[id]?.lifecycle ?? null) : null;
      return { ...state, subagents: withKey(state.subagents, id, { lifecycle, progress: payload }) };
    }
    case "message_start": {
      const { message, messageId } = messageEvent(frame);
      return message.role === "assistant" ? { ...state, streaming: { message_id: messageId ?? null, message } } : state;
    }
    case "message_update":
      return applyMessageUpdate(state, frame);
    case "message_end":
      return endMessage(state, frame);
    case "tool_execution_start": {
      const start = toolExecutionStartEvent(frame);
      return updateTool(state, start.toolCallId, start.toolName, "running", (run) => ({
        ...run,
        status: "running",
        args: start.args ?? null,
        intent: start.intent ?? null,
      }));
    }
    case "tool_execution_update": {
      const update = toolExecutionUpdateEvent(frame);
      return updateTool(state, update.toolCallId, update.toolName, "running", (run) => ({
        ...run,
        args: update.args ?? run.args,
        partial_result: update.partialResult ?? null,
        background: jobFrame(update.partialResult) ?? run.background,
      }));
    }
    case "tool_stream_update": {
      const update = toolStreamUpdateEvent(frame);
      return updateTool(state, update.toolCallId, update.toolName, "pending", (run) => ({
        ...run,
        preview: update.update ?? null,
      }));
    }
    case "tool_execution_end": {
      const end = toolExecutionEndEvent(frame);
      return updateTool(state, end.toolCallId, end.toolName, "running", (run) => ({
        ...run,
        result: end.result ?? null,
        status: end.isError === true ? "failed" : "succeeded",
        // The start result of a background job: its text is omp's notice to the model, not the job's output; later
        // updates, which may have come first, are newer.
        background: run.background ?? startedJob(jobFrame(end.result)),
      }));
    }
    case "queue_update": {
      const queue = queueUpdateEvent(frame);
      return {
        ...state,
        queue: { steering: queue.steering, follow_up: queue.followUp, live_steered: queue.liveSteered },
      };
    }
    case "live_phase": {
      const { phase } = livePhaseEvent(frame);
      // omp repeats the `error` phase after `live_end`: phases only apply to an open session (PRT-052).
      return state.live === null ? state : { ...state, live: { ...state.live, phase } };
    }
    case "live_transcript": {
      const event = liveTranscriptEvent(frame);
      if (state.live === null) {
        return state;
      }
      const live = liveTranscript(state.live, event);
      return live === state.live ? state : { ...state, live };
    }
    case "goal_updated":
      return { ...state, goal: goalUpdatedEvent(frame).goal };
    case "live_end": {
      const { error } = liveEndEvent(frame);
      return { ...state, live: null, live_error: error === undefined ? null : liveErrorMessage(error) };
    }
  }
  if (Object.hasOwn(NOTICE_FRAMES, type)) {
    const noticeType = type as NoticeType;
    // The cast pairs `noticeType` with the payload its own decoder produced.
    return notice(state, { type: noticeType, data: NOTICE_FRAMES[noticeType](frame) } as NoticeFrame);
  }
  if (Object.hasOwn(IGNORED_FRAMES, type)) {
    return state;
  }
  throw new Invalid("unknown frame type");
}

function applyOmp(state: ThreadState, frame: JsonValue): ThreadState {
  const type = isObject(frame) ? frame.type : undefined;
  if (isObject(frame) && typeof type === "string") {
    try {
      return applyFrame(state, type, frame);
    } catch (error) {
      if (!(error instanceof Invalid)) {
        throw error;
      }
    }
  }
  // A frame omp-rpc cannot decode reaches the reducer as `unknown` (DMN-042).
  return notice(state, { type: "unknown", data: frame });
}

/**
 * Applies one event. Frames that do not change the visible state return `state` itself; an event that is not a valid
 * `ThreadEvent` (it would not have decoded in Rust) is ignored the same way.
 */
export function reduce(state: ThreadState, event: ThreadEvent): ThreadState {
  const value: unknown = event;
  if (!isObject(value)) {
    return state;
  }
  switch (value.kind) {
    case "omp":
      return value.frame === undefined ? state : applyOmp(state, value.frame as JsonValue);
    case "history": {
      const session = decodeOrUndefined(sessionInfo, value.session);
      const messages = decodeOrUndefined(arrayOf(agentMessage), value.messages);
      if (session === undefined || messages === undefined) {
        return state;
      }
      return {
        ...state,
        session,
        transcript: messages.map((message) => ({ message_id: null, message })),
        // The restored branch has its own list (omp's `getLatestTodoSnapshotIdentity`).
        todo_phases:
          messages.reduceRight<TodoPhase[] | undefined>(
            (found, message) => found ?? committedTodoPhases(message),
            undefined,
          ) ?? [],
        streaming: null,
        // Their positions referred to the replaced transcript; a command's output and the URLs omp opened stay (a
        // restart after `/mcp reauth` resumes the same transcript), at most after its last message.
        notices: state.notices
          .filter(({ frame }) => frame.type === "command_output" || frame.type === "open_url")
          .map((kept) => ({ ...kept, position: Math.min(kept.position, messages.length) })),
        // A new process numbers its requests from scratch, so old prompt ids would collide.
        prompts: {},
      };
    }
    case "process": {
      const process = decodeOrUndefined(processState, value.state);
      if (process === undefined) {
        return state;
      }
      if (process.state === "running" || process.state === "starting") {
        return { ...state, process };
      }
      // Nothing can answer, settle or finish streaming without a process (DMN-035); a live session ends with it
      // (DMN-103).
      return { ...state, process, settled: true, interactions: [], streaming: null, live: null };
    }
    case "interaction_resolved":
      return typeof value.id === "string" && decodeOrUndefined(resolution, value.resolution) !== undefined
        ? closeInteraction(state, value.id)
        : state;
    case "live_session": {
      if (value.owner === undefined || value.owner === null) {
        return { ...state, live: null };
      }
      const owner = decodeOrUndefined(liveOwner, value.owner);
      return owner === undefined
        ? state
        : {
            ...state,
            live: { owner, phase: null, transcript: [], context_end: state.transcript.length },
            live_error: null,
          };
    }
    case "open_file": {
      const request = decodeOrUndefined(openFileRequest, value);
      return request === undefined ? state : { ...state, open_file: request };
    }
    case "notify": {
      const request = decodeOrUndefined(notifyRequest, value);
      return request === undefined ? state : { ...state, notify: request };
    }
    case "unknown":
      return value.raw === undefined ? state : notice(state, { type: "unknown", data: value.raw as JsonValue });
    default:
      return state;
  }
}

/** Applies `events` in order to a fresh state. */
export function replay(events: Iterable<ThreadEvent>): ThreadState {
  let state = emptyThreadState();
  for (const event of events) {
    state = reduce(state, event);
  }
  return state;
}
