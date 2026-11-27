import { describe, expect, test } from "bun:test";
import { type AgentMessage, type AssistantContent, emptyThreadState, type ThreadState } from "@omfp/core";
import {
  type MinimapLayout,
  type MinimapPiece,
  type MinimapRow,
  type MinimapScene,
  minimapBand,
  minimapBox,
  minimapCovers,
  minimapDragScrollTop,
  minimapMetrics,
  minimapOffset,
  minimapRow,
  minimapScrollTop,
  minimapShapes,
  ScrollIntent,
  sameMinimapScene,
  wrapWords,
} from "../../src/vm/minimap.ts";
import { items, rowContent } from "../../src/vm/transcript.ts";

function stateOf(...messages: AgentMessage[]): ThreadState {
  return { ...emptyThreadState(), transcript: messages.map((message) => ({ message_id: null, message })) };
}

/** What the first row of `state` shows. */
function firstContent(state: ThreadState) {
  const [item] = items(state);
  return item === undefined ? undefined : rowContent(state, item);
}

/** How the first row of `state` is drawn. */
function firstRow(state: ThreadState): MinimapRow | undefined {
  const content = firstContent(state);
  return content === undefined ? undefined : minimapRow(content);
}

/** A layout's pieces as `[ink, top, lines, words]`, a box's words empty. */
function pieces(layout: MinimapLayout | undefined): unknown[] {
  return (layout?.pieces ?? []).map(({ ink, top, lines, words }) => [ink, top, lines, words ?? []]);
}

const read = (id: string): AssistantContent => ({
  type: "toolCall",
  id,
  name: "read",
  arguments: { path: "src/a.rs" },
});
const result = (id: string, isError = false): AgentMessage => ({
  role: "toolResult",
  toolCallId: id,
  toolName: "read",
  content: [{ type: "text", text: "text" }],
  isError,
});

describe("minimap rows", () => {
  // spec: UI-861
  test("a row's kind comes from its message: user text, assistant text and tools, tools only, a failure", () => {
    expect(firstRow(stateOf({ role: "user", content: "hello" }))?.kind).toBe("user");
    expect(
      firstRow(stateOf({ role: "assistant", content: [{ type: "text", text: "Reading" }, read("a")] }, result("a")))
        ?.kind,
    ).toBe("assistant");
    expect(firstRow(stateOf({ role: "assistant", content: [read("a")] }, result("a")))?.kind).toBe("tool");
    expect(firstRow(stateOf({ role: "assistant", content: [read("a")] }, result("a", true)))?.kind).toBe("error");
    // The same content draws the same row object, so a redraw can compare rows one by one.
    const state = stateOf({ role: "user", content: "hello" });
    const row = firstRow(state);
    expect(firstRow(state)).toBe(row);
  });

  // spec: UI-861
  test("words wrap greedily at the columns from an indent, a word longer than a line cut across lines", () => {
    expect(wrapWords("the quick brown fox jumps", 10)).toEqual([
      [0, 3, 4, 5],
      [0, 5, 6, 3],
      [0, 5],
    ]);
    expect(wrapWords("abcdefghijklmnopqrstuvwxy z", 10, 2)).toEqual([
      [2, 8],
      [2, 8],
      [2, 8],
      [2, 1, 4, 1],
    ]);
    expect(wrapWords("  ", 10)).toEqual([]);
  });

  // spec: UI-861
  test("a reply's miniature is its content: its name, wrapped words, a heading, a list, code on a background, a tool card's box and title", () => {
    const text = "## Plan\n\nRead the file first.\n\n- one\n- two\n\n```rust\nlet x = 1;\n```";
    const content = firstContent(
      stateOf({ role: "assistant", content: [{ type: "text", text }, read("a")] }, result("a")),
    );
    if (content?.type !== "message") {
      throw new Error("no message row");
    }
    const [, block] = content.view.blocks;
    const title = block?.type === "tool" ? `${block.card.name} ${block.card.summary}` : "";
    const row = minimapRow(content);
    const layout = row.layout(40);
    expect(pieces(layout)).toEqual([
      ["label", 0, 1, wrapWords(content.view.label, 40)[0]],
      ["heading", 1, 1.25, [0, 4]],
      ["text", 2.75, 1, [0, 4, 5, 3, 9, 4, 14, 6]],
      ["text", 4.25, 1, [0, 1, 2, 3]],
      ["text", 5.25, 1, [0, 1, 2, 3]],
      ["codeBlock", 6.75, 1.5, []],
      ["code", 7, 1, [1, 3, 5, 1, 7, 1, 9, 2]],
      ["toolBar", 9.25, 1.5, []],
      ["label", 9.25, 1, wrapWords(title, 40, 1)[0]],
    ]);
    // The card's real height is the DOM's: it takes the row's spare height; nothing above it is pushed.
    expect(layout.pieces.map((piece) => [piece.flex, piece.pushed])).toEqual([
      ...Array.from({ length: 7 }, () => [false, 0]),
      [true, 0],
      [false, 0],
    ]);
    expect([layout.lines, layout.flex]).toEqual([11.25, 1.5]);
    // Laid out once per number of columns.
    expect(row.layout(40)).toBe(layout);
    expect(row.layout(20)).not.toBe(layout);
  });

  // spec: UI-861
  test("the user's text keeps its line breaks; a failed card starts open with its output, in `error`", () => {
    const user = firstContent(stateOf({ role: "user", content: "first line\n\nsecond" }));
    if (user?.type !== "message") {
      throw new Error("no message row");
    }
    expect(pieces(minimapRow(user).layout(40))).toEqual([
      ["label", 0, 1, wrapWords(user.view.label, 40)[0]],
      ["text", 1, 1, [0, 5, 6, 4]],
      ["text", 3, 1, [0, 6]],
    ]);
    const failed = firstRow(stateOf({ role: "assistant", content: [read("a")] }, result("a", true)))?.layout(40);
    expect(
      failed?.pieces.filter((piece) => piece.words === undefined).map((piece) => [piece.ink, piece.lines]),
    ).toEqual([["errorBar", 2.5]]);
  });
});

