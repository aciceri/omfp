/** L4: `omfp:debug` over a fake client (UI-946..UI-949). */

import { afterEach, describe, expect, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import { cleanup, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OPEN_FILES, SELECTED_THREAD } from "../../src/sdk/index.ts";
import { DebugWidget } from "../../src/widgets/debug/index.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

type Obj = Record<string, unknown>;

/** A `debug` call and its result, as omp 19.1 ends their messages. */
function debugMessages(id: string, args: Obj, details: Obj): Obj[] {
  return [
    { role: "assistant", content: [{ type: "toolCall", id, name: "debug", arguments: args }] },
    { role: "toolResult", toolCallId: id, toolName: "debug", details: { success: true, ...details } },
  ];
}

/** The `message_end` frames of `messages`. */
function ends(messages: Obj[]): ThreadEvent[] {
  return messages.map((message) => ({ kind: "omp", frame: { type: "message_end", message } }) as ThreadEvent);
}

function debugCall(id: string, args: Obj, details: Obj): ThreadEvent[] {
  return ends(debugMessages(id, args, details));
}

const running = { id: "s1", adapter: "lldb-dap", cwd: "/work/w1", program: "/work/w1/app", status: "running" };
const stopped = {
  ...running,
  status: "stopped",
  stopReason: "breakpoint",
  frameName: "main",
  source: { path: "src/main.c" },
  line: 42,
};

function setup(select = true) {
  const base = testHost();
  if (select) {
    base.host.window.set(SELECTED_THREAD, "t1");
  }
  const test = renderWithHost(<DebugWidget />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "one", "w1")] });
  test.client.thread("t1", []);
  test.flush();
  return { ...test, user: userEvent.setup() };
}

describe("omfp:debug", () => {
  // spec: UI-946
  test("without a selected thread, or before the agent debugs, an empty state", () => {
    setup(false);
    expect(screen.getByTestId("debug-no-thread").textContent).toContain("No thread selected");
    cleanup();
    setup();
    expect(screen.getByTestId("debug-none").textContent).toContain("No debug session");
  });

  // spec: UI-946, UI-947, UI-948, UI-949, UI-119
  test("follows the agent's debug results: the stop, its stack and variables, the breakpoints; places open files", async () => {
    const test = setup();
    test.client.threadEvents(
      "t1",
      ...debugCall("c1", { action: "launch", program: "app" }, { action: "launch", snapshot: running }),
      ...debugCall(
        "c2",
        { action: "set_breakpoint", file: "src/main.c", line: 42 },
        { action: "set_breakpoint", snapshot: running, breakpoints: [{ id: 1, verified: true, line: 42 }] },
      ),
    );
    test.flush();
    const session = screen.getByTestId("debug-session");
    expect(within(session).getByRole("heading", { level: 2, name: "/work/w1/app" })).toBeDefined();
    expect(session.querySelector(".omfp-bbox__end")?.textContent).toBe("running");
    expect(screen.queryByTestId("debug-stop")).toBeNull();
    expect(screen.getByTestId("debug-stack").textContent).toContain("has not read the stack");
    expect(screen.getAllByTestId("debug-breakpoint").map((row) => row.textContent)).toEqual(["src/main.c:42"]);

    test.client.threadEvents(
      "t1",
      ...debugCall("c3", { action: "continue" }, { action: "continue", snapshot: stopped }),
      ...debugCall(
        "c4",
        { action: "stack_trace" },
        {
          action: "stack_trace",
          snapshot: stopped,
          stackFrames: [{ id: 1, name: "main", source: { path: "src/main.c" }, line: 42, column: 3 }],
        },
      ),
      ...debugCall(
        "c5",
        { action: "variables", scope_id: 2 },
        { action: "variables", snapshot: stopped, variables: [{ name: "n", value: "7", type: "int" }] },
      ),
    );
    test.flush();
    expect(screen.getByTestId("debug-session").querySelector(".omfp-bbox__end")?.textContent).toBe("stopped");
    expect(screen.getByTestId("debug-stop").textContent).toBe("Stopped at src/main.c:42 (main) on breakpoint");
    expect(screen.getAllByTestId("debug-frame").map((row) => row.textContent)).toEqual(["main src/main.c:42"]);
    expect(screen.getAllByTestId("debug-variable").map((row) => row.textContent)).toEqual(["n: int7"]);

    await test.user.click(within(screen.getByTestId("debug-stop")).getByTestId("debug-place"));
    // Resolved against the session's folder `/work/w1`, the workspace's: its tab is the workspace-relative path.
    expect(test.host.window.get(OPEN_FILES).w1?.tabs.map(({ path, line }) => ({ path, line }))).toEqual([
      { path: "src/main.c", line: 42 },
    ]);
  });

  // spec: UI-946
  test("after a branch navigation it shows the restored branch, not the runs of the branch left", () => {
    const test = setup();
    const launch = debugMessages("c1", { action: "launch", program: "app" }, { action: "launch", snapshot: running });
    const step = debugMessages("c2", { action: "continue" }, { action: "continue", snapshot: stopped });
    const result = step[1];
    test.client.threadEvents(
      "t1",
      ...ends(launch),
      {
        kind: "omp",
        frame: { type: "tool_execution_start", toolCallId: "c2", toolName: "debug", args: {} },
      } as ThreadEvent,
      {
        kind: "omp",
        frame: { type: "tool_execution_end", toolCallId: "c2", toolName: "debug", result, isError: false },
      } as ThreadEvent,
      ...ends(step),
    );
    test.flush();
    expect(screen.getByTestId("debug-session").querySelector(".omfp-bbox__end")?.textContent).toBe("stopped");
    // Back on the branch before `continue`: the reducer keeps the completed tool run, the transcript drops it.
    test.client.threadEvents("t1", {
      kind: "history",
      session: { session_id: "S", resumed: true },
      messages: launch,
    } as ThreadEvent);
    test.flush();
    expect(screen.getByTestId("debug-session").querySelector(".omfp-bbox__end")?.textContent).toBe("running");
    expect(screen.queryByTestId("debug-stop")).toBeNull();
  });
});
