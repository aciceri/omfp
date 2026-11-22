/**
 * omp's own bookkeeping in the transcript (UI-043): `custom` records omp keeps out of its own transcript
 * (`display: false`, e.g. the `<system-notice>` tool inventory it sends the model at startup) or that wrap a
 * `<system-notice>`, and its info notices about its own setup (the `toolCommands` mount notice, `xdev` in sessions recorded by omp 18.x, hidden unless opted in).
 * When visible, each renders as one muted `system` row: its first line, expandable to the whole text when there is more.
 * The spoken conversation of an ended live session (`omfp-voice`, DMN-113) is such a row too (UI-199): `Voice
 * conversation · N turns`, expandable to its turns.
 */

import type { AgentMessage, NoticeFrame } from "@omfp/core";
import { isObject } from "../sdk/json.ts";

export interface SystemRow {
  /** The first non-blank line of the text; the record's `customType` when it has no text. */
  readonly summary: string;
  /** The whole text, `undefined` when the summary is all of it (nothing to expand). */
  readonly text: string | undefined;
}

/** Sources of the omp notices that only report omp's own setup. */
const SYSTEM_NOTICE_SOURCES: readonly string[] = ["xdev", "toolCommands"];

/** The `customType` of the spoken conversation of an ended live session (DMN-113). */
const VOICE_LOG = "omfp-voice";

const SYSTEM_NOTICE_OPEN = "<system-notice>";
const SYSTEM_NOTICE_TAGS = /<\/?system-notice>/g;

/** The system row of `message`, `undefined` when it is not omp's bookkeeping. */
export function systemMessage(message: AgentMessage): SystemRow | undefined {
  if (message.role !== "custom") {
    return undefined;
  }
  const raw = contentText(message.content);
  if (message.customType === VOICE_LOG) {
    return voiceRow(raw, message.details);
  }
  if (message.display !== false && !raw.trimStart().startsWith(SYSTEM_NOTICE_OPEN)) {
    return undefined;
  }
  const text = raw.replace(SYSTEM_NOTICE_TAGS, "").trim();
  return systemRow(text === "" && typeof message.customType === "string" ? message.customType : text);
}

/** The system row of a notice frame, `undefined` when it is not omp's bookkeeping. */
export function systemNotice(frame: NoticeFrame): SystemRow | undefined {
  if (frame.type !== "notice") {
    return undefined;
  }
  const { level, message, source } = frame.data;
  return level === "info" && source !== undefined && SYSTEM_NOTICE_SOURCES.includes(source)
    ? systemRow(message)
    : undefined;
}

function systemRow(text: string): SystemRow {
  const whole = text.trim();
  const summary = whole.split("\n", 1)[0]?.trim() ?? "";
  return { summary, text: summary === whole ? undefined : whole };
}

/** The row of a voice conversation: its first line, then its turns (without the note to the agent). */
function voiceRow(content: string, details: unknown): SystemRow {
  const summary = content.trim().split("\n", 1)[0]?.trim() ?? "";
  const turns = isObject(details) && Array.isArray(details.turns) ? details.turns : [];
  const lines = turns.flatMap((turn) =>
    isObject(turn) && typeof turn.text === "string"
      ? [`${turn.role === "user" ? "User" : "Assistant"}: ${turn.text}`]
      : [],
  );
  return { summary, text: lines.length === 0 ? undefined : [summary, ...lines].join("\n") };
}

/** A string, or the text parts of a content list one per line. */
function contentText(content: unknown): string {
  if (typeof content === "string") {
    return content;
  }
  if (Array.isArray(content)) {
    return content.flatMap((part) => (isObject(part) && typeof part.text === "string" ? [part.text] : [])).join("\n");
  }
  return "";
}
