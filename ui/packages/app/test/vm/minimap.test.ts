import { describe, expect, test } from "bun:test";
import { type AgentMessage, type AssistantContent, emptyThreadState, type ThreadState } from "@omfp/core";
import {
  type MinimapRow,
  type MinimapScene,
  minimapBox,
  minimapRow,
  minimapScrollTop,
  minimapShapes,
  ScrollIntent,
  sameMinimapScene,
} from "../../src/vm/minimap.ts";
import { items, rowContent } from "../../src/vm/transcript.ts";

function stateOf(...messages: AgentMessage[]): ThreadState {
  return { ...emptyThreadState(), transcript: messages.map((message) => ({ message_id: null, message })) };
}

/** How the first row of `state` is drawn. */
function firstRow(state: ThreadState): MinimapRow | undefined {
  const [item] = items(state);
  const content = item === undefined ? undefined : rowContent(state, item);
  return content === undefined ? undefined : minimapRow(content);
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
  test("a row's kind and parts come from its message: user text, assistant text and tools, tools only, a failure", () => {
    expect(firstRow(stateOf({ role: "user", content: "hello" }))).toEqual({ kind: "user", parts: ["text"] });
    expect(
      firstRow(stateOf({ role: "assistant", content: [{ type: "text", text: "Reading" }, read("a")] }, result("a"))),
    ).toEqual({ kind: "assistant", parts: ["text", "tool"] });
    expect(firstRow(stateOf({ role: "assistant", content: [read("a")] }, result("a")))).toEqual({
      kind: "tool",
      parts: ["tool"],
    });
    expect(firstRow(stateOf({ role: "assistant", content: [read("a")] }, result("a", true)))).toEqual({
      kind: "error",
      parts: ["error"],
    });
    // The same content draws the same row object, so a redraw can compare rows one by one.
    const state = stateOf({ role: "user", content: "hello" });
    const row = firstRow(state);
    expect(firstRow(state)).toBe(row);
  });
});

/** A 40 px wide map, 500 px tall, of a 1000 px content: every length halves. */
const SCENE: MinimapScene = {
  rows: [
    { kind: "user", parts: ["text"] },
    { kind: "tool", parts: ["tool", "error"] },
    { kind: "assistant", parts: ["text"] },
  ],
  items: [
    { start: 0, end: 40 },
    { start: 40, end: 80 },
    { start: 80, end: 84 },
  ],
  listTop: 0,
  scrollHeight: 1000,
  width: 40,
  height: 500,
  matches: [1],
  current: 2,
};

describe("minimap drawing", () => {
  // spec: UI-861
  test("a block per row, faded lines for text and bars for tools when the row is tall enough, marks for matches and focus", () => {
    const shapes = minimapShapes(SCENE);
    // Blocks across the map, a pixel apart once three pixels tall.
    expect(shapes.get("user")).toEqual([0, 0, 40, 19]);
    expect(shapes.get("tool")).toEqual([0, 20, 40, 19]);
    expect(shapes.get("assistant")).toEqual([0, 40, 40, 2]);
    // A text part: a 1 px line every 2 px, inset from the left edge, of ragged lengths.
    const lines = shapes.get("line") ?? [];
    expect(lines.length).toBe(9 * 4);
    expect([lines[0], lines[1], lines[3], lines[4], lines[5], lines[7]]).toEqual([2, 0, 1, 2, 2, 1]);
    expect(lines[2]).toBeCloseTo(36 * 0.92);
    // Tool parts share the row: a bar each, a failed one apart.
    expect(shapes.get("toolBar")).toEqual([2, 20, 36, 8.5]);
    expect(shapes.get("errorBar")).toEqual([2, 29.5, 36, 8.5]);
    // The 2 px row is a plain block; the search match marked at the right edge, the focus at the left.
    expect(shapes.get("match")).toEqual([38, 20, 2, 20]);
    expect(shapes.get("focus")).toEqual([0, 40, 2, 2]);
  });

  // spec: UI-863
  test("a scene draws the same picture as another with the same inputs, rows and matches compared one by one", () => {
    expect(sameMinimapScene(SCENE, { ...SCENE, rows: [...SCENE.rows], matches: [1] })).toBe(true);
    expect(sameMinimapScene(SCENE, { ...SCENE, items: [...SCENE.items] })).toBe(false);
    expect(sameMinimapScene(SCENE, { ...SCENE, rows: SCENE.rows.slice(1) })).toBe(false);
    expect(sameMinimapScene(SCENE, { ...SCENE, matches: [] })).toBe(false);
    expect(sameMinimapScene(SCENE, { ...SCENE, current: undefined })).toBe(false);
    expect(sameMinimapScene(SCENE, { ...SCENE, scrollHeight: 1200 })).toBe(false);
  });
});

describe("minimap box and jumps", () => {
  // spec: UI-860
  test("the viewport's box is where the view is, scaled, and at least 6 px tall within the map", () => {
    expect(minimapBox({ scrollTop: 1000, scrollHeight: 4000, clientHeight: 500 }, 500)).toEqual({
      top: 125,
      height: 62.5,
    });
    // 10,000 messages: a sliver of a view, centred on it.
    const long = { scrollHeight: 1_000_000, clientHeight: 500 };
    expect(minimapBox({ ...long, scrollTop: 0 }, 500)).toEqual({ top: 0, height: 6 });
    expect(minimapBox({ ...long, scrollTop: 500_000 }, 500)).toEqual({ top: 247.125, height: 6 });
    expect(minimapBox({ ...long, scrollTop: 999_500 }, 500)).toEqual({ top: 494, height: 6 });
    expect(minimapBox({ scrollTop: 0, scrollHeight: 0, clientHeight: 500 }, 500)).toEqual({ top: 0, height: 0 });
  });

  // spec: UI-862
  test("a press centres the view on the place it points at, within the content", () => {
    const content = { scrollHeight: 4000, clientHeight: 500 };
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
