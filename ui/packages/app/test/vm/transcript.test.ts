import { describe, expect, test } from "bun:test";
import {
  type AgentMessage,
  emptyThreadState,
  type JsonObject,
  type NoticeFrame,
  reduce,
  type ThreadEvent,
  type ThreadState,
} from "@omfp/core";
import { daySeparator, items, messageView, rowContent, streamingView } from "../../src/vm/transcript.ts";
import { fixture, fixtureUntil, isFrame } from "../widgets/chat/fixtures.ts";

function kinds(state: ThreadState): string[] {
  return items(state).map((item) =>
    item.kind.type === "message" ? messageView(state, item.kind.index).role : item.kind.type,
  );
}

function withMessage(message: AgentMessage): ThreadState {
  return { ...emptyThreadState(), transcript: [{ message_id: null, message }] };
}

describe("transcript items", () => {
  // spec: UI-040
  test("a text reply is a user message and an assistant text", () => {
    const state = fixture("text_reply");
    expect(kinds(state)).toEqual(["user", "assistant"]);
    const [block, ...rest] = messageView(state, 1).blocks;
    expect(rest).toEqual([]);
    expect(block?.type).toBe("text");
    expect(block?.type === "text" && block.text.length > 0).toBe(true);
  });

  // spec: UI-050
  test("tool results shown in their card are not listed again", () => {
    const state = fixture("bash_tool_call");
    expect(state.transcript.length).toBe(4);
    expect(kinds(state)).toEqual(["user", "assistant", "assistant"]);
  });

  // spec: UI-042
  test("the streaming reply is the last item with its text so far", () => {
    const state = fixtureUntil("text_reply", isFrame("message_update", "text_end"));
    expect(kinds(state).at(-1)).toBe("streaming");
    const blocks = streamingView(state)?.blocks ?? [];
    expect(blocks.length).toBe(1);
    expect(blocks[0]?.type === "text" && blocks[0].text.length > 0).toBe(true);
  });

  // spec: UI-043
  test("notices keep their order", () => {
    const frame: NoticeFrame = { type: "unknown", data: { type: "mystery" } };
    const state: ThreadState = {
      ...fixture("text_reply"),
      notices: [
        { position: 1, frame },
        { position: 2, frame },
        { position: 9, frame },
      ],
    };
    expect(kinds(state)).toEqual(["user", "notice", "assistant", "notice", "notice"]);
    expect(items(state).map((item) => item.key)).toEqual(["m0", "n0", "m1", "n1", "n2"]);
  });

  // spec: UI-043
  test("new and retained xdev info notices stay in state but only appear while opted in", () => {
    const mounted: ThreadEvent = {
      kind: "omp",
      frame: { type: "notice", level: "info", source: "xdev", message: "xd://: mounted mcp__test" },
    };
    const unmounted: ThreadEvent = {
      kind: "omp",
      frame: { type: "notice", level: "info", source: "xdev", message: "xd://: unmounted mcp__test\nNo devices." },
    };
    const fresh = reduce(emptyThreadState(), mounted);
    const retained = reduce(fixture("text_reply"), mounted);
    const mountRow = { summary: "xd://: mounted mcp__test", text: undefined };
    const unmountRow = { summary: "xd://: unmounted mcp__test", text: "xd://: unmounted mcp__test\nNo devices." };
    for (const [state, expected] of [
      [fresh, [mountRow]],
      [retained, [mountRow]],
      [reduce(retained, unmounted), [mountRow, unmountRow]],
    ] as const) {
      const before = structuredClone(state);
      const visible = (show?: boolean) =>
        items(state, show).flatMap((item) => {
          const content = rowContent(state, item);
          return content?.type === "system" ? [content.row] : [];
        });
      expect(items(state).filter((item) => item.kind.type === "notice")).toEqual([]);
      expect(visible(false)).toEqual([]);
      expect(visible(true)).toEqual([...expected]);
      expect(visible(false)).toEqual([]);
      expect(state).toEqual(before);
    }
  });

  // spec: UI-043
  test("filtering leaves warnings, errors, other sources, generic notices and custom system records untouched", () => {
    const frames: NoticeFrame[] = [
      { type: "notice", data: { level: "warning", source: "xdev", message: "mount warning" } },
      { type: "notice", data: { level: "error", source: "xdev", message: "mount failed" } },
      { type: "notice", data: { level: "info", source: "collab", message: "joined" } },
      { type: "notice", data: { level: "info", message: "ordinary notice" } },
      { type: "unknown", data: { type: "mystery", source: "xdev", level: "info" } },
      { type: "manual_compaction_start", data: {} },
    ];
    const state: ThreadState = {
      ...withMessage({
        role: "custom",
        display: false,
        customType: "xdev-mount-notice",
        content: "<system-notice>\nDevice inventory\n</system-notice>",
      }),
      notices: frames.map((frame) => ({ position: 1, frame })),
    };
    const rows = items(state).map((item) => rowContent(state, item));
    expect(rows.map((row) => row?.type)).toEqual([
      "system",
      "notice",
      "notice",
      "notice",
      "notice",
      "notice",
      "notice",
    ]);
    expect(rows[0]).toEqual({ type: "system", row: { summary: "Device inventory", text: undefined } });
    expect(items(state, true).map((item) => rowContent(state, item))).toEqual(rows);
  });

  // spec: UI-041
  test("thinking is its own block unless it is narration", () => {
    const state = withMessage({
      role: "assistant",
      content: [
        { type: "thinking", thinking: "let me think" },
        { type: "thinking", thinking: "Here is the plan", narration: true },
        { type: "text", text: "done" },
      ],
    });
    expect(messageView(state, 0).blocks).toEqual([
      { type: "thinking", text: "let me think" },
      { type: "text", text: "Here is the plan" },
      { type: "text", text: "done" },
    ]);
  });

  // spec: UI-043
  test("other message kinds render as records with their JSON", () => {
    const view = messageView(withMessage({ role: "bashExecution", command: "ls", output: "a\nb", exitCode: 0 }), 0);
    expect([view.role, view.label]).toEqual(["record", "bash"]);
    const [block] = view.blocks;
    expect(block?.type === "record" && block.summary).toBe("ls");
    expect(block?.type === "record" && block.json.includes('"exitCode": 0')).toBe(true);
  });

  // spec: UI-043
  test("a record with nothing to summarize has no record block to expand", () => {
    const view = messageView(withMessage({ role: "custom", customType: "marker", content: "", display: true }), 0);
    expect([view.role, view.label]).toEqual(["record", "custom"]);
    expect(view.blocks).toEqual([]);
  });

  test("user messages keep their images and steering mark", () => {
    const view = messageView(
      withMessage({
        role: "user",
        steering: true,
        content: [
          { type: "text", text: "look" },
          { type: "image", data: "AAAA", mimeType: "image/jpeg" },
        ],
      }),
      0,
    );
    expect(view.steering).toBe(true);
    expect(view.blocks).toEqual([
      { type: "text", text: "look" },
      { type: "image", mime: "image/jpeg", data: "AAAA" },
    ]);
  });

  // spec: UI-472
  test("a tool result's images reach its card with alt text naming the tool and its subject; no base64 in its text", () => {
    const png = { type: "image", data: "iVBORw0KGgo=", mimeType: "image/png" } as const;
    const state: ThreadState = {
      ...emptyThreadState(),
      transcript: [
        {
          message_id: null,
          message: {
            role: "assistant",
            content: [{ type: "toolCall", id: "c1", name: "read", arguments: { path: "/w/plot.png" } }],
          },
        },
        {
          message_id: null,
          message: {
            role: "toolResult",
            toolCallId: "c1",
            toolName: "read",
            content: [{ type: "text", text: "Read image file [image/png]" }, png, { ...png, mimeType: "image/jpeg" }],
          },
        },
      ],
    };
    const [block] = messageView(state, 0).blocks;
    const card = block?.type === "tool" ? block.card : undefined;
    expect(card?.images.map((image) => [image.mime, image.data, image.alt])).toEqual([
      ["image/png", "iVBORw0KGgo=", "Image 1 of 2 from read /w/plot.png"],
      ["image/jpeg", "iVBORw0KGgo=", "Image 2 of 2 from read /w/plot.png"],
    ]);
    expect(card?.result).toBe("Read image file [image/png]\n[image]\n[image]");
  });
});