const piece = (ink: MinimapPiece["ink"], top: number, words: readonly number[] | undefined, lines = 1) => ({
  ink,
  top,
  lines,
  words,
  flex: words === undefined,
  pushed: 0,
});

/** A row drawing `layout` at any columns. */
const rowOf = (kind: MinimapRow["kind"], layout: MinimapLayout): MinimapRow => ({ kind, layout: () => layout });

/**
 * A 44 px wide map (40 px of room for 40 columns: a pixel each), lines 2 px apart: a user's row of a name and two
 * words, a tool row of a card (title on it) and a line of text below it, a 2 px row of one word.
 */
const SCENE: MinimapScene = {
  rows: [
    rowOf("user", {
      pieces: [piece("label", 0, [0, 3]), piece("text", 1, [0, 4, 5, 2])],
      lines: 2.5,
      flex: 0,
    }),
    rowOf("tool", {
      pieces: [
        piece("toolBar", 0, undefined, 1.5),
        piece("label", 0, [1, 4]),
        { ...piece("text", 2, [0, 4]), pushed: 1.5 },
      ],
      lines: 3.5,
      flex: 1.5,
    }),
    rowOf("assistant", { pieces: [piece("text", 0, [0, 3])], lines: 1, flex: 0 }),
  ],
  items: [
    { start: 0, end: 160 },
    { start: 160, end: 320 },
    { start: 320, end: 336 },
  ],
  listTop: 0,
  width: 44,
  top: 0,
  height: 500,
  columns: 40,
  pitch: 2,
  matches: [1],
  current: 2,
};

/** Rounded to a thousandth, for the word positions' fractions. */
const rounded = (rects: readonly number[] | undefined) => (rects ?? []).map((value) => Math.round(value * 1000) / 1000);

