import { describe, expect, test } from "bun:test";
import type { AvailableSlashCommand } from "@omfp/core";
import {
  acceptGhost,
  activityOf,
  alternative,
  base64,
  behavior,
  canAbort,
  commandsOf,
  complete,
  completeFile,
  fileMention,
  fileSuggestions,
  followGhost,
  fuzzyMatches,
  INTERRUPT_WINDOW_MS,
  imageAttachment,
  interruptAction,
  isBusy,
  keyAction,
  type Modifiers,
  queueKeys,
  reasoningPreview,
  restoreCleared,
  restoreDraft,
  restoredTexts,
  restoreFailedPrompt,
  slashSuggestions,
  type WordGhost,
  wordAt,
  wordGhost,
} from "../../src/vm/composer.ts";
import { fixture, fixtureUntil, isFrame } from "../widgets/chat/fixtures.ts";

const COMMANDS: AvailableSlashCommand[] = [
  { name: "compact", source: "builtin", description: "Compact the context" },
  { name: "model", source: "builtin", aliases: ["m"], input: { hint: "<model>" } },
  { name: "mcp", source: "builtin" },
  { name: "copy", source: "builtin", aliases: ["cp"] },
];

const PLAIN: Modifiers = { shift: false, alt: false, ctrl: false, meta: false };
const IDLE = { busy: false, hasSuggestions: false };
const BUSY = { busy: true, hasSuggestions: false };

const names = (draft: string) => slashSuggestions(draft, COMMANDS).map((suggestion) => suggestion.name);

