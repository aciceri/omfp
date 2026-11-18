/**
 * The conversation search's highlights (UI-736): the occurrences of the query in the rendered rows' text, painted with
 * the CSS Custom Highlight API so the rendered DOM (Markdown, memoized rows) stays as React left it. Every transcript
 * shares the two highlights through the SDK, each adding its own ranges. Where the browser has no custom highlights
 * (WKWebView before macOS 14.2), boxes in an overlay over the list mark the occurrences instead; the overlay is the
 * only DOM written.
 */

import {
  highlightRegistry,
  type MessageView,
  searchOccurrences as occurrences,
  paintSearchHighlights,
  type SearchMarks,
  searchRevealScrollLeft,
} from "omfp/sdk";
import { type RefObject, useCallback, useEffect, useRef } from "react";

/** Nothing marked. */
export const NO_MARKS: SearchMarks = { matches: [], current: undefined };

/** The text the search reads in a message: the user's text, the assistant's Markdown, an expanded thinking block. */
const TEXT_ROOTS = '[data-testid="plain-text"], [data-testid="markdown"], .omfp-reasoning__text';
/** Text the search does not read: tool cards (with their own transcripts), controls (not the inline path links of
 * Markdown, which are the message's text), decorations, and what a diagram's SVG holds but never shows (UI-981). */
const OUTSIDE = '[data-testid="tool-card"]';
const SKIPPED = 'button:not(.omfp-path-link), [aria-hidden="true"], svg :is(style, title, desc)';
/** Text in different blocks is separated, so an occurrence never spans two paragraphs or cells. */
const BLOCKS = "p, li, pre, blockquote, h1, h2, h3, h4, h5, h6, dt, dd, td, th, tr, div";
/** Overflow values that clip an element's content to its box. */
const CLIPPING = /^(?:auto|scroll|hidden|clip)$/;

/** The occurrences of `pattern` in the text of the rendered row `item`, in reading order. */
export function textRanges(item: Element, pattern: RegExp): Range[] {
  const roots = [...item.querySelectorAll(TEXT_ROOTS)].filter((root) => root.closest(OUTSIDE) === null);
  return roots
    .filter((root) => !roots.some((other) => other !== root && other.contains(root)))
    .flatMap((root) => rootRanges(root, pattern));
}

/** A text node and where its text starts in the joined text of its root. */
interface Piece {
  readonly node: Text;
  readonly at: number;
}

function rootRanges(root: Element, pattern: RegExp): Range[] {
  const document = root.ownerDocument;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const pieces: Piece[] = [];
  let text = "";
  let block: Element | null = null;
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const parent = node.parentElement;
    const skipped = parent?.closest(SKIPPED);
    if (!(node instanceof Text) || parent === null || (skipped != null && root.contains(skipped))) {
      continue;
    }
    const nodeBlock = parent.closest(BLOCKS);
    if (pieces.length > 0 && nodeBlock !== block) {
      text += "\n";
    }
    block = nodeBlock;
    pieces.push({ node, at: text.length });
    text += node.data;
  }
  return occurrences(text, pattern).flatMap(({ start, end }) => {
    const from = pieces.findLast((piece) => piece.at <= start && start < piece.at + piece.node.length);
    const to = pieces.find((piece) => piece.at < end && end <= piece.at + piece.node.length);
    if (from === undefined || to === undefined) {
      return [];
    }
    const range = document.createRange();
    range.setStart(from.node, start - from.at);
    range.setEnd(to.node, end - to.at);
    return [range];
  });
}

/**
 * The labels of the collapsed thinking blocks of the rendered row `item` (showing `view`) whose text contains
 * `pattern`: marked so a match in a collapsed block shows (visiting the message expands them).
 */
export function collapsedThinkingHints(item: Element, view: MessageView, pattern: RegExp): Range[] {
  const elements = [...item.querySelectorAll<HTMLElement>('[data-testid="thinking"]')].filter(
    (element) => element.closest(OUTSIDE) === null,
  );
  const blocks = view.blocks.flatMap((block) => (block.type === "thinking" ? [block] : []));
  return blocks.flatMap((block, ordinal) => {
    const element = elements[ordinal];
    const label = element?.querySelector(".omfp-reasoning__label");
    if (element?.dataset.expanded !== "false" || label == null || block.text.search(pattern) < 0) {
      return [];
    }
    const range = item.ownerDocument.createRange();
    range.selectNodeContents(label);
    return [range];
  });
}

/**
 * Scrolls the containers between the rendered row `item` and the current occurrence `mark` that scroll sideways (a
 * wide Markdown table) so that it shows; the transcript then scrolls it into view vertically.
 */
