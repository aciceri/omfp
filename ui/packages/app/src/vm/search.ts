/**
 * Conversation search (UI-730, UI-736): the query as a case-insensitive pattern, its occurrences in a text, which
 * messages it matches, the order Enter and Shift-Enter visit occurrences in, and the scroll that brings the current
 * occurrence into view. The transcript finds the occurrences of the rendered messages in their DOM text (Markdown
 * renders differently from its source), so a message's occurrences are counted once it is rendered.
 */

import type { MessageView } from "./transcript.ts";
import type { ItemEdges } from "./virtual.ts";

/** Characters with a meaning in a regular expression. */
const SYNTAX = /[\\^$.*+?()[\]{}|/]/g;

/** A pattern finding every occurrence of `query` (trimmed) regardless of case; `undefined` for a blank query. */
export function searchPattern(query: string): RegExp | undefined {
  const needle = query.trim();
  return needle === "" ? undefined : new RegExp(needle.replace(SYNTAX, "\\$&"), "giu");
}

/** Where an occurrence is in a text: `start` inclusive, `end` exclusive, in UTF-16 code units. */
export interface TextSpan {
  readonly start: number;
  readonly end: number;
}

/** Every occurrence of `pattern` (from {@link searchPattern}) in `text`, in order, without overlaps. */
export function occurrences(text: string, pattern: RegExp): TextSpan[] {
  return Array.from(text.matchAll(pattern), (match) => ({ start: match.index, end: match.index + match[0].length }));
}

/** The indexes of the blocks of `view` the search reads (text and thinking) that contain `pattern`. */
export function matchingBlocks(view: MessageView, pattern: RegExp): number[] {
  return view.blocks.flatMap((block, index) =>
    (block.type === "text" || block.type === "thinking") && block.text.search(pattern) >= 0 ? [index] : [],
  );
}

/** Where the search is: the matching row and its occurrence among the rendered ones, `-1` for its last. */
export interface SearchCursor {
  readonly row: number;
  readonly occurrence: number;
}

/** `occurrence` of a cursor (`-1`: the last) among `count` rendered occurrences, clamped to them. */
export function occurrenceIndex(occurrence: number, count: number): number {
  return occurrence < 0 || occurrence >= count ? Math.max(count - 1, 0) : occurrence;
}

/**
 * Where Enter (`direction` 1) or Shift-Enter (-1) goes from `cursor` among the matching rows `matches` (ascending),
 * `rendered` being the count of occurrences of the cursor's row as rendered (`undefined` when unknown): the next or
 * previous occurrence of that row while it has one, else the first occurrence of the next matching row, or the last of
 * the previous one, wrapping. From no cursor, or one on a row that no longer matches, Enter goes to the first
 * matching row and Shift-Enter to the last. `undefined` when nothing matches.
 */
export function stepSearch(
  cursor: SearchCursor | undefined,
  direction: 1 | -1,
  matches: readonly number[],
  rendered: number | undefined,
): SearchCursor | undefined {
  const at = cursor === undefined ? -1 : matches.indexOf(cursor.row);
  if (cursor !== undefined && at >= 0 && rendered !== undefined && rendered > 0) {
    const next = occurrenceIndex(cursor.occurrence, rendered) + direction;
    if (next >= 0 && next < rendered) {
      return { row: cursor.row, occurrence: next };
    }
  }
  const count = matches.length;
  const target = at < 0 ? (direction === 1 ? 0 : count - 1) : (at + direction + count) % count;
  const row = matches[target];
  return row === undefined ? undefined : { row, occurrence: direction === 1 ? 0 : -1 };
}

/**
 * The scroll (a delta for `scrollTop`) that shows the current occurrence at `mark` of the item at `item`, both
 * relative to the viewport's top, in a viewport `viewHeight` tall; `undefined` when it already shows. The item's top
 * goes to the viewport's top when the occurrence then still shows (as a visit without one does), else the occurrence
 * goes to the viewport's middle.
 */
export function revealScroll(item: ItemEdges, mark: ItemEdges | undefined, viewHeight: number): number | undefined {
  if (mark === undefined) {
    return item.top >= 0 && item.top < viewHeight ? undefined : item.top;
  }
  if (mark.top >= 0 && mark.bottom <= viewHeight) {
    return undefined;
  }
  if (mark.bottom - item.top <= viewHeight) {
    return item.top;
  }
  return mark.top - (viewHeight - (mark.bottom - mark.top)) / 2;
}

/** A box's horizontal edges, in px from the same origin. */
export interface InlineEdges {
  readonly left: number;
  readonly right: number;
}

/**
 * The horizontal scroll (a delta for `scrollLeft`) of a container scrolling sideways (a wide table) whose visible box
 * is `viewport` that shows the current occurrence at `mark`: `undefined` when it already shows, else the occurrence
 * goes to the middle.
 */
export function revealScrollLeft(viewport: InlineEdges, mark: InlineEdges): number | undefined {
  if (mark.left >= viewport.left && mark.right <= viewport.right) {
    return undefined;
  }
  return mark.left - viewport.left - (viewport.right - viewport.left - (mark.right - mark.left)) / 2;
}
