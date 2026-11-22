/** L0: the debug session of `omfp:debug` (UI-946..UI-949), from the thread's `debug` tool results. */

import { describe, expect, test } from "bun:test";
import type { ThreadState, TranscriptEntry } from "@omfp/core";
import { debugView } from "../../src/vm/debug.ts";

type Obj = Record<string, unknown>;

/** omp 18.22's `DapSessionSummary` (the `snapshot` of every debug result), trimmed to what the view reads. */
function snapshot(fields: Obj = {}): Obj {
  return { id: "s1", adapter: "debugpy", cwd: "/w", program: "/w/job.py", status: "running", ...fields };
}

let counter = 0;

/** A `debug` call (the assistant's tool call) and its result with `details`. */
function debugCall(args: Obj, details: Obj, isError = false): TranscriptEntry[] {
  counter += 1;
  const id = `call-${counter}`;
  return [
    {
      message_id: null,
      message: { role: "assistant", content: [{ type: "toolCall", id, name: "debug", arguments: args }] },
    },
    {
      message_id: null,
      message: {
        role: "toolResult",
        toolCallId: id,
        toolName: "debug",
        isError,
        details: { success: true, ...details },
      },
    },
  ] as TranscriptEntry[];
}

/** The same call through the `xd://debug` device: a `write` whose details carry the inner debug details. */
function deviceCall(args: Obj, details: Obj): TranscriptEntry[] {
  counter += 1;
  const id = `write-${counter}`;
  return [
    {
      message_id: null,
      message: {
        role: "assistant",
        content: [{ type: "toolCall", id, name: "write", arguments: { path: "xd://debug", content: "{}" } }],
      },
    },
    {
      message_id: null,
      message: {
        role: "toolResult",
        toolCallId: id,
        toolName: "write",
        details: { xdev: { tool: "debug", mode: "execute", args, inner: { success: true, ...details } } },
      },
    },
  ] as TranscriptEntry[];
}

/** The same call as the `debug` tool command of a `bash` call (omp 19): its single dispatch carries the details. */
function commandCall(args: Obj, details: Obj, extra: Obj[] = []): TranscriptEntry[] {
  counter += 1;
  const id = `bash-${counter}`;
  const dispatch = { tool: "debug", mode: "execute", exitCode: 0, args, inner: { success: true, ...details } };
  return [
    {
      message_id: null,
      message: {
        role: "assistant",
        content: [{ type: "toolCall", id, name: "bash", arguments: { command: "debug set_breakpoint" } }],
      },
    },
    {
      message_id: null,
      message: {
        role: "toolResult",
        toolCallId: id,
        toolName: "bash",
        details: { toolCommands: [dispatch, ...extra] },
      },
    },
  ] as TranscriptEntry[];
}

function state(...calls: TranscriptEntry[][]): Pick<ThreadState, "transcript"> {
  return { transcript: calls.flat() };
}

const stopped = snapshot({
  status: "stopped",
  stopReason: "breakpoint",
  frameName: "main",
  source: { path: "/w/job.py", name: "job.py" },
  line: 12,
});

const launch = debugCall({ action: "launch", program: "job.py" }, { action: "launch", snapshot: snapshot() });
const breakpoint = debugCall(
  { action: "set_breakpoint", file: "job.py", line: 12 },
  { action: "set_breakpoint", snapshot: snapshot(), breakpoints: [{ id: 1, verified: true, line: 12 }] },
);
const toStop = debugCall({ action: "continue" }, { action: "continue", snapshot: stopped, state: "stopped" });
const stack = debugCall(
  { action: "stack_trace" },
  {
    action: "stack_trace",
    snapshot: stopped,
    stackFrames: [
      { id: 7, name: "main", source: { path: "/w/job.py" }, line: 12, column: 1 },
      { id: 8, name: "<module>", line: 0, column: 0, instructionPointerReference: "0x10" },
    ],
  },
);
const variables = debugCall(
  { action: "variables", scope_id: 3 },
  {
    action: "variables",
    snapshot: stopped,
    variables: [
      { name: "count", value: "3", type: "int", variablesReference: 0 },
      { name: "items", value: "[1, 2]", variablesReference: 4 },
    ],
  },
);

