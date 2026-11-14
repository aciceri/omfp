/** L4: `omfp:activity` over a fake client (UI-241). */

import { afterEach, describe, expect, test } from "bun:test";
import type { ActivityEntry, ListActivityParams } from "@omfp/protocol";
import { act, cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { type FilePoll, SELECTED_THREAD } from "../../src/sdk/index.ts";
import { BUILTIN_WIDGETS } from "../../src/view/registry.ts";
import { DEFAULT_MANIFEST } from "../../src/view/views.ts";
import { ActivityWidget } from "../../src/widgets/activity/index.ts";
import { activityRows, withNewer, withOlder } from "../../src/widgets/activity/rows.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { FakeClient, renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

/** The host's poll, run by the test. */
class ManualPoll {
  readonly checks = new Set<() => void>();
  readonly poll: FilePoll = (check) => {
    this.checks.add(check);
    return () => {
      this.checks.delete(check);
    };
  };

  run(): void {
    act(() => {
      for (const check of [...this.checks]) {
        check();
      }
    });
  }
}

function entry(seq: number, threadId: string, method: string, params: unknown, error: string | null = null) {
  return { seq, at: 1_731_000_000_000 + seq, threadId, client: "omp", method, params, error } as ActivityEntry;
}

/** The daemon's log, newest last. */
const LOG = [
  entry(1, "t1", "jobs.start", { workspaceId: "w1", command: "make test" }),
  entry(2, "01JGONE", "thread.prompt", { threadId: "t1", text: "go" }, "thread busy"),
  entry(3, "t1", "threads.create", { workspace: { id: "w1" }, prompt: "fix it" }),
];

/** `activity.list` over `log` as the daemon answers it (PRT-086). */
function answer(log: readonly ActivityEntry[], params: ListActivityParams) {
  const matching = log
    .filter((candidate) => params.threadId === undefined || candidate.threadId === params.threadId)
    .filter((candidate) => params.before === undefined || params.before === null || candidate.seq < params.before)
    .filter((candidate) => params.after === undefined || params.after === null || candidate.seq > params.after)
    .reverse();
  const limit = params.limit ?? 2;
  return { entries: matching.slice(0, limit), more: matching.length > limit };
}

function setup(props: Record<string, unknown> = {}) {
  const poll = new ManualPoll();
  const log = [...LOG];
  const base = testHost(new FakeClient(), { filePoll: poll.poll });
  base.client.answers.set("activity.list", (params) => answer(log, params as ListActivityParams));
  const test = renderWithHost(<ActivityWidget {...props} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [{ ...workspace("w1"), name: "checkout" }],
    threads: [thread("t1", "orchestrator", "w1")],
  });
  test.flush();
  return { ...test, poll, log, user: userEvent.setup() };
}

function seqs(): number[] {
  return screen.queryAllByTestId("activity-row").map((row) => Number(row.dataset.seq));
}

function texts(testid: string): string[] {
  return screen.queryAllByTestId(testid).map((element) => element.textContent ?? "");
}