/** A user's message, an assistant turn of two tool calls (thinking first) and a reply, a steer and its answer. */
function turns(): ThreadState {
  const call = (id: string, thinking?: string): AgentMessage => ({
    role: "assistant",
    content: [
      ...(thinking === undefined ? [] : [{ type: "thinking" as const, thinking }]),
      { type: "toolCall", id, name: "eval", arguments: { code: "1" } },
    ],
  });
  const result = (id: string): AgentMessage => ({
    role: "toolResult",
    toolCallId: id,
    toolName: "eval",
    content: [{ type: "text", text: "1" }],
  });
  const messages: AgentMessage[] = [
    { role: "user", content: "plot it" },
    call("c1"),
    result("c1"),
    call("c2", "again"),
    result("c2"),
    { role: "assistant", content: [{ type: "text", text: "done" }] },
    { role: "user", steering: true, content: "bigger" },
    { role: "assistant", content: [{ type: "text", text: "ok" }] },
  ];
  return { ...emptyThreadState(), transcript: messages.map((message) => ({ message_id: null, message })) };
}

/** Each message row's role, `+` after it when it continues its turn (no name line). */
function turnMarks(state: ThreadState): string[] {
  return items(state).flatMap((item) => {
    const content = rowContent(state, item);
    return content?.type === "message" ? [`${content.view.role}${content.continues ? "+" : ""}`] : [];
  });
}

