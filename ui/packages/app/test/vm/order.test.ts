/** L1: the order of the left column — sections by position, drop targets, the move keys, params, announcements. */

import { describe, expect, test } from "bun:test";
import {
  announcement,
  isNoop,
  keyTarget,
  placed,
  type ReorderSource,
  reorderCall,
  type SectionBox,
  threadTargetAt,
  workspaceTargetAt,
} from "../../src/vm/order.ts";
import { sidebarSections } from "../../src/vm/threads.ts";
import { NO_FILTER, thread, workspace } from "../support/fixtures.ts";

/** `shop` (t1, t2, t3, an archived one), then `docs` (d1), then the empty `misc`. */
const SECTIONS = sidebarSections(
  [
    thread("t3", "three", "shop", { position: 3 }),
    thread("t1", "one", "shop", { position: 0 }),
    thread("old", "old", "shop", { position: 1, archivedAt: 5 }),
    thread("t2", "two", "shop", { position: 2 }),
    thread("d1", "doc", "docs", { position: 0 }),
  ],
  [
    { ...workspace("misc"), position: 2 },
    { ...workspace("shop"), position: 0 },
    { ...workspace("docs"), position: 1 },
  ],
  NO_FILTER,
);

const T1: ReorderSource = { kind: "thread", id: "t1", workspaceId: "shop" };
const T2: ReorderSource = { kind: "thread", id: "t2", workspaceId: "shop" };
const T3: ReorderSource = { kind: "thread", id: "t3", workspaceId: "shop" };
const SHOP: ReorderSource = { kind: "workspace", id: "shop" };
const DOCS: ReorderSource = { kind: "workspace", id: "docs" };
const MISC: ReorderSource = { kind: "workspace", id: "misc" };

