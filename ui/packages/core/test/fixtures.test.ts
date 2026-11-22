/**
 * Replays of the recorded thread fixtures (real omp output) through the reducer; port of
 * `crates/omfp-core/tests/fixtures.rs`.
 */

import { describe, expect, test } from "bun:test";
import { reduce, replay, type ThreadState, type ToolCall } from "../src/index.ts";
import {
  canonical,
  FIXTURE_DIR,
  fixture,
  indexOf,
  jsonlNames,
  messageText,
  stateThrough,
  streamingText,
} from "./helpers.ts";

function lastMessageText(state: ThreadState): string {
  const last = state.transcript.at(-1);
  return last === undefined ? "" : messageText(last.message);
}

function transcriptTexts(state: ThreadState): string[] {
  return state.transcript.map((entry) => messageText(entry.message));
}

function firstToolCall(state: ThreadState): ToolCall | undefined {
  const block = state.streaming?.message.content?.[0];
  return block?.type === "toolCall" ? block : undefined;
}

describe("fixtures", () => {
  // spec: ARC-006, TST-012, DMN-043
  test("text_reply ends with the assistant text, settled", () => {
    const state = replay(fixture("text_reply"));
    expect(state.process).toEqual({ state: "running" });
    expect(state.settled).toBe(true);
    expect(state.streaming).toBeNull();
    expect(transcriptTexts(state)).toEqual(["say hi", "Hello from the fake model"]);
    expect(state.transcript.map((entry) => entry.message_id)).toEqual(["msg-1", "msg-2"]);
    expect(state.session?.session_id).toBe("<uuid>");
    expect(state.session?.resumed).toBe(false);
    expect(state.prompts.req_7?.status).toBe("completed");
    expect(state.commands.some((command) => command.name === "security")).toBe(true);
    // setWidget without lines clears the widget.
    expect(state.presentation.widgets).toEqual({});
  });

  // spec: ARC-006, TST-012
  test("text_reply streams text deltas", () => {
    const events = fixture("text_reply");
    expect(stateThrough(events, indexOf(events, "agent_start", 0)).settled).toBe(false);
    expect(stateThrough(events, indexOf(events, "message_start", 1)).streaming?.message_id).toBe("msg-2");
    // text_start empties the block.
    expect(streamingText(stateThrough(events, indexOf(events, "text_start", 0)))).toBe("");

    const afterDeltas = stateThrough(events, indexOf(events, "text_delta", 2));
    expect(streamingText(afterDeltas)).toBe("Hello from t");
    expect(afterDeltas.settled).toBe(false);

    expect(streamingText(stateThrough(events, indexOf(events, "text_end", 0)))).toBe("Hello from the fake model");

    const ended = stateThrough(events, indexOf(events, "message_end", 1));
    expect(ended.streaming).toBeNull();
    expect(lastMessageText(ended)).toBe("Hello from the fake model");
    // Settled only by session_settled.
    expect(ended.settled).toBe(false);
  });

  // spec: ARC-006, TST-012
  test("bash_tool_call runs one tool to success", () => {
    const state = replay(fixture("bash_tool_call"));
    expect(state.tools.length).toBe(1);
    const run = state.tools[0];
    expect(run?.tool_call_id).toBe("call_bash_1");
    expect(run?.tool_name).toBe("bash");
    expect(run?.status).toBe("succeeded");
    expect(run?.args).toMatchObject({ command: "echo tool-output-123" });
    expect(JSON.stringify(run?.result)).toContain("tool-output-123");
    expect(lastMessageText(state)).toBe("The command printed tool-output-123.");
    expect(state.transcript.map((entry) => entry.message.role)).toEqual([
      "user",
      "assistant",
      "toolResult",
      "assistant",
    ]);
    expect(state.settled).toBe(true);
  });

  // spec: ARC-006, TST-012
  test("bash_tool_call streams arguments then runs", () => {
    const events = fixture("bash_tool_call");

    const streamed = firstToolCall(stateThrough(events, indexOf(events, "toolcall_delta", 5)));
    expect(streamed?.id).toBe("call_bash_1");
    expect(streamed?.partialArgs).toBe('{"command":"echo tool-output-123"}');

    const ended = firstToolCall(stateThrough(events, indexOf(events, "toolcall_end", 0)));
    expect(ended?.arguments).toMatchObject({ command: "echo tool-output-123" });

    const running = stateThrough(events, indexOf(events, "tool_execution_start", 0));
    expect(running.tools[0]?.status).toBe("running");
    expect(running.tools[0]?.partial_result).toBeNull();

    const updated = stateThrough(events, indexOf(events, "tool_execution_update", 0));
    expect(JSON.stringify(updated.tools[0]?.partial_result)).toContain("tool-output-123");
    expect(updated.tools[0]?.status).toBe("running");
  });

  // spec: ARC-006, TST-012, DMN-050, DMN-051
  test("approval_granted shows then resolves the select", () => {
    const events = fixture("approval_granted");

    const asked = stateThrough(events, indexOf(events, "extension_ui_request", 1));
    expect(asked.interactions.length).toBe(1);
    const request = asked.interactions[0];
    if (request?.method !== "select") {
      throw new Error(`select expected: ${JSON.stringify(asked.interactions)}`);
    }
    expect(request.title.startsWith("Allow tool: bash")).toBe(true);
    expect(request.options).toEqual(["Approve", "Approve bash for this session", "Deny"]);

    expect(stateThrough(events, indexOf(events, "interaction_resolved", 0)).interactions).toEqual([]);

    const state = replay(events);
    expect(state.interactions).toEqual([]);
    expect(state.tools[0]?.status).toBe("succeeded");
    expect(lastMessageText(state)).toBe("Done after approval.");
  });

  // spec: ARC-006, TST-012, DMN-050, DMN-051, DMN-055
  test("ask_answered shows the whole form once", () => {
    const events = fixture("ask_answered");

    let state = replay([]);
    let mostPending = 0;
    let forms: string[] = [];
    for (const item of events) {
      state = reduce(state, item);
      mostPending = Math.max(mostPending, state.interactions.length);
      if (forms.length === 0 && state.interactions.length > 0) {
        forms = state.interactions.map((request) => (request.method === "form" ? request.html : ""));
      }
    }
    expect(forms.length).toBe(1);
    expect(forms[0]).toContain("Which database?");
    expect(forms[0]).toContain("Which features?");
    expect(mostPending).toBe(1);

    expect(state.interactions).toEqual([]);
    expect(state.tools.find((run) => run.tool_call_id === "call_ask_1")?.status).toBe("succeeded");
    expect(lastMessageText(state)).toBe("Using Postgres with Auth and Elasticsearch.");
  });

  // spec: ARC-006, TST-012, TST-011
  test("follow_up_then_abort tracks the queue", () => {
    const events = fixture("follow_up_then_abort");

    const queued = stateThrough(events, indexOf(events, "queue_update", 0));
    expect(queued.queue.follow_up).toEqual(["later please"]);
    expect(queued.queue.steering).toEqual([]);

    const state = replay(events);
    expect(state.queue).toEqual({ steering: [], follow_up: [], live_steered: 0 });
    const last = state.transcript.at(-1)?.message;
    expect(last?.role).toBe("assistant");
    expect(last?.role === "assistant" ? last.stopReason : undefined).toBe("aborted");
    expect(state.prompts.req_7?.status).toBe("aborted");
    expect(state.streaming).toBeNull();
    expect(state.settled).toBe(true);
  });

  // spec: ARC-006, TST-012, DMN-032, DMN-043
  test("restart_with_history keeps the history then new messages", () => {
    const state = replay(fixture("restart_with_history"));
    expect(transcriptTexts(state)).toEqual(["first question", "first answer", "second question", "second answer"]);
    expect(state.transcript.map((entry) => entry.message_id)).toEqual([null, null, "msg-1", "msg-2"]);
    expect(state.session?.resumed).toBe(true);
  });

  // spec: PRT-021, TST-011
  test("a snapshot round trip then the remaining events equals the full replay", () => {
    for (const name of jsonlNames(FIXTURE_DIR)) {
      const events = fixture(name);
      const expected = canonical(replay(events));
      for (let split = 0; split <= events.length; split += 1) {
        const snapshot = replay(events.slice(0, split));
        const restored: unknown = JSON.parse(JSON.stringify(snapshot));
        expect(restored).toEqual(snapshot);
        // Equal to a `ThreadState` per the assertion above.
        const resumed = events.slice(split).reduce(reduce, restored as ThreadState);
        if (canonical(resumed) !== expected) {
          throw new Error(`${name}: split at ${split} differs from the full replay`);
        }
      }
    }
  });
});
