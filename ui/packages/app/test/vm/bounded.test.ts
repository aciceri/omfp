/** L0: how much of a long tool-card body shows, live and settled (UI-935). */

import { describe, expect, test } from "bun:test";
import type { DiffLine } from "@omfp/kit";
import { ansiLines } from "../../src/vm/ansi.ts";
import {
  AGENT_ITEMS,
  ANSWER_LINES,
  BODY_LINES,
  bodyCut,
  DIFF_HUNKS,
  DIFF_LINES,
  diffCut,
  earlierLines,
  follows,
  LIST_ITEMS,
  LOCATION_ITEMS,
  liveBody,
  MATCH_ITEMS,
  QUESTION_ITEMS,
  READ_LINES,
  textLines,
  todoShown,
  wheelScrolls,
} from "../../src/vm/bounded.ts";
import { card, composingCard, streaming } from "../support/cards.ts";

/** A diff of `hunks` hunks of `lines` added lines each. */
function diff(hunks: number, lines: number): DiffLine[] {
  return Array.from({ length: hunks }, (_, hunk) => [
    { kind: "hunk" as const, text: `@@ -${hunk * 10 + 1} +${hunk * 10 + 1} @@` },
    ...Array.from({ length: lines }, (_, at) => ({ kind: "add" as const, text: `line ${at}`, newLine: at + 1 })),
  ]).flat();
}

describe("bounded bodies", () => {
  // spec: UI-935
  test("omp's limits: 12 lines, a diff's 40 lines and 8 hunks, a read's 10 lines, each list's items", () => {
    expect([BODY_LINES, DIFF_LINES, DIFF_HUNKS, READ_LINES, ANSWER_LINES]).toEqual([12, 40, 8, 10, 4]);
    expect([LIST_ITEMS, MATCH_ITEMS, LOCATION_ITEMS, QUESTION_ITEMS, AGENT_ITEMS]).toEqual([8, 6, 2, 3, 4]);
  });

  // spec: UI-935
  test("a text's lines: a trailing newline ends the last one", () => {
    expect(textLines("a\nb\n")).toEqual(["a", "b"]);
    expect(textLines("a\n\nb")).toEqual(["a", "", "b"]);
    expect(textLines("")).toEqual([""]);
  });

  // spec: UI-935
  test("an output's lines keep the styles set on earlier lines", () => {
    expect(ansiLines("a\n\u001b[31mb\nc\u001b[0m d\n")).toEqual([
      [{ text: "a", style: [] }],
      [{ text: "b", style: ["fg-red"] }],
      [
        { text: "c", style: ["fg-red"] },
        { text: " d", style: [] },
      ],
    ]);
    expect(ansiLines("")).toEqual([[]]);
  });

  // spec: UI-935
  test("a settled body's preview: its first lines or items, an output's last ones; none when it fits", () => {
    expect(bodyCut(12)).toBeUndefined();
    expect(bodyCut(30)).toEqual({ start: 0, end: 12 });
    expect(bodyCut(30, BODY_LINES, "tail")).toEqual({ start: 18, end: 30 });
    expect(bodyCut(3, LOCATION_ITEMS)).toEqual({ start: 0, end: 2 });
  });

  // spec: UI-935
  test("a long todo phase shows its tasks in progress, the open ones after them and the last one closed", () => {
    const statuses = (done: number, open: number) => [
      ...Array.from({ length: done }, () => "completed"),
      ...Array.from({ length: open }, () => "pending"),
    ];
    expect(todoShown(statuses(2, 6))).toBeUndefined();
    // Nothing open: the first 8.
    expect(todoShown(statuses(10, 0))).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
    // Five done, then a task in progress and ten pending: the last done, the active one and the next seven.
    const working = statuses(5, 11);
    working[5] = "in_progress";
    expect(todoShown(working)).toEqual([4, 5, 6, 7, 8, 9, 10, 11, 12]);
    // A task in progress far down is always shown, ahead of the pending ones before it.
    const late = statuses(0, 12);
    late[11] = "in_progress";
    expect(todoShown(late)).toEqual([11]);
  });

  // spec: UI-935
  test("a settled diff's preview: at most 40 lines and 8 hunks; none when it fits", () => {
    expect(diffCut(diff(2, 10))).toBeUndefined();
    expect(diffCut(diff(1, 60))).toEqual({ start: 0, end: 40 });
    // Ten hunks of two lines: the preview ends before the ninth hunk.
    expect(diffCut(diff(10, 2))).toEqual({ start: 0, end: 24 });
    expect(diffCut(diff(8, 2))).toBeUndefined();
  });

  // spec: UI-935
  test("a live window follows its end, stops when the user scrolls it up, follows again back at its end", () => {
    // Scrolled to its end by itself, or by the user: following.
    expect(follows(false, { top: 900, height: 1000, client: 100 }, 0)).toBe(true);
    // Its content grew under it: still where it scrolled itself, it keeps following.
    expect(follows(true, { top: 900, height: 1200, client: 100 }, 900)).toBe(true);
    // The user scrolled it up: it stops, and stays stopped while the content grows.
    expect(follows(true, { top: 400, height: 1200, client: 100 }, 900)).toBe(false);
    expect(follows(false, { top: 400, height: 1400, client: 100 }, 900)).toBe(false);
  });

  // spec: UI-935
  test("a wheel turn stays in a body it scrolls; at its edge that way it goes on to the page", () => {
    const middle = { top: 400, height: 1000, client: 100 };
    expect([wheelScrolls(middle, -40), wheelScrolls(middle, 40), wheelScrolls(middle, 0)]).toEqual([true, true, false]);
    expect(wheelScrolls({ top: 0, height: 1000, client: 100 }, -40)).toBe(false);
    expect(wheelScrolls({ top: 900, height: 1000, client: 100 }, 40)).toBe(false);
    // A body that fits scrolls nothing.
    expect(wheelScrolls({ top: 0, height: 80, client: 100 }, 40)).toBe(false);
  });

  // spec: UI-935
  test("the lines above a live window: from where it is scrolled, else all but its last 12", () => {
    expect(earlierLines(30, { top: 0, height: 0, client: 0 })).toBe(18);
    expect(earlierLines(5, { top: 0, height: 0, client: 0 })).toBe(0);
    expect(earlierLines(100, { top: 880, height: 1000, client: 120 })).toBe(88);
    expect(earlierLines(100, { top: 0, height: 100, client: 120 })).toBe(0);
  });

  // spec: UI-935
  test("a body is live while its call is written, waits or runs; settled once it ended", () => {
    expect(liveBody(composingCard("write", '{"path": "a.md", "content": "# A'))).toBe(true);
    expect(liveBody(streaming("bash_tool_call", "call_bash_1"))).toBe(true);
    expect(liveBody(card("bash_tool_call", "call_bash_1"))).toBe(false);
    expect(liveBody(card("bash_tool_error", "call_bash_1"))).toBe(false);
  });
});
