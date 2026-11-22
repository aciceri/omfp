/** L4: reordering `omfp:threads`, the left column — the daemon's order, the pointer drag, the move keys. */

import { afterEach, describe, expect, test } from "bun:test";
import type { IndexSnapshot } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ERRORS } from "../../src/sdk/index.ts";
import { ThreadsWidget } from "../../src/widgets/threads/index.ts";
import { LONG_PRESS_MS } from "../../src/widgets/threads/reorder.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost } from "../support/host.tsx";

afterEach(cleanup);

/** `checkout` (t1, t4) above `scratch` (t2). */
const INDEX: Partial<IndexSnapshot> = {
  workspaces: [
    { ...workspace("w1"), name: "checkout", position: 0 },
    { ...workspace("w2"), name: "scratch", position: 1 },
  ],
  threads: [
    thread("t1", "checkout-v2", "w1", { position: 0 }),
    thread("t2", "flaky", "w2", { position: 0 }),
    thread("t4", "payments", "w1", { position: 1 }),
  ],
};

function setup(index: Partial<IndexSnapshot> = INDEX) {
  const test = renderWithHost(<ThreadsWidget />);
  test.client.answers.set("threads.search", () => ({ results: [] }));
  test.client.connected();
  test.client.index(index);
  test.flush();
  return { ...test, user: userEvent.setup() };
}

function rows(): string[] {
  return screen.queryAllByTestId("thread-row").map((row) => row.dataset.threadId ?? "");
}

function sectionIds(): string[] {
  return screen.queryAllByTestId("workspace-section").map((candidate) => candidate.dataset.workspaceId ?? "");
}

function section(workspaceId: string): HTMLElement {
  const found = screen.getAllByTestId("workspace-section").find((each) => each.dataset.workspaceId === workspaceId);
  if (found === undefined) {
    throw new Error(`no section ${workspaceId}`);
  }
  return found;
}

/** The select button of thread `id`. */
function row(id: string): HTMLElement {
  const found = screen.getAllByTestId("thread-row").find((each) => each.dataset.threadId === id);
  if (found === undefined) {
    throw new Error(`no row ${id}`);
  }
  return found;
}

/** The list item of thread `id`. */
function item(id: string): HTMLElement {
  return row(id).closest("li") ?? document.body;
}

function focusedThread(): string | null {
  return document.activeElement?.getAttribute("data-thread-id") ?? null;
}

function rect(top: number, height: number): DOMRect {
  return { top, bottom: top + height, left: 0, right: 200, width: 200, height, x: 0, y: top, toJSON: () => ({}) };
}

/**
 * Lays the column out for the pointer, top to bottom: each section a 20 px head, its thread rows of 20 px, a 20 px
 * "+ thread", then a 10 px gap — `checkout` 0–80 (t1 20–40, t4 40–60), `scratch` 90–150 (t2 110–130).
 */
function layout(): void {
  let y = 0;
  for (const each of screen.getAllByTestId("workspace-section")) {
    const top = y;
    y += 20;
    for (const listed of each.querySelectorAll<HTMLElement>("[data-reorder-thread]")) {
      const rowTop = y;
      listed.getBoundingClientRect = () => rect(rowTop, 20);
      y += 20;
    }
    y += 20;
    const bottom = y;
    each.getBoundingClientRect = () => rect(top, bottom - top);
    y += 10;
  }
}

/** A pointer of `kind` at `y`. */
function pointer(y: number, kind = "mouse") {
  return { pointerId: 1, isPrimary: true, button: 0, pointerType: kind, clientX: 10, clientY: y };
}

/** Presses `element` with the mouse at `from`, moves to `to` and, unless `release` is false, releases there. */
function drag(element: HTMLElement, from: number, to: number, release = true): void {
  layout();
  fireEvent.pointerDown(element, pointer(from));
  fireEvent.pointerMove(window, pointer(from + 6));
  fireEvent.pointerMove(window, pointer(to));
  if (release) {
    fireEvent.pointerUp(window, pointer(to));
  }
}