describe("minimap drawing", () => {
  // spec: UI-861
  test("each row an eighth of its size with its words, its card taking the spare height, marks for matches and focus", () => {
    const shapes = minimapShapes(SCENE);
    // Tints across the map, a pixel apart once three pixels tall.
    expect(shapes.get("user")).toEqual([0, 0, 44, 19]);
    expect(shapes.get("tool")).toEqual([0, 20, 44, 19]);
    expect(shapes.get("assistant")).toEqual([0, 40, 44, 2]);
    // Words a pixel per column from the inset, in the middle of their 2 px line.
    expect(rounded(shapes.get("label"))).toEqual([2, 0.4, 3, 1.2, 3, 20.4, 4, 1.2]);
    // The card takes what the row's 19 px leave its text (12 px over its 1.5 lines): 15 px; the text below it moves.
    expect(shapes.get("toolBar")).toEqual([2, 20, 40, 15]);
    expect(rounded(shapes.get("text"))).toEqual([2, 2.4, 4, 1.2, 7, 2.4, 2, 1.2, 2, 36.4, 4, 1.2, 2, 40.4, 3, 1.2]);
    // The search match marked at the right edge, the focus at the left.
    expect(shapes.get("match")).toEqual([42, 20, 2, 20]);
    expect(shapes.get("focus")).toEqual([0, 40, 2, 2]);
  });

  // spec: UI-861, UI-863
  test("only the rows in the canvas's band are drawn, from its top, and a row's content is cut at its end", () => {
    const band = minimapShapes({ ...SCENE, top: 20, height: 22 });
    expect(band.get("user")).toBeUndefined();
    expect(band.get("tool")).toEqual([0, 0, 44, 19]);
    expect(band.get("assistant")).toEqual([0, 20, 44, 2]);
    expect(band.get("focus")).toEqual([0, 20, 2, 2]);
    // Lines 12 px apart: each row's line of text would end past the row's 19 px (at 21.6 px in the user's row, below
    // the card shrunk to a quarter of its 18 px in the tool row), so only the names and the card's title show.
    const tall = minimapShapes({ ...SCENE, pitch: 12 });
    expect(tall.get("text")).toBeUndefined();
    expect(tall.get("toolBar")).toEqual([2, 20, 40, 4.5]);
    expect(rounded(tall.get("label"))).toEqual([2, 2.4, 3, 7.2, 3, 22.4, 4, 7.2]);
  });

  // spec: UI-863
  test("a scene draws the same picture as another with the same inputs, rows and matches compared one by one", () => {
    expect(sameMinimapScene(SCENE, { ...SCENE, rows: [...SCENE.rows], matches: [1] })).toBe(true);
    expect(sameMinimapScene(SCENE, { ...SCENE, items: [...SCENE.items] })).toBe(false);
    expect(sameMinimapScene(SCENE, { ...SCENE, rows: SCENE.rows.slice(1) })).toBe(false);
    expect(sameMinimapScene(SCENE, { ...SCENE, matches: [] })).toBe(false);
    expect(sameMinimapScene(SCENE, { ...SCENE, current: undefined })).toBe(false);
    expect(sameMinimapScene(SCENE, { ...SCENE, top: 20 })).toBe(false);
    expect(sameMinimapScene(SCENE, { ...SCENE, columns: 60 })).toBe(false);
  });

  // spec: UI-861
  test("the map is as wide as the text's miniature, its text wraps at the columns its font leaves, a line an eighth tall", () => {
    expect(minimapMetrics(800, 16, 24)).toEqual({ width: 104, columns: 100, pitch: 3 });
    expect(minimapMetrics(2000, 16, 24)).toEqual({ width: 160, columns: 250, pitch: 3 });
    // No layout yet (or none read): the narrowest map, the fewest columns, a line of 1.5 ems of 16 px.
    expect(minimapMetrics(0, Number.NaN, Number.NaN)).toEqual({ width: 48, columns: 20, pitch: 3 });
  });
});

/** 10,000 px of miniature on a 500 px map: the map scrolls with the view. */
const LONG = { scrollHeight: 80_000, clientHeight: 500 };

