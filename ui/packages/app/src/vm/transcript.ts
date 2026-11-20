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
import { type NoticeRow, noticeRow, repeatsSkippedCompaction } from "./notice.ts";
import { type SystemRow, systemMessage, systemNotice } from "./system.ts";
import { type CardStatus, oneLine, prettyJson, type ToolCard, toolCard } from "./tool.ts";

/**
 * What an item renders: `state.transcript[index]`, `state.notices[index]` or `state.streaming`. A turn runs from a
 * user's message to the next (a steer or follow-up starts one too): an assistant message after the turn's first
 * `continues` it, shown under the first one's name line without its own (UI-470). A message sent on another day than
 * the message shown before it carries a `day` mark, shown as a day separator above it (UI-750). A message whose reads
 * run on into the next messages of its turn shows their reads too (UI-765): `joined` are those messages' indexes,
 * and `live` says the reply being streamed joins it as well (it then has no item of its own).
 */
export type ItemKind =
  | {
      readonly type: "message";
      readonly index: number;
      readonly continues: boolean;
      readonly day: DayMark | undefined;
      readonly joined: readonly number[];
      readonly live: boolean;
    }
  | { readonly type: "notice"; readonly index: number }
  | { readonly type: "streaming"; readonly continues: boolean };

/**
 * A day separator before a message (UI-750): when the message was sent, and whether it is the transcript's first
 * message with a send time (its separator shows only when that day is not today).
 */
export interface DayMark {
  readonly at: number;
  readonly first: boolean;
}

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

const TRANSCRIPT_INDEXES = new WeakMap<readonly TranscriptEntry[], Derived<TranscriptIndex>>();

function transcriptIndex(transcript: readonly TranscriptEntry[]): TranscriptIndex {
  return derive(TRANSCRIPT_INDEXES, transcript, [], () => {
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
    return { called, results };
  });
}

const RUN_INDEXES = new WeakMap<readonly ToolRun[], Derived<ReadonlyMap<string, ToolRun>>>();

/** The first run of each tool call id. */
function runsById(tools: readonly ToolRun[]): ReadonlyMap<string, ToolRun> {
  return derive(RUN_INDEXES, tools, [], () => {
    const byId = new Map<string, ToolRun>();
    for (const run of tools) {
      if (!byId.has(run.tool_call_id)) {
        byId.set(run.tool_call_id, run);
      }
    }
    return byId;
  });
}

/** The run and the result of the tool call `id`. */
function callInputs(state: ThreadState, id: string): [ToolRun | undefined, ToolResultMessage | undefined] {
  return [runsById(state.tools).get(id), transcriptIndex(state.transcript).results.get(id)];
}

/** The last item list built for a transcript array, with the notices and streaming flags it was built for. */
interface ItemList {
  readonly notices: readonly Notice[];
  readonly streaming: boolean;
  readonly liveReads: boolean;
  readonly showXdevNotices: boolean;
  readonly items: readonly Item[];
}

const ITEM_LISTS = new WeakMap<readonly TranscriptEntry[], ItemList>();

/** The `joined` of a message whose reads do not run on. */
const NOT_JOINED: readonly number[] = [];

/** A `read` of a file or a folder, which a read group takes (UI-765); a URL's read is a fetch, shown on its own. */
function readsFile(block: AssistantContent): boolean {
  if (block.type !== "toolCall" || block.name !== "read") {
    return false;
  }
  const path = block.arguments?.path;
  return typeof path !== "string" || !/^https?:\/\//i.test(path);
}

/** Text or thinking without a visible character: it shows nothing, so it does not end a run of reads. */
function blankContent(block: AssistantContent): boolean {
  return (
    (block.type === "text" && (block.text ?? "").trim() === "") ||
    (block.type === "thinking" && (block.thinking ?? "").trim() === "")
  );
}

/** Whether `message` shows nothing but reads of files (at least one), so they join the reads before it (UI-765). */
function onlyReads(message: AssistantMessage): boolean {
  const content = message.content ?? [];
  return (
    message.errorMessage === undefined &&
    content.some(readsFile) &&
    content.every((block) => readsFile(block) || blankContent(block))
  );
}

/** Whether the last thing `message` shows is a read of a file, which reads in the next messages continue. */
function endsWithRead(message: AssistantMessage): boolean {
  const last = (message.content ?? []).findLast((block) => !blankContent(block));
  return message.errorMessage === undefined && last !== undefined && readsFile(last);
}