export function revealInline(mark: Range, item: Element): void {
  for (
    let element = mark.startContainer.parentElement;
    element !== null && element !== item;
    element = element.parentElement
  ) {
    if (element.scrollWidth > element.clientWidth) {
      const scroll = searchRevealScrollLeft(element.getBoundingClientRect(), mark.getBoundingClientRect());
      if (scroll !== undefined) {
        element.scrollLeft += scroll;
      }
    }
  }
}

/** A box in viewport coordinates. */
interface Box {
  readonly left: number;
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
}

function intersect(a: Box, b: Box): Box {
  return {
    left: Math.max(a.left, b.left),
    top: Math.max(a.top, b.top),
    right: Math.min(a.right, b.right),
    bottom: Math.min(a.bottom, b.bottom),
  };
}

/** What the containers between `range` and `stop` that clip their overflow (a wide table's scroller) leave visible. */
function visibleBox(range: Range, stop: Element): Box {
  const start = range.commonAncestorContainer;
  const view = stop.ownerDocument.defaultView;
  let box: Box = { left: -Infinity, top: -Infinity, right: Infinity, bottom: Infinity };
  for (
    let element = start instanceof Element ? start : start.parentElement;
    element !== null && element !== stop;
    element = element.parentElement
  ) {
    const style = view?.getComputedStyle(element);
    if (style !== undefined && (CLIPPING.test(style.overflowX) || CLIPPING.test(style.overflowY))) {
      box = intersect(box, element.getBoundingClientRect());
    }
  }
  return box;
}

/**
 * Without custom highlights: one box per line of each occurrence in `overlay`, which covers the list, cut to what the
 * containers scrolling inside the list leave visible (a line scrolled out of a wide table gets no box).
 */
export function paintOverlay(overlay: HTMLElement, marks: SearchMarks): void {
  const origin = overlay.getBoundingClientRect();
  const list = overlay.parentElement ?? overlay;
  const boxes = [
    ...marks.matches.map((range) => [range, false] as const),
    ...(marks.current === undefined ? [] : [[marks.current, true] as const]),
  ].flatMap(([range, current]) => {
    const visible = visibleBox(range, list);
    return Array.from(range.getClientRects()).flatMap((rect) => {
      const shown = intersect(rect, visible);
      if (shown.right <= shown.left || shown.bottom <= shown.top) {
        return [];
      }
      const box = overlay.ownerDocument.createElement("div");
      box.className = "omfp-transcript__mark";
      box.dataset.current = String(current);
      box.style.top = `${shown.top - origin.top}px`;
      box.style.left = `${shown.left - origin.left}px`;
      box.style.width = `${shown.right - shown.left}px`;
      box.style.height = `${shown.bottom - shown.top}px`;
      return [box];
    });
  });
  overlay.replaceChildren(...boxes);
}

/**
 * While `active`, paints what `marks()` returns over the rows rendered in `list`: at once, after every change to them
 * (a row mounting, a thinking block expanding, streamed text; in the overlay also a row moving, the list resizing and
 * content scrolling inside it, such as a wide table) and on each call of the returned function; clears them when
 * `active` ends or the transcript unmounts. Without custom highlights the boxes go into `overlay`.
 */
export function useSearchHighlights(
  list: RefObject<HTMLElement | null>,
  overlay: RefObject<HTMLElement | null>,
  active: boolean,
  marks: () => SearchMarks,
): () => void {
  const latest = useRef(marks);
  latest.current = marks;
  const paint = useRef<() => void>(() => {});
  useEffect(() => {
    const root = list.current;
    if (!active || root === null) {
      return;
    }
    const registry = highlightRegistry();
    const owner = {};
    const run = () => {
      const found = latest.current();
      if (registry !== undefined) {
        paintSearchHighlights(registry, owner, found);
      } else if (overlay.current !== null) {
        paintOverlay(overlay.current, found);
      }
    };
    paint.current = run;
    run();
    const mutations = new MutationObserver((records) => {
      const box = overlay.current;
      if (records.some((record) => box === null || !box.contains(record.target))) {
        run();
      }
    });
    mutations.observe(root, {
      childList: true,
      subtree: true,
      characterData: true,
      ...(registry === undefined ? { attributes: true, attributeFilter: ["style"] } : {}),
    });
    const overlayOnly = registry === undefined;
    const resizes = overlayOnly && typeof ResizeObserver !== "undefined" ? new ResizeObserver(run) : null;
    resizes?.observe(root);
    // Scroll events do not bubble: a capturing listener sees the scrolling of every container inside the list.
    if (overlayOnly) {
      root.addEventListener("scroll", run, true);
    }
    return () => {
      mutations.disconnect();
      resizes?.disconnect();
      root.removeEventListener("scroll", run, true);
      paint.current = () => {};
      if (registry !== undefined) {
        paintSearchHighlights(registry, owner, undefined);
      } else {
        overlay.current?.replaceChildren();
      }
    };
  }, [active, list, overlay]);
  return useCallback(() => paint.current(), []);
}
