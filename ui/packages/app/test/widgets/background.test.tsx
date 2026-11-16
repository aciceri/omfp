/** L4: `omfp:background` over a fake client (UI-271..UI-273, UI-490..UI-492), fed the recorded async `bash` and
 * `task` fixtures. */

import { afterEach, describe, expect, spyOn, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BACKGROUND_MEMORY } from "../../src/sdk/background.ts";
import { OPEN_SUBAGENT, REVEAL_WIDGET, SELECTED_THREAD } from "../../src/sdk/index.ts";
import { BackgroundWidget } from "../../src/widgets/background/index.ts";
import { renderWithHost, testHost } from "../support/host.tsx";
import { fixtureEvents, isFrame, summary } from "./chat/fixtures.ts";

afterEach(cleanup);

const THREAD = "01T";
const bash = fixtureEvents("async_bash_tool_call");
/** Up to the call's end: the job runs. */
const running = bash.slice(0, bash.findIndex(isFrame("tool_execution_end")) + 1);
const task = fixtureEvents("task_tool_call");
/** Up to the subagent's first progress: it runs. */
const started = task.slice(0, task.findIndex(isFrame("subagent_progress")) + 1);

const TASK = { role: "user", content: [{ type: "text", text: "Count the functions in src/lib.rs" }], timestamp: 0 };
const CALL = {
  role: "assistant",
  content: [{ type: "toolCall", id: "c1", name: "read", arguments: {}, intent: "Reading the library" }],
};
const ANSWER = { role: "assistant", content: [{ type: "text", text: "src/lib.rs defines 2 functions." }] };

function setup(threadEvents: readonly ThreadEvent[]) {
  const base = testHost();
  base.client.answers.set("thread.subagents", () => ({ subagents: [] }));
  base.client.answers.set("thread.subagent_cancel", () => ({}));
  base.client.answers.set("thread.subagent_steer", () => ({}));
  base.client.answers.set("thread.prompt", () => ({}));
  base.host.window.set(SELECTED_THREAD, THREAD);
  const test = renderWithHost(<BackgroundWidget />, base);
  test.client.connected();
  test.client.index({ threads: [summary(THREAD)] });
  test.client.thread(THREAD, threadEvents);
  test.flush();
  return { ...test, user: userEvent.setup() };
}

function rows() {
  return screen.getAllByTestId("background-row").map((row) => [row.dataset.kind, row.dataset.id, row.dataset.status]);
}