/**
 * The transcript's rows in order: each notice goes after the messages that preceded it, tool results shown inside
 * their call's card are left out, and the streaming reply comes last; each message and the streaming reply say
 * whether they continue their turn (UI-470). Informational xdev notices are hidden unless opted in (UI-043); omp's
 * `/compact` output repeating the skipped compaction before it always is (UI-911).
 * An assistant message showing only reads joins the row before it when that row ends with reads and nothing came
 * between them (a notice, a record, a day separator), and so does the streaming reply (UI-765).
 * The same array while the transcript, notices, streaming flags and visibility setting stay the same.
 */
export function items(state: ThreadState, showXdevNotices = false): readonly Item[] {
  const streaming = state.streaming !== null;
  const liveReads = state.streaming !== null && onlyReads(state.streaming.message);
  const known = ITEM_LISTS.get(state.transcript);
  if (
    known !== undefined &&
    known.notices === state.notices &&
    known.streaming === streaming &&
    known.liveReads === liveReads &&
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
      if (repeatsSkippedCompaction(state.notices, notice)) {
        continue;
      }
      result.push({ key: `n${notice}`, kind: { type: "notice", index: notice } });
    }
  };
  // Whether the current turn has shown an assistant message (and so its name line) yet.
  let answered = false;
  // The local day of the last shown message with a send time.
  let lastDay: number | undefined;
  // The `joined` of the last row while it ends with reads, which a message showing only reads continues (UI-765).
  let open: number[] | undefined;
  // How many rows there were once the open row was added: one added after it closes it.
  let openAt = 0;
  for (const [index, { message }] of state.transcript.entries()) {
    noticesUpTo(index);
    if (message.role === "toolResult" && message.toolCallId !== undefined && called.has(message.toolCallId)) {
      continue;
    }
    const at = sentAt(message);
    const day = at === undefined ? undefined : startOfDay(at);
    const mark = at === undefined || day === lastDay ? undefined : { at, first: lastDay === undefined };
    lastDay = day ?? lastDay;
    const assistant = message.role === "assistant";
    if (assistant && open !== undefined && openAt === result.length && mark === undefined && onlyReads(message)) {
      open.push(index);
      continue;
    }
    const joined = assistant && endsWithRead(message) ? [] : undefined;
    result.push({
      key: `m${index}`,
      kind: {
        type: "message",
        index,
        continues: assistant && answered,
        day: mark,
        joined: joined ?? NOT_JOINED,
        live: false,
      },
    });
    open = joined;
    openAt = result.length;
    answered = assistant || (answered && message.role !== "user");
  }
  noticesUpTo(Number.POSITIVE_INFINITY);
  const last = result.at(-1);
  if (liveReads && open !== undefined && openAt === result.length && last?.kind.type === "message") {
    result[result.length - 1] = { ...last, kind: { ...last.kind, live: true } };
  } else if (streaming) {
    result.push({
      key: "streaming",
      kind: { type: "streaming", continues: answered },
    });
  }
  ITEM_LISTS.set(state.transcript, { notices: state.notices, streaming, liveReads, showXdevNotices, items: result });
  return result;
}

const LAST_USER = new WeakMap<readonly TranscriptEntry[], Derived<number>>();

/** The index of the transcript's last user message, `-1` without one; computed once per transcript array. */
function lastUserIndex(transcript: readonly TranscriptEntry[]): number {
  return derive(LAST_USER, transcript, [], () => transcript.findLastIndex(({ message }) => message.role === "user"));
}

/**
 * Whether `item` belongs to the turn its thread still works on (UI-922): the thread is not settled and no user message
 * follows the item's message. The reply being streamed belongs to it while the thread works; a notice never does.
 */
export function inOpenTurn(state: ThreadState, item: Item): boolean {
  if (state.settled) {
    return false;
  }
  switch (item.kind.type) {
    case "streaming":
      return true;
    case "message":
      return item.kind.index >= lastUserIndex(state.transcript);
    case "notice":
      return false;
  }
}

/** A message's send time (omp's `timestamp`, ms): none when absent or not positive (recorded fixtures zero them). */
function sentAt(message: AgentMessage): number | undefined {
  const { timestamp } = message;
  return typeof timestamp === "number" && Number.isFinite(timestamp) && timestamp > 0 ? timestamp : undefined;
}

