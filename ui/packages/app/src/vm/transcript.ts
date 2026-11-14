/**
 * The transcript as a flat list of items (messages, inline notices, the streaming reply) and the display blocks of
 * each message. Items only hold indices into the thread state, so building the list stays cheap for long transcripts
 * and the virtualized list (UI-042) renders the visible ones only.
 *
 * Streaming must not cost in proportion to the transcript (UI-191): the reducer shares every unchanged part of the
 * state, and this module keeps what it derives from those parts by identity — the item list per transcript, the
 * lookups of tool calls and results per transcript, each message's view while its tool runs and results stay the
 * same, each content block's display block. A delta then yields the same item list and the same row content for
 * every finished message, so their (memoized) rows do not render again; only the streaming reply's row does.
 */

import type {
  AgentMessage,
  AssistantContent,
  AssistantMessage,
  Notice,
  ThreadState,
  ToolResultMessage,
  ToolRun,
  TranscriptEntry,
  UserContent,
} from "@omfp/core";
import { isObject } from "../sdk/json.ts";
import { strings } from "../strings/index.ts";
import { type NoticeRow, noticeRow } from "./notice.ts";
import { type SystemRow, systemMessage, systemNotice } from "./system.ts";
import { oneLine, prettyJson, type ToolCard, toolCard } from "./tool.ts";

/**
 * What an item renders: `state.transcript[index]`, `state.notices[index]` or `state.streaming`. A turn runs from a
 * user's message to the next (a steer or follow-up starts one too): an assistant message after the turn's first
 * `continues` it, shown under the first one's name line without its own (UI-470).
 */
export type ItemKind =
  | { readonly type: "message"; readonly index: number; readonly continues: boolean }
  | { readonly type: "notice"; readonly index: number }
  | { readonly type: "streaming"; readonly continues: boolean };

/** One row of the transcript with a key that stays stable while the transcript grows. */
export interface Item {
  readonly key: string;
  readonly kind: ItemKind;
}

/** What one transcript array holds, looked up by tool call id: the calls it makes and the first result of each. */
interface TranscriptIndex {
  readonly called: ReadonlySet<string>;
  readonly results: ReadonlyMap<string, ToolResultMessage>;
}

const TRANSCRIPT_INDEXES = new WeakMap<readonly TranscriptEntry[], TranscriptIndex>();

function transcriptIndex(transcript: readonly TranscriptEntry[]): TranscriptIndex {
  let index = TRANSCRIPT_INDEXES.get(transcript);
  if (index === undefined) {
    const called = new Set<string>();
    const results = new Map<string, ToolResultMessage>();
    for (const { message } of transcript) {
      if (message.role === "assistant") {
        for (const block of message.content ?? []) {
          if (block.type === "toolCall" && block.id !== undefined) {
            called.add(block.id);
          }
        }
      } else if (
        message.role === "toolResult" &&
        message.toolCallId !== undefined &&
        !results.has(message.toolCallId)
      ) {
        results.set(message.toolCallId, message);
      }
    }
    index = { called, results };
    TRANSCRIPT_INDEXES.set(transcript, index);
  }
  return index;
}

const RUN_INDEXES = new WeakMap<readonly ToolRun[], ReadonlyMap<string, ToolRun>>();

/** The first run of each tool call id. */
function runsById(tools: readonly ToolRun[]): ReadonlyMap<string, ToolRun> {
  let runs = RUN_INDEXES.get(tools);
  if (runs === undefined) {
    const byId = new Map<string, ToolRun>();
    for (const run of tools) {
      if (!byId.has(run.tool_call_id)) {
        byId.set(run.tool_call_id, run);
      }
    }
    runs = byId;
    RUN_INDEXES.set(tools, runs);
  }
  return runs;
}

/** The last item list built for a transcript array, with the notices and streaming flag it was built for. */
interface ItemList {
  readonly notices: readonly Notice[];
  readonly streaming: boolean;
  readonly showXdevNotices: boolean;
  readonly items: readonly Item[];
}

const ITEM_LISTS = new WeakMap<readonly TranscriptEntry[], ItemList>();

/**
 * The transcript's rows in order: each notice goes after the messages that preceded it, tool results shown inside
 * their call's card are left out, and the streaming reply comes last; each message and the streaming reply say
 * whether they continue their turn (UI-470). Informational xdev notices are hidden unless opted in (UI-043).
 * The same array while the transcript, notices, streaming flag and visibility setting stay the same.
 */
