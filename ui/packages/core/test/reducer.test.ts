/** Example tests for every event kind the reducer handles (TST-010); port of `crates/omfp-core/tests/reducer.rs`. */

import { describe, expect, test } from "bun:test";
import {
  type AssistantContent,
  type BackgroundJob,
  dialogId,
  emptyThreadState,
  type JobState,
  type JsonObject,
  type JsonValue,
  type NoticeFrame,
  reduce,
  replay,
  type ThreadEvent,
  type ThreadState,
} from "../src/index.ts";
import { assistantText, deepFreeze, event, messageText, omp, streamingText } from "./helpers.ts";

function userMessage(text: string): JsonObject {
  return { role: "user", content: [{ type: "text", text }], timestamp: 0 };
}

function assistantMessage(text: string): JsonObject {
  return { role: "assistant", content: [{ type: "text", text }], stopReason: "stop", timestamp: 0 };
}

function messageStart(id: string, message: JsonValue): ThreadEvent {
  return omp({ type: "message_start", messageId: id, message });
}

function messageEnd(id: string, message: JsonValue): ThreadEvent {
  return omp({ type: "message_end", messageId: id, message });
}

function update(id: string, assistantEvent: JsonValue): ThreadEvent {
  return omp({
    type: "message_update",
    messageId: id,
    message: { role: "assistant" },
    assistantMessageEvent: assistantEvent,
  });
}

function processEvent(state: JsonValue): ThreadEvent {
  return event({ kind: "process", state });
}

function uiRequest(fields: JsonObject): ThreadEvent {
  return omp({ type: "extension_ui_request", ...fields });
}

function select(id: string): ThreadEvent {
  return uiRequest({ id, method: "select", title: "Pick", options: ["a", "b"] });
}

function interactionIds(state: ThreadState): string[] {
  return state.interactions.map((request) => dialogId(request) ?? `unexpected ${request.method}`);
}

function transcriptTexts(state: ThreadState): string[] {
  return state.transcript.map((entry) => messageText(entry.message));
}

function history(messages: JsonValue[]): ThreadEvent {
  return event({ kind: "history", session: { session_id: "s1", resumed: true }, messages });
}

function firstBlock(state: ThreadState): AssistantContent | undefined {
  return state.streaming?.message.content?.[0];
}

function apply(state: ThreadState, ...events: ThreadEvent[]): ThreadState {
  return events.reduce(reduce, state);
}

