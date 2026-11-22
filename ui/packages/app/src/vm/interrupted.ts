/**
 * A turn a daemon restart interrupted (UI-932, PRT-229): the summary's `interruptedAt` says when the daemon stopped
 * while the thread's agent worked; the chat shows it with Resume (`thread.resume`) until something continues the turn.
 */

import type { ThreadSummary } from "@omfp/protocol";
import { strings } from "../strings/index.ts";
import { formatWhen } from "./jobs.ts";

/** What the chat's notice of an interrupted turn shows. */
export interface InterruptedTurn {
  /** When the daemon stopped (unix ms). */
  readonly at: number;
  /** The notice: the turn was interrupted, and when ({@link formatWhen}: the date too unless on the day of `now`). */
  readonly text: string;
}

/** The notice of `summary`'s interrupted turn, `undefined` when it has none. */
export function interruptedTurn(
  summary: Pick<ThreadSummary, "interruptedAt"> | undefined,
  now: number = Date.now(),
): InterruptedTurn | undefined {
  const at = summary?.interruptedAt;
  if (at === undefined || at === null) {
    return undefined;
  }
  return { at, text: strings.chat.interrupted.text(formatWhen(at, now)) };
}
