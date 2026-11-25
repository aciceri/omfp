/**
 * Tool cards (UI-050): name, one-line argument summary, status, duration, and expandable pretty JSON arguments and
 * text result, and the result's images (UI-472); tools with a recorded shape add their specialized view
 * (`tool-view.ts`). Failed tools are distinct and expanded by default (UI-052). While the model writes a call's
 * arguments, its card is built from the arguments parsed so far (UI-054), re-parsed only once the text grew enough.
 */

import type { JobState, JsonValue, ToolCall, ToolResultMessage, ToolRun, UserContent } from "@omfp/core";
import { isObject } from "../sdk/json.ts";
import { strings } from "../strings/index.ts";
import { parsePartialJson } from "./partial-json.ts";
import {
  deviceCall,
  quietFailure,
  type ResultImage,
  shownArguments,
  type ToolView,
  toolOutput,
  toolView,
} from "./tool-view.ts";

/** Longest argument summary. */
const SUMMARY_CHARS = 96;
/** Longest subject in an image's alt text. */
const SUBJECT_CHARS = 80;
/** The arguments naming what a call works on, by preference: the subject of its images' alt text. */
const SUBJECT_KEYS = ["path", "url", "file", "command", "code", "query", "pattern"] as const;
/** omp's tool-call intent field (`INTENT_FIELD`), which it splits off the arguments it runs the tool with. */
const INTENT_FIELD = "i";
/** Below this many characters of argument text, every delta re-parses it (UI-054). */
const REPARSE_ALWAYS = 2048;
/** Calls whose last parse is kept: the ones being written, and a few before them. */
const PARSES_KEPT = 16;
/** Characters of the parsed text checked against the next text: the cache's key is only the call's id. */
const HEAD_CHARS = 64;

/** `text` with whitespace runs collapsed to one space, cut to `max` characters with an ellipsis. */
export function oneLine(text: string, max: number): string {
  const flat = text.split(/\s+/).filter(Boolean).join(" ");
  const chars = [...flat];
  if (chars.length <= max) {
    return flat;
  }
  return `${chars.slice(0, Math.max(0, max - 1)).join("")}…`;
}

/** Pretty-printed JSON. */
export function prettyJson(value: unknown): string {
  return JSON.stringify(value, null, 2) ?? "null";
}

/** Progress shown on a card. */
export type CardStatus = "composing" | "pending" | "running" | "succeeded" | "failed";

/** Everything a tool card shows. */
export interface ToolCard {
  readonly toolCallId: string;
  readonly name: string;
  readonly summary: string;
  readonly status: CardStatus;
  /** Milliseconds from the end of the calling message to the result, when both carry timestamps. */
  readonly durationMs: number | undefined;
  /** Pretty JSON, of the arguments parsed so far while composing (UI-054); their text when it holds no object. */
  readonly args: string;
  /** Text parts of the result (or of the latest partial result), else its pretty JSON. */
  readonly result: string | undefined;
  /** The result's images, with their alt text (UI-472). */
  readonly images: readonly CardImage[];
  /** The specialized view of the tool (`generic` for the fallback card). */
  readonly view: ToolView;
}

/** An image of a call's result with its alt text, which names the tool and what it worked on (UI-472). */
export interface CardImage extends ResultImage {
  readonly alt: string;
}

/**
 * Where a message stands in its thread's conversation (UI-922): `open` in the turn the thread still works on, `latest`
 * in the latest exchange of a settled thread (no user message follows it), `past` once a user message follows it.
 */
export type Turn = "open" | "latest" | "past";

/**
 * Failed cards and live file mutations (an `edit`, a `write`, a widget mount's code) start open; successful history
 * starts collapsed (UI-052, UI-695). A card of `omfp_card` is open while it runs and while no user message follows its
 * message (`turn` not `past`); a waiting card only while its thread still works on that turn (`open`), so an answered
 * question makes way (UI-922). Otherwise collapsed, failed or not.
 */
export function expandedByDefault(card: ToolCard, turn: Turn = "past"): boolean {
  const active = card.status === "composing" || card.status === "pending" || card.status === "running";
  if (card.view.kind === "card") {
    return active || turn === "open" || (turn === "latest" && !card.view.wait);
  }
  const mutation = card.view.kind === "edit" || card.view.kind === "write" || card.view.kind === "mount";
  return card.status === "failed" || (active && mutation);
}

/**
 * The card of one tool call. `call` is the assistant's tool-call block (absent when only the run is known), `run` the
 * execution tracked by the reducer, `result` the transcript's tool result message, `calledAt` the calling message's
 * completion timestamp. A `bash` call running one tool command (or an 18.x `write` to an `xd://` device) is the card of that tool (UI-915).
 * While the model writes the arguments, the block's `arguments` are `{}` (or missing) beside their text so far
 * (`partialArgs`): the card is built from that text, parsed (UI-054).
 */
