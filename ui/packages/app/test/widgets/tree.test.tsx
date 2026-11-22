/** L4: `omfp:tree` over a fake client (UI-083). */

import { afterEach, describe, expect, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { draftKey, ERRORS, SELECTED_THREAD, SUCCESSES } from "../../src/sdk/index.ts";
import { TreeWidget } from "../../src/widgets/tree/index.ts";
import { renderWithHost, testHost } from "../support/host.tsx";
import { fixtureEvents, summary } from "./chat/fixtures.ts";

afterEach(cleanup);

const THREAD = "01T";

function message(id: string, parentId: string | null, role: string, content: string, children: unknown[] = []) {
  return { entry: { id, parentId, type: "message", message: { role, content, timestamp: 0 } }, children };
}

/** Two turns, the second answer's branch and a branch from the first answer; the leaf on the second answer. */
const TREE = {
  leafId: "a2",
  tree: [
    message("u1", null, "user", "first question", [
      {
        ...message("a1", "u1", "assistant", "first answer", [
          message("u2", "a1", "user", "second question", [message("a2", "u2", "assistant", "second answer")]),
          message("u3", "a1", "user", "other question"),
        ]),
        label: "checkpoint",
      },
    ]),
  ],
};

function setup(events: readonly ThreadEvent[] = fixtureEvents("text_reply"), select = true) {
  const base = testHost();
  base.client.answers.set("thread.tree", () => TREE);
  base.client.answers.set("thread.navigate", () => ({ cancelled: false, editorText: "second question" }));
  base.client.answers.set("threads.fork", () => ({ ...summary("01F"), title: "01T (fork)" }));
  base.client.answers.set("thread.label", () => ({}));
  if (select) {
    base.host.window.set(SELECTED_THREAD, THREAD);
  }
  const test = renderWithHost(<TreeWidget />, base);
  test.client.connected();
  test.client.index({ threads: [summary(THREAD)] });
  test.client.thread(THREAD, events);
  test.flush();
  return { ...test, user: userEvent.setup() };
}

function row(id: string): HTMLElement {
  const found = screen.getAllByTestId("tree-row").find((element) => element.dataset.entry === id);
  if (found === undefined) {
    throw new Error(`no row ${id}`);
  }
  return found;
}

describe("omfp:tree", () => {
  // spec: UI-083, UI-187
  test("the session tree in a box titled on its border: the branch shown is active, the leaf says so", async () => {
    const test = setup();
    expect(test.client.callsTo("thread.tree").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
    const box = await screen.findByTestId("tree");
    expect(box.classList.contains("omfp-bbox")).toBe(true);
    expect(within(box).getByRole("heading", { level: 2, name: "Session tree" })).toBeDefined();
    expect(screen.getAllByTestId("tree-row").map((element) => element.dataset.entry)).toEqual([
      "u1",
      "a1",
      "u2",
      "a2",
      "u3",
    ]);
    expect(screen.getAllByTestId("tree-row").filter((element) => element.dataset.active !== undefined).length).toBe(4);
    expect(row("a2").dataset.leaf).toBe("true");
    expect(within(row("a2")).queryByTestId("tree-navigate")).toBeNull();
    expect(within(row("a1")).getByTestId("tree-label").textContent).toBe("checkpoint");
    // Only branches the transcript does not show offer to summarize what is left.
    expect(within(row("u3")).getByTestId("tree-summarize")).toBeDefined();
    expect(within(row("u1")).queryByTestId("tree-summarize")).toBeNull();
  });

  // spec: UI-083, UI-520
  test("going to a user message puts its text in the composer, after what the draft already had", async () => {
    const test = setup();
    test.host.window.set(draftKey(THREAD), { text: "half typed", images: [], pastes: [] });
    await test.user.click(within(await screen.findByTestId("tree")).getAllByTestId("tree-navigate")[2] as HTMLElement);
    expect(test.client.callsTo("thread.navigate").map((call) => call.params)).toEqual([
      { threadId: THREAD, entryId: "u2", summarize: false },
    ]);
    await waitFor(() => expect(test.host.window.get(draftKey(THREAD)).text).toBe("half typed\n\nsecond question"));
    // The tree is read again after the move.
    await waitFor(() => expect(test.client.callsTo("thread.tree").length).toBe(2));
    // The move is the user's action: a success toast says what happened.
    expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual([
      "Moved in the session tree; the message is back in the composer",
    ]);

    await test.user.click(within(row("u3")).getByTestId("tree-summarize"));
    expect(test.client.callsTo("thread.navigate").at(-1)?.params).toEqual({
      threadId: THREAD,
      entryId: "u3",
      summarize: true,
    });
  });

  // spec: UI-083, UI-520, UI-662, UI-666
  test("fork a new thread from an entry, shown with a success toast", async () => {
    const test = setup();
    await test.user.click(within(await screen.findByTestId("tree")).getAllByTestId("tree-fork")[1] as HTMLElement);
    expect(test.client.callsTo("threads.fork").map((call) => call.params)).toEqual([
      { threadId: THREAD, entryId: "a1" },
    ]);
    expect(test.client.callsTo("thread.fork")).toEqual([]);
    await waitFor(() => expect(test.host.window.get(SELECTED_THREAD)).toBe("01F"));
    expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual([
      "Forked into \u201c01T (fork)\u201d",
    ]);
  });

  // spec: UI-083
  test("label an entry; an empty label removes it", async () => {
    const test = setup();
    await screen.findByTestId("tree");
    await test.user.click(within(row("u1")).getByTestId("tree-label-edit"));
    await test.user.type(within(row("u1")).getByTestId("tree-label-input"), "start");
    await test.user.click(within(row("u1")).getByTestId("tree-label-save"));
    await test.user.click(within(row("a1")).getByTestId("tree-label-edit"));
    await test.user.clear(within(row("a1")).getByTestId("tree-label-input"));
    await test.user.click(within(row("a1")).getByTestId("tree-label-save"));
    expect(test.client.callsTo("thread.label").map((call) => call.params)).toEqual([
      { threadId: THREAD, entryId: "u1", label: "start" },
      { threadId: THREAD, entryId: "a1" },
    ]);
  });

  // spec: UI-083, DMN-070
  test("a stopped thread's tree is read only on request; without a thread, an empty state", async () => {
    const stopped = fixtureEvents("text_reply").filter((event) => event.kind !== "process");
    const test = setup(stopped);
    expect(test.client.callsTo("thread.tree")).toEqual([]);
    await test.user.click(screen.getByTestId("tree-load"));
    expect(test.client.callsTo("thread.tree").length).toBe(1);
    expect(await screen.findByTestId("tree")).toBeDefined();
    cleanup();
    setup(fixtureEvents("text_reply"), false);
    expect(screen.getByTestId("tree-no-thread").textContent).toContain("No thread selected");
  });

  // spec: UI-663
  test(":fork forks from the focused row of the tree, and without one says what to select", async () => {
    const test = setup();
    await screen.findByTestId("tree");
    within(row("u2")).getByTestId("tree-label-edit").focus();
    test.host.commands.run("thread.fork", [], ["tree"]);
    await waitFor(() =>
      expect(test.client.callsTo("threads.fork").map((call) => call.params)).toEqual([
        { threadId: THREAD, entryId: "u2" },
      ]),
    );
    (document.activeElement as HTMLElement | null)?.blur();
    test.host.commands.run("thread.fork", [], ["tree"]);
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["Select a message of the tree to fork from"]);
  });
});