describe("turns", () => {
  // spec: UI-470
  test("the assistant's messages after the first of a turn continue it; a user message starts the next", () => {
    expect(turnMarks(turns())).toEqual(["user", "assistant", "assistant+", "assistant+", "user", "assistant"]);
  });

  // spec: UI-470
  test("a notice or a record inside a turn does not start a new one", () => {
    const state = turns();
    const transcript = [...state.transcript];
    transcript.splice(3, 0, { message_id: null, message: { role: "bashExecution", command: "ls", output: "a" } });
    const frame: NoticeFrame = { type: "unknown", data: { type: "mystery" } };
    expect(turnMarks({ ...state, transcript, notices: [{ position: 3, frame }] })).toEqual([
      "user",
      "assistant",
      "record",
      "assistant+",
      "assistant+",
      "user",
      "assistant",
    ]);
  });

  // spec: UI-470, UI-191
  test("the streaming reply continues the turn the transcript ends in, and starts one after a user message", () => {
    const streaming: ThreadState["streaming"] = {
      message_id: "live",
      message: { role: "assistant", content: [{ type: "text", text: "…" }] },
    };
    const state = turns();
    expect(turnMarks({ ...state, streaming }).at(-1)).toBe("assistant+");
    const asked = { ...state, transcript: state.transcript.slice(0, 7), streaming };
    expect(turnMarks(asked).slice(-2)).toEqual(["user", "assistant"]);
  });
});

/** omp's `message_update` frame for the reply being streamed. */
function update(assistantMessageEvent: JsonObject): ThreadEvent {
  return {
    kind: "omp",
    frame: { type: "message_update", messageId: "live", message: { role: "assistant" }, assistantMessageEvent },
  };
}

/** The finished tool-call fixture, then a reply streaming a finished text block and a second one growing. */
function streamingStates(): [ThreadState, ThreadState] {
  const opened = [
    { kind: "omp", frame: { type: "message_start", messageId: "live", message: { role: "assistant", content: [] } } },
    update({ type: "text_start", contentIndex: 0, block: { type: "text", text: "" } }),
    update({ type: "text_delta", contentIndex: 0, delta: "First block." }),
    update({ type: "text_end", contentIndex: 0, content: "First block.", block: { type: "text" } }),
    update({ type: "text_start", contentIndex: 1, block: { type: "text", text: "" } }),
    update({ type: "text_delta", contentIndex: 1, delta: "Second" }),
  ] satisfies ThreadEvent[];
  const before = opened.reduce(reduce, fixture("bash_tool_call"));
  return [before, reduce(before, update({ type: "text_delta", contentIndex: 1, delta: " half" }))];
}

describe("streaming cost", () => {
  // spec: UI-191
  test("a delta keeps the item list and the content of every row but the streaming one", () => {
    const [before, after] = streamingStates();
    expect(items(after)).toBe(items(before));
    const rows = items(after);
    expect(rows.at(-1)?.kind.type).toBe("streaming");
    for (const row of rows.slice(0, -1)) {
      expect(rowContent(after, row)).toBe(rowContent(before, row));
    }
    const streaming = rows.at(-1);
    expect(streaming && rowContent(after, streaming)).not.toBe(streaming && rowContent(before, streaming));
  });

  // spec: UI-191
  test("a delta changes the growing block of the streaming reply only", () => {
    const [before, after] = streamingStates();
    const [done, growing] = streamingView(after)?.blocks ?? [];
    const [doneBefore, growingBefore] = streamingView(before)?.blocks ?? [];
    expect(done).toBe(doneBefore);
    expect(growing).toEqual({ type: "text", text: "Second half" });
    expect(growingBefore).toEqual({ type: "text", text: "Second" });
  });

  // spec: UI-191
  test("a finished message keeps its view until its tool run or result changes", () => {
    const state = fixture("bash_tool_call");
    const view = messageView(state, 1);
    expect(messageView({ ...state, settled: !state.settled }, 1)).toBe(view);
    const tools = state.tools.map((run) => ({ ...run, status: "failed" as const }));
    expect(messageView({ ...state, tools }, 1)).not.toBe(view);
  });
});