describe("omfp:activity", () => {
  // spec: UI-241, PRT-086
  test("the log newest first: thread, method, what the params name and the error; older pages on request", async () => {
    const { client, user } = setup();
    await waitFor(() => expect(seqs()).toEqual([3, 2]));
    expect(texts("activity-thread")).toEqual(["orchestrator", "01JGONE"]);
    expect(texts("activity-method")).toEqual(["threads.create", "thread.prompt"]);
    expect(texts("activity-target")).toEqual(["checkout", "orchestrator"]);
    expect(texts("activity-error")).toEqual(["failed: thread busy"]);
    expect(screen.getAllByTestId("activity-row")[1]?.dataset.failed).toBe("true");
    const [known, gone] = screen.getAllByTestId("activity-thread");
    expect(known?.hasAttribute("disabled")).toBe(false);
    expect(gone?.hasAttribute("disabled")).toBe(true);

    await user.click(screen.getByTestId("activity-older"));
    await waitFor(() => expect(seqs()).toEqual([3, 2, 1]));
    expect(texts("activity-target")[2]).toBe("checkout · make test");
    expect(screen.queryAllByTestId("activity-older").length).toBe(0);
    expect(client.callsTo("activity.list").map((call) => call.params)).toEqual([{}, { before: 2 }]);
  });

  // spec: UI-241
  test("newer entries arrive on the poll; a page that left some out starts again from the newest", async () => {
    const { client, poll, log } = setup();
    await waitFor(() => expect(seqs()).toEqual([3, 2]));
    poll.run();
    await waitFor(() => expect(client.callsTo("activity.list").length).toBe(2));
    expect(seqs()).toEqual([3, 2]);

    log.push(entry(4, "t1", "views.mount", { workspaceId: "w1" }));
    poll.run();
    await waitFor(() => expect(seqs()).toEqual([4, 3, 2]));
    expect(client.callsTo("activity.list")[2]?.params).toEqual({ after: 3 });

    log.push(entry(5, "t1", "a", {}), entry(6, "t1", "b", {}), entry(7, "t1", "c", {}));
    poll.run();
    await waitFor(() => expect(seqs()).toEqual([7, 6]));
    expect(screen.getByTestId("activity-older")).toBeDefined();
  });

  // spec: UI-241
  test("a thread's name selects it; This thread keeps the selected thread's entries", async () => {
    const { client, host, user } = setup();
    await waitFor(() => expect(seqs()).toEqual([3, 2]));
    expect(screen.getByTestId("activity-this-thread").hasAttribute("disabled")).toBe(true);
    await user.click(screen.getAllByTestId("activity-thread")[0] ?? document.body);
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");

    await user.click(screen.getByTestId("activity-this-thread"));
    await waitFor(() => expect(seqs()).toEqual([3, 1]));
    expect(client.callsTo("activity.list").at(-1)?.params).toEqual({ threadId: "t1" });
    expect(screen.getByTestId("activity-this-thread").getAttribute("aria-pressed")).toBe("true");
  });

  // spec: UI-241
  test("the threadId prop fixes the thread; an empty log says so", async () => {
    const { client } = setup({ threadId: "01JQUIET" });
    await screen.findByTestId("activity-empty");
    expect(client.callsTo("activity.list").map((call) => call.params)).toEqual([{ threadId: "01JQUIET" }]);
    expect(screen.queryAllByTestId("activity-this-thread").length).toBe(0);
  });

  // spec: UI-241, UI-300
  test("a built-in `:widget add` offers, not in the default view", () => {
    expect(BUILTIN_WIDGETS.has("omfp:activity")).toBe(true);
    expect(JSON.stringify(DEFAULT_MANIFEST)).not.toContain("omfp:activity");
  });

  // spec: UI-241
  test("pages join without repeating or skipping entries", () => {
    const [one, two, three] = LOG;
    if (one === undefined || two === undefined || three === undefined) {
      throw new Error("fixture");
    }
    const shown = { entries: [three, two], more: true };
    expect(withNewer(shown, { entries: [three], more: false })).toBe(shown);
    expect(withNewer(shown, { entries: [entry(4, "t1", "x", {})], more: true }).entries.map((e) => e.seq)).toEqual([4]);
    expect(withOlder(shown, { entries: [two, one], more: false })).toEqual({ entries: [three, two, one], more: false });
    const rows = activityRows([entry(9, "t1", "m", null)], new Map(), new Map(), "New thread", (client) => client);
    expect(rows[0]).toMatchObject({ thread: "t1", known: false, target: undefined, error: null });
  });

  // spec: UI-241, AGT-053, PRT-094
  test("an MCP client's entry names the client, with no thread to select", async () => {
    const { poll, log } = setup();
    await waitFor(() => expect(seqs()).toEqual([3, 2]));
    log.push({
      ...entry(4, "", "threads.create", { workspace: { new: {} }, title: "x" }),
      threadId: null,
      client: "claude-code",
    });
    poll.run();
    await waitFor(() => expect(seqs()).toEqual([4, 3, 2]));
    expect(texts("activity-mcp")).toEqual(["mcp: claude-code"]);
    expect(texts("activity-thread")).toEqual(["orchestrator", "01JGONE"]);
    expect(texts("activity-method")[0]).toBe("threads.create");
  });
});