export function toolCard(
  toolCallId: string,
  call: ToolCall | undefined,
  run: ToolRun | undefined,
  result: ToolResultMessage | undefined,
  calledAt: number | undefined,
): ToolCard {
  const called = call?.name ?? run?.tool_name ?? result?.toolName ?? strings.chat.tool.fallbackName;
  const partial = call?.partialArgs;
  // `arguments` with a key are complete: `toolcall_end` carries them; omp's frames carry `{}` while they are written.
  const complete = Object.keys(call?.arguments ?? {}).length > 0;
  const composing = (run?.args ?? null) === null && partial !== undefined && !complete;
  const streamed = composing ? streamedArguments(toolCallId, partial) : undefined;
  // The arguments parsed so far, else omp's own copy once the run started (defaults filled in, its `i` intent field
  // split off), else the call's.
  const written: unknown = streamed?.args ?? run?.args ?? call?.arguments ?? undefined;
  // Between the end of the arguments and the start of the run, the intent is still in the arguments' `i` field.
  const intentField = isObject(written) ? written[INTENT_FIELD] : undefined;
  const intent =
    call?.intent ?? run?.intent ?? streamed?.intent ?? (typeof intentField === "string" ? intentField : undefined);
  const finishedAt = result?.timestamp;
  const value = run?.result ?? run?.partial_result ?? null;
  const returned = toolOutput(value ?? result);
  const device = deviceCall(called, written, returned);
  const name = device?.name ?? called;
  const args: unknown = device === undefined ? written : device.args;
  const output = device === undefined ? returned : device.output;
  const job = run?.background ?? null;
  // A background job's state is the card's (UI-270): the call itself ended when the job started.
  const status = job === null ? statusOf(composing, run, result) : JOB_STATUS[job.state];
  const summary = summaryOf(name, intent, args);
  const ended = status === "succeeded" || status === "failed";
  return {
    toolCallId,
    name,
    summary,
    status:
      status === "succeeded" &&
      job === null &&
      (quietFailure(name, output) !== undefined || (device?.exitCode ?? 0) !== 0)
        ? "failed"
        : status,
    durationMs:
      job === null && calledAt !== undefined && finishedAt !== undefined && calledAt > 0 && finishedAt >= calledAt
        ? finishedAt - calledAt
        : undefined,
    args:
      args === undefined
        ? (partial ?? "{}")
        : streamed !== undefined && args === streamed.args
          ? streamed.json
          : prettyJson(args),
    // A tool command's card shows the tool's output, without the exit line of the `bash` call that ran it.
    result:
      job === null
        ? called === "bash" && device !== undefined
          ? device.output?.text
          : resultOf(run, result)
        : (job.output ?? undefined),
    images: cardImages(output?.images ?? [], name, subjectOf(args) ?? summary),
    // omp's preview of the call (an `edit`'s diff) stands for its result until the call ends (UI-055).
    view: toolView(name, args, output, status === "succeeded", job, ended, ended ? null : (run?.preview ?? null)),
  };
}

/** A call's arguments parsed from their text so far (UI-054). */
interface StreamedArguments {
  /** Length of the text parsed, and its first characters. */
  readonly length: number;
  readonly head: string;
  /** The arguments without omp's intent field, which is `intent`. */
  readonly args: Readonly<Record<string, unknown>>;
  readonly intent: string | undefined;
  /** `args` as pretty JSON. */
  readonly json: string;
}

/** The last parse of the arguments of the calls being written, by call id, oldest first. */
const STREAMED = new Map<string, StreamedArguments>();

/**
 * The arguments of call `toolCallId` parsed from their text so far, `partial` (UI-054). Like omp's
 * `parseStreamingJsonThrottled`, a long text is parsed again only once it grew by a 32nd since its last parse, so
 * writing a long body costs a bounded number of parses; a short one is parsed on every delta.
 */
function streamedArguments(toolCallId: string, partial: string): StreamedArguments {
  const last = STREAMED.get(toolCallId);
  if (
    last !== undefined &&
    partial.length >= REPARSE_ALWAYS &&
    partial.length >= last.length &&
    partial.length - last.length < partial.length >> 5 &&
    partial.startsWith(last.head)
  ) {
    return last;
  }
  const parsed = parsePartialJson(partial);
  const object: Readonly<Record<string, unknown>> = isObject(parsed) ? parsed : {};
  const { [INTENT_FIELD]: intent, ...args } = object;
  const next: StreamedArguments = {
    length: partial.length,
    head: partial.slice(0, HEAD_CHARS),
    args,
    intent: typeof intent === "string" ? intent : undefined,
    json: prettyJson(args),
  };
  STREAMED.delete(toolCallId);
  STREAMED.set(toolCallId, next);
  for (const oldest of STREAMED.keys()) {
    if (STREAMED.size <= PARSES_KEPT) {
      break;
    }
    STREAMED.delete(oldest);
  }
  return next;
}

