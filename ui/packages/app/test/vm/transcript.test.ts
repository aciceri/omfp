import { describe, expect, test } from "bun:test";
import {
  type AgentMessage,
  type AssistantContent,
  emptyThreadState,
  type JsonObject,
  type NoticeFrame,
  reduce,
  type ThreadEvent,
  type ThreadState,
} from "@omfp/core";
import {
  type Block,
  daySeparator,
  type Item,
  inOpenTurn,
  items,
  type MessageView,
  messageView,
  rowContent,
  streamingView,
} from "../../src/vm/transcript.ts";
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

  // spec: UI-911, UI-913
  test("omp's `/compact` output repeating the skipped compaction before it is left out; other outputs stay", () => {
    const skipped: NoticeFrame = {
      type: "manual_compaction_end",
      data: { aborted: false, skipped: true, errorMessage: "Already compacted" },
    };
    const repeat: NoticeFrame = { type: "command_output", data: { text: "Compaction failed: Already compacted" } };
    const failed: NoticeFrame = {
      type: "manual_compaction_end",
      data: { aborted: false, errorMessage: "No model available" },
    };
    const other: NoticeFrame = { type: "command_output", data: { text: "Compaction failed: No model available" } };
    const state: ThreadState = {
      ...fixture("text_reply"),
      notices: [skipped, repeat, failed, other, repeat, skipped, repeat].map((frame) => ({ position: 2, frame })),
    };
    // The first and last repeats follow a skipped end; the middle one follows another command output.
    expect(items(state).map((item) => item.key)).toEqual(["m0", "m1", "n0", "n2", "n3", "n4", "n5"]);
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

describe("open turn", () => {
  const open = (state: ThreadState) => items(state).map((item) => inOpenTurn(state, item));

  // spec: UI-922
  test("rows from the last user message on are in the open turn while the thread works, none once it settles", () => {
    const working = { ...turns(), settled: false };
    expect(open(working)).toEqual([false, false, false, false, true, true]);
    expect(open({ ...working, settled: true })).toEqual([false, false, false, false, false, false]);
  });

  // spec: UI-922
  test("the streaming reply is in it; a notice never is", () => {
    const streaming: ThreadState["streaming"] = {
      message_id: "live",
      message: { role: "assistant", content: [{ type: "text", text: "…" }] },
    };
    const frame: NoticeFrame = { type: "unknown", data: { type: "mystery" } };
    const working = { ...turns(), settled: false, streaming, notices: [{ position: 8, frame }] };
    expect(open(working).slice(-3)).toEqual([true, false, true]);
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

/** A `read` call of `path`. */
function readCall(id: string, path: string): AssistantContent {
  return { type: "toolCall", id, name: "read", arguments: { path } };
}

/** The result of read `id`: its text, failed with `isError`. */
function readResult(id: string, text: string, isError = false): AgentMessage {
  return { role: "toolResult", toolCallId: id, toolName: "read", content: [{ type: "text", text }], isError };
}

/** One assistant message of `content`, then `results`. */
function replyWith(content: AssistantContent[], ...results: AgentMessage[]): ThreadState {
  const messages: AgentMessage[] = [{ role: "assistant", content }, ...results];
  return { ...emptyThreadState(), transcript: messages.map((message) => ({ message_id: null, message })) };
}

/** Each block of `view`: a read group as `reads` and its calls, a tool card as `tool` and its call, else its type. */
function shape(view: MessageView | undefined): string[] {
  return (view?.blocks ?? []).map((block: Block) => {
    switch (block.type) {
      case "readGroup":
        return `reads ${block.cards.map((card) => card.toolCallId).join(",")}`;
      case "tool":
        return `tool ${block.card.toolCallId}`;
      default:
        return block.type;
    }
  });
}

/** The state of the read group of the first message of `state`. */
function groupStatus(state: ThreadState): string | undefined {
  const [block] = messageView(state, 0).blocks;
  return block?.type === "readGroup" ? block.status : undefined;
}

describe("read groups", () => {
  // spec: UI-765
  test("consecutive reads of one message are one group in call order; a lone read keeps its own card", () => {
    const three = replyWith([readCall("a", "src/a.rs"), readCall("b", "src/b.rs:10-20"), readCall("c", "src")]);
    expect(shape(messageView(three, 0))).toEqual(["reads a,b,c"]);
    expect(shape(messageView(replyWith([readCall("a", "src/a.rs")]), 0))).toEqual(["tool a"]);
  });

  // spec: UI-765
  test("text, thinking, another tool or a URL's read between reads ends the run", () => {
    const state = replyWith([
      { type: "text", text: "Reading." },
      readCall("a", "src/a.rs"),
      readCall("b", "src/b.rs"),
      { type: "text", text: "And this one." },
      readCall("c", "src/c.rs"),
      { type: "toolCall", id: "d", name: "bash", arguments: { command: "ls" } },
      readCall("e", "src/e.rs"),
      readCall("f", "src/f.rs"),
      { type: "thinking", thinking: "one more" },
      readCall("g", "src/g.rs"),
      readCall("h", "https://example.com/page"),
      readCall("i", "src/i.rs"),
    ]);
    expect(shape(messageView(state, 0))).toEqual([
      "text",
      "reads a,b",
      "text",
      "tool c",
      "tool d",
      "reads e,f",
      "thinking",
      "tool g",
      "tool h",
      "tool i",
    ]);
  });

  // spec: UI-765, UI-052
  test("the group runs while any read runs, else fails when a read failed; each read keeps its own state", () => {
    const calls = [readCall("a", "src/a.rs"), readCall("b", "src/missing.rs")];
    expect(groupStatus(replyWith(calls))).toBe("running");
    expect(groupStatus(replyWith(calls, readResult("a", "1:fn a() {}"), readResult("b", "1:fn b() {}")))).toBe(
      "succeeded",
    );
    const failed = replyWith(calls, readResult("a", "1:fn a() {}"), readResult("b", "Path not found", true));
    expect(groupStatus(failed)).toBe("failed");
    const [group] = messageView(failed, 0).blocks;
    expect(group?.type === "readGroup" && group.cards.map((card) => card.status)).toEqual(["succeeded", "failed"]);
    const running: ThreadState = {
      ...replyWith(calls, readResult("b", "Path not found", true)),
      tools: [
        {
          tool_call_id: "a",
          tool_name: "read",
          args: { path: "src/a.rs" },
          intent: null,
          status: "running",
          partial_result: null,
          preview: null,
          result: null,
          background: null,
        },
      ],
    };
    expect(groupStatus(running)).toBe("running");
  });

  // spec: UI-765, UI-191
  test("a read streamed after another joins its group, which keeps its identity while its reads stay", () => {
    const first = readCall("a", "src/a.rs");
    const second = readCall("b", "src/b.rs");
    const streamingWith = (content: AssistantContent[]): ThreadState => ({
      ...emptyThreadState(),
      streaming: { message_id: "live", message: { role: "assistant", content } },
    });
    expect(shape(streamingView(streamingWith([first])))).toEqual(["tool a"]);
    const joined = streamingView(streamingWith([first, second]));
    expect(shape(joined)).toEqual(["reads a,b"]);
    const before = streamingView(streamingWith([first, second, { type: "text", text: "Both" }]));
    const after = streamingView(streamingWith([first, second, { type: "text", text: "Both read." }]));
    expect(shape(after)).toEqual(["reads a,b", "text"]);
    expect(after?.blocks[0]).toBe(before?.blocks[0]);
    expect(after?.blocks[0]).toBe(joined?.blocks[0]);
  });
});

/** `messages` as a transcript; assistant messages are given as their content. */
function transcriptOf(...messages: (AgentMessage | AssistantContent[])[]): ThreadState {
  return {
    ...emptyThreadState(),
    transcript: messages.map((message) => ({
      message_id: null,
      message: Array.isArray(message) ? { role: "assistant", content: message } : message,
    })),
  };
}

/** Each row of `state`: its key, then the shape of its view (see {@link shape}). */
function rowShapes(state: ThreadState): string[][] {
  return items(state).map((item) => {
    const content = rowContent(state, item);
    return [item.key, ...(content?.type === "message" ? shape(content.view) : [content?.type ?? "none"])];
  });
}

describe("read groups across messages", () => {
  const ask: AgentMessage = { role: "user", content: "read them" };

  // spec: UI-765, UI-042
  test("reads in the next messages of a turn join the row whose reads they continue; their results do not break it", () => {
    const state = transcriptOf(
      ask,
      [{ type: "text", text: "Looking." }, readCall("a", "src/a.rs")],
      readResult("a", "1:a"),
      [readCall("b", "src/b.rs")],
      readResult("b", "Path not found", true),
      [
        { type: "text", text: "  " },
        { type: "thinking", thinking: "" },
        readCall("c", "src/c.rs"),
        readCall("d", "src"),
      ],
      readResult("c", "1:c"),
      readResult("d", "c.rs"),
      [{ type: "text", text: "Done." }],
    );
    expect(rowShapes(state)).toEqual([
      ["m0", "text"],
      ["m1", "text", "reads a,b,c,d"],
      ["m8", "text"],
    ]);
    const [, group] = items(state);
    expect(group?.kind.type === "message" && group.kind.joined).toEqual([3, 5]);
    const content = group === undefined ? undefined : rowContent(state, group);
    const block = content?.type === "message" ? content.view.blocks[1] : undefined;
    expect(block?.type === "readGroup" && block.cards.map((card) => card.status)).toEqual([
      "succeeded",
      "failed",
      "succeeded",
      "succeeded",
    ]);
    expect(block?.type === "readGroup" && block.status).toBe("failed");
    // The joined row keeps its content while nothing it shows changes (UI-191).
    expect(group && rowContent({ ...state, settled: !state.settled }, group)).toBe(content);
  });

  // spec: UI-765
  test("a message showing anything else, or reads after something else, keeps its own row", () => {
    const read = (id: string) => readCall(id, `src/${id}.rs`);
    expect(rowShapes(transcriptOf([read("a")], [{ type: "thinking", thinking: "hm" }, read("b")]))).toEqual([
      ["m0", "tool a"],
      ["m1", "thinking", "tool b"],
    ]);
    expect(rowShapes(transcriptOf([read("a")], [read("b"), { type: "text", text: "so" }]))).toEqual([
      ["m0", "tool a"],
      ["m1", "tool b", "text"],
    ]);
    expect(rowShapes(transcriptOf([read("a"), { type: "text", text: "so" }], [read("b")]))).toEqual([
      ["m0", "tool a", "text"],
      ["m1", "tool b"],
    ]);
    const bash: AssistantContent = { type: "toolCall", id: "x", name: "bash", arguments: { command: "ls" } };
    expect(rowShapes(transcriptOf([read("a")], [bash], [read("b")]))).toEqual([
      ["m0", "tool a"],
      ["m1", "tool x"],
      ["m2", "tool b"],
    ]);
    expect(rowShapes(transcriptOf([read("a")], [readCall("u", "https://example.com")]))).toEqual([
      ["m0", "tool a"],
      ["m1", "tool u"],
    ]);
    const failed: AgentMessage = { role: "assistant", content: [read("a")], errorMessage: "overloaded" };
    expect(rowShapes(transcriptOf(failed, [read("b")]))).toEqual([
      ["m0", "tool a"],
      ["m1", "tool b"],
    ]);
  });

  // spec: UI-765, UI-043, UI-750
  test("a notice or another day between two messages of reads keeps them apart", () => {
    const frame: NoticeFrame = { type: "unknown", data: { type: "mystery" } };
    const reads = transcriptOf([readCall("a", "src/a.rs")], [readCall("b", "src/b.rs")]);
    expect(rowShapes(reads)).toEqual([["m0", "reads a,b"]]);
    expect(rowShapes({ ...reads, notices: [{ position: 1, frame }] })).toEqual([
      ["m0", "tool a"],
      ["n0", "notice"],
      ["m1", "tool b"],
    ]);
    const days = transcriptOf(
      { role: "assistant", content: [readCall("a", "src/a.rs")], timestamp: local(12, 23) },
      { role: "assistant", content: [readCall("b", "src/b.rs")], timestamp: local(13, 1) },
    );
    expect(rowShapes(days)).toEqual([
      ["m0", "tool a"],
      ["m1", "tool b"],
    ]);
  });

  // spec: UI-765, UI-191
  test("a streamed reply of reads joins the open row, which streams; the item list stays while it grows", () => {
    const state = transcriptOf(ask, [readCall("a", "src/a.rs")], readResult("a", "1:a"));
    const streamingWith = (content: AssistantContent[]): ThreadState => ({
      ...state,
      streaming: { message_id: "live", message: { role: "assistant", content } },
    });
    const composing = streamingWith([{ type: "toolCall", id: "b", name: "read", partialArgs: '{"pa' }]);
    const live = items(composing);
    expect(live.map((item) => item.key)).toEqual(["m0", "m1"]);
    expect(live[1]?.kind.type === "message" && live[1].kind.live).toBe(true);
    const row = live[1] === undefined ? undefined : rowContent(composing, live[1]);
    expect(row?.type === "message" && row.streaming).toBe(true);
    expect(rowShapes(composing)).toEqual([
      ["m0", "text"],
      ["m1", "reads a,b"],
    ]);
    const written = streamingWith([readCall("b", "src/b.rs")]);
    expect(items(written)).toBe(live);
    expect(rowContent(written, live[0] as Item)).toBe(rowContent(composing, live[0] as Item));
    expect(rowShapes(streamingWith([{ type: "thinking", thinking: "next" }]))).toEqual([
      ["m0", "text"],
      ["m1", "tool a"],
      ["streaming", "thinking"],
    ]);
  });
});
