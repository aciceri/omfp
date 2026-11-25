/**
 * Bounded tool-card bodies (UI-935): how much of a long body (a diff, code, a command, an output, a page, JSON) a card
 * shows. While its call is written, waits or runs, a window about `BODY_LINES` tall follows the newest lines, an
 * "… N earlier lines" marker above it, and pauses while the user scrolls it up; once the call ended, a preview of
 * `BODY_LINES` lines (a diff's of `DIFF_LINES` lines and `DIFF_HUNKS` hunks) expands on demand to the whole body,
 * which the card scrolls within about a viewport. These are omp's TUI limits (`PREVIEW_LIMITS`, the streaming tails of
 * `tools/edit.ts` and `tools/write.ts`).
 */

import type { DiffLine } from "@omfp/kit";
import type { ToolCard } from "./tool.ts";

/** Lines of a live window, and of a settled body's preview. */
export const BODY_LINES = 12;
/** Lines of a settled diff's preview. */
export const DIFF_LINES = 40;
/** Hunks of a settled diff's preview. */
export const DIFF_HUNKS = 8;

/** The lines of a text body: a trailing newline ends its last line rather than starting an empty one. */
export function textLines(text: string): string[] {
  const lines = text.split("\n");
  if (lines.length > 1 && lines.at(-1) === "") {
    lines.pop();
  }
  return lines;
}

/** Whether a card's body is live: its call is being written, waits or runs, or its background command still runs. */
export function liveBody(card: ToolCard): boolean {
  return (
    card.status === "composing" ||
    card.status === "pending" ||
    card.status === "running" ||
    (card.view.kind === "bash" && card.view.background?.state === "running")
  );
}

/** The lines `[start, end)` of a settled body its preview shows. */
export interface Cut {
  readonly start: number;
  readonly end: number;
}

/**
 * The preview of a settled body of `total` lines: its first `BODY_LINES`, or its last for an output, whose end is what
 * matters (`tail`); `undefined` when the whole body fits.
 */
export function bodyCut(total: number, anchor: "head" | "tail" = "head"): Cut | undefined {
  if (total <= BODY_LINES) {
    return undefined;
  }
  return anchor === "head" ? { start: 0, end: BODY_LINES } : { start: total - BODY_LINES, end: total };
}

/** The preview of a settled diff: its first `DIFF_LINES` lines, ending before its hunk past `DIFF_HUNKS`. */
export function diffCut(lines: readonly DiffLine[]): Cut | undefined {
  let end = Math.min(lines.length, DIFF_LINES);
  let hunks = 0;
  for (const [index, line] of lines.entries()) {
    if (index >= end) {
      break;
    }
    if (line.kind === "hunk" && ++hunks > DIFF_HUNKS) {
      end = index;
    }
  }
  return end >= lines.length ? undefined : { start: 0, end };
}

/** Where a live window is scrolled, in pixels: its scroll offset, its content's height and its own. */
export interface WindowScroll {
  readonly top: number;
  readonly height: number;
  readonly client: number;
}

/**
 * Whether a live window follows its newest lines once scrolled to `scroll`: at its end (a pixel of slack for rounding)
 * it does; moved by the user away from `written`, where it last scrolled itself, it stops; otherwise it keeps
 * `following`.
 */
export function follows(following: boolean, scroll: WindowScroll, written: number): boolean {
  if (scroll.top + scroll.client >= scroll.height - 2) {
    return true;
  }
  return Math.abs(scroll.top - written) > 1 ? false : following;
}

/**
 * The lines of a live window's `total` above what it shows: estimated from where it is scrolled once it is laid out,
 * else every line before its last `BODY_LINES`.
 */
export function earlierLines(total: number, scroll: WindowScroll): number {
  if (scroll.height <= 0) {
    return Math.max(0, total - BODY_LINES);
  }
  if (scroll.height <= scroll.client) {
    return 0;
  }
  return Math.min(total, Math.round((total * scroll.top) / scroll.height));
}