/** The start of the local calendar day holding `at` (ms). */
export function startOfDay(at: number): number {
  const date = new Date(at);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/** What a day separator shows: its label and the local date (`YYYY-MM-DD`) for `<time dateTime>`. */
export interface DaySeparator {
  readonly label: string;
  readonly date: string;
}

/**
 * The separator a day mark shows at `now` (UI-750), in the local time zone: `Today`, `Yesterday`, else the short
 * weekday, day and month (`Thu 12 Nov`), with the year when it is not this year's. None for the first message when it
 * was sent today. `locales` defaults to the user's.
 */
export function daySeparator(mark: DayMark, now: number, locales?: Intl.LocalesArgument): DaySeparator | undefined {
  const day = new Date(startOfDay(mark.at));
  const today = new Date(startOfDay(now));
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const date = `${String(day.getFullYear()).padStart(4, "0")}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;
  if (day.getTime() === today.getTime()) {
    return mark.first ? undefined : { label: strings.chat.today, date };
  }
  if (day.getTime() === yesterday.getTime()) {
    return { label: strings.chat.yesterday, date };
  }
  const year = day.getFullYear() === today.getFullYear() ? {} : { year: "numeric" as const };
  const label = new Intl.DateTimeFormat(locales, { weekday: "short", day: "numeric", month: "short", ...year }).format(
    day,
  );
  return { label, date };
}

/** Who wrote a message; `record` for tool results without a visible call, bash executions, summaries, … */
export type Role = "user" | "assistant" | "record";

/** One displayed part of a message. */
export type Block =
  /** Markdown for the assistant (UI-040), plain text for everyone else. */
  | { readonly type: "text"; readonly text: string }
  /** Collapsed by default (UI-041); `key` preserves an explicit choice while a live message settles (UI-694). */
  | { readonly type: "thinking"; readonly text: string; readonly key?: string }
  | { readonly type: "image"; readonly mime: string; readonly data: string }
  | { readonly type: "tool"; readonly card: ToolCard }
  /**
   * A run of two or more consecutive `read` calls of one message, shown as one card (UI-765): their cards in call order
   * and the run's state, `running` while any of them is live, else `failed` when any failed.
   */
  | { readonly type: "readGroup"; readonly cards: readonly ToolCard[]; readonly status: CardStatus }
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
/** Read groups by the block of their first read; the same group while its reads' blocks stay. */
const GROUPS = new WeakMap<Block, Derived<Block>>();
/** Views of rows whose reads continue in the next messages (UI-765), by the row's own message. */
const JOINED = new WeakMap<AgentMessage, Derived<MessageView>>();

/** The run and the result of each tool call of `message`: what its view depends on besides the message itself. */
function toolInputs(state: ThreadState, message: AgentMessage): unknown[] {
  return message.role === "assistant"
    ? (message.content ?? []).flatMap((block) => (block.type === "toolCall" ? callInputs(state, block.id ?? "") : []))
    : [];
}

/** The view of `state.transcript[index]`; the same object while the message, its tool runs and results stay. */
export function messageView(state: ThreadState, index: number): MessageView {
  const entry = state.transcript[index];
  if (entry === undefined) {
    return { role: "record", label: strings.chat.missingMessage, blocks: [], error: undefined, steering: false };
  }
  const { message } = entry;
  return derive(VIEWS, message, [entry.message_id, ...toolInputs(state, message)], () => {
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
        return assistantView(state, message, entry.message_id);
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
  const { message, message_id: messageId } = state.streaming;
  return derive(VIEWS, message, [messageId, ...toolInputs(state, message)], () =>
    assistantView(state, message, messageId),
  );
}

function assistantView(state: ThreadState, message: AssistantMessage, messageId: string | null): MessageView {
  return {
    role: "assistant",
    label: strings.chat.roleAssistant,
    blocks: groupReads(assistantBlocks(state, message, messageId)),
    error: message.errorMessage,
    steering: false,
  };
}

/** The display blocks of an assistant message's content, before its reads are grouped. */
function assistantBlocks(state: ThreadState, message: AssistantMessage, messageId: string | null): Block[] {
  const completedAt = typeof message.completedAt === "number" ? message.completedAt : undefined;
  const calledAt = completedAt ?? message.timestamp;
  return (message.content ?? []).flatMap((block, index): Block[] => {
    const thinkingKey = messageId === null ? {} : { key: `message/${messageId}/thinking/${index}` };
    switch (block.type) {
      case "text":
        return [derive(BLOCKS, block, [], () => ({ type: "text", text: block.text ?? "" }))];
      case "thinking":
        return [
          derive(BLOCKS, block, [messageId, index], () => ({
            type: block.narration === true ? "text" : "thinking",
            text: block.thinking ?? "",
            ...(block.narration === true ? {} : thinkingKey),
          })),
        ];
      case "redactedThinking":
        return [
          derive(BLOCKS, block, [messageId, index], () => ({
            type: "thinking",
            text: strings.chat.redactedThinking,
            ...thinkingKey,
          })),
        ];
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
        const [run, result] = callInputs(state, id);
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
}

/**
 * The view of a row whose reads continue in the next messages of its turn (UI-765): the message at `index`, then the
 * reads of the transcript's messages `joined` and, when `live`, of the reply being streamed. The same object while
 * those messages, their tool runs and results stay.
 */
function joinedView(state: ThreadState, index: number, joined: readonly number[], live: boolean): MessageView {
  const parts: { message: AssistantMessage; messageId: string | null }[] = [];
  for (const at of [index, ...joined]) {
    const entry = state.transcript[at];
    if (entry?.message.role === "assistant") {
      parts.push({ message: entry.message, messageId: entry.message_id });
    }
  }
  if (live && state.streaming !== null) {
    parts.push({ message: state.streaming.message, messageId: state.streaming.message_id });
  }
  const [first] = parts;
  if (first === undefined) {
    return messageView(state, index);
  }
  const inputs = parts.flatMap(({ message, messageId }) => [message, messageId, ...toolInputs(state, message)]);
  return derive(JOINED, first.message, inputs, () => ({
    role: "assistant",
    label: strings.chat.roleAssistant,
    blocks: groupReads(parts.flatMap(({ message, messageId }) => assistantBlocks(state, message, messageId))),
    error: first.message.errorMessage,
    steering: false,
  }));
}

type ToolBlock = Extract<Block, { type: "tool" }>;

/** A call the read group takes: a `read` of a file or a folder (a URL's read is a fetch, shown on its own). */
function groupsRead(block: Block): block is ToolBlock {
  return block.type === "tool" && block.card.name === "read" && block.card.view.kind === "read";
}

/**
 * `blocks` with each run of two or more consecutive `read` calls in one group (UI-765); any other visible block (text,
 * thinking, an image, another tool) ends the run, blank text or thinking inside a run is left out, and a lone read
 * keeps its own card.
 */
function groupReads(blocks: readonly Block[]): Block[] {
  const grouped: Block[] = [];
  let run: ToolBlock[] = [];
  // Blank blocks after the run's last read: dropped if another read follows, else shown where they are.
  let blanks: Block[] = [];
  const close = () => {
    const [first] = run;
    if (first !== undefined && run.length > 1) {
      const reads = run;
      grouped.push(derive(GROUPS, first, reads, () => readGroup(reads)));
    } else {
      grouped.push(...run);
    }
    grouped.push(...blanks);
    run = [];
    blanks = [];
  };
  for (const block of blocks) {
    if (groupsRead(block)) {
      run.push(block);
      blanks = [];
    } else if (run.length > 0 && (block.type === "text" || block.type === "thinking") && block.text.trim() === "") {
      // Blank text or thinking shows nothing, so it does not end the run.
      blanks.push(block);
    } else {
      close();
      grouped.push(block);
    }
  }
  close();
  return grouped;
}

function readGroup(reads: readonly ToolBlock[]): Block {
  const cards = reads.map((read) => read.card);
  const live = cards.some((card) => card.status !== "succeeded" && card.status !== "failed");
  const failed = cards.some((card) => card.status === "failed");
  return { type: "readGroup", cards, status: live ? "running" : failed ? "failed" : "succeeded" };
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
            view:
              item.kind.joined.length === 0 && !item.kind.live
                ? messageView(state, item.kind.index)
                : joinedView(state, item.kind.index, item.kind.joined, item.kind.live),
            streaming: item.kind.live,
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