describe("reducer", () => {
  // spec: ARC-006
  test("a new thread is stopped, settled and empty", () => {
    const state = emptyThreadState();
    expect(state.process).toEqual({ state: "stopped" });
    expect(state.settled).toBe(true);
    expect(state.transcript).toEqual([]);
    expect(state.streaming).toBeNull();
    expect(replay([])).toEqual(state);
  });

  // spec: TST-010, DMN-032
  test("history replaces the transcript and binds the session", () => {
    const state = replay([
      messageEnd("m1", userMessage("old")),
      messageStart("m2", assistantMessage("partial")),
      history([userMessage("q"), assistantMessage("a")]),
    ]);
    expect(transcriptTexts(state)).toEqual(["q", "a"]);
    expect(state.transcript[0]?.message_id).toBeNull();
    expect(state.streaming).toBeNull();
    expect(state.session).toEqual({ session_id: "s1", resumed: true });
  });

  // spec: TST-010
  test("process state follows process events", () => {
    const state = replay([processEvent({ state: "starting" }), processEvent({ state: "running" })]);
    expect(state.process).toEqual({ state: "running" });
  });

  // spec: TST-010
  test("agent_start unsettles and session_settled settles", () => {
    const busy = replay([processEvent({ state: "running" }), omp({ type: "agent_start" })]);
    expect(busy.settled).toBe(false);
    const settled = replay([
      processEvent({ state: "running" }),
      omp({ type: "agent_start" }),
      omp({ type: "agent_end", messages: [] }),
    ]);
    expect(settled.settled).toBe(false);
    expect(reduce(settled, omp({ type: "session_settled" })).settled).toBe(true);
  });

  // spec: TST-010
  test("a paused run and its late terminal end settle only on session_settled", () => {
    // omp 18.21+: a run paused on a background job ends with a terminal `agent_end` and no `agent_start` before it
    // when the job goes away without resuming it.
    const paused = replay([
      processEvent({ state: "running" }),
      omp({ type: "agent_start" }),
      omp({ type: "agent_end", messages: [], isTerminal: false, yielded: true, awaitingAsyncWork: true }),
    ]);
    expect(paused.settled).toBe(false);
    const ended = reduce(paused, omp({ type: "agent_end", messages: [], isTerminal: true, yielded: true }));
    expect(ended.settled).toBe(false);
    expect(reduce(ended, omp({ type: "session_settled" })).settled).toBe(true);
  });

  // spec: TST-010, DMN-035
  test("leaving running settles and cancels pending interactions", () => {
    const running = [
      processEvent({ state: "running" }),
      omp({ type: "agent_start" }),
      select("ui-1"),
      messageStart("m1", assistantMessage("half")),
    ];
    const failed = replay([...running, processEvent({ state: "failed", message: "exited", exit_code: 1 })]);
    expect(failed.settled).toBe(true);
    expect(failed.interactions).toEqual([]);
    expect(failed.streaming).toBeNull();
    expect(failed.process).toEqual({ state: "failed", message: "exited", exit_code: 1 });

    const left = replay([...running, processEvent({ state: "stopped" })]);
    expect(left.settled).toBe(true);
    expect(left.interactions).toEqual([]);

    const restarting = replay([...running, processEvent({ state: "starting" })]);
    expect(restarting.settled).toBe(false);
    expect(interactionIds(restarting)).toEqual(["ui-1"]);
  });

  // spec: TST-010, DMN-043
  test("message_end appends and replaces by message id", () => {
    const state = replay([
      messageEnd("m1", userMessage("hi")),
      messageEnd("m2", assistantMessage("first")),
      messageEnd("m2", assistantMessage("revised")),
      omp({ type: "message_end", message: userMessage("no id") }),
      omp({ type: "message_end", message: userMessage("no id") }),
    ]);
    expect(transcriptTexts(state)).toEqual(["hi", "revised", "no id", "no id"]);
    expect(state.transcript.map((entry) => entry.message_id)).toEqual(["m1", "m2", null, null]);
  });

  // spec: TST-010, DMN-043
  test("agent_end messages are not the transcript", () => {
    const state = replay([
      messageEnd("m1", userMessage("kept")),
      omp({ type: "agent_end", messages: [userMessage("compacted"), assistantMessage("away")] }),
    ]);
    expect(transcriptTexts(state)).toEqual(["kept"]);
  });

  // spec: TST-010
  test("message_start begins streaming only for assistant messages", () => {
    expect(replay([messageStart("m1", userMessage("hi"))]).streaming).toBeNull();

    const streaming = replay([messageStart("m2", assistantMessage("Hel"))]).streaming;
    expect(streaming?.message_id).toBe("m2");
    expect(streaming === null ? undefined : assistantText(streaming.message)).toBe("Hel");
  });

  // spec: TST-010
  test("text deltas rebuild the block and the end is authoritative", () => {
    const state = replay([
      messageStart("m1", { role: "assistant", content: [{ type: "text", text: "ahead" }] }),
      update("m1", { type: "text_start", contentIndex: 0, block: { type: "text", text: "" } }),
      update("m1", { type: "text_delta", contentIndex: 0, delta: "Hel" }),
      update("m1", { type: "text_delta", contentIndex: 0, delta: "lo" }),
    ]);
    expect(streamingText(state)).toBe("Hello");

    const ended = reduce(
      state,
      update("m1", {
        type: "text_end",
        contentIndex: 0,
        content: "Hello!",
        block: { type: "text", textSignature: "sig" },
      }),
    );
    expect(firstBlock(ended)).toEqual({ type: "text", text: "Hello!", textSignature: "sig" });
  });

  // spec: TST-010
  test("thinking deltas rebuild the block", () => {
    const state = replay([
      messageStart("m1", { role: "assistant", content: [] }),
      update("m1", { type: "thinking_start", contentIndex: 0, block: { type: "thinking", thinking: "" } }),
      update("m1", { type: "thinking_delta", contentIndex: 0, delta: "hmm " }),
      update("m1", { type: "thinking_delta", contentIndex: 0, delta: "ok" }),
    ]);
    expect(firstBlock(state)).toEqual({ type: "thinking", thinking: "hmm ok" });

    const ended = reduce(
      state,
      update("m1", {
        type: "thinking_end",
        contentIndex: 0,
        content: "hmm ok.",
        block: { type: "thinking", thinkingSignature: "sig" },
      }),
    );
    expect(firstBlock(ended)).toEqual({ type: "thinking", thinking: "hmm ok.", thinkingSignature: "sig" });
  });

  // spec: TST-010
  test("toolcall deltas accumulate argument JSON until the end", () => {
    const state = replay([
      messageStart("m1", { role: "assistant", content: [] }),
      update("m1", {
        type: "toolcall_start",
        contentIndex: 0,
        block: { type: "toolCall", id: "c1", name: "bash", arguments: {}, partialArgs: '{"cm' },
      }),
      update("m1", { type: "toolcall_delta", contentIndex: 0, delta: '{"cmd":' }),
      update("m1", { type: "toolcall_delta", contentIndex: 0, delta: '"ls"}' }),
    ]);
    const call = firstBlock(state);
    expect(call?.type).toBe("toolCall");
    expect(call?.id).toBe("c1");
    expect(call?.name).toBe("bash");
    expect(call?.partialArgs).toBe('{"cmd":"ls"}');

    const ended = reduce(
      state,
      update("m1", {
        type: "toolcall_end",
        contentIndex: 0,
        toolCall: { type: "toolCall", id: "c1", name: "bash", arguments: { cmd: "ls" } },
      }),
    );
    expect(firstBlock(ended)).toEqual({ type: "toolCall", id: "c1", name: "bash", arguments: { cmd: "ls" } });
  });

  // spec: TST-010
  test("image_end places the whole image", () => {
    const state = replay([
      messageStart("m1", { role: "assistant", content: [] }),
      update("m1", {
        type: "image_end",
        contentIndex: 0,
        content: { type: "image", data: "AAAA", mimeType: "image/png" },
      }),
    ]);
    expect(firstBlock(state)).toEqual({ type: "image", data: "AAAA", mimeType: "image/png" });
  });

  // spec: TST-010
  test("start restarts from the carried message", () => {
    const state = replay([
      messageStart("m1", assistantMessage("first try")),
      update("m1", { type: "start", message: { role: "assistant", content: [] } }),
      update("m1", { type: "text_start", contentIndex: 0, block: { type: "text", text: "" } }),
      update("m1", { type: "text_delta", contentIndex: 0, delta: "again" }),
    ]);
    expect(streamingText(state)).toBe("again");
  });

  // spec: TST-010
  test("deltas without message_start begin a streaming message", () => {
    const state = replay([
      update("m9", { type: "text_start", contentIndex: 0, block: { type: "text", text: "" } }),
      update("m9", { type: "text_delta", contentIndex: 0, delta: "x" }),
    ]);
    expect(state.streaming?.message_id).toBe("m9");
    expect(streamingText(state)).toBe("x");
  });

  // spec: TST-010
  test("deltas for missing or mismatched blocks are ignored", () => {
    const state = replay([
      messageStart("m1", { role: "assistant", content: [] }),
      update("m1", { type: "text_delta", contentIndex: 3, delta: "lost" }),
      update("m1", { type: "text_start", contentIndex: 0, block: { type: "text", text: "" } }),
      update("m1", { type: "thinking_delta", contentIndex: 0, delta: "wrong kind" }),
      update("m1", { type: "toolcall_delta", contentIndex: 0, delta: "wrong kind" }),
      update("m1", { type: "text_delta", contentIndex: -1, delta: "negative" }),
      update("m1", { type: "text_delta", delta: "no index" }),
    ]);
    expect(streamingText(state)).toBe("");
    expect(state.streaming?.message.content?.length).toBe(1);
  });

  // spec: TST-010
  test("an assistant message_end clears streaming but injected records do not", () => {
    const state = replay([
      messageStart("m1", { role: "assistant", content: [] }),
      update("m1", { type: "text_start", contentIndex: 0, block: { type: "text", text: "" } }),
      messageEnd("m2", { role: "custom", customType: "irc", content: "aside" }),
    ]);
    expect(state.streaming?.message_id).toBe("m1");
    expect(state.transcript.length).toBe(1);

    const ended = reduce(state, messageEnd("m1", assistantMessage("final")));
    expect(ended.streaming).toBeNull();
    expect(transcriptTexts(ended).at(-1)).toBe("final");
  });

  // spec: PRT-100
  test("a background job lives on after the call ends", () => {
    const frame = (kind: string, state: string, text: string, exitCode?: number): ThreadEvent => {
      const details: JsonObject = { async: { state, jobId: "bg_1", type: "bash" } };
      if (exitCode !== undefined) {
        details.exitCode = exitCode;
      }
      const result = { content: [{ type: "text", text }], details };
      return omp(
        kind === "tool_execution_end"
          ? { type: kind, toolCallId: "t1", toolName: "bash", result, isError: false }
          : { type: kind, toolCallId: "t1", toolName: "bash", partialResult: result },
      );
    };
    const job = (state: JobState, output: string, exitCode: number | null = null): BackgroundJob => ({
      job_id: "bg_1",
      state,
      output: output === "" ? null : output,
      exit_code: exitCode,
    });
    const start = omp({
      type: "tool_execution_start",
      toolCallId: "t1",
      toolName: "bash",
      args: { command: "make", async: true },
    });
    const started = [start, frame("tool_execution_end", "running", "Background job bg_1 started")];
    expect(replay(started).tools[0]?.status).toBe("succeeded");
    // The start result's text is omp's notice to the model, not the job's output.
    expect(replay(started).tools[0]?.background).toEqual(job("running", ""));
    const building = [...started, frame("tool_execution_update", "running", "building")];
    expect(replay(building).tools[0]?.background).toEqual(job("running", "building"));
    const done = replay([...building, frame("tool_execution_update", "completed", "built", 0)]);
    expect(done.tools[0]?.background).toEqual(job("completed", "built", 0));
    expect(done.tools[0]?.status).toBe("succeeded");
    // A job frame that comes before the call's end is newer than the start result.
    const raced = replay([
      start,
      frame("tool_execution_update", "running", "early"),
      frame("tool_execution_end", "running", "Background job bg_1 started"),
    ]);
    expect(raced.tools[0]?.background).toEqual(job("running", "early"));
    // Failed and cancelled jobs; frames without a job or with an unknown state leave it unset.
    expect(replay([frame("tool_execution_update", "failed", "boom", 2)]).tools[0]?.background).toEqual(
      job("failed", "boom", 2),
    );
    expect(replay([frame("tool_execution_update", "cancelled", "stopped")]).tools[0]?.background).toEqual(
      job("cancelled", "stopped"),
    );
    expect(replay([frame("tool_execution_update", "paused", "x")]).tools[0]?.background).toBeNull();
  });

  // spec: TST-010
  test("tool runs follow execution events", () => {
    const state = replay([
      omp({ type: "tool_execution_start", toolCallId: "t1", toolName: "bash", args: { cmd: "ls" }, intent: "list" }),
      omp({ type: "tool_execution_start", toolCallId: "t2", toolName: "read", args: {} }),
      omp({ type: "tool_execution_update", toolCallId: "t1", toolName: "bash", partialResult: { out: "a" } }),
      omp({ type: "tool_execution_update", toolCallId: "t1", toolName: "bash", partialResult: { out: "ab" } }),
      omp({
        type: "tool_execution_end",
        toolCallId: "t1",
        toolName: "bash",
        result: { out: "abc" },
        isError: false,
      }),
      omp({ type: "tool_execution_end", toolCallId: "t2", toolName: "read", result: "nope", isError: true }),
    ]);
    expect(state.tools.map((run) => run.tool_call_id)).toEqual(["t1", "t2"]);
    expect(state.tools[0]).toEqual({
      tool_call_id: "t1",
      tool_name: "bash",
      args: { cmd: "ls" },
      intent: "list",
      status: "succeeded",
      partial_result: { out: "ab" },
      preview: null,
      result: { out: "abc" },
      background: null,
    });
    expect(state.tools[1]?.status).toBe("failed");
    expect(state.tools[1]?.result).toBe("nope");
  });

  // spec: TST-010
  test("tool stream updates preview and announce runs", () => {
    const pending = replay([
      omp({ type: "tool_stream_update", toolCallId: "t1", toolName: "edit", update: { delta: "{" } }),
    ]);
    expect(pending.tools.length).toBe(1);
    expect(pending.tools[0]?.status).toBe("pending");
    expect(pending.tools[0]?.preview).toEqual({ delta: "{" });

    const running = apply(
      pending,
      omp({ type: "tool_execution_start", toolCallId: "t1", toolName: "edit", args: { p: 1 } }),
      omp({ type: "tool_stream_update", toolCallId: "t1", toolName: "edit", update: { delta: "}" } }),
    );
    expect(running.tools.length).toBe(1);
    expect(running.tools[0]?.status).toBe("running");
    expect(running.tools[0]?.args).toEqual({ p: 1 });
    expect(running.tools[0]?.preview).toEqual({ delta: "}" });
  });

  // spec: TST-010
  test("tool events for unknown runs create them", () => {
    const state = replay([
      omp({ type: "tool_execution_update", toolCallId: "u", toolName: "bash", partialResult: 1 }),
      omp({ type: "tool_execution_end", toolCallId: "e", toolName: "bash", result: 2 }),
    ]);
    expect(state.tools[0]?.status).toBe("running");
    expect(state.tools[0]?.partial_result).toBe(1);
    expect(state.tools[1]?.status).toBe("succeeded");
    expect(state.tools[1]?.result).toBe(2);
  });

  // spec: TST-010, TST-011
  test("the queue equals the last queue_update", () => {
    const state = replay([
      omp({ type: "queue_update", steering: ["s1"], followUp: ["f1", "f2"], liveSteered: 1 }),
      omp({ type: "queue_update", steering: [], followUp: ["f2"], liveSteered: 0 }),
    ]);
    expect(state.queue).toEqual({ steering: [], follow_up: ["f2"], live_steered: 0 });
  });

  // spec: TST-010, DMN-050
  test("dialog requests become pending interactions in order", () => {
    const state = replay([
      select("s"),
      uiRequest({ id: "c", method: "confirm", title: "Sure?", message: "Really" }),
      uiRequest({ id: "i", method: "input", title: "Name" }),
      uiRequest({ id: "e", method: "editor", title: "Edit", promptStyle: true }),
      uiRequest({
        id: "a",
        method: "ask",
        questions: [{ id: "q", question: "Which?", options: [{ label: "x" }] }],
      }),
    ]);
    expect(interactionIds(state)).toEqual(["s", "c", "i", "e", "a"]);
  });

  // spec: TST-010, DMN-050
  test("a select request keeps omp's selection fields", () => {
    const fields = {
      id: "s",
      method: "select" as const,
      title: "Context\n\nWhich?",
      options: ["a", "b", "Done selecting"],
      initialIndex: 1,
      selectionMarker: "checkbox" as const,
      checkedIndices: [0],
      markableCount: 2,
    };
    const state = replay([uiRequest(fields)]);
    expect(state.interactions).toEqual([{ type: "extension_ui_request", ...fields }]);
  });

  // spec: TST-010, DMN-051
  test("interaction_resolved closes the matching request", () => {
    const state = replay([
      select("s1"),
      select("s2"),
      event({ kind: "interaction_resolved", id: "s1", resolution: { by: "client", client: "ui" } }),
      event({ kind: "interaction_resolved", id: "zz", resolution: { by: "client", client: "ui" } }),
    ]);
    expect(interactionIds(state)).toEqual(["s2"]);
  });

  // spec: TST-010, DMN-052
  test("cancel closes the target request", () => {
    const state = replay([select("s1"), select("s2"), uiRequest({ id: "x", method: "cancel", targetId: "s2" })]);
    expect(interactionIds(state)).toEqual(["s1"]);
  });

  // spec: TST-010, DMN-053
  test("presentation requests update the presentation state", () => {
    const state = replay([
      uiRequest({ id: "1", method: "setStatus", statusKey: "git", statusText: "clean" }),
      uiRequest({ id: "2", method: "setStatus", statusKey: "tmp", statusText: "x" }),
      uiRequest({ id: "3", method: "setStatus", statusKey: "tmp" }),
      uiRequest({
        id: "4",
        method: "setWidget",
        widgetKey: "w",
        widgetLines: ["l1", "l2"],
        widgetPlacement: "belowEditor",
      }),
      uiRequest({ id: "5", method: "setWidget", widgetKey: "gone", widgetLines: ["x"] }),
      uiRequest({ id: "6", method: "setWidget", widgetKey: "gone" }),
      uiRequest({ id: "7", method: "setTitle", title: "My thread" }),
      uiRequest({ id: "8", method: "set_editor_text", text: "draft" }),
      uiRequest({ id: "9", method: "notify", message: "hello", notifyType: "warning" }),
      uiRequest({ id: "10", method: "open_url", url: "https://example.test/login" }),
    ]);
    const presentation = state.presentation;
    expect(presentation.statuses).toEqual({ git: "clean" });
    expect(presentation.widgets).toEqual({ w: { lines: ["l1", "l2"], placement: "belowEditor" } });
    expect(presentation.title).toBe("My thread");
    expect(presentation.editor_text).toBe("draft");
    expect(presentation.notifications).toEqual([{ id: "9", message: "hello", notifyType: "warning" }]);
    expect(presentation.open_url).toEqual({ id: "10", url: "https://example.test/login" });
    expect(state.interactions).toEqual([]);
  });

  // spec: TST-010
  test("available_commands_update replaces the commands", () => {
    const state = replay([
      omp({ type: "available_commands_update", commands: [{ name: "old", source: "builtin" }] }),
      omp({ type: "available_commands_update", commands: [{ name: "new", source: "skill" }] }),
    ]);
    expect(state.commands.map((command) => command.name)).toEqual(["new"]);
  });

  // spec: TST-010, PRT-030
  test("prompt results are kept by prompt id", () => {
    const state = replay([
      omp({ type: "prompt_result", id: "p1", agentInvoked: true, status: "completed", sessionSettled: false }),
      omp({
        type: "prompt_result",
        id: "p2",
        agentInvoked: true,
        status: "error",
        sessionSettled: true,
        error: { message: "boom", retryable: false },
      }),
      omp({ type: "prompt_result", agentInvoked: false, status: "completed", sessionSettled: true }),
    ]);
    expect(Object.keys(state.prompts).sort()).toEqual(["p1", "p2"]);
    expect(state.prompts.p2?.status).toBe("error");
  });

  // spec: TST-010, PRT-055
  test("subagent frames keep the latest lifecycle and progress", () => {
    const lifecycle = (status: string) => ({
      type: "subagent_lifecycle",
      payload: { id: "sa1", agent: "task", agentSource: "bundled", status, index: 0 },
    });
    const progress = (index: number, record: JsonObject) => ({
      type: "subagent_progress",
      payload: { index, agent: "task", agentSource: "bundled", task: "t", progress: record },
    });
    const state = replay([
      omp(lifecycle("started")),
      omp(progress(0, { id: "sa1", status: "running" })),
      omp(progress(0, { id: "sa1", status: "completed" })),
      omp(lifecycle("completed")),
      omp(progress(1, {})),
    ]);
    expect(Object.keys(state.subagents)).toEqual(["sa1"]);
    expect(state.subagents.sa1?.lifecycle?.status).toBe("completed");
    expect(state.subagents.sa1?.progress?.progress.status).toBe("completed");
  });

  // spec: TST-010, PRT-057
  test("todo phases are the last committed todo result of the transcript", () => {
    const todo = (id: string, op: string, isError: boolean, tasks: JsonValue[]): JsonObject => ({
      role: "toolResult",
      toolCallId: id,
      toolName: "todo",
      isError,
      timestamp: 0,
      content: [{ type: "text", text: "list" }],
      details: { op, phases: [{ name: "Tasks", tasks }], storage: "session" },
    });
    const read = { content: "Read", status: "completed" };
    const fix = { content: "Fix", status: "in_progress" };
    const contents = (state: ThreadState) =>
      state.todo_phases.flatMap((phase) => phase.tasks.map((task) => [task.content, task.status]));
    const state = replay([
      messageEnd("t1", todo("t1", "init", false, [read, fix])),
      // A failed call and a `view` commit nothing (omp's `committedTodoPhases`).
      messageEnd("t2", todo("t2", "done", true, [])),
      messageEnd("t3", todo("t3", "view", false, [])),
    ]);
    expect(contents(state)).toEqual([
      ["Read", "completed"],
      ["Fix", "in_progress"],
    ]);
    // History (a restart, a fork or a navigation) recomputes them from the restored branch.
    expect(contents(reduce(state, history([todo("h1", "init", false, [read]), userMessage("again")])))).toEqual([
      ["Read", "completed"],
    ]);
    expect(reduce(state, history([userMessage("before the list")])).todo_phases).toEqual([]);
  });

  // spec: TST-010, PRT-057
  test("a blocked task of an older omp reads as pending", () => {
    // omp 18.19 dropped the `blocked` status; sessions recorded before still hold it, in history and in new frames.
    const todo: JsonObject = {
      role: "toolResult",
      toolCallId: "t1",
      toolName: "todo",
      isError: false,
      timestamp: 0,
      content: [],
      details: {
        op: "init",
        phases: [
          {
            name: "Old",
            tasks: [
              { content: "Read", status: "completed" },
              { content: "Ship", status: "blocked", blocker: "review" },
            ],
          },
        ],
      },
    };
    const expected = [
      {
        name: "Old",
        tasks: [
          { content: "Read", status: "completed" as const },
          { content: "Ship", status: "pending" as const },
        ],
      },
    ];
    expect(replay([messageEnd("t1", todo)]).todo_phases).toEqual(expected);
    expect(replay([history([todo])]).todo_phases).toEqual(expected);
  });

  // spec: TST-010, PRT-056
  test("goal_updated keeps the session's goal", () => {
    const goal = {
      id: "g1",
      objective: "Ship it",
      status: "active" as const,
      tokensUsed: 10,
      timeUsedSeconds: 1.5,
      createdAt: 1,
      updatedAt: 2,
      tokenBudget: 1000,
    };
    const state = replay([omp({ type: "goal_updated", goal, state: { enabled: true, mode: "active", goal } })]);
    expect(state.goal).toEqual(goal);
    // A history keeps it (the goal belongs to the omp session, not the transcript); `goal: null` drops it.
    expect(reduce(state, history([])).goal).toEqual(goal);
    expect(reduce(state, omp({ type: "goal_updated", goal: null })).goal).toBeNull();
    // Like omp-rpc, a frame without `goal` or with a malformed one does not decode.
    expect(reduce(state, omp({ type: "goal_updated" })).notices.map((notice) => notice.frame.type)).toEqual([
      "unknown",
    ]);
  });

  // spec: TST-010, UI-043, DMN-042
  test("noteworthy frames become positioned notices", () => {
    const state = replay([
      omp({ type: "auto_compaction_start", reason: "threshold", action: "context-full" }),
      messageEnd("m1", userMessage("hi")),
      omp({ type: "auto_compaction_end", action: "context-full", aborted: false, willRetry: false }),
      omp({ type: "manual_compaction_start" }),
      omp({ type: "manual_compaction_end", aborted: true }),
      omp({ type: "auto_retry_start", attempt: 1, maxAttempts: 3, delayMs: 10, errorMessage: "429" }),
      omp({ type: "auto_retry_end", success: true, attempt: 1 }),
      omp({ type: "notice", level: "info", message: "saved" }),
      omp({ type: "extension_error", extensionPath: "/x.ts", event: "e", error: "bad" }),
      omp({ type: "rpc_frame_error", error: "too big", originalType: "message_update" }),
      omp({ type: "brand_new_frame", x: 1 }),
      event({ kind: "unknown", raw: { weird: true } }),
    ]);
    expect(state.notices.map((notice) => notice.frame.type)).toEqual([
      "auto_compaction_start",
      "auto_compaction_end",
      "manual_compaction_start",
      "manual_compaction_end",
      "auto_retry_start",
      "auto_retry_end",
      "notice",
      "extension_error",
      "rpc_frame_error",
      "unknown",
      "unknown",
    ]);
    expect(state.notices.map((notice) => notice.position)).toEqual([0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
    const unknown: NoticeFrame[] = [
      { type: "unknown", data: { type: "brand_new_frame", x: 1 } },
      { type: "unknown", data: { weird: true } },
    ];
    expect(state.notices.slice(9).map((notice) => notice.frame)).toEqual(unknown);
    // Payloads are the decoded structs: no `type` tag, defaults filled in.
    expect(state.notices[5]?.frame).toEqual({
      type: "auto_retry_end",
      data: { success: true, attempt: 1, retryErrors: [] },
    });
  });

  // spec: TST-010
  test("history drops notices placed in the previous transcript", () => {
    const state = replay([omp({ type: "notice", level: "info", message: "old" }), history([])]);
    expect(state.notices).toEqual([]);
  });

  // spec: TST-010, PRT-120
  test("command output and open_url become notices that survive history", () => {
    const state = replay([
      messageEnd("m1", userMessage("hi")),
      omp({ type: "notice", level: "info", message: "dropped" }),
      omp({ type: "command_output", text: "Added /w/app." }),
      messageEnd("m2", assistantMessage("done")),
      uiRequest({ id: "10", method: "open_url", url: "https://example.test/login", instructions: "Paste the code" }),
      // A restart resumes a shorter branch: positions past its end move to its end.
      history([userMessage("hi")]),
    ]);
    const kept: NoticeFrame[] = [
      { type: "command_output", data: { text: "Added /w/app." } },
      { type: "open_url", data: { id: "10", url: "https://example.test/login", instructions: "Paste the code" } },
    ];
    expect(state.notices.map((notice) => notice.frame)).toEqual(kept);
    expect(state.notices.map((notice) => notice.position)).toEqual([1, 1]);
    expect(state.presentation.open_url?.url).toBe("https://example.test/login");
  });

  // spec: TST-010, PRT-030
  test("history drops prompt results of the previous process", () => {
    // omp numbers requests `req_N` per process, so ids restart after a restart (a new History).
    const state = replay([
      omp({ type: "prompt_result", id: "req_5", agentInvoked: true, status: "completed", sessionSettled: true }),
      history([]),
    ]);
    expect(state.prompts).toEqual({});
  });

  // spec: TST-010
  test("frames the reducer does not need leave the state unchanged", () => {
    const base = deepFreeze(replay([processEvent({ state: "running" }), messageEnd("m1", userMessage("hi"))]));
    for (const frame of [
      { type: "ready", protocolVersion: 2 },
      { type: "advisor_cost_changed" },
      { type: "turn_start" },
      { type: "live_levels", input: 0.1, output: 0.2 },
      { type: "live_phase", phase: "listening" },
      { type: "dictation_transcript", captureId: "c", text: "t" },
      { type: "model_changed" },
      { type: "btw_delta", recordId: "r", delta: "d" },
      {
        type: "message_update",
        messageId: "m2",
        message: { role: "assistant" },
        assistantMessageEvent: { type: "done", reason: "stop" },
      },
    ]) {
      expect(reduce(base, omp(frame))).toBe(base);
    }
  });

  // spec: TST-010, DMN-042
  test("frames of a known type that do not decode become unknown notices", () => {
    // The daemon forwards what omp-rpc cannot decode as `unknown` (DMN-042); the reducer does the same for frames it
    // receives raw.
    for (const frame of [
      { type: "message_end", message: { role: "nobody" } },
      { type: "tool_execution_start", toolName: "bash" },
      { type: "queue_update", steering: "s", followUp: [] },
      { type: "extension_ui_request", id: "x", method: "teleport" },
      { type: "notice", level: "loud", message: "m" },
      { type: "prompt_result", id: "p", status: "completed" },
      "not an object",
      42,
    ]) {
      const state = replay([omp(frame)]);
      expect(state.notices).toEqual([{ position: 0, frame: { type: "unknown", data: frame } }]);
    }
  });

  // spec: ARC-006
  test("invalid thread events leave the state unchanged", () => {
    const base = deepFreeze(emptyThreadState());
    for (const value of [
      null,
      "omp",
      { kind: "teleport" },
      { kind: "omp" },
      { kind: "history", session: { session_id: "s" }, messages: [] },
      { kind: "history", session: { session_id: "s", resumed: false }, messages: [{ role: "nobody" }] },
      { kind: "process", state: { state: "failed" } },
      { kind: "interaction_resolved", id: "x", resolution: { by: "robot" } },
      { kind: "unknown" },
    ]) {
      expect(reduce(base, event(value))).toBe(base);
    }
  });
});