/** Local time on `day` of November 2026 (or of another month and year), so the tests hold in any time zone. */
function local(day: number, hour: number, month = 11, year = 2026): number {
  return new Date(year, month - 1, day, hour, 30).getTime();
}

/** A transcript of user messages sent at `times` (`undefined`: without a timestamp). */
function sentAt(...times: (number | undefined)[]): ThreadState {
  return {
    ...emptyThreadState(),
    transcript: times.map((timestamp, index) => ({
      message_id: null,
      message: { role: "user", content: `m${index}`, ...(timestamp === undefined ? {} : { timestamp }) },
    })),
  };
}

function days(state: ThreadState) {
  return items(state).map((item) => (item.kind.type === "message" ? item.kind.day : undefined));
}

describe("day separators", () => {
  // spec: UI-750
  test("a message on another local day than the previous one is marked, midnight being the boundary", () => {
    const state = sentAt(local(12, 10), local(12, 23), local(13, 0), local(13, 9), local(15, 8));
    expect(days(state)).toEqual([
      { at: local(12, 10), first: true },
      undefined,
      { at: local(13, 0), first: false },
      undefined,
      { at: local(15, 8), first: false },
    ]);
  });

  // spec: UI-750
  test("messages without a usable timestamp are never marked nor compared", () => {
    const state = sentAt(undefined, local(12, 10), undefined, 0, local(12, 18), undefined, local(13, 9));
    expect(days(state)).toEqual([
      undefined,
      { at: local(12, 10), first: true },
      undefined,
      undefined,
      undefined,
      undefined,
      { at: local(13, 9), first: false },
    ]);
  });

  // spec: UI-750
  test("tool results shown in their call's card do not count as the previous message", () => {
    const state: ThreadState = {
      ...emptyThreadState(),
      transcript: [
        {
          message_id: null,
          message: {
            role: "assistant",
            content: [{ type: "toolCall", id: "c1", name: "bash", arguments: {} }],
            timestamp: local(12, 23),
          },
        },
        { message_id: null, message: { role: "toolResult", toolCallId: "c1", content: [], timestamp: local(13, 1) } },
        { message_id: null, message: { role: "user", content: "next", timestamp: local(13, 9) } },
      ],
    };
    expect(days(state)).toEqual([
      { at: local(12, 23), first: true },
      { at: local(13, 9), first: false },
    ]);
  });

  // spec: UI-750
  test("the first message is labelled only when it was not sent today", () => {
    const now = local(16, 9);
    expect(daySeparator({ at: local(16, 1), first: true }, now)).toBeUndefined();
    expect(daySeparator({ at: local(16, 1), first: false }, now)).toEqual({ label: "Today", date: "2026-11-16" });
    expect(daySeparator({ at: local(15, 23), first: true }, now)).toEqual({ label: "Yesterday", date: "2026-11-15" });
    expect(daySeparator({ at: local(14, 23), first: true }, now, "en-GB")).toEqual({
      label: "Sat 14 Nov",
      date: "2026-11-14",
    });
  });

  // spec: UI-750
  test("the year shows only for another year than the current one", () => {
    const newYear = local(1, 9, 1, 2027);
    expect(daySeparator({ at: local(12, 10), first: true }, local(16, 9), "en-GB")?.label).toBe("Thu 12 Nov");
    expect(daySeparator({ at: local(12, 10, 11, 2025), first: true }, local(16, 9), "en-GB")?.label).toBe(
      "Wed, 12 Nov 2025",
    );
    expect(daySeparator({ at: local(31, 22, 12, 2026), first: false }, newYear, "en-GB")?.label).toBe("Yesterday");
    expect(daySeparator({ at: local(30, 22, 12, 2026), first: false }, newYear, "en-GB")).toEqual({
      label: "Wed, 30 Dec 2026",
      date: "2026-12-30",
    });
  });
});