describe("omfp:threads reordering", () => {
  // spec: UI-620
  test("sections and rows follow the daemon's positions, in every window as they change", () => {
    const { client, flush } = setup();
    expect(sectionIds()).toEqual(["w1", "w2"]);
    expect(rows()).toEqual(["t1", "t4", "t2"]);
    client.indexEvents(
      { kind: "workspace_upserted", workspace: { ...workspace("w2"), name: "scratch", position: 0 } },
      { kind: "workspace_upserted", workspace: { ...workspace("w1"), name: "checkout", position: 1 } },
      { kind: "thread_upserted", thread: thread("t4", "payments", "w1", { position: 0 }) },
      { kind: "thread_upserted", thread: thread("t1", "checkout-v2", "w1", { position: 1 }) },
    );
    flush();
    expect(sectionIds()).toEqual(["w2", "w1"]);
    expect(rows()).toEqual(["t2", "t4", "t1"]);
  });

  // spec: UI-621, UI-627, PRT-171
  test("a dragged row shows where it lands and drops before another row of its workspace", async () => {
    const { client } = setup();
    client.answers.set("threads.reorder", () => ({}));
    drag(row("t4"), 50, 25, false);
    expect(screen.getByTestId("sidebar").dataset.reordering).toBe("thread");
    expect(item("t4").dataset.dragging).toBe("true");
    expect(item("t1").dataset.drop).toBe("before");
    expect(section("w1").dataset.dropInto).toBe("true");
    fireEvent.pointerUp(window, pointer(25));
    expect(client.callsTo("threads.reorder").map((call) => call.params)).toEqual([{ threadId: "t4", beforeId: "t1" }]);
    expect(item("t1").dataset.drop).toBeUndefined();
    expect(screen.getByTestId("sidebar").dataset.reordering).toBeUndefined();
    // The click ending the drag selects nothing.
    await waitFor(() => expect(row("t4").getAttribute("aria-current")).toBeNull());
  });

  // spec: UI-621, UI-627, PRT-171
  test("a row dropped on another workspace moves the thread there; past the last row it goes last", () => {
    const { client } = setup();
    client.answers.set("threads.reorder", () => ({}));
    drag(row("t4"), 50, 115, false);
    expect(section("w2").dataset.dropInto).toBe("true");
    expect(item("t2").dataset.drop).toBe("before");
    fireEvent.pointerUp(window, pointer(115));
    drag(row("t1"), 30, 140, false);
    const list = item("t2").closest("ul") ?? document.body;
    expect(list.dataset.drop).toBe("end");
    fireEvent.pointerUp(window, pointer(140));
    expect(client.callsTo("threads.reorder").map((call) => call.params)).toEqual([
      { threadId: "t4", workspaceId: "w2", beforeId: "t2" },
      { threadId: "t1", workspaceId: "w2" },
    ]);
  });

  // spec: UI-621
  test("Escape drops nothing, nor does a drop back in place; a press that does not move selects the row", async () => {
    const { client, user } = setup();
    drag(row("t4"), 50, 25, false);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(item("t1").dataset.drop).toBeUndefined();
    fireEvent.pointerUp(window, pointer(25));
    drag(row("t1"), 30, 45);
    expect(client.callsTo("threads.reorder")).toEqual([]);
    await user.click(row("t4"));
    expect(row("t4").getAttribute("aria-current")).toBe("true");
  });

  // spec: UI-622, UI-627, PRT-170
  test("a section dragged by its head drops before another section", () => {
    const { client } = setup();
    client.answers.set("workspaces.reorder", () => ({}));
    drag(within(section("w2")).getByTestId("workspace-section-name"), 100, 10, false);
    expect(section("w2").dataset.dragging).toBe("true");
    expect(section("w1").dataset.drop).toBe("before");
    fireEvent.pointerUp(window, pointer(10));
    // Not from the head's buttons: the menu stays a menu.
    drag(within(section("w1")).getByTestId("delete-workspace"), 10, 200);
    expect(client.callsTo("workspaces.reorder").map((call) => call.params)).toEqual([
      { workspaceId: "w2", beforeId: "w1" },
    ]);
  });

  // spec: UI-623
  test("a finger drags after a long press; moving sooner scrolls instead", async () => {
    const { client } = setup();
    client.answers.set("threads.reorder", () => ({}));
    layout();
    // Moving before the long press is a scroll: the page may scroll, nothing drags.
    fireEvent.pointerDown(row("t4"), pointer(50, "touch"));
    fireEvent.pointerMove(window, pointer(30, "touch"));
    const scroll = new Event("touchmove", { cancelable: true });
    window.dispatchEvent(scroll);
    expect(scroll.defaultPrevented).toBe(false);
    expect(screen.getByTestId("sidebar").dataset.reordering).toBeUndefined();
    fireEvent.pointerUp(window, pointer(30, "touch"));

    fireEvent.pointerDown(row("t4"), pointer(50, "touch"));
    await waitFor(() => expect(screen.getByTestId("sidebar").dataset.reordering).toBe("thread"), {
      timeout: LONG_PRESS_MS * 3,
    });
    const menu = new Event("contextmenu", { cancelable: true });
    window.dispatchEvent(menu);
    expect(menu.defaultPrevented).toBe(true);
    const held = new Event("touchmove", { cancelable: true });
    window.dispatchEvent(held);
    expect(held.defaultPrevented).toBe(true);
    fireEvent.pointerMove(window, pointer(25, "touch"));
    fireEvent.pointerUp(window, pointer(25, "touch"));
    expect(client.callsTo("threads.reorder").map((call) => call.params)).toEqual([{ threadId: "t4", beforeId: "t1" }]);
  });

  // spec: UI-624, UI-625
  test("Alt-arrows move the focused row in its workspace or into the next one, announced, the focus kept", async () => {
    const { client, flush, host } = setup();
    client.answers.set("threads.reorder", () => ({}));
    act(() => row("t1").focus());
    act(() => {
      host.commands.run("threads.moveDown", [], ["threads"]);
    });
    expect(client.callsTo("threads.reorder").map((call) => call.params)).toEqual([{ threadId: "t1" }]);
    await waitFor(() =>
      expect(host.announcer.getSnapshot().message).toBe("Moved checkout-v2 to position 2 of 2 in checkout"),
    );
    client.indexEvents(
      { kind: "thread_upserted", thread: thread("t4", "payments", "w1", { position: 0 }) },
      { kind: "thread_upserted", thread: thread("t1", "checkout-v2", "w1", { position: 1 }) },
    );
    flush();
    expect(rows()).toEqual(["t4", "t1", "t2"]);
    expect(focusedThread()).toBe("t1");

    act(() => {
      host.commands.run("threads.moveToNextWorkspace", [], ["threads"]);
    });
    expect(client.callsTo("threads.reorder")[1]?.params).toEqual({ threadId: "t1", workspaceId: "w2", beforeId: "t2" });
    await waitFor(() =>
      expect(host.announcer.getSnapshot().message).toBe("Moved checkout-v2 to scratch, position 1 of 2"),
    );
    client.indexEvents(
      { kind: "thread_upserted", thread: thread("t1", "checkout-v2", "w2", { position: 0 }) },
      { kind: "thread_upserted", thread: thread("t2", "flaky", "w2", { position: 1 }) },
    );
    flush();
    expect(rows()).toEqual(["t4", "t1", "t2"]);
    expect(within(section("w2")).getAllByTestId("thread-row")[0]?.dataset.threadId).toBe("t1");
    expect(focusedThread()).toBe("t1");
    // At the top already: nothing to do.
    act(() => {
      host.commands.run("threads.moveUp", [], ["threads"]);
    });
    expect(client.callsTo("threads.reorder").length).toBe(2);
  });

  // spec: UI-624, UI-625
  test("Alt-arrows on a section's head move the workspace", async () => {
    const { client, host } = setup();
    client.answers.set("workspaces.reorder", () => ({}));
    act(() => within(section("w1")).getByTestId("workspace-collapse").focus());
    act(() => {
      host.commands.run("threads.moveDown", [], ["threads"]);
    });
    expect(client.callsTo("workspaces.reorder").map((call) => call.params)).toEqual([{ workspaceId: "w1" }]);
    await waitFor(() =>
      expect(host.announcer.getSnapshot().message).toBe("Moved workspace checkout to position 2 of 2"),
    );
  });

  // spec: UI-626, UI-625
  test("while a filter is typed nothing drags or moves, and a refused move goes to the errors", async () => {
    const { client, host, user } = setup();
    await user.type(screen.getByTestId("thread-filter"), "pay");
    drag(row("t4"), 50, 10);
    act(() => row("t4").focus());
    act(() => {
      host.commands.run("threads.moveUp", [], ["threads"]);
    });
    expect(host.announcer.getSnapshot().message).toBe("Clear the filter to reorder");
    expect(client.callsTo("threads.reorder")).toEqual([]);
    await user.clear(screen.getByTestId("thread-filter"));

    act(() => row("t4").focus());
    act(() => {
      host.commands.run("threads.moveUp", [], ["threads"]);
    });
    act(() => client.callsTo("threads.reorder")[0]?.reject(new Error("thread t1 is not in workspace w1")));
    await waitFor(() =>
      expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["thread t1 is not in workspace w1"]),
    );
  });
});