/** The first argument of {@link SUBJECT_KEYS} that is a string, on one line. */
function subjectOf(args: unknown): string | undefined {
  if (!isObject(args)) {
    return undefined;
  }
  const subject = SUBJECT_KEYS.map((key) => args[key]).find((value) => typeof value === "string" && value !== "");
  return typeof subject === "string" ? oneLine(subject, SUBJECT_CHARS) : undefined;
}

/** `images` with their alt text: which image of how many, from tool `name` working on `subject`. */
function cardImages(images: readonly ResultImage[], name: string, subject: string): CardImage[] {
  return images.map((image, index) => ({
    ...image,
    alt: strings.chat.tool.resultImage(name, subject, index + 1, images.length),
  }));
}

/** The card status of a background job's state; a cancelled job is drawn failed. */
const JOB_STATUS: Readonly<Record<JobState, CardStatus>> = {
  running: "running",
  completed: "succeeded",
  failed: "failed",
  cancelled: "failed",
};

/**
 * The header's one-line summary: the intent, else the arguments the card does not show elsewhere (`shownArguments`),
 * those parsed so far while the model writes them (UI-054).
 */
function summaryOf(name: string, intent: string | undefined, args: unknown): string {
  if (intent !== undefined) {
    return oneLine(intent, SUMMARY_CHARS);
  }
  return isObject(args) ? summarize(args, shownArguments(name) ?? []) : "";
}

function resultOf(run: ToolRun | undefined, result: ToolResultMessage | undefined): string | undefined {
  const value = run?.result ?? run?.partial_result ?? null;
  if (value !== null) {
    return resultText(value);
  }
  return result?.content === undefined ? undefined : userContentText(result.content);
}

/** `composing` while the model writes the arguments, even once omp sent a preview of the call (a pending run). */
function statusOf(composing: boolean, run: ToolRun | undefined, result: ToolResultMessage | undefined): CardStatus {
  if (run?.status === "failed" || result?.isError === true) {
    return "failed";
  }
  if (run?.status === "succeeded" || (run === undefined && result !== undefined)) {
    return "succeeded";
  }
  if (run?.status === "running") {
    return "running";
  }
  return composing ? "composing" : "pending";
}

/** `key: value` of the scalar arguments but `omitted`, by key, on one line. */
function summarize(args: Readonly<Record<string, unknown>>, omitted: readonly string[]): string {
  const parts = Object.keys(args)
    .filter((key) => !omitted.includes(key))
    .sort()
    .flatMap((key) => {
      const value = args[key];
      return typeof value === "string" || typeof value === "number" || typeof value === "boolean"
        ? [`${key}: ${String(value)}`]
        : [];
    });
  return oneLine(parts.join(", "), SUMMARY_CHARS);
}

/**
 * The text parts of a tool result value (`{content: [{type: "text", text}], …}`), its images as `[image]` (their
 * base64 is never text, UI-472), else its pretty JSON.
 */
export function resultText(value: JsonValue): string {
  if (isObject(value) && Array.isArray(value.content)) {
    const texts = value.content.flatMap((part) => {
      if (!isObject(part)) {
        return [];
      }
      if (part.type === "image") {
        return [strings.chat.tool.image];
      }
      return part.type === "text" && typeof part.text === "string" ? [part.text] : [];
    });
    if (texts.length > 0) {
      return texts.join("\n");
    }
  }
  return typeof value === "string" ? value : prettyJson(value);
}

function userContentText(content: readonly UserContent[]): string {
  return content.map((part) => (part.type === "text" ? (part.text ?? "") : strings.chat.tool.image)).join("\n");
}

/** `1.2 s`, `350 ms`, `2 min 5 s`; never negative: a duration below zero (clocks that disagree, UI-680) is `0 ms`. */
export function formatDuration(signed: number): string {
  const ms = Math.max(0, signed);
  const { duration } = strings.chat;
  if (ms < 1000) {
    return duration.ms(ms);
  }
  const seconds = ms / 1000;
  if (seconds < 60) {
    return duration.seconds(seconds);
  }
  const minutes = Math.floor(seconds / 60);
  return duration.minutes(minutes, seconds - minutes * 60);
}
