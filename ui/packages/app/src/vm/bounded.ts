/**
 * Bounded tool-card bodies (UI-935): how much of a long body (a diff, code, a command, an output, a page, JSON) or a
 * long list (files, matches, links, locations, diagnostics, symbols, questions, subagents, tasks) a card shows. While
 * its call is written, waits or runs, a window about `BODY_LINES` tall follows the newest lines, an "… N earlier
 * lines" marker above it, and pauses while the user scrolls it up; once the call ended, a preview of `BODY_LINES`
 * lines (a diff's of `DIFF_LINES` lines and `DIFF_HUNKS` hunks, each card's own number of items) expands on demand to
 * the whole body, which the card scrolls within about a viewport. These are omp's TUI limits (`PREVIEW_LIMITS`, the
 * streaming tails of `tools/edit.ts` and `tools/write.ts`, `COLLAPSED_AGENT_LIMIT`, `selectCollapsedTodos`).
 */

import type { DiffLine } from "@omfp/kit";
import type { ToolCard } from "./tool.ts";

/** Lines of a live window, and of a settled body's preview. */
export const BODY_LINES = 12;
/** Lines of a settled diff's preview. */
export const DIFF_LINES = 40;
/** Hunks of a settled diff's preview. */
export const DIFF_HUNKS = 8;
/** Lines of a settled `read`'s preview (omp's terminal preview). */
export const READ_LINES = 10;
/** Items of a settled list's preview: files, links, diagnostics, symbols, code actions, a phase's tasks. */
export const LIST_ITEMS = 8;
/** Matches of a search result's file shown before it is expanded. */
export const MATCH_ITEMS = 6;
/** Locations of an `lsp` answer shown before it is expanded. */
export const LOCATION_ITEMS = 2;
/** Lines of an `lsp` hover or plain answer shown before it is expanded. */
export const ANSWER_LINES = 4;
/** Questions of an `ask` shown before it is expanded. */
export const QUESTION_ITEMS = 3;
/** Subagents of a `task` shown before it is expanded. */
export const AGENT_ITEMS = 4;

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
 * The preview of a settled body of `total` lines or items: its first `limit`, or its last for an output, whose end is
 * what matters (`tail`); `undefined` when the whole body fits.
 */
export function bodyCut(total: number, limit = BODY_LINES, anchor: "head" | "tail" = "head"): Cut | undefined {
  if (total <= limit) {
    return undefined;
  }
  return anchor === "head" ? { start: 0, end: limit } : { start: total - limit, end: total };
}

/**
 * The tasks of a todo phase (by their `statuses`) its settled preview shows, in order; `undefined` when all fit in
 * `LIST_ITEMS`. As omp's `selectCollapsedTodos`: every task in progress, then the open tasks after the first of them
 * (or the first open ones), up to `LIST_ITEMS`, after the last closed task, so a task just finished stays in view.
 */
export function todoShown(statuses: readonly string[]): number[] | undefined {
  if (statuses.length <= LIST_ITEMS) {
    return undefined;
  }
  const indexes = statuses.map((_, index) => index);
  // Done or dropped.
  const closed = statuses.map((status) => status === "completed" || status === "abandoned");
  const open = indexes.filter((index) => !closed[index]);
  if (open.length === 0) {
    return indexes.slice(0, LIST_ITEMS);
  }
  const active = open.filter((index) => statuses[index] === "in_progress");
  const first = active[0] ?? open[0] ?? 0;
  const following = open.filter((index) => index >= first && !active.includes(index));
  const lead = indexes.filter((index) => closed[index]).slice(-1);
  return [...lead, ...[...active, ...following].slice(0, LIST_ITEMS)].sort((a, b) => a - b);
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
 * Whether a wheel turn of `deltaY` scrolls a body scrolled to `scroll` itself: it does unless the body is already at
 * its edge that way, where the wheel goes on to scroll the page around it.
 */
export function wheelScrolls(scroll: WindowScroll, deltaY: number): boolean {
  if (deltaY < 0) {
    return scroll.top > 0;
  }
  return deltaY > 0 && scroll.top + scroll.client < scroll.height - 1;
}

/**
 * The lines (or items) of a live window's `total` above what it shows: estimated from where it is scrolled once it is
 * laid out, else every one before its last `limit`.
 */
export function earlierLines(total: number, scroll: WindowScroll, limit = BODY_LINES): number {
  if (scroll.height <= 0) {
    return Math.max(0, total - limit);
  }
  if (scroll.height <= scroll.client) {
    return 0;
  }
  return Math.min(total, Math.round((total * scroll.top) / scroll.height));
}
