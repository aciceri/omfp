/**
 * The composer (UI-060..064): send mode while busy, `/` command completion, `@` file completion (UI-732), key bindings,
 * image attachments, drafts restored after an abort or a failed send, and `Ctrl-C` clearing the draft, then aborting
 * (UI-063).
 */

import type { AvailableSlashCommand, ImageContent, ThreadState } from "@omfp/core";
import type { PromptBehavior } from "@omfp/protocol";
import { MAC } from "../keymap/keymap.ts";
import type { Draft } from "../sdk/index.ts";
import { isObject } from "../sdk/json.ts";
import { expandDraft } from "./paste.ts";

/** Most suggestions shown at once. */
const MAX_SUGGESTIONS = 8;

/** Nothing to send. */
export function isBlank(draft: Draft): boolean {
  return draft.text.trim() === "" && draft.images.length === 0;
}

/** The agent is working: messages sent now are queued (UI-061). */
export function isBusy(state: ThreadState): boolean {
  return !state.settled;
}
/** The compact activity shown over a busy thread's composer (UI-693). */
export type ThreadActivity =
  | { readonly key: string; readonly kind: "thinking"; readonly preview?: string }
  | { readonly key: string; readonly kind: "tool"; readonly name: string; readonly intent?: string }
  | { readonly key: "working"; readonly kind: "working" };

/**
 * The most specific live activity known for a busy thread: an executing tool, then its streamed thinking block (with
 * a plain-text preview of its latest line when it has one), otherwise generic work. Settled history has no live
 * activity (UI-693).
 */
export function activityOf(state: ThreadState): ThreadActivity | undefined {
  if (state.settled) {
    return undefined;
  }
  for (let index = state.tools.length - 1; index >= 0; index -= 1) {
    const tool = state.tools[index];
    if (
      tool !== undefined &&
      (tool.status === "pending" || tool.status === "running" || tool.background?.state === "running")
    ) {
      return {
        key: `tool/${tool.tool_call_id}`,
        kind: "tool",
        name: tool.tool_name,
        ...(tool.intent === null || tool.intent === undefined || tool.intent === "" ? {} : { intent: tool.intent }),
      };
    }
  }
  const streaming = state.streaming;
  const content = streaming?.message.content;
  if (streaming !== null && content !== undefined) {
    for (let index = content.length - 1; index >= 0; index -= 1) {
      const block = content[index];
      if (block?.type === "thinking") {
        const preview = reasoningPreview(block.thinking ?? "");
        return {
          key: `thinking/${streaming.message_id}/${index}`,
          kind: "thinking",
          ...(preview === undefined ? {} : { preview }),
        };
      }
    }
  }
  return { key: "working", kind: "working" };
}

/** Longest reasoning preview kept; the activity row ellipsizes it further to its width (UI-693). */
const PREVIEW_MAX = 200;

/**
 * The latest line of streamed reasoning as one line of plain text: Markdown heading, quote and list markers,
 * emphasis, code and link syntax dropped, whitespace collapsed, bounded to {@link PREVIEW_MAX} characters. Only the
 * tail of the text is scanned, so a long block costs no more than its last lines. `undefined` when nothing readable
 * remains (UI-693).
 */
export function reasoningPreview(thinking: string): string | undefined {
  let end = thinking.length;
  while (end > 0) {
    const start = thinking.lastIndexOf("\n", end - 1) + 1;
    const line = plainLine(thinking.slice(start, end));
    if (line !== "") {
      return line.length > PREVIEW_MAX ? `${line.slice(0, PREVIEW_MAX - 1).trimEnd()}…` : line;
    }
    end = start - 1;
  }
  return undefined;
}