describe("order", () => {
  // spec: UI-620
  test("sections follow the workspaces' positions, threads theirs; equal positions keep the names' order", () => {
    expect(SECTIONS.map((section) => [section.workspace.id, section.threadIds, section.archivedIds])).toEqual([
      ["shop", ["t1", "t2", "t3"], ["old"]],
      ["docs", ["d1"], []],
      ["misc", [], []],
    ]);
    const tied = sidebarSections([], [workspace("b"), workspace("a")], NO_FILTER);
    expect(tied.map((section) => section.workspace.id)).toEqual(["a", "b"]);
  });

  // spec: UI-621, DMN-282
  test("a placed id goes just before another, or last", () => {
    expect(placed(["a", "b", "c"], "c", "a")).toEqual(["c", "a", "b"]);
    expect(placed(["a", "b", "c"], "a", undefined)).toEqual(["b", "c", "a"]);
    expect(placed(["a", "b"], "x", "b")).toEqual(["a", "x", "b"]);
  });

  // spec: UI-621
  test("a dragged thread lands before the first row below the pointer of the section under it, else last", () => {
    const boxes: SectionBox[] = [
      {
        workspaceId: "shop",
        top: 0,
        bottom: 100,
        rows: [
          { id: "t1", middle: 30 },
          { id: "t2", middle: 50 },
        ],
      },
      { workspaceId: "docs", top: 110, bottom: 160, rows: [{ id: "d1", middle: 140 }] },
      { workspaceId: "misc", top: 170, bottom: 190, rows: [] },
    ];
    expect(threadTargetAt(boxes, 10)).toEqual({ kind: "thread", workspaceId: "shop", beforeId: "t1" });
    expect(threadTargetAt(boxes, 45)).toEqual({ kind: "thread", workspaceId: "shop", beforeId: "t2" });
    expect(threadTargetAt(boxes, 80)).toEqual({ kind: "thread", workspaceId: "shop" });
    // Between two sections: the next one.
    expect(threadTargetAt(boxes, 105)).toEqual({ kind: "thread", workspaceId: "docs", beforeId: "d1" });
    expect(threadTargetAt(boxes, 150)).toEqual({ kind: "thread", workspaceId: "docs" });
    // An empty (or collapsed) section, and below every section: last of the last one.
    expect(threadTargetAt(boxes, 180)).toEqual({ kind: "thread", workspaceId: "misc" });
    expect(threadTargetAt(boxes, 500)).toEqual({ kind: "thread", workspaceId: "misc" });
    expect(threadTargetAt([], 10)).toBeUndefined();
  });

  // spec: UI-622
  test("a dragged section lands before the first section whose middle is below the pointer, else last", () => {
    const boxes: SectionBox[] = [
      { workspaceId: "docs", top: 0, bottom: 100, rows: [] },
      { workspaceId: "misc", top: 110, bottom: 130, rows: [] },
    ];
    expect(workspaceTargetAt(boxes, 40)).toEqual({ kind: "workspace", beforeId: "docs" });
    expect(workspaceTargetAt(boxes, 60)).toEqual({ kind: "workspace", beforeId: "misc" });
    expect(workspaceTargetAt(boxes, 125)).toEqual({ kind: "workspace" });
  });

  // spec: UI-621, UI-622
  test("a drop back in place does nothing; one into another workspace always moves", () => {
    expect(isNoop(SECTIONS, T1, { kind: "thread", workspaceId: "shop", beforeId: "t2" })).toBe(true);
    expect(isNoop(SECTIONS, T3, { kind: "thread", workspaceId: "shop" })).toBe(true);
    expect(isNoop(SECTIONS, T3, { kind: "thread", workspaceId: "shop", beforeId: "t2" })).toBe(false);
    expect(isNoop(SECTIONS, T3, { kind: "thread", workspaceId: "docs" })).toBe(false);
    expect(isNoop(SECTIONS, MISC, { kind: "workspace" })).toBe(true);
    expect(isNoop(SECTIONS, SHOP, { kind: "workspace", beforeId: "docs" })).toBe(true);
    expect(isNoop(SECTIONS, MISC, { kind: "workspace", beforeId: "shop" })).toBe(false);
  });

  // spec: UI-624
  test("the keys move a thread one place or into the next workspace's top or the previous one's end", () => {
    expect(keyTarget(SECTIONS, T2, "up")).toEqual({ kind: "thread", workspaceId: "shop", beforeId: "t1" });
    expect(keyTarget(SECTIONS, T1, "down")).toEqual({ kind: "thread", workspaceId: "shop", beforeId: "t3" });
    expect(keyTarget(SECTIONS, T2, "down")).toEqual({ kind: "thread", workspaceId: "shop" });
    expect(keyTarget(SECTIONS, T1, "up")).toBeUndefined();
    expect(keyTarget(SECTIONS, T3, "down")).toBeUndefined();
    expect(keyTarget(SECTIONS, T2, "nextWorkspace")).toEqual({ kind: "thread", workspaceId: "docs", beforeId: "d1" });
    const doc: ReorderSource = { kind: "thread", id: "d1", workspaceId: "docs" };
    expect(keyTarget(SECTIONS, doc, "prevWorkspace")).toEqual({ kind: "thread", workspaceId: "shop" });
    expect(keyTarget(SECTIONS, doc, "nextWorkspace")).toEqual({ kind: "thread", workspaceId: "misc" });
    expect(keyTarget(SECTIONS, T1, "prevWorkspace")).toBeUndefined();
  });

  // spec: UI-624
  test("the keys move a workspace one place, never into another workspace", () => {
    expect(keyTarget(SECTIONS, DOCS, "up")).toEqual({ kind: "workspace", beforeId: "shop" });
    expect(keyTarget(SECTIONS, SHOP, "down")).toEqual({ kind: "workspace", beforeId: "misc" });
    expect(keyTarget(SECTIONS, DOCS, "down")).toEqual({ kind: "workspace" });
    expect(keyTarget(SECTIONS, SHOP, "up")).toBeUndefined();
    expect(keyTarget(SECTIONS, MISC, "down")).toBeUndefined();
    expect(keyTarget(SECTIONS, DOCS, "nextWorkspace")).toBeUndefined();
  });

  // spec: PRT-170, PRT-171
  test("a thread names its workspace only when it changes", () => {
    expect(reorderCall(T3, { kind: "thread", workspaceId: "shop", beforeId: "t1" })).toEqual({
      method: "threads.reorder",
      params: { threadId: "t3", beforeId: "t1" },
    });
    expect(reorderCall(T3, { kind: "thread", workspaceId: "docs" })).toEqual({
      method: "threads.reorder",
      params: { threadId: "t3", workspaceId: "docs" },
    });
    expect(reorderCall(MISC, { kind: "workspace", beforeId: "shop" })).toEqual({
      method: "workspaces.reorder",
      params: { workspaceId: "misc", beforeId: "shop" },
    });
  });

  // spec: UI-625
  test("the announcement names the new place and the workspace", () => {
    expect(announcement(SECTIONS, T3, { kind: "thread", workspaceId: "shop", beforeId: "t1" }, "three")).toBe(
      "Moved three to position 1 of 3 in shop",
    );
    expect(announcement(SECTIONS, T3, { kind: "thread", workspaceId: "docs" }, "three")).toBe(
      "Moved three to docs, position 2 of 2",
    );
    expect(announcement(SECTIONS, MISC, { kind: "workspace", beforeId: "docs" }, "misc")).toBe(
      "Moved workspace misc to position 2 of 3",
    );
  });
});