export function items(state: ThreadState, showXdevNotices = false): readonly Item[] {
  const streaming = state.streaming !== null;
  const known = ITEM_LISTS.get(state.transcript);
  if (
    known !== undefined &&
    known.notices === state.notices &&
    known.streaming === streaming &&
    known.showXdevNotices === showXdevNotices
  ) {
    return known.items;
  }
  const { called } = transcriptIndex(state.transcript);
  const result: Item[] = [];
  let notice = 0;
  const noticesUpTo = (position: number) => {
    for (; notice < state.notices.length && (state.notices[notice]?.position ?? 0) <= position; notice++) {
      const entry = state.notices[notice];
      if (!showXdevNotices && entry !== undefined && systemNotice(entry.frame) !== undefined) {
        continue;
      }
      result.push({ key: `n${notice}`, kind: { type: "notice", index: notice } });
    }
  };
  // Whether the current turn has shown an assistant message (and so its name line) yet.
  let answered = false;
  for (const [index, { message }] of state.transcript.entries()) {
    noticesUpTo(index);
    if (message.role === "toolResult" && message.toolCallId !== undefined && called.has(message.toolCallId)) {
      continue;
    }
    const assistant = message.role === "assistant";
    result.push({ key: `m${index}`, kind: { type: "message", index, continues: assistant && answered } });
    answered = assistant || (answered && message.role !== "user");
  }
  noticesUpTo(Number.POSITIVE_INFINITY);
  if (streaming) {
    result.push({ key: "streaming", kind: { type: "streaming", continues: answered } });
  }
  ITEM_LISTS.set(state.transcript, { notices: state.notices, streaming, showXdevNotices, items: result });
  return result;
}

/** Who wrote a message; `record` for tool results without a visible call, bash executions, summaries, … */
export type Role = "user" | "assistant" | "record";

/** One displayed part of a message. */
export type Block =
  /** Markdown for the assistant (UI-040), plain text for everyone else. */
  | { readonly type: "text"; readonly text: string }
  /** Collapsed by default (UI-041). */
  | { readonly type: "thinking"; readonly text: string }
  | { readonly type: "image"; readonly mime: string; readonly data: string }
  | { readonly type: "tool"; readonly card: ToolCard }
  /** A message kind without its own renderer: a one-line summary and its JSON. */
  | { readonly type: "record"; readonly summary: string; readonly json: string };

/** A message ready to render. */
export interface MessageView {
  readonly role: Role;
  /** `You`, `Assistant`, or the record kind. */
  readonly label: string;
  readonly blocks: readonly Block[];
  /** The provider's error when the reply failed. */
  readonly error: string | undefined;
  /** A user message sent as steering. */
  readonly steering: boolean;
}

/** A view or block with the inputs it was derived from besides its source object. */
interface Derived<T> {
  readonly inputs: readonly unknown[];
  readonly value: T;
}

function sameInputs(a: readonly unknown[], b: readonly unknown[]): boolean {
  return a.length === b.length && a.every((input, index) => input === b[index]);
}

/** `make()` for `source` and `inputs`, or the value derived earlier from the same source and the same inputs. */
function derive<K extends object, T>(
  cache: WeakMap<K, Derived<T>>,
  source: K,
  inputs: readonly unknown[],
  make: () => T,
): T {
  const known = cache.get(source);
  if (known !== undefined && sameInputs(known.inputs, inputs)) {
    return known.value;
  }
  const value = make();
  cache.set(source, { inputs, value });
  return value;
}

const VIEWS = new WeakMap<AgentMessage, Derived<MessageView>>();
const BLOCKS = new WeakMap<AssistantContent, Derived<Block>>();

/** The run and the result of each tool call of `message`: what its view depends on besides the message itself. */
function toolInputs(state: ThreadState, message: AgentMessage): unknown[] {
  if (message.role !== "assistant") {
    return [];
  }
  const inputs: unknown[] = [];
  for (const block of message.content ?? []) {
    if (block.type === "toolCall") {
      const id = block.id ?? "";
      inputs.push(runsById(state.tools).get(id), transcriptIndex(state.transcript).results.get(id));
    }
  }
  return inputs;
}

/** The view of `state.transcript[index]`; the same object while the message, its tool runs and results stay. */
export function messageView(state: ThreadState, index: number): MessageView {
  const message = state.transcript[index]?.message;
  if (message === undefined) {
    return { role: "record", label: strings.chat.missingMessage, blocks: [], error: undefined, steering: false };
  }
  return derive(VIEWS, message, toolInputs(state, message), () => {
    switch (message.role) {
      case "user":
        return {
          role: "user",
          label: strings.chat.roleUser,
          blocks: contentBlocks(message.content),
          error: undefined,
          steering: message.steering === true,
        };
      case "assistant":
        return assistantView(state, message);
      default:
        return recordView(message);
    }
  });
}

/** The view of the reply being streamed, if any; unchanged blocks keep their identity from one delta to the next. */
export function streamingView(state: ThreadState): MessageView | undefined {
  if (state.streaming === null) {
    return undefined;
  }
  const { message } = state.streaming;
  return derive(VIEWS, message, toolInputs(state, message), () => assistantView(state, message));
}

