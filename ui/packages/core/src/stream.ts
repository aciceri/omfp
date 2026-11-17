/**
 * Rebuilding a streaming assistant message from `messageUpdates: "delta"` events (port of `omfp-core/src/stream.rs`;
 * omp `docs/rpc.md`, "Event Stream Schema"): `*_start` carries the block shell with its streamed content emptied,
 * deltas append to the block at `contentIndex`, and `*_end` is authoritative for that block. Every function returns a
 * new message and leaves its inputs untouched.
 */

import { isObject } from "./json.ts";
import type { StreamingMessage } from "./state.ts";
import {
  type AssistantContent,
  type AssistantMessage,
  type AssistantMessageEvent,
  type AssistantMessageEventType,
  assistantContent,
  assistantContentList,
} from "./wire.ts";

/** The block kinds that stream a string, by their `type`. */
type Streamed = "text" | "thinking" | "toolCall";

/** Key of the streamed string of each kind; a tool call's is omp's own name for its partial argument JSON. */
const STREAMED_KEY: { [K in Streamed]: string } = { text: "text", thinking: "thinking", toolCall: "partialArgs" };

/** An empty assistant message with id `messageId`. */
export function newStreamingMessage(messageId: string | null): StreamingMessage {
  return { message_id: messageId, message: { role: "assistant", content: [] } };
}

/** `block` with its streamed string set by `update` (from the current string, `""` when missing or not a string). */
function withStreamed(block: AssistantContent, kind: Streamed, update: (current: string) => string): AssistantContent {
  if (block.type !== kind) {
    return block;
  }
  const key = STREAMED_KEY[kind];
  const current = block[key];
  return { ...block, [key]: update(typeof current === "string" ? current : "") };
}

/** A `contentIndex` usable as an array index (omp-rpc: an `i64` that fits a `usize`). */
function slot(index: unknown): number | undefined {
  return typeof index === "number" && Number.isSafeInteger(index) && index >= 0 ? index : undefined;
}

/** Sets the block at `index`, appending when the index is past the end. Undecodable content starts over. */
function put(message: AssistantMessage, index: number, block: AssistantContent): AssistantMessage {
  const content = [...(assistantContentList(message.content) ?? [])];
  if (index < content.length) {
    content[index] = block;
  } else {
    content.push(block);
  }
  return { ...message, content };
}

/** The block an event puts at `index` of `message`; `undefined` when it changes nothing. */
type Step = (message: AssistantMessage, event: AssistantMessageEvent, index: number) => AssistantContent | undefined;

/** `*_start`: the carried shell (or an empty block of `kind`) with its streamed string emptied. */
function begin(kind: Streamed): Step {
  return (_message, event) => withStreamed(assistantContent(event.block) ?? { type: kind }, kind, () => "");
}

/** `*_delta`: appends to the block at `index` when it is of `kind`. */
function append(kind: Streamed): Step {
  return (message, event, index) => {
    const delta = event.delta;
    const block = assistantContentList(message.content)?.[index];
    return typeof delta === "string" && block?.type === kind
      ? withStreamed(block, kind, (current) => current + delta)
      : undefined;
  };
}

/** `text_end` / `thinking_end`: the carried shell (else the rebuilt block) with `content` as its text. */
function finish(kind: Streamed): Step {
  return (message, event, index) => {
    const content = typeof event.content === "string" ? event.content : undefined;
    const block = assistantContent(event.block) ?? assistantContentList(message.content)?.[index] ?? { type: kind };
    return withStreamed(block, kind, (current) => content ?? current);
  };
}

/** `image_end` / `toolcall_end`: the complete block carried under `key`, as a block of `type`. */
function carried(key: string, type: "image" | "toolCall"): Step {
  return (_message, event) => {
    const block = event[key];
    return isObject(block) ? { ...block, type } : undefined;
  };
}

/** The step of every event that rebuilds a block; `start` replaces the message, `done` and `error` carry nothing to
 * rebuild (`message_end` follows). */
const STEPS: { [T in AssistantMessageEventType]?: Step } = {
  text_start: begin("text"),
  thinking_start: begin("thinking"),
  toolcall_start: begin("toolCall"),
  text_delta: append("text"),
  thinking_delta: append("thinking"),
  toolcall_delta: append("toolCall"),
  text_end: finish("text"),
  thinking_end: finish("thinking"),
  image_end: carried("content", "image"),
  toolcall_end: carried("toolCall", "toolCall"),
};

function rebuild(message: AssistantMessage, event: AssistantMessageEvent): AssistantMessage {
  if (event.type === "start") {
    // omp-rpc decodes `message` as an open record: any object does.
    return isObject(event.message) ? (event.message as AssistantMessage) : message;
  }
  const step = STEPS[event.type];
  const index = slot(event.contentIndex);
  if (step === undefined || index === undefined) {
    return message;
  }
  const block = step(message, event, index);
  return block === undefined ? message : put(message, index, block);
}

/** Applies one delta-mode event. */
export function applyStreamEvent(streaming: StreamingMessage, event: AssistantMessageEvent): StreamingMessage {
  const next = rebuild(streaming.message, event);
  return next === streaming.message ? streaming : { ...streaming, message: next };
}