describe("omfp:background", () => {
  // spec: UI-271
  test("a running job: its command, state and last line", () => {
    setup(running);
    expect(rows()).toEqual([["job", "bg_1", "running"]]);
    expect(screen.getByTestId("background-label").textContent).toBe("sleep 1; echo bg-output-42");
    expect(screen.getByTestId("background-state").textContent).toBe("running");
    // The call's own result is omp's notice to the model: no output yet.
    expect(screen.queryByTestId("background-last-line")).toBeNull();
  });

  // spec: UI-680, UI-681
  test("a job seen a moment ago runs for no time yet, however the wall clock jumps", () => {
    // Each wall-clock reading is 181 ms after the previous one (a clock that moves under the window, or readings taken
    // at different moments): the durations run on the window's monotonic clock, from when it first saw the job.
    let wall = 1_000_000;
    const clock = spyOn(Date, "now").mockImplementation(() => {
      wall += 181;
      return wall;
    });
    try {
      setup(running);
      expect(screen.getByTestId("background-duration").textContent).toBe("0 ms");
    } finally {
      clock.mockRestore();
    }
  });

  // spec: UI-271
  test("a running job's output: its last line", () => {
    // Up to the job's completed frame: its last running frame has output.
    setup(bash.slice(0, bash.findLastIndex(isFrame("tool_execution_update"))));
    expect(screen.getByTestId("background-last-line").textContent).toBe("bg-output-42");
  });

  // spec: UI-271
  test("a finished job keeps its final output; nothing to stop", () => {
    setup(bash);
    expect(rows()).toEqual([["job", "bg_1", "completed"]]);
    expect(screen.getByTestId("background-state").textContent).toBe("done");
    expect(screen.queryByTestId("background-stop")).toBeNull();
  });

  // spec: UI-272
  test("stopping a job asks the agent with a steer and shows the request", async () => {
    const test = setup(running);
    await test.user.click(screen.getByTestId("background-stop"));
    expect(test.client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: THREAD, text: "Stop background job bg_1.", behavior: "steer" },
    ]);
    expect(screen.getByTestId("background-state").textContent).toBe("stop requested");
    expect(screen.queryByTestId("background-stop")).toBeNull();
  });

  // spec: UI-271, UI-272, UI-491
  test("a running subagent is cancelled from its row and steered from its detail", async () => {
    const test = setup(started);
    await waitFor(() => expect(rows()).toEqual([["subagent", "Counter", "running"]]));
    expect(screen.getByTestId("background-label").textContent).toBe("Count the functions in src/lib.rs");
    await test.user.click(screen.getByTestId("background-cancel"));
    expect(test.client.callsTo("thread.subagent_cancel").map((call) => call.params)).toEqual([
      { threadId: THREAD, subagentId: "Counter" },
    ]);
    await test.user.click(screen.getByRole("button", { name: "Subagent Counter" }));
    const detail = screen.getByTestId("background-subagent");
    await test.user.type(within(detail).getByTestId("subagent-steer-input"), "Only count public ones");
    await test.user.click(within(detail).getByTestId("subagent-steer"));
    expect(test.client.callsTo("thread.subagent_steer").map((call) => call.params)).toEqual([
      { threadId: THREAD, subagentId: "Counter", text: "Only count public ones" },
    ]);
    await waitFor(() =>
      expect((within(detail).getByTestId("subagent-steer-input") as HTMLInputElement).value).toBe(""),
    );
    // One Cancel per subagent: the row's.
    expect(screen.getAllByTestId("background-cancel").length).toBe(1);
    expect(within(detail).queryAllByTestId("subagent-cancel").length).toBe(0);
  });

  // spec: UI-490, UI-187, UI-081
  test("a subagent's row expands its detail in place: kind, use and its transcript, read again from `nextByte`", async () => {
    const test = setup(task);
    await waitFor(() => expect(rows()).toEqual([["subagent", "Counter", "completed"]]));
    const row = screen.getByRole("button", { name: "Subagent Counter" });
    expect(row.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryAllByTestId("background-subagent").length).toBe(0);
    await test.user.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    // In place, under its row: nothing revealed, no other widget asked to open it.
    expect(test.host.window.get(REVEAL_WIDGET)).toBeUndefined();
    expect(test.host.window.get(OPEN_SUBAGENT)).toBeUndefined();
    const detail = screen.getByTestId("background-subagent");
    expect(detail.classList.contains("omfp-bbox")).toBe(true);
    expect(detail.closest("li")?.querySelector('[data-testid="background-row"]')?.getAttribute("data-id")).toBe(
      "Counter",
    );
    expect(within(detail).getByTestId("background-subagent-use").textContent).toBe("task · 1 tool · fake/scripted");
    const reads = () => test.client.callsTo("thread.subagent_messages");
    expect(reads().map((call) => call.params)).toEqual([{ threadId: THREAD, subagentId: "Counter", fromByte: 0 }]);
    reads()[0]?.resolve({
      sessionFile: "s",
      fromByte: 0,
      nextByte: 400,
      reset: false,
      entries: [],
      messages: [TASK, CALL],
    });
    await waitFor(() => expect(within(detail).getAllByTestId("subagent-line").length).toBe(2));
    await test.user.click(within(detail).getByTestId("subagent-more"));
    expect(reads()[1]?.params).toEqual({ threadId: THREAD, subagentId: "Counter", fromByte: 400 });
    reads()[1]?.resolve({
      sessionFile: "s",
      fromByte: 400,
      nextByte: 520,
      reset: false,
      entries: [],
      messages: [ANSWER],
    });
    await waitFor(() =>
      expect(
        within(detail)
          .getAllByTestId("subagent-line")
          .map((line) => [line.dataset.role, line.textContent]),
      ).toEqual([
        ["user", "▸ taskCount the functions in src/lib.rs"],
        ["tool", "→ read: Reading the library"],
        ["assistant", "◆ subagentsrc/lib.rs defines 2 functions."],
      ]),
    );
    // A finished subagent cannot be steered.
    expect(within(detail).queryAllByTestId("subagent-steer").length).toBe(0);
    // Its row folds it again.
    await test.user.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryAllByTestId("background-subagent").length).toBe(0);
  });

  // spec: UI-492
  test("an open-subagent request for this thread expands that subagent", async () => {
    const test = setup(task);
    await waitFor(() => expect(rows()).toEqual([["subagent", "Counter", "completed"]]));
    act(() => test.host.window.set(OPEN_SUBAGENT, { threadId: "01OTHER", subagentId: "Counter", nonce: 1 }));
    expect(screen.queryAllByTestId("background-subagent").length).toBe(0);
    act(() => test.host.window.set(OPEN_SUBAGENT, { threadId: THREAD, subagentId: "Counter", nonce: 2 }));
    await waitFor(() => expect(screen.getAllByTestId("background-subagent").length).toBe(1));
    expect(screen.getByRole("button", { name: "Subagent Counter" }).getAttribute("aria-expanded")).toBe("true");
  });

  // spec: UI-271, UI-400
  test("a job's row reveals the chat when its card is not shown, also when it is mounted in a tab not selected", async () => {
    const test = setup(running);
    await test.user.click(screen.getByRole("button", { name: "Show the card of background command bg_1" }));
    expect(test.host.window.get(REVEAL_WIDGET)?.reference).toBe("omfp:chat");
    act(() => test.host.window.set(REVEAL_WIDGET, undefined));
    // Hidden as `<Activity mode="hidden">` hides a tab's content.
    const hidden = document.createElement("div");
    hidden.style.setProperty("display", "none", "important");
    hidden.innerHTML = '<div data-testid="tool-card" data-call="call_bash_bg"></div>';
    document.body.append(hidden);
    try {
      await test.user.click(screen.getByRole("button", { name: "Show the card of background command bg_1" }));
      expect(test.host.window.get(REVEAL_WIDGET)?.reference).toBe("omfp:chat");
    } finally {
      hidden.remove();
    }
  });

  // spec: UI-271
  test("nothing in the background", () => {
    setup(fixtureEvents("text_reply"));
    expect(screen.getByTestId("background-none").textContent).toContain("Nothing runs in the background");
  });

  // spec: UI-641, UI-642, UI-643
  test("finished rows: five in sight, the others under Finished (N), a failed one until opened, Clear finished", async () => {
    // Job `bg_<n>` of its own call, ending as `state`.
    const job = (events: readonly ThreadEvent[], n: number, state = "completed"): ThreadEvent[] =>
      JSON.parse(
        JSON.stringify(events)
          .replaceAll("call_bash_bg", `call_bash_bg${n}`)
          .replaceAll('"bg_1"', `"bg_${n}"`)
          .replaceAll('"state":"completed"', `"state":"${state}"`),
      );
    const finished = [2, 3, 4, 5, 6, 7, 8].flatMap((n) => job(bash, n));
    const test = setup([...job(bash, 1, "failed"), ...finished, ...job(running, 9)]);
    const section = (testId: string) =>
      screen.queryAllByTestId(testId).flatMap((list) =>
        within(list)
          .getAllByTestId("background-row")
          .map((row) => row.dataset.id),
      );
    expect(section("background-running")).toEqual(["bg_9"]);
    // The failed job stays in sight beside the five most recently finished.
    expect(section("background-recent")).toEqual(["bg_8", "bg_7", "bg_6", "bg_5", "bg_4", "bg_1"]);
    const toggle = screen.getByTestId("background-older-toggle");
    expect([toggle.textContent, toggle.getAttribute("aria-expanded")]).toEqual(["Finished (2)", "false"]);
    await test.user.click(toggle);
    expect(section("background-older")).toEqual(["bg_3", "bg_2"]);

    // Opened, the failed job joins the others.
    await test.user.click(screen.getByRole("button", { name: "Show the card of background command bg_1" }));
    expect(section("background-recent")).toEqual(["bg_8", "bg_7", "bg_6", "bg_5", "bg_4"]);
    expect(section("background-older")).toEqual(["bg_3", "bg_2", "bg_1"]);

    // Clear finished leaves what runs, and the window remembers it.
    await test.user.click(screen.getByTestId("background-clear"));
    expect(screen.getAllByTestId("background-row").map((row) => row.dataset.id)).toEqual(["bg_9"]);
    expect(screen.queryAllByTestId("background-clear").length).toBe(0);
    expect(test.host.window.get(BACKGROUND_MEMORY)[THREAD]?.cleared.length).toBe(8);
  });
});