describe("debugView", () => {
  // spec: UI-946
  test("no debug result: no session", () => {
    expect(debugView({ transcript: [] })).toBeUndefined();
    expect(debugView(state(debugCall({ action: "launch" }, { action: "launch" }, true)))).toBeUndefined();
  });

  // spec: UI-946, UI-947
  test("the stop location, the stack and the variables read there, and the breakpoints", () => {
    const view = debugView(state(launch, breakpoint, toStop, stack, variables));
    expect(view?.adapter).toBe("debugpy");
    expect(view?.program).toBe("/w/job.py");
    expect([view?.status, view?.statusLabel]).toEqual(["stopped", "stopped"]);
    expect(view?.stop).toEqual({
      reason: "breakpoint",
      frame: "main",
      location: { path: "/w/job.py", line: 12 },
      where: "/w/job.py:12",
    });
    expect(view?.stack).toEqual([
      { id: 7, name: "main", location: { path: "/w/job.py", line: 12 }, where: "/w/job.py:12" },
      { id: 8, name: "<module>", location: undefined, where: "0x10" },
    ]);
    expect(view?.variables).toEqual([
      { name: "count", value: "3", type: "int" },
      { name: "items", value: "[1, 2]", type: undefined },
    ]);
    expect(view?.breakpoints).toEqual([
      {
        label: "job.py:12",
        location: { path: "/w/job.py", line: 12 },
        verified: true,
        condition: undefined,
        message: undefined,
      },
    ]);
  });

  // spec: UI-947
  test("a step makes the stack and variables read before it stale; a later stop moves the location", () => {
    const step = debugCall(
      { action: "step_over" },
      { action: "step_over", snapshot: { ...stopped, stopReason: "step", line: 13 } },
    );
    const view = debugView(state(launch, breakpoint, toStop, stack, variables, step));
    expect(view?.stop).toEqual({
      reason: "step",
      frame: "main",
      location: { path: "/w/job.py", line: 13 },
      where: "/w/job.py:13",
    });
    expect(view?.stack).toEqual([]);
    expect(view?.variables).toEqual([]);
    expect(view?.breakpoints.map((row) => row.label)).toEqual(["job.py:12"]);
  });

  // spec: UI-948
  test("breakpoints follow set and remove per file, function breakpoints replace theirs", () => {
    const second = debugCall(
      { action: "set_breakpoint", file: "lib.py", line: 4, condition: "x > 1" },
      {
        action: "set_breakpoint",
        snapshot: snapshot(),
        breakpoints: [{ verified: false, line: 4, condition: "x > 1", message: "no code here" }],
      },
    );
    const fn = debugCall(
      { action: "set_breakpoint", function: "run" },
      { action: "set_breakpoint", snapshot: snapshot(), functionBreakpoints: [{ name: "run", verified: true }] },
    );
    const removed = debugCall(
      { action: "remove_breakpoint", file: "job.py", line: 12 },
      { action: "remove_breakpoint", snapshot: snapshot(), breakpoints: [] },
    );
    expect(debugView(state(launch, breakpoint, second, fn))?.breakpoints).toEqual([
      {
        label: "job.py:12",
        location: { path: "/w/job.py", line: 12 },
        verified: true,
        condition: undefined,
        message: undefined,
      },
      {
        label: "lib.py:4",
        location: { path: "/w/lib.py", line: 4 },
        verified: false,
        condition: "x > 1",
        message: "no code here",
      },
      { label: "run", location: undefined, verified: true, condition: undefined, message: undefined },
    ]);
    expect(debugView(state(launch, breakpoint, second, fn, removed))?.breakpoints.map((row) => row.label)).toEqual([
      "lib.py:4",
      "run",
    ]);
  });

  // spec: UI-946, UI-948
  test("through the xd://debug device; a new session starts afresh; the end keeps the exit code", () => {
    const device = deviceCall(
      { action: "set_breakpoint", file: "job.py", line: 2 },
      { action: "set_breakpoint", snapshot: snapshot(), breakpoints: [{ verified: true, line: 2 }] },
    );
    expect(debugView(state(launch, device))?.breakpoints.map((row) => row.label)).toEqual(["job.py:2"]);
    const relaunch = debugCall(
      { action: "launch", program: "other.py" },
      { action: "launch", snapshot: snapshot({ id: "s2", program: "/w/other.py" }) },
    );
    const end = debugCall(
      { action: "terminate" },
      {
        action: "terminate",
        snapshot: snapshot({ id: "s2", program: "/w/other.py", status: "terminated", exitCode: 1 }),
      },
    );
    const view = debugView(state(launch, device, relaunch, end));
    expect(view?.program).toBe("/w/other.py");
    expect([view?.status, view?.stop, view?.exitCode]).toEqual(["terminated", undefined, 1]);
    expect(view?.breakpoints).toEqual([]);
  });

  // spec: UI-946, UI-948, UI-915
  test("through the debug tool command of a bash call; a bash call with several dispatches is not read", () => {
    const set = { action: "set_breakpoint", file: "job.py", line: 2 };
    const result = { action: "set_breakpoint", snapshot: snapshot(), breakpoints: [{ verified: true, line: 2 }] };
    expect(debugView(state(launch, commandCall(set, result)))?.breakpoints.map((row) => row.label)).toEqual([
      "job.py:2",
    ]);
    const twice = commandCall(set, result, [{ tool: "lsp", mode: "execute", exitCode: 0 }]);
    expect(debugView(state(launch, twice))?.breakpoints).toEqual(debugView(state(launch))?.breakpoints);
  });

  // spec: UI-949
  test("relative places resolve against the session's folder, labelled as the agent and the adapter gave them", () => {
    const at = snapshot({ cwd: "/w/app", status: "stopped", source: { path: "./src/main.c" }, line: 42 });
    const relativeBreakpoint = debugCall(
      { action: "set_breakpoint", file: "../lib/util.c", line: 7 },
      { action: "set_breakpoint", snapshot: snapshot({ cwd: "/w/app" }), breakpoints: [{ verified: true, line: 7 }] },
    );
    const relativeStack = debugCall(
      { action: "stack_trace" },
      {
        action: "stack_trace",
        snapshot: at,
        stackFrames: [{ id: 1, name: "main", source: { path: "src/main.c" }, line: 42 }],
      },
    );
    const view = debugView(state(launch, relativeBreakpoint, relativeStack));
    expect(view?.stop).toEqual({
      reason: undefined,
      frame: undefined,
      location: { path: "/w/app/src/main.c", line: 42 },
      where: "./src/main.c:42",
    });
    expect(view?.stack).toEqual([
      { id: 1, name: "main", location: { path: "/w/app/src/main.c", line: 42 }, where: "src/main.c:42" },
    ]);
    expect(view?.breakpoints.map(({ label, location }) => ({ label, location }))).toEqual([
      { label: "../lib/util.c:7", location: { path: "/w/lib/util.c", line: 7 } },
    ]);
  });

  // spec: UI-948
  test("one file's breakpoints whatever the spelling: the latest list replaces them under the latest spelling", () => {
    const set = (file: string, lines: number[]) =>
      debugCall(
        { action: "set_breakpoint", file, line: lines.at(-1) },
        {
          action: "set_breakpoint",
          snapshot: snapshot(),
          breakpoints: lines.map((line) => ({ verified: true, line })),
        },
      );
    const clear = (file: string) =>
      debugCall(
        { action: "remove_breakpoint", file, line: 3 },
        { action: "remove_breakpoint", snapshot: snapshot(), breakpoints: [] },
      );
    const labels = (...calls: TranscriptEntry[][]) =>
      debugView(state(launch, ...calls))?.breakpoints.map(({ label, location }) => ({ label, location }));
    expect(labels(set("src/main.py", [3]), set("./src/main.py", [3, 9]))).toEqual([
      { label: "./src/main.py:3", location: { path: "/w/src/main.py", line: 3 } },
      { label: "./src/main.py:9", location: { path: "/w/src/main.py", line: 9 } },
    ]);
    expect(labels(set("src/main.py", [3]), clear("./src/main.py"))).toEqual([]);
    expect(labels(set("src/main.py", [3]), clear("/w/src/main.py"))).toEqual([]);
  });
});
