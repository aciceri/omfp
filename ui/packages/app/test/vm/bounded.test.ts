/** L0: how much of a long tool-card body shows, live and settled (UI-935). */

import { describe, expect, test } from "bun:test";
import type { DiffLine } from "@omfp/kit";
import { ansiLines } from "../../src/vm/ansi.ts";
import {
  BODY_LINES,
  bodyCut,
  DIFF_HUNKS,
  DIFF_LINES,
  diffCut,
  earlierLines,
  follows,
  liveBody,
  textLines,
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
  test("omp's limits: 12 lines, a diff's 40 lines and 8 hunks", () => {
    expect([BODY_LINES, DIFF_LINES, DIFF_HUNKS]).toEqual([12, 40, 8]);
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
  test("a settled body's preview: its first 12 lines, an output's last 12; none when it fits", () => {
    expect(bodyCut(12)).toBeUndefined();
    expect(bodyCut(30)).toEqual({ start: 0, end: 12 });
    expect(bodyCut(30, "tail")).toEqual({ start: 18, end: 30 });
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
