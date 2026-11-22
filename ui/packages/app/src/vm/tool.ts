/**
 * Tool cards (UI-050): name, one-line argument summary, status, duration, and expandable pretty JSON arguments and
 * text result, and the result's images (UI-472); tools with a recorded shape add their specialized view
 * (`tool-view.ts`). Failed tools are distinct and expanded by default (UI-052).
 */

import type { JobState, JsonValue, ToolCall, ToolResultMessage, ToolRun, UserContent } from "@omfp/core";
import { isObject } from "../sdk/json.ts";
import { strings } from "../strings/index.ts";
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
  /** Pretty JSON, or the partial argument text while composing. */
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
 * Failed cards and live file mutations start open; successful history starts collapsed (UI-052, UI-695). A card of
 * `omfp_card` is open while it runs and while no user message follows its message (`turn` not `past`); a waiting
 * card only while its thread still works on that turn (`open`), so an answered question makes way (UI-922). Otherwise
 * collapsed, failed or not.
 */
export function expandedByDefault(card: ToolCard, turn: Turn = "past"): boolean {
  const active = card.status === "composing" || card.status === "pending" || card.status === "running";
  if (card.view.kind === "card") {
    return active || turn === "open" || (turn === "latest" && !card.view.wait);
  }
  return card.status === "failed" || (active && (card.view.kind === "edit" || card.view.kind === "write"));
}

/**
 * The card of one tool call. `call` is the assistant's tool-call block (absent when only the run is known), `run` the
 * execution tracked by the reducer, `result` the transcript's tool result message, `calledAt` the calling message's
 * completion timestamp. A `bash` call running one tool command (or an 18.x `write` to an `xd://` device) is the card of that tool (UI-915).
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
  // omp's own copy of the arguments (defaults filled in, its `i` intent field split off) once the run started.
  const written: JsonValue | undefined = run?.args ?? call?.arguments ?? undefined;
  const intent = call?.intent ?? run?.intent ?? undefined;
  const finishedAt = result?.timestamp;
  const value = run?.result ?? run?.partial_result ?? null;
  const returned = toolOutput(value ?? result);
  const device = deviceCall(called, written, partial, returned);
  const name = device?.name ?? called;
  const args: unknown = device === undefined ? written : device.args;
  const output = device === undefined ? returned : device.output;
  const job = run?.background ?? null;
  // A background job's state is the card's (UI-270): the call itself ended when the job started.
  const status =
    job === null ? statusOf(partial !== undefined && written === undefined, run, result) : JOB_STATUS[job.state];
  const summary = summaryOf(name, intent, args, partial);
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
    args: args !== undefined ? prettyJson(args) : (partial ?? "{}"),
    // A tool command's card shows the tool's output, without the exit line of the `bash` call that ran it.
    result:
      job === null
        ? called === "bash" && device !== undefined
          ? device.output?.text
          : resultOf(run, result)
        : (job.output ?? undefined),
    images: cardImages(output?.images ?? [], name, subjectOf(args) ?? summary),
    view: toolView(name, args, output, status === "succeeded", job, status === "succeeded" || status === "failed"),
  };
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
 * else, while the model writes the arguments of a tool without a specialized subject, their partial text.
 */
function summaryOf(name: string, intent: string | undefined, args: unknown, partial: string | undefined): string {
  if (intent !== undefined) {
    return oneLine(intent, SUMMARY_CHARS);
  }
  const shown = shownArguments(name);
  if (isObject(args)) {
    return summarize(args, shown ?? []);
  }
  return partial === undefined || shown !== undefined ? "" : oneLine(partial, SUMMARY_CHARS);
}

function resultOf(run: ToolRun | undefined, result: ToolResultMessage | undefined): string | undefined {
  const value = run?.result ?? run?.partial_result ?? null;
  if (value !== null) {
    return resultText(value);
  }
  return result?.content === undefined ? undefined : userContentText(result.content);
}

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
  return run === undefined && composing ? "composing" : "pending";
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
