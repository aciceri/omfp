/**
 * Inline notice rows (UI-043): compaction, retries, omp notices, extension and frame errors as compact rows; frames
 * the reducer could not interpret as a generic row with expandable JSON. A local command's output and a URL omp asks
 * to open are cards (UI-345, UI-347) with the URLs omp opened (UI-346).
 */

import type { NoticeFrame } from "@omfp/core";
import { isObject } from "../sdk/json.ts";
import { strings } from "../strings/index.ts";
import { openedUrls } from "./opened-url.ts";
import { prettyJson } from "./tool.ts";

export type Level = "info" | "warning" | "error";

/** Stable kind token of a row: `command` the output of a local slash command, `open-url` omp's `open_url` request. */
export type NoticeKind =
  | "compaction"
  | "retry"
  | "notice"
  | "extension-error"
  | "frame-error"
  | "unknown"
  | "command"
  | "open-url";

export interface NoticeRow {
  readonly kind: NoticeKind;
  readonly level: Level;
  readonly text: string;
  /** Pretty JSON of an unknown frame, shown on demand. */
  readonly json: string | undefined;
  /** The URLs omp opened in the browser of the daemon's host or asks to open (UI-346, UI-347). */
  readonly urls: readonly string[];
}

const text = strings.chat.notice;

function row(kind: NoticeKind, level: Level, message: string): NoticeRow {
  return { kind, level, text: message, json: undefined, urls: [] };
}

/** The row of a notice frame. */
export function noticeRow(frame: NoticeFrame): NoticeRow {
  switch (frame.type) {
    case "auto_compaction_start":
      return row("compaction", "info", text.compacting(frame.data.reason, frame.data.action));
    case "manual_compaction_start":
      return row("compaction", "info", text.compactingManual);
    case "auto_compaction_end":
    case "manual_compaction_end":
      return compactionEnd(frame.data.aborted, frame.data.skipped, frame.data.errorMessage);
    case "auto_retry_start": {
      const { attempt, maxAttempts, delayMs, errorMessage } = frame.data;
      return row("retry", "warning", text.retrying(attempt, maxAttempts, delayMs / 1000, errorMessage));
    }
    case "auto_retry_end":
      return frame.data.success
        ? row("retry", "info", text.retrySucceeded(frame.data.attempt))
        : row("retry", "error", text.retryFailed(frame.data.attempt, frame.data.finalError ?? text.unknownError));
    case "notice": {
      const { level, message, source } = frame.data;
      return row("notice", level, source === undefined ? message : text.sourced(source, message));
    }
    case "extension_error": {
      const { extensionPath, event, error } = frame.data;
      return row("extension-error", "error", text.extensionError(extensionPath, event, error));
    }
    case "rpc_frame_error": {
      const { originalType, error } = frame.data;
      return row(
        "frame-error",
        "error",
        originalType === undefined ? text.frameError(error) : text.frameErrorOf(originalType, error),
      );
    }
    case "command_output":
      return { ...row("command", "info", frame.data.text), urls: openedUrls(frame) };
    case "open_url":
      return { ...row("open-url", "info", frame.data.instructions ?? text.openUrl), urls: openedUrls(frame) };
    case "unknown": {
      const kind = isObject(frame.data) ? frame.data.type : undefined;
      return {
        kind: "unknown",
        level: "info",
        text: typeof kind === "string" ? text.unrecognizedOf(kind) : text.unrecognized,
        json: prettyJson(frame.data),
        urls: [],
      };
    }
  }
}

/** A compaction's outcome; a skipped one (omp 19.2.0: `Nothing to compact (session too small)`, `Already compacted`)
 * carries its reason in `errorMessage` but failed nothing (UI-911). */
function compactionEnd(aborted: boolean, skipped: boolean | undefined, error: string | undefined): NoticeRow {
  if (skipped === true) {
    return row(
      "compaction",
      "info",
      error === undefined ? text.compactionSkipped : text.compactionSkippedBecause(error),
    );
  }
  if (error !== undefined) {
    return row("compaction", "error", text.compactionFailed(error));
  }
  if (aborted) {
    return row("compaction", "warning", text.compactionAborted);
  }
  return row("compaction", "info", text.compacted);
}

/**
 * Whether `notices[index]` is the output omp's `/compact` command prints for a compaction it skipped (`Compaction
 * failed: <reason>`, omp 19.2.0): the skipped row just before it already says why, so the transcript leaves it out
 * (UI-911). Only when the latest compaction end or command output before it is that skipped end, with the same reason.
 */
export function repeatsSkippedCompaction(notices: readonly { readonly frame: NoticeFrame }[], index: number): boolean {
  const frame = notices[index]?.frame;
  if (frame?.type !== "command_output") {
    return false;
  }
  for (let before = index - 1; before >= 0; before--) {
    const earlier = notices[before]?.frame;
    switch (earlier?.type) {
      case "auto_compaction_end":
      case "manual_compaction_end": {
        const { skipped, errorMessage } = earlier.data;
        return (
          skipped === true && errorMessage !== undefined && frame.data.text === `${OMP_COMPACT_FAILED}${errorMessage}`
        );
      }
      case "command_output":
        return false;
      default:
        break;
    }
  }
  return false;
}

/** How omp's `/compact` command starts its output for a compaction that did not happen. */
const OMP_COMPACT_FAILED = "Compaction failed: ";
