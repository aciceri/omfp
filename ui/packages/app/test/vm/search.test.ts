/** L1: the conversation search's model (UI-730, UI-736): pattern, occurrences, matching blocks, visit order, reveal. */

import { describe, expect, test } from "bun:test";
import {
  matchingBlocks,
  occurrenceIndex,
  occurrences,
  revealScroll,
  revealScrollLeft,
  searchPattern,
  stepSearch,
} from "../../src/vm/search.ts";
import type { MessageView } from "../../src/vm/transcript.ts";

/** The text of every occurrence of `query` in `text`. */
function found(text: string, query: string): string[] {
  const pattern = searchPattern(query);
  return pattern === undefined ? [] : occurrences(text, pattern).map(({ start, end }) => text.slice(start, end));
}

describe("search", () => {
  // spec: UI-730, UI-736
  test("the query matches regardless of case, literally, trimmed; a blank one matches nothing", () => {
    expect(searchPattern("   ")).toBeUndefined();
    expect(found("Plan the plan, PLAN.", " plan ")).toEqual(["Plan", "plan", "PLAN"]);
    expect(found("a.b axb (a.b)", "a.b")).toEqual(["a.b", "a.b"]);
    expect(found("x[1]+y* | $z / \\w {2}?", "[1]+y* | $z / \\w {2}?")).toEqual(["[1]+y* | $z / \\w {2}?"]);
    expect(found("Über über", "ÜBER")).toEqual(["Über", "über"]);
  });

  // spec: UI-736
  test("occurrences are UTF-16 spans in order, without overlaps", () => {
    const pattern = searchPattern("aa");
    expect(pattern && occurrences("aaaa 🙂aa", pattern)).toEqual([
      { start: 0, end: 2 },
      { start: 2, end: 4 },
      { start: 7, end: 9 },
    ]);
  });

  // spec: UI-730, UI-736
  test("a message matches by its text and thinking blocks, not its tools or records", () => {
    const view: MessageView = {
      role: "assistant",
      label: "Assistant",
      error: undefined,
      steering: false,
      blocks: [
        { type: "thinking", text: "The PLAN" },
        { type: "record", summary: "plan", json: '"plan"' },
        { type: "text", text: "no match" },
        { type: "text", text: "a plan" },
      ],
    };
    const pattern = searchPattern("plan");
    expect(pattern && matchingBlocks(view, pattern)).toEqual([0, 3]);
    expect(pattern && matchingBlocks({ ...view, blocks: [] }, pattern)).toEqual([]);
  });

  // spec: UI-736
  test("a cursor's occurrence is clamped to the rendered ones, -1 being the last", () => {
    expect(occurrenceIndex(-1, 3)).toBe(2);
    expect(occurrenceIndex(1, 3)).toBe(1);
    expect(occurrenceIndex(5, 3)).toBe(2);
    expect(occurrenceIndex(0, 0)).toBe(0);
    expect(occurrenceIndex(-1, 0)).toBe(0);
  });

  // spec: UI-730, UI-736
  test("Enter visits every occurrence of a message before the next message, Shift-Enter backwards, both wrapping", () => {
    const matches = [2, 5, 9];
    // Within a row with three rendered occurrences.
    expect(stepSearch({ row: 5, occurrence: 0 }, 1, matches, 3)).toEqual({ row: 5, occurrence: 1 });
    expect(stepSearch({ row: 5, occurrence: 1 }, 1, matches, 3)).toEqual({ row: 5, occurrence: 2 });
    expect(stepSearch({ row: 5, occurrence: 2 }, 1, matches, 3)).toEqual({ row: 9, occurrence: 0 });
    expect(stepSearch({ row: 5, occurrence: -1 }, -1, matches, 3)).toEqual({ row: 5, occurrence: 1 });
    expect(stepSearch({ row: 5, occurrence: 1 }, -1, matches, 3)).toEqual({ row: 5, occurrence: 0 });
    // Shift-Enter from the first occurrence goes to the previous message's last.
    expect(stepSearch({ row: 5, occurrence: 0 }, -1, matches, 3)).toEqual({ row: 2, occurrence: -1 });
    // Wrapping at both ends.
    expect(stepSearch({ row: 9, occurrence: -1 }, 1, matches, 2)).toEqual({ row: 2, occurrence: 0 });
    expect(stepSearch({ row: 2, occurrence: 0 }, -1, matches, 1)).toEqual({ row: 9, occurrence: -1 });
    // A row whose occurrences are unknown (not rendered) or not rendered as text moves on.
    expect(stepSearch({ row: 5, occurrence: 0 }, 1, matches, undefined)).toEqual({ row: 9, occurrence: 0 });
    expect(stepSearch({ row: 5, occurrence: 0 }, 1, matches, 0)).toEqual({ row: 9, occurrence: 0 });
    // From nowhere, or from a row that no longer matches: the first or the last match.
    expect(stepSearch(undefined, 1, matches, undefined)).toEqual({ row: 2, occurrence: 0 });
    expect(stepSearch(undefined, -1, matches, undefined)).toEqual({ row: 9, occurrence: -1 });
    expect(stepSearch({ row: 4, occurrence: 1 }, 1, matches, 3)).toEqual({ row: 2, occurrence: 0 });
    expect(stepSearch({ row: 4, occurrence: 1 }, 1, [], 3)).toBeUndefined();
  });

  // spec: UI-736
  test("a visit scrolls only when the occurrence is out of view: to the message's top if it then shows, else centred", () => {
    const view = 600;
    // Shown: no scroll.
    expect(revealScroll({ top: -400, bottom: 900 }, { top: 100, bottom: 120 }, view)).toBeUndefined();
    // Below the viewport, near the message's top: the message's top goes to the viewport's top.
    expect(revealScroll({ top: 700, bottom: 1000 }, { top: 900, bottom: 920 }, view)).toBe(700);
    // Deep in a long message: the occurrence goes to the middle.
    expect(revealScroll({ top: -2000, bottom: 3000 }, { top: 1500, bottom: 1520 }, view)).toBe(1500 - 290);
    expect(revealScroll({ top: -2000, bottom: 3000 }, { top: -800, bottom: -780 }, view)).toBe(-800 - 290);
    // No rendered occurrence: the message's top, unless it shows.
    expect(revealScroll({ top: 100, bottom: 900 }, undefined, view)).toBeUndefined();
    expect(revealScroll({ top: -300, bottom: 900 }, undefined, view)).toBe(-300);
    expect(revealScroll({ top: 650, bottom: 900 }, undefined, view)).toBe(650);
  });

  // spec: UI-736
  test("a visit scrolls a wide table sideways only when the occurrence is outside it, centring it", () => {
    const table = { left: 100, right: 400 };
    expect(revealScrollLeft(table, { left: 120, right: 160 })).toBeUndefined();
    expect(revealScrollLeft(table, { left: 600, right: 640 })).toBe(500 - 130);
    expect(revealScrollLeft(table, { left: 20, right: 60 })).toBe(-80 - 130);
    expect(revealScrollLeft(table, { left: 380, right: 420 })).toBe(280 - 130);
  });
});