describe("minimap box and jumps", () => {
  // spec: UI-860
  test("the map scrolls with the view once its miniature is taller than it, the box where the view is on it", () => {
    // 4,000 px fit the 500 px map at an eighth: no scroll, the box an eighth of the view.
    expect(minimapOffset({ scrollTop: 1000, scrollHeight: 4000, clientHeight: 500 }, 500)).toBe(0);
    expect(minimapBox({ scrollTop: 1000, scrollHeight: 4000, clientHeight: 500 }, 500)).toEqual({
      top: 125,
      height: 62.5,
    });
    // Taller: the map's top at the view's top, its end at the view's end, the box between.
    expect([0, 39_750, 79_500].map((scrollTop) => minimapOffset({ ...LONG, scrollTop }, 500))).toEqual([0, 4750, 9500]);
    expect([0, 39_750, 79_500].map((scrollTop) => minimapBox({ ...LONG, scrollTop }, 500))).toEqual([
      { top: 0, height: 62.5 },
      { top: 218.75, height: 62.5 },
      { top: 437.5, height: 62.5 },
    ]);
    // A short view still gets a box to see; no content, none.
    expect(minimapBox({ scrollTop: 0, scrollHeight: 4000, clientHeight: 20 }, 500)).toEqual({ top: 0, height: 6 });
    expect(minimapBox({ scrollTop: 0, scrollHeight: 0, clientHeight: 500 }, 500)).toEqual({ top: 0, height: 0 });
  });

  // spec: UI-863
  test("the canvas holds three map heights around the map's window, drawn again once the map leaves them", () => {
    expect(minimapBand({ ...LONG, scrollTop: 39_750 }, 500)).toEqual({ top: 4250, height: 1500 });
    expect(minimapBand({ ...LONG, scrollTop: 0 }, 500)).toEqual({ top: 0, height: 1500 });
    expect(minimapBand({ ...LONG, scrollTop: 79_500 }, 500)).toEqual({ top: 8500, height: 1500 });
    expect(minimapBand({ scrollTop: 0, scrollHeight: 4000, clientHeight: 500 }, 500)).toEqual({ top: 0, height: 500 });
    const band = { top: 4250, height: 1500 };
    expect(minimapCovers(band, { ...LONG, scrollTop: 39_750 }, 500)).toBe(true);
    // The map's window at 5,225..5,725, then at 5,700..6,200.
    expect(minimapCovers(band, { ...LONG, scrollTop: 43_725 }, 500)).toBe(true);
    expect(minimapCovers(band, { ...LONG, scrollTop: 47_700 }, 500)).toBe(false);
  });

  // spec: UI-862
  test("a press centres the view on the place it points at, a drag moves the box with the pointer, within the content", () => {
    const content = { scrollTop: 0, scrollHeight: 4000, clientHeight: 500 };
    expect(minimapDragScrollTop(100, 500, content)).toBe(800);
    expect(minimapDragScrollTop(218.75, 500, { ...LONG, scrollTop: 0 })).toBe(39_750);
    expect(minimapDragScrollTop(600, 500, { ...LONG, scrollTop: 0 })).toBe(79_500);
    expect(minimapDragScrollTop(-10, 500, content)).toBe(0);
    // On the scrolled map, what the press points at is where the map is scrolled to.
    expect(minimapScrollTop(250, 500, { ...LONG, scrollTop: 39_750 })).toBe(39_750);
    expect(minimapScrollTop(0, 500, { ...LONG, scrollTop: 39_750 })).toBe(37_750);
    expect(minimapScrollTop(250, 500, content)).toBe(1750);
    expect(minimapScrollTop(10, 500, content)).toBe(0);
    expect(minimapScrollTop(500, 500, content)).toBe(3500);
    expect(minimapScrollTop(100, 0, content)).toBe(0);
  });
});

describe("the user's scrolls", () => {
  // spec: UI-860
  test("the user is scrolling for half a second after their input, and while a gesture lasts", () => {
    const intent = new ScrollIntent();
    expect(intent.active(0)).toBe(false);
    // A wheel notch, a key, a command: its smooth scroll goes on for a while.
    intent.input(1000);
    expect([intent.active(1000), intent.active(1499), intent.active(1500)]).toEqual([true, true, false]);
    // A drag of the scrollbar, however long; released, its last page still comes.
    intent.hold();
    expect(intent.active(9000)).toBe(true);
    intent.release(9000, true);
    expect([intent.active(9499), intent.active(9500)]).toEqual([true, false]);
    // A touch that did not move (a tap) scrolls nothing once it ends; two at once last until both end.
    intent.hold();
    intent.hold();
    intent.release(20_000, false);
    expect(intent.active(20_000)).toBe(true);
    intent.release(20_000, false);
    expect(intent.active(20_000)).toBe(false);
    // An end with no gesture held does not cut the next one short.
    intent.release(30_000, false);
    intent.hold();
    expect(intent.active(40_000)).toBe(true);
  });

  // spec: UI-860
  test("a scroll event is the user's unless it finds the view where the app put it, or layout moved it with no input", () => {
    const intent = new ScrollIntent();
    const at = (scrollTop: number, scrollHeight: number, clientHeight: number) => ({
      scrollTop,
      scrollHeight,
      clientHeight,
    });
    // Following the bottom as a reply streams: the view is where the follow put it.
    intent.wrote(4400);
    expect(intent.scrolled(at(4400, 5000, 600), 0)).toBe(false);
    // The composer shrank once a message was sent: the view grew and the browser clamped it to the new end.
    expect(intent.scrolled(at(4300, 5000, 700), 0)).toBe(false);
    // A row measured: the content grew and the view moved with it.
    expect(intent.scrolled(at(4350, 5050, 700), 0)).toBe(false);
    // With no input, the view alone moved: a scrollbar sending the page no pointer events (WebKitGTK), the user's.
    expect(intent.scrolled(at(4000, 5050, 700), 0)).toBe(true);
    // A wheel down at the bottom, then a streamed delta followed: the view is where the follow put it.
    intent.input(1000);
    intent.wrote(4500);
    expect(intent.scrolled(at(4500, 5200, 700), 1000)).toBe(false);
    // The user's next input: their scrolls are theirs, wherever a follow had put the view and while the content grows.
    intent.input(1100);
    expect(intent.scrolled(at(4500, 5300, 700), 1100)).toBe(true);
  });
});