/** One Markdown line without its syntax, cheaply: no full parse, only the markers a reasoning summary uses. */
function plainLine(line: string): string {
  return line
    .replace(/^\s*(?:#{1,6}\s+|>\s*|[-*+]\s+|\d+[.)]\s+)*/, "")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[*`~]+/g, "")
    .replace(/(^|[^\p{L}\p{N}_])_+|_+(?=[^\p{L}\p{N}_]|$)/gu, "$1")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * There is something to interrupt: the agent works or messages wait in the queue. `Ctrl-C` then aborts (UI-063,
 * UI-368); Esc never does, it only leaves the composer (UI-095).
 */
export function canAbort(state: ThreadState): boolean {
  return isBusy(state) || state.queue.steering.length > 0 || state.queue.follow_up.length > 0;
}

/** What the send action does: a new turn when idle; while busy queued until the turn ends, or steered into it. */
export type SendMode = "prompt" | "follow-up" | "steer";

/** Busy threads steer into the running turn unless the queue modifier (Ctrl, ⌘ or Shift with Enter) is held. */
export function sendMode(busy: boolean, queueModifier: boolean): SendMode {
  if (!busy) {
    return "prompt";
  }
  return queueModifier ? "follow-up" : "steer";
}

/** The `behavior` of `thread.prompt`. */
export function behavior(mode: SendMode): PromptBehavior | undefined {
  switch (mode) {
    case "prompt":
      return undefined;
    case "follow-up":
      return "followUp";
    case "steer":
      return "steer";
  }
}

/** The other busy mode, offered as a second button. */
export function alternative(mode: SendMode): SendMode | undefined {
  switch (mode) {
    case "prompt":
      return undefined;
    case "follow-up":
      return "steer";
    case "steer":
      return "follow-up";
  }
}

/** Modifier keys held with a key. */
export interface Modifiers {
  readonly shift: boolean;
  readonly alt: boolean;
  readonly ctrl: boolean;
  readonly meta: boolean;
}

/**
 * The modifier that turns a busy send into a queued follow-up: Ctrl or ⌘ (Shift too). Alt with Enter is a new line,
 * also on macOS, where Option+Enter arrives as `Enter` with `altKey`.
 */
export function queueModifier(modifiers: Modifiers): boolean {
  return !modifiers.alt && (modifiers.shift || modifiers.ctrl || modifiers.meta);
}

/** The keys that queue a follow-up, as the chips show them: `⌘`+Enter on macOS, Ctrl+Enter elsewhere. */
export function queueKeys(mac = MAC): string {
  return mac ? "Meta-Enter" : "Ctrl-Enter";
}

/** Ctrl+Q: queue a follow-up, as in omp's TUI. */
function isQueueChord(key: string, modifiers: Modifiers): boolean {
  return key.toLowerCase() === "q" && modifiers.ctrl && !modifiers.alt && !modifiers.meta && !modifiers.shift;
}

/** What a key press in the composer does. */
export type KeyAction =
  /** Let the text area handle it (typing). */
  | { readonly type: "none" }
  /** Alt+Enter: insert a line break at the caret. */
  | { readonly type: "newline" }
  /** An auto-repeated Enter: swallowed, so holding the key sends one message, not one per repeat. */
  | { readonly type: "ignore" }
  | { readonly type: "send"; readonly mode: SendMode }
  /** Esc: leave the composer (`focus.leave`, UI-095), busy or not; it never aborts (UI-063). */
  | { readonly type: "leave" }
  /** Tab with suggestions: complete the first one. */
  | { readonly type: "complete" };

/** What the composer knows when a key is pressed. */
export interface KeyContext {
  readonly busy: boolean;
  readonly hasSuggestions: boolean;
  /** The key is auto-repeating (`KeyboardEvent.repeat`). */
  readonly repeat?: boolean;
  /**
   * A touch screen (`pointer: coarse`, UI-421): its on-screen keyboard's Enter has no modifier, so a plain Enter is
   * left to the text area (a line break) and the send button sends.
   */
  readonly touch?: boolean;
}

/** The composer's key bindings. */
export function keyAction(key: string, modifiers: Modifiers, context: KeyContext): KeyAction {
  switch (key) {
    case "Enter":
      if (modifiers.alt) {
        return { type: "newline" };
      }
      if (context.touch === true && !queueModifier(modifiers)) {
        return { type: "none" };
      }
      return context.repeat === true
        ? { type: "ignore" }
        : { type: "send", mode: sendMode(context.busy, queueModifier(modifiers)) };
    case "Escape":
      return { type: "leave" };
    case "Tab":
      return context.hasSuggestions && !modifiers.shift ? { type: "complete" } : { type: "none" };
    default:
      if (context.busy && isQueueChord(key, modifiers)) {
        return context.repeat === true ? { type: "ignore" } : { type: "send", mode: "follow-up" };
      }
      return { type: "none" };
  }
}

/** One `/` completion. */
export interface Suggestion {
  readonly name: string;
  readonly description: string | undefined;
  /** The usage hint of the command's argument. */
  readonly hint: string | undefined;
}

/**
 * Commands matching the draft while it is a bare `/prefix` (no space yet): names first, then aliases, in the order of
 * `thread.commands` (kept fresh by `available_commands_update`, UI-060).
 */
export function slashSuggestions(draft: string, commands: readonly AvailableSlashCommand[]): Suggestion[] {
  if (!draft.startsWith("/")) {
    return [];
  }
  const prefix = draft.slice(1);
  if (/\s/.test(prefix)) {
    return [];
  }
  const byName = commands.filter((command) => command.name.startsWith(prefix));
  const byAlias = commands.filter(
    (command) => !command.name.startsWith(prefix) && (command.aliases ?? []).some((alias) => alias.startsWith(prefix)),
  );
  return [...byName, ...byAlias].slice(0, MAX_SUGGESTIONS).map((command) => ({
    name: command.name,
    description: command.description,
    hint: command.input?.hint,
  }));
}

/** The draft text after picking `suggestion`. */
export function complete(suggestion: Suggestion): string {
  return `/${suggestion.name} `;
}

/** What ends a `@` file mention before the caret, as in omp's TUI editor: whitespace, quotes and `=`. */
const MENTION_DELIMITER = /[\s"'=]/;

/** A `@` file mention being typed: `text.slice(start, end)` is replaced when a file is picked (UI-732). */
export interface FileMention {
  /** The index of the `@`. */
  readonly start: number;
  /** The caret. */
  readonly end: number;
  /** The fuzzy query typed after `@` (or `@"`). */
  readonly query: string;
  /** Typed as `@"…`: the picked path is quoted, as omp's TUI does. */
  readonly quoted: boolean;
}

/**
 * The `@` file mention ending at `caret` (UI-732), as omp's TUI finds it: the `@` starts a token (at the line's start or
 * after a space, a quote or `=`, so an e-mail address is no mention), the query runs to the caret; `@"` opens a quoted
 * one, which may contain spaces until its closing quote.
 */
export function fileMention(text: string, caret: number): FileMention | undefined {
  const end = Math.min(Math.max(caret, 0), text.length);
  const lineStart = text.lastIndexOf("\n", end - 1) + 1;
  const line = text.slice(lineStart, end);
  let quote: number | undefined;
  for (let index = 0; index < line.length; index += 1) {
    if (line[index] === '"') {
      quote = quote === undefined ? index : undefined;
    }
  }
  if (
    quote !== undefined &&
    line[quote - 1] === "@" &&
    (quote === 1 || MENTION_DELIMITER.test(line[quote - 2] ?? ""))
  ) {
    return { start: lineStart + quote - 1, end, query: line.slice(quote + 1), quoted: true };
  }
  let tokenStart = line.length;
  while (tokenStart > 0 && !MENTION_DELIMITER.test(line[tokenStart - 1] ?? "")) {
    tokenStart -= 1;
  }
  return line[tokenStart] === "@"
    ? { start: lineStart + tokenStart, end, query: line.slice(tokenStart + 1), quoted: false }
    : undefined;
}

/**
 * The text after picking the workspace-relative `path` for `mention`, with the caret after it (UI-732): `@path ` as
 * omp's TUI inserts it, so omp reads the file into the prompt's context (`@"path" ` when the path has a space or the
 * mention was quoted); the text after the caret stays.
 */
export function completeFile(text: string, mention: FileMention, path: string): { text: string; caret: number } {
  const inserted = mention.quoted || path.includes(" ") ? `@"${path}" ` : `@${path} `;
  return {
    text: `${text.slice(0, mention.start)}${inserted}${text.slice(mention.end)}`,
    caret: mention.start + inserted.length,
  };
}

/**
 * Whether `path` still matches `query` as `workspaces.find_file { fuzzy: true }` does (a case-insensitive subsequence,
 * whitespace ignored): narrows the last answer while the next one is on its way (UI-732).
 */
export function fuzzyMatches(query: string, path: string): boolean {
  const lower = path.toLowerCase();
  let at = 0;
  for (const letter of query.toLowerCase()) {
    if (/\s/.test(letter)) {
      continue;
    }
    const found = lower.indexOf(letter, at);
    if (found < 0) {
      return false;
    }
    at = found + letter.length;
  }
  return true;
}

/**
 * The file suggestions for `query` (UI-732): the answer's paths when it was for `query`, else those of an older
 * answer still matching it; at most {@link MAX_SUGGESTIONS}.
 */
export function fileSuggestions(query: string, answer: { query: string; paths: readonly string[] }): string[] {
  const paths = answer.query === query ? answer.paths : answer.paths.filter((path) => fuzzyMatches(query, path));
  return paths.slice(0, MAX_SUGGESTIONS);
}

/** The commands of a `thread.commands` result (`{ commands }`), for a thread whose state has none yet. */
export function commandsOf(result: unknown): AvailableSlashCommand[] {
  if (!isObject(result) || !Array.isArray(result.commands)) {
    return [];
  }
  return result.commands.filter(
    (command): command is AvailableSlashCommand => isObject(command) && typeof command.name === "string",
  );
}

/**
 * The draft after an abort: the restored steering and follow-up texts, oldest first, then what was being typed, one
 * blank line apart (UI-063).
 */
export function restoreDraft(draft: string, steering: readonly string[], followUp: readonly string[]): string {
  return [...steering, ...followUp, draft]
    .map((text) => text.trimEnd())
    .filter((text) => text.trim() !== "")
    .join("\n\n");
}

/**
 * The draft after the draft `failed` failed to send: its text, then what was typed since, one blank line apart; its
 * images, then the images attached since; its paste chips, and those typed since unless both have chips, whose numbers
 * would clash: the later ones then become their text. Nothing the user wrote is lost (UI-064, UI-357).
 */
export function restoreFailedPrompt(draft: Draft, failed: Draft): Draft {
  const since = failed.pastes.length === 0 || draft.pastes.length === 0 ? draft : expandDraft(draft);
  return {
    text: restoreDraft(since.text, [failed.text], []),
    images: [...failed.images, ...since.images],
    pastes: [...failed.pastes, ...since.pastes],
  };
}

/** A second `Ctrl-C` within this many milliseconds of the one that cleared the draft aborts (UI-063). */
export const INTERRUPT_WINDOW_MS = 1000;

/** What `Ctrl-C` in the composer does (UI-063). */
export type Interrupt = "clear" | "abort";

/**
 * `Ctrl-C` at `now` (UI-063): clears a non-empty draft; aborts on an empty one, or when the previous `Ctrl-C`
 * (`lastPress`) was at most {@link INTERRUPT_WINDOW_MS} ago.
 */
export function interruptAction(draft: Draft, now: number, lastPress: number | undefined): Interrupt {
  if (lastPress !== undefined && now - lastPress <= INTERRUPT_WINDOW_MS) {
    return "abort";
  }
  return draft.text === "" && draft.images.length === 0 ? "abort" : "clear";
}

/** The draft after `u` restored the draft `Ctrl-C` cleared, put back as a failed prompt is (UI-063). */
export function restoreCleared(draft: Draft, cleared: Draft): Draft {
  return restoreFailedPrompt(draft, cleared);
}

/** The texts of a `thread.abort` result (`{ steering: [{ text }], followUp: [{ text }] }`). */
export function restoredTexts(result: unknown): { steering: string[]; followUp: string[] } {
  const texts = (list: unknown) =>
    Array.isArray(list)
      ? list.flatMap((entry) => (isObject(entry) && typeof entry.text === "string" ? [entry.text] : []))
      : [];
  return isObject(result)
    ? { steering: texts(result.steering), followUp: texts(result.followUp) }
    : { steering: [], followUp: [] };
}

/** An image attachment from pasted or dropped bytes; `undefined` for anything but `image/*` (UI-060). */
export function imageAttachment(mime: string, bytes: Uint8Array): ImageContent | undefined {
  return mime.startsWith("image/") ? { type: "image", data: base64(bytes), mimeType: mime } : undefined;
}

/** Standard base64 with padding. */
export function base64(bytes: Uint8Array): string {
  const chunk = 0x8000;
  let binary = "";
  for (let start = 0; start < bytes.length; start += chunk) {
    binary += String.fromCharCode(...bytes.subarray(start, start + chunk));
  }
  return btoa(binary);
}
