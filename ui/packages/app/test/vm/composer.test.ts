import { describe, expect, test } from "bun:test";
import type { AvailableSlashCommand } from "@omfp/core";
import {
  activityOf,
  alternative,
  base64,
  behavior,
  canAbort,
  commandsOf,
  complete,
  INTERRUPT_WINDOW_MS,
  imageAttachment,
  interruptAction,
  isBusy,
  keyAction,
  type Modifiers,
  queueKeys,
  restoreCleared,
  restoreDraft,
  restoredTexts,
  restoreFailedPrompt,
  slashSuggestions,
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
    expect(activityOf(thinking)).toEqual({ key: "thinking/live/0", kind: "thinking" });

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

  // spec: UI-060
  test("slash completion matches names then aliases until a space", () => {
    expect(names("/c")).toEqual(["compact", "copy"]);
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