function assistantView(state: ThreadState, message: AssistantMessage): MessageView {
  const completedAt = typeof message.completedAt === "number" ? message.completedAt : undefined;
  const calledAt = completedAt ?? message.timestamp;
  const blocks = (message.content ?? []).flatMap((block): Block[] => {
    switch (block.type) {
      case "text":
        return [derive(BLOCKS, block, [], () => ({ type: "text", text: block.text ?? "" }))];
      case "thinking":
        return [
          derive(BLOCKS, block, [], () => ({
            type: block.narration === true ? "text" : "thinking",
            text: block.thinking ?? "",
          })),
        ];
      case "redactedThinking":
        return [derive(BLOCKS, block, [], () => ({ type: "thinking", text: strings.chat.redactedThinking }))];
      case "image":
        return [
          derive(BLOCKS, block, [], () => ({
            type: "image",
            mime: block.mimeType ?? "image/png",
            data: block.data ?? "",
          })),
        ];
      case "toolCall": {
        const id = block.id ?? "";
        const run = runsById(state.tools).get(id);
        const result = transcriptIndex(state.transcript).results.get(id);
        return [
          derive(BLOCKS, block, [run, result, calledAt], () => ({
            type: "tool",
            card: toolCard(id, block, run, result, calledAt),
          })),
        ];
      }
      default:
        return [];
    }
  });
  return {
    role: "assistant",
    label: strings.chat.roleAssistant,
    blocks,
    error: message.errorMessage,
    steering: false,
  };
}

/** What a transcript row shows. */
export type RowContent =
  /** A message; one that `continues` its turn shows no name line (UI-470). */
  | { readonly type: "message"; readonly view: MessageView; readonly streaming: boolean; readonly continues: boolean }
  /** omp's own bookkeeping, as a muted row (UI-043). */
  | { readonly type: "system"; readonly row: SystemRow }
  | { readonly type: "notice"; readonly row: NoticeRow };

const ROWS = new WeakMap<object, RowContent>();

/** `content`, or the equal content cached for `source` (so a memoized row skips rendering). */
function rowFor(source: object, content: RowContent): RowContent {
  const known = ROWS.get(source);
  if (known !== undefined && sameContent(known, content)) {
    return known;
  }
  ROWS.set(source, content);
  return content;
}

function sameContent(a: RowContent, b: RowContent): boolean {
  switch (a.type) {
    case "message":
      return b.type === "message" && a.view === b.view && a.streaming === b.streaming && a.continues === b.continues;
    case "system":
    case "notice":
      return a.type === b.type;
  }
}

/**
 * What `item` shows; the same object while its inputs stay the same, so a delta of the streaming reply leaves every
 * other row's content as it was (UI-191). `undefined` when the item points past the state.
 */
export function rowContent(state: ThreadState, item: Item): RowContent | undefined {
  switch (item.kind.type) {
    case "message": {
      const message = state.transcript[item.kind.index]?.message;
      if (message === undefined) {
        return undefined;
      }
      const known = ROWS.get(message);
      if (known?.type === "system") {
        return known;
      }
      const system = systemMessage(message);
      return system === undefined
        ? rowFor(message, {
            type: "message",
            view: messageView(state, item.kind.index),
            streaming: false,
            continues: item.kind.continues,
          })
        : rowFor(message, { type: "system", row: system });
    }
    case "notice": {
      const notice = state.notices[item.kind.index];
      if (notice === undefined) {
        return undefined;
      }
      const known = ROWS.get(notice);
      if (known !== undefined) {
        return known;
      }
      const system = systemNotice(notice.frame);
      return rowFor(
        notice,
        system === undefined ? { type: "notice", row: noticeRow(notice.frame) } : { type: "system", row: system },
      );
    }
    case "streaming": {
      const view = streamingView(state);
      return view === undefined || state.streaming === null
        ? undefined
        : rowFor(state.streaming.message, { type: "message", view, streaming: true, continues: item.kind.continues });
    }
  }
}

function contentBlocks(content: string | readonly UserContent[] | undefined): Block[] {
  if (content === undefined) {
    return [];
  }
  if (typeof content === "string") {
    return [{ type: "text", text: content }];
  }
  return content.map((part) =>
    part.type === "text"
      ? { type: "text", text: part.text ?? "" }
      : { type: "image", mime: part.mimeType ?? "image/png", data: part.data ?? "" },
  );
}

/** Fields whose value summarizes a record, by preference. */
const SUMMARY_KEYS = ["content", "summary", "command", "code", "output", "text"] as const;

/** A generic row for message kinds without their own renderer: kind label, one-line summary, JSON (if summarized). */
function recordView(message: AgentMessage): MessageView {
  const fields: Readonly<Record<string, unknown>> = message;
  const key = SUMMARY_KEYS.find((name) => fields[name] !== undefined);
  const summary = oneLine(key === undefined ? "" : textOf(fields[key]), 200);
  return {
    role: "record",
    label: strings.chat.records[message.role] ?? message.role,
    blocks: summary === "" ? [] : [{ type: "record", summary, json: prettyJson(message) }],
    error: undefined,
    steering: false,
  };
}

/** A string, or the text parts of a content list, or compact JSON. */
function textOf(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  if (Array.isArray(value)) {
    return value.flatMap((part) => (isObject(part) && typeof part.text === "string" ? [part.text] : [])).join(" ");
  }
  return JSON.stringify(value) ?? "";
}