describe("composer", () => {
  // spec: UI-693
  test("activity identifies a live tool, live thinking, generic work, and no settled work", () => {
    const settled = fixture("text_reply");
    const thinking = {
      ...settled,
      settled: false,
      streaming: {
        message_id: "live",
        message: { role: "assistant" as const, content: [{ type: "thinking" as const, thinking: "Considering" }] },
      },
      tools: [],
    };
    expect(activityOf(thinking)).toEqual({ key: "thinking/live/0", kind: "thinking", preview: "Considering" });

    const tool = {
      ...thinking,
      tools: [
        {
          tool_call_id: "call_edit_1",
          tool_name: "edit",
          args: { input: "[src/lib.rs#1234]" },
          intent: "Saturating the addition",
          status: "running" as const,
          partial_result: null,
          preview: null,
          result: null,
          background: null,
        },
      ],
    };
    expect(activityOf(tool)).toEqual({
      key: "tool/call_edit_1",
      kind: "tool",
      name: "edit",
      intent: "Saturating the addition",
    });
    expect(activityOf({ ...thinking, streaming: null })).toEqual({ key: "working", kind: "working" });
    expect(activityOf(settled)).toBeUndefined();
  });

  // spec: UI-693
  test("live thinking previews its latest readable line as plain text, or falls back without text", () => {
    const streamed = (blocks: { type: "thinking"; thinking?: string }[]) => ({
      ...fixture("text_reply"),
      settled: false,
      streaming: { message_id: "live", message: { role: "assistant" as const, content: blocks } },
      tools: [],
    });
    expect(
      activityOf(streamed([{ type: "thinking", thinking: "**Assessing Slack unread activity options**" }])),
    ).toEqual({ key: "thinking/live/0", kind: "thinking", preview: "Assessing Slack unread activity options" });
    // Later deltas replace the preview with the latest line; the key (and so the elapsed time) stays.
    expect(
      activityOf(
        streamed([{ type: "thinking", thinking: "**Assessing Slack unread activity options**\n\nThe user wants the" }]),
      ),
    ).toEqual({ key: "thinking/live/0", kind: "thinking", preview: "The user wants the" });
    // The latest thinking block wins over an earlier one.
    expect(
      activityOf(
        streamed([
          { type: "thinking", thinking: "First" },
          { type: "thinking", thinking: "Second" },
        ]),
      ),
    ).toEqual({ key: "thinking/live/1", kind: "thinking", preview: "Second" });
    // No displayable text: the generic fallback, without a preview.
    expect(activityOf(streamed([{ type: "thinking" }]))).toEqual({ key: "thinking/live/0", kind: "thinking" });
    expect(activityOf(streamed([{ type: "thinking", thinking: " \n**\n  " }]))).toEqual({
      key: "thinking/live/0",
      kind: "thinking",
    });
  });

  // spec: UI-693
  test("the reasoning preview drops Markdown syntax, collapses whitespace and is bounded", () => {
    expect(reasoningPreview("## Planning   the\tfix\n\n")).toBe("Planning the fix");
    expect(reasoningPreview("intro\n> - *Check* `cargo` [docs](https://x.y) and __bold__")).toBe(
      "Check cargo docs and bold",
    );
    expect(reasoningPreview("1. Read snake_case_names ~~not~~ _now_")).toBe("Read snake_case_names not now");
    expect(reasoningPreview("")).toBeUndefined();
    const long = reasoningPreview(`**${"word ".repeat(100)}**`);
    expect([long?.length, long?.endsWith("word…")]).toEqual([200, true]);
  });

  // spec: UI-061
  test("busy threads steer unless the modifier (or ctrl+q) queues a follow-up", () => {
    const busy = fixtureUntil("follow_up_then_abort", isFrame("queue_update"));
    expect(isBusy(busy)).toBe(true);
    expect(isBusy(fixture("text_reply"))).toBe(false);
    const shift = { ...PLAIN, shift: true };
    const ctrl = { ...PLAIN, ctrl: true };
    expect(keyAction("Enter", PLAIN, BUSY)).toEqual({ type: "send", mode: "steer" });
    expect(keyAction("Enter", ctrl, BUSY)).toEqual({ type: "send", mode: "follow-up" });
    expect(keyAction("Enter", { ...PLAIN, meta: true }, BUSY)).toEqual({ type: "send", mode: "follow-up" });
    expect(keyAction("Enter", shift, BUSY)).toEqual({ type: "send", mode: "follow-up" });
    expect(keyAction("q", ctrl, BUSY)).toEqual({ type: "send", mode: "follow-up" });
    expect(keyAction("q", ctrl, { ...BUSY, repeat: true })).toEqual({ type: "ignore" });
    // Idle, the queue keys send like Enter; ctrl+q is left to the keymap.
    expect(keyAction("Enter", ctrl, IDLE)).toEqual({ type: "send", mode: "prompt" });
    expect(keyAction("q", ctrl, IDLE)).toEqual({ type: "none" });
    expect([queueKeys(false), queueKeys(true)]).toEqual(["Ctrl-Enter", "Meta-Enter"]);
    expect(behavior("follow-up")).toBe("followUp");
    expect(behavior("steer")).toBe("steer");
    expect(behavior("prompt")).toBeUndefined();
    expect(alternative("follow-up")).toBe("steer");
    expect(alternative("prompt")).toBeUndefined();
  });

  // spec: UI-060
  test("alt+enter is a newline (macOS's Option+Enter too) and tab completes only with suggestions", () => {
    expect(keyAction("Enter", { ...PLAIN, alt: true }, IDLE)).toEqual({ type: "newline" });
    expect(keyAction("Enter", { ...PLAIN, alt: true, shift: true }, BUSY)).toEqual({ type: "newline" });
    expect(keyAction("Tab", PLAIN, { ...IDLE, hasSuggestions: true })).toEqual({ type: "complete" });
    expect(keyAction("Tab", PLAIN, IDLE)).toEqual({ type: "none" });
  });

  // spec: UI-061
  test("an auto-repeated enter or queue key sends nothing", () => {
    expect(keyAction("Enter", PLAIN, { ...IDLE, repeat: true })).toEqual({ type: "ignore" });
    expect(keyAction("Enter", { ...PLAIN, shift: true }, { ...BUSY, repeat: true })).toEqual({ type: "ignore" });
  });

  // spec: UI-421
  test("on a touch screen a plain enter is left to the text area (a newline); modified enters keep their roles", () => {
    expect(keyAction("Enter", PLAIN, { ...IDLE, touch: true })).toEqual({ type: "none" });
    expect(keyAction("Enter", PLAIN, { ...BUSY, touch: true, repeat: true })).toEqual({ type: "none" });
    expect(keyAction("Enter", { ...PLAIN, ctrl: true }, { ...BUSY, touch: true })).toEqual({
      type: "send",
      mode: "follow-up",
    });
    expect(keyAction("Enter", { ...PLAIN, alt: true }, { ...IDLE, touch: true })).toEqual({ type: "newline" });
    expect(keyAction("Enter", PLAIN, { ...IDLE, touch: false })).toEqual({ type: "send", mode: "prompt" });
  });

  // spec: UI-063, UI-095
  test("escape leaves the composer, busy or not; abortable means busy or queued", () => {
    expect(keyAction("Escape", PLAIN, BUSY)).toEqual({ type: "leave" });
    expect(keyAction("Escape", PLAIN, IDLE)).toEqual({ type: "leave" });
    const queued = fixtureUntil("follow_up_then_abort", isFrame("message_start"));
    expect(canAbort(fixture("text_reply"))).toBe(false);
    expect(canAbort({ ...fixture("text_reply"), queue: { steering: [], follow_up: ["x"], live_steered: 0 } })).toBe(
      true,
    );
    expect(canAbort(queued)).toBe(true);
  });

  // spec: UI-175
  test("a word ends at the caret as omp asks: letters, marks and apostrophes, nothing word-like after", () => {
    expect(wordAt("hel", 3)).toBe("hel");
    expect(wordAt("say hel", 7)).toBe("hel");
    expect(wordAt("don't", 5)).toBe("don't");
    expect(wordAt("café", 4)).toBe("café");
    expect(wordAt("cafe\u0301", 5)).toBe("cafe\u0301");
    expect(wordAt("email me@work", 13)).toBe("work");
    expect(wordAt("hello", 3)).toBeUndefined();
    expect(wordAt("hel'", 3)).toBeUndefined();
    expect(wordAt("hel ", 4)).toBeUndefined();
    expect(wordAt("abc1", 4)).toBeUndefined();
    expect(wordAt("", 0)).toBeUndefined();
    expect(wordAt("hel", 0)).toBeUndefined();
  });

  // spec: UI-175
  test("a word completes only at the end of its line, where the ghost covers no text", () => {
    expect(wordAt("say hel\nmore", 7)).toBe("hel");
    expect(wordAt("say hel\r\nmore", 7)).toBeUndefined();
    expect(wordAt("hel world", 3)).toBeUndefined();
    expect(wordAt("hel.", 3)).toBeUndefined();
    expect(wordAt("hel ", 3)).toBeUndefined();
  });

  // spec: UI-175, UI-732
  test("a word never completes inside a `@` file mention", () => {
    expect(wordAt("@src", 4)).toBeUndefined();
    expect(wordAt("look at @ma", 11)).toBeUndefined();
    expect(wordAt('look at @"my fi', 15)).toBeUndefined();
    expect(wordAt("look at @main.rs then", 21)).toBe("then");
  });

  // spec: UI-175, UI-060
  test("a word never completes in the bare `/command` token", () => {
    expect(wordAt("/comp", 5)).toBeUndefined();
    expect(wordAt("/model\nnext", 11)).toBe("next");
    expect(wordAt("/compact focus", 14)).toBe("focus");
    expect(wordAt("see /usr", 8)).toBe("usr");
  });

  // spec: UI-175
  test("an answer becomes a ghost only with a suffix", () => {
    expect(wordGhost("say hel", 7, "lo")).toEqual({ text: "say hel", cursor: 7, suffix: "lo" });
    expect(wordGhost("say hel", 7, "")).toBeUndefined();
    expect(wordGhost("say hel", 7, null)).toBeUndefined();
    expect(wordGhost("say hel", 7, undefined)).toBeUndefined();
  });

  // spec: UI-175, UI-177
  test("typing the ghost's next characters narrows it; anything else ends it", () => {
    const ghost: WordGhost = { text: "say hel!", cursor: 7, suffix: "lo" };
    expect(followGhost(ghost, "say hel!", 7)).toEqual({ type: "keep", ghost });
    expect(followGhost(ghost, "say hell!", 8)).toEqual({
      type: "keep",
      ghost: { text: "say hell!", cursor: 8, suffix: "o" },
    });
    // As omp projects it: case-insensitively.
    expect(followGhost(ghost, "say helL!", 8)).toEqual({
      type: "keep",
      ghost: { text: "say helL!", cursor: 8, suffix: "o" },
    });
    expect(followGhost(ghost, "say hello!", 9)).toEqual({ type: "typed" });
    expect(followGhost(ghost, "say helx!", 8)).toEqual({ type: "past" });
    expect(followGhost(ghost, "say hel !", 8)).toEqual({ type: "past" });
    expect(followGhost(ghost, "say he!", 6)).toEqual({ type: "past" });
    expect(followGhost(ghost, "say hel!", 3)).toEqual({ type: "past" });
    expect(followGhost(ghost, "say hell!", 7)).toEqual({ type: "past" });
    expect(followGhost(ghost, "Say hell!", 8)).toEqual({ type: "past" });
    expect(followGhost(ghost, "say hell?", 8)).toEqual({ type: "past" });
  });

  // spec: UI-176
  test("accepting a ghost inserts its suffix at the caret, with or without a space, the caret after it", () => {
    const ghost: WordGhost = { text: "say hel, ok", cursor: 7, suffix: "lo" };
    expect(acceptGhost(ghost, true)).toEqual({ text: "say hello , ok", caret: 10 });
    expect(acceptGhost(ghost, false)).toEqual({ text: "say hello, ok", caret: 9 });
  });

  // spec: UI-176, UI-060, UI-732
  test("with a ghost, tab accepts it with a space and arrow right without; an open list keeps tab", () => {
    const ghost = { ...IDLE, ghost: true };
    expect(keyAction("Tab", PLAIN, ghost)).toEqual({ type: "accept", space: true });
    expect(keyAction("ArrowRight", PLAIN, ghost)).toEqual({ type: "accept", space: false });
    expect(keyAction("Tab", PLAIN, { ...ghost, hasSuggestions: true })).toEqual({ type: "complete" });
    expect(keyAction("Tab", { ...PLAIN, shift: true }, ghost)).toEqual({ type: "none" });
    expect(keyAction("ArrowRight", { ...PLAIN, shift: true }, ghost)).toEqual({ type: "none" });
    expect(keyAction("ArrowRight", { ...PLAIN, ctrl: true }, ghost)).toEqual({ type: "none" });
    expect(keyAction("ArrowRight", { ...PLAIN, alt: true }, ghost)).toEqual({ type: "none" });
    expect(keyAction("ArrowRight", PLAIN, IDLE)).toEqual({ type: "none" });
    expect(keyAction("Tab", PLAIN, { ...IDLE, ghost: false })).toEqual({ type: "none" });
  });

  // spec: UI-060, UI-978
  test("slash completion matches names then aliases until a space", () => {
    // omp's TUI-only `collab`, `clear` and `cleanse` follow, disabled.
    expect(names("/c")).toEqual(["compact", "copy", "collab", "clear", "cleanse"]);
    expect(names("/m")).toEqual(["model", "mcp"]);
    expect(names("/cp")).toEqual(["copy"]);
    expect(names("/model gpt")).toEqual([]);
    expect(names("hello /m")).toEqual([]);
    const [model] = slashSuggestions("/mo", COMMANDS);
    expect(model?.hint).toBe("<model>");
    expect(model === undefined ? "" : complete(model)).toBe("/model ");
  });

  // spec: UI-060
  test("suggestions come from the thread's available commands", () => {
    const state = fixture("text_reply");
    const first = state.commands[0]?.name;
    expect(first).toBeDefined();
    expect(slashSuggestions(`/${first}`, state.commands)[0]?.name).toBe(first);
  });

  // spec: UI-060
  test("a thread.commands result gives the commands while the state has none", () => {
    const compact: AvailableSlashCommand = { name: "compact", source: "builtin" };
    expect(commandsOf({ commands: [compact, { nope: 1 }] })).toEqual([compact]);
    expect(commandsOf(undefined)).toEqual([]);
  });

  // spec: UI-732
  test("a `@` at a token's start before the caret is a file mention; one inside a word is not", () => {
    const at = (text: string, caret = text.length) => fileMention(text, caret);
    expect(at("@")).toEqual({ start: 0, end: 1, query: "", quoted: false });
    expect(at("look at @src/ma")).toEqual({ start: 8, end: 15, query: "src/ma", quoted: false });
    expect(at("first\n@rea")).toEqual({ start: 6, end: 10, query: "rea", quoted: false });
    expect(at("key=@conf")).toEqual({ start: 4, end: 9, query: "conf", quoted: false });
    expect(at('see @"my fi')).toEqual({ start: 4, end: 11, query: "my fi", quoted: true });
    expect(at('say "hi @src')).toEqual({ start: 8, end: 12, query: "src", quoted: false });
    // The caret, not the text's end, ends the query.
    expect(at("@src/main.rs tail", 4)).toEqual({ start: 0, end: 4, query: "src", quoted: false });
    expect(at("mail me@example.com")).toBeUndefined();
    expect(at("@src/main.rs done")).toBeUndefined();
    expect(at('@"a b" c')).toBeUndefined();
    expect(at("plain text")).toBeUndefined();
    expect(at("")).toBeUndefined();
  });

  // spec: UI-732
  test("picking a file inserts `@path ` as omp's TUI does, quoted with a space, keeping the text after the caret", () => {
    const pick = (text: string, caret: number, path: string) => {
      const mention = fileMention(text, caret);
      return mention === undefined ? undefined : completeFile(text, mention, path);
    };
    expect(pick("read @ma", 8, "src/main.rs")).toEqual({ text: "read @src/main.rs ", caret: 18 });
    expect(pick("read @ma and more", 8, "src/main.rs")).toEqual({ text: "read @src/main.rs  and more", caret: 18 });
    expect(pick("@not", 4, "docs/my notes.md")).toEqual({ text: '@"docs/my notes.md" ', caret: 20 });
    expect(pick('@"rea', 5, "README.md")).toEqual({ text: '@"README.md" ', caret: 13 });
  });

  // spec: UI-732
  test("file suggestions: the answer for the query, else the older answer narrowed as find_file fuzzy matches", () => {
    const paths = ["src/main.rs", "src/lib.rs", "README.md", ...Array.from({ length: 10 }, (_, i) => `f${i}.ts`)];
    expect(fileSuggestions("src", { query: "src", paths })).toEqual(paths.slice(0, 8));
    expect(fileSuggestions("smain", { query: "s", paths })).toEqual(["src/main.rs"]);
    expect(fileSuggestions("RE ad", { query: "r", paths })).toEqual(["README.md"]);
    expect(fuzzyMatches("SRCLIB", "src/lib.rs")).toBe(true);
    expect(fuzzyMatches("bil", "src/lib.rs")).toBe(false);
  });

  // spec: UI-063
  test("an abort puts the queued texts back before the draft", () => {
    expect(restoreDraft("still typing", ["steer me"], ["later please"])).toBe(
      "steer me\n\nlater please\n\nstill typing",
    );
    expect(restoreDraft("", [], ["later please"])).toBe("later please");
    expect(restoredTexts({ steering: [{ text: "a" }], followUp: [{ text: "b", images: [] }] })).toEqual({
      steering: ["a"],
      followUp: ["b"],
    });
    expect(restoredTexts(null)).toEqual({ steering: [], followUp: [] });
  });

  // spec: UI-064
  test("a failed send puts its text and images back before what was typed since", () => {
    const image = { type: "image" as const, data: "AA==", mimeType: "image/png" };
    expect(
      restoreFailedPrompt({ text: "more", images: [], pastes: [] }, { text: "first", images: [image], pastes: [] }),
    ).toEqual({
      text: "first\n\nmore",
      images: [image],
      pastes: [],
    });
  });

  // spec: UI-063, UI-095
  test("Ctrl-C clears a non-empty draft, aborts an empty one or on a second press within the window", () => {
    const image = { type: "image" as const, data: "AA==", mimeType: "image/png" };
    const draft = (text: string, images: (typeof image)[] = []) => ({ text, images, pastes: [] });
    expect(interruptAction(draft("typing"), 5000, undefined)).toBe("clear");
    expect(interruptAction(draft("", [image]), 5000, undefined)).toBe("clear");
    expect(interruptAction(draft(" "), 5000, undefined)).toBe("clear");
    expect(interruptAction(draft(""), 5000, undefined)).toBe("abort");
    expect(interruptAction(draft("typing"), 5000 + INTERRUPT_WINDOW_MS, 5000)).toBe("abort");
    expect(interruptAction(draft("typing"), 5001 + INTERRUPT_WINDOW_MS, 5000)).toBe("clear");
  });

  // spec: UI-063
  test("u puts the cleared draft back before what was typed since", () => {
    const image = { type: "image" as const, data: "AA==", mimeType: "image/png" };
    expect(
      restoreCleared({ text: "", images: [], pastes: [] }, { text: "cleared", images: [image], pastes: [] }),
    ).toEqual({
      text: "cleared",
      images: [image],
      pastes: [],
    });
    expect(
      restoreCleared({ text: "new", images: [], pastes: [] }, { text: "cleared", images: [], pastes: [] }),
    ).toEqual({
      text: "cleared\n\nnew",
      images: [],
      pastes: [],
    });
  });

  // spec: UI-064, UI-357
  test("a restored draft keeps its paste chips; chips typed since become text when both have some", () => {
    const cleared = { text: "see [Paste #1, +11 lines]", images: [], pastes: [{ id: 1, text: "x\n".repeat(10) }] };
    expect(restoreCleared({ text: "plain", images: [], pastes: [] }, cleared)).toEqual({
      text: "see [Paste #1, +11 lines]\n\nplain",
      images: [],
      pastes: cleared.pastes,
    });
    const since = { text: "and [Paste #1, 1001 chars]", images: [], pastes: [{ id: 1, text: "y".repeat(1001) }] };
    expect(restoreCleared(since, cleared)).toEqual({
      text: `see [Paste #1, +11 lines]\n\nand ${"y".repeat(1001)}`,
      images: [],
      pastes: cleared.pastes,
    });
  });

  // spec: UI-060
  test("only images become attachments encoded as base64", () => {
    const bytes = (text: string) => new TextEncoder().encode(text);
    expect(base64(bytes(""))).toBe("");
    expect(base64(bytes("f"))).toBe("Zg==");
    expect(base64(bytes("ab"))).toBe("YWI=");
    expect(base64(bytes("foobar"))).toBe("Zm9vYmFy");
    expect(imageAttachment("image/png", new Uint8Array([0x89, 0x50, 0x4e, 0x47]))).toEqual({
      type: "image",
      data: "iVBORw==",
      mimeType: "image/png",
    });
    expect(imageAttachment("text/plain", bytes("hi"))).toBeUndefined();
  });
});
