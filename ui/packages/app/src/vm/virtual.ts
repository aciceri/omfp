/**
 * Transcript scrolling (UI-042): `@tanstack/react-virtual` keeps only the visible items in the DOM; this module decides
 * when the view follows the bottom, which keys the transcript scrolls itself (WebKitGTK ignores PageUp on a focused
 * container, S-11), and where `nav.*` commands go.
 *
 * Pinned: while streaming, growth keeps the view at the bottom. Only the user unpins it — wheel up, a scroll-up key,
 * or any move of the view up from where the last scroll or follow left it, unless the view sits at the very end
 * (shrinking content clamps it there). Layout never moves a pinned view up otherwise: the transcript turns off the
 * virtualizer's scroll adjustments and the browser's scroll anchoring while pinned. A move up unpins even within
 * {@link PIN_THRESHOLD_PX} of the bottom: smooth scrolling (WebKitGTK's wheel) and touchpads scroll a few pixels per
 * frame, and re-pinning on such a step lets the next streamed delta pull the view back to the bottom. Moving down to
 * within the threshold, or being clamped to the very end, pins it again. A scroll event that did not move the view
 * changes nothing: WebKitGTK dispatches the one a follow caused at the next frame, possibly after the user's wheel.
 */

/** The view counts as at the bottom within this distance. */
const PIN_THRESHOLD_PX = 32;
/** The view is at the very end of the content less than this far from it (rounded positions: WebKitGTK reports -1 at
 * the end, and 1 after the first 2 px frame of a smooth scroll up). */
const END_PX = 1;
/** Farther than this from the bottom, an unpinned transcript offers a button back to the end (UI-044). */
const JUMP_THRESHOLD_PX = 3 * PIN_THRESHOLD_PX;

/** Whether the view is far enough above the end to offer jumping back to it (UI-044). */
export function farFromBottom({ scrollTop, scrollHeight, clientHeight }: ScrollPosition): boolean {
  return scrollHeight - scrollTop - clientHeight > JUMP_THRESHOLD_PX;
}

/** What one transcript knows about its scrolling, kept per thread so switching threads keeps the position. */
export interface PinState {
  /** Follow the bottom as content grows. */
  readonly pinned: boolean;
  /** Last scroll position while unpinned, restored when the list mounts again unpinned. Pinned, the list follows the
   * bottom and the position is not stored: a layout scroll while streaming then changes nothing (no render, UI-191). */
  readonly scrollTop: number;
}

export const INITIAL_PIN: PinState = { pinned: true, scrollTop: 0 };

/** Where the scroll container's view is. */
export interface ScrollPosition {
  readonly scrollTop: number;
  readonly scrollHeight: number;
  readonly clientHeight: number;
}

/** A copy of a scroll container's position as it is now (an element's own values change as it scrolls). */
export function scrollPosition({ scrollTop, scrollHeight, clientHeight }: ScrollPosition): ScrollPosition {
  return { scrollTop, scrollHeight, clientHeight };
}

export type PinEvent =
  | ({
      readonly type: "scroll";
      /** The position before this scroll (the last scroll event's, or where following the bottom put it). */
      readonly previous: ScrollPosition | undefined;
    } & ScrollPosition)
  /** Wheel up, a scroll-up key outside a form control, a navigation command moving up. */
  | { readonly type: "userScrollUp" }
  /** The user went to the end (`G`, `End`). */
  | { readonly type: "pin" };

/**
 * The view moved up from `previous` (where the last scroll or follow left it) to somewhere above the very end: the
 * user scrolled, since layout only moves a pinned view down or clamps it to the end. Whatever reports it first — the
 * scroll event, or a follow that runs before it (WebKitGTK dispatches scroll events at the next frame) — unpins.
 *
 * A move up no further than the end moved up (the content shrank, or the view grew) is the browser clamping the view,
 * even when the rounded positions leave it {@link END_PX} or more above the end: a view of a fractional height (the
 * composer's text lines and paddings make one) shows WebKitGTK's end as 1 px above it.
 */
export function movedUpByUser(previous: ScrollPosition | undefined, position: ScrollPosition): boolean {
  if (previous === undefined) {
    return false;
  }
  const { scrollTop, scrollHeight, clientHeight } = position;
  const up = previous.scrollTop - scrollTop;
  const endUp = previous.scrollHeight - previous.clientHeight - (scrollHeight - clientHeight);
  return up > 1 && up > endUp + 1 && scrollHeight - scrollTop - clientHeight >= END_PX;
}

/** The state after `event`; the same object when nothing changed. */
export function pinReducer(state: PinState, event: PinEvent): PinState {
  let next: PinState;
  switch (event.type) {
    case "scroll": {
      const below = event.scrollHeight - event.scrollTop - event.clientHeight;
      const previous = event.previous?.scrollTop;
      const movedDown = previous === undefined || event.scrollTop > previous + 1;
      const movedUp = previous !== undefined && event.scrollTop < previous - 1;
      const pinned =
        !movedUpByUser(event.previous, event) &&
        (state.pinned || (movedDown && below <= PIN_THRESHOLD_PX) || (movedUp && below < END_PX));
      next = { ...state, scrollTop: pinned ? state.scrollTop : event.scrollTop, pinned };
      break;
    }
    case "userScrollUp":
    case "pin":
      next = { ...state, pinned: event.type === "pin" };
      break;
  }
  return next.pinned === state.pinned && next.scrollTop === state.scrollTop ? state : next;
}

/** Where a freshly mounted list scrolls to: the stored position when the user had scrolled up, `undefined` for the
 * bottom. */
export function mountScrollTop(state: PinState): number | undefined {
  return state.pinned ? undefined : state.scrollTop;
}

/** Keys that scroll up and so unpin (unless typed into a form control, where they edit or move within it). */
export function isScrollUpKey(key: string): boolean {
  return key === "ArrowUp" || key === "PageUp" || key === "Home";
}

/** The scroll position after a paging key the transcript handles itself, `undefined` for other keys. */
export function keyScrollTop(
  key: string,
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
): number | undefined {
  const page = Math.max(clientHeight - PIN_THRESHOLD_PX, clientHeight / 2);
  const bottom = Math.max(0, scrollHeight - clientHeight);
  switch (key) {
    case "PageUp":
      return Math.max(0, scrollTop - page);
    case "PageDown":
      return Math.min(bottom, scrollTop + page);
    case "Home":
      return 0;
    case "End":
      return bottom;
    default:
      return undefined;
  }
}

/** The transcript-scope navigation commands (UI-095). */
export type NavCommand = "nav.next" | "nav.prev" | "nav.top" | "nav.bottom" | "nav.halfDown" | "nav.halfUp";

/** The commands that move the current item; `nav.halfDown`/`nav.halfUp` scroll instead ({@link halfPageScrollTop}). */
export type MoveCommand = Exclude<NavCommand, "nav.halfDown" | "nav.halfUp">;

/**
 * The item index a move command goes to from `current` (`undefined`: nothing focused yet, start from the bottom) among
 * `count` items.
 */
export function navTarget(command: MoveCommand, current: number | undefined, count: number): number {
  if (count === 0) {
    return -1;
  }
  const last = count - 1;
  const clamp = (index: number) => Math.min(last, Math.max(0, index));
  switch (command) {
    case "nav.next":
      return current === undefined ? last : clamp(current + 1);
    case "nav.prev":
      return current === undefined ? last : clamp(current - 1);
    case "nav.top":
      return 0;
    case "nav.bottom":
      return last;
  }
}

/** An item's edges relative to the top of the transcript's viewport, in px (negative: above it). */
export interface ItemEdges {
  readonly top: number;
  readonly bottom: number;
}

/** A pager step (UI-370) is this share of the viewport's height. */
const PAGER_STEP = 0.4;
/** The last pager step over an item leaves this much room past its edge. */
const PAGER_MARGIN_PX = 16;
/** An edge this close to the viewport's edge counts as shown (rounded positions). */
const EDGE_PX = 1;

/**
 * `nav.next` (`direction` 1) or `nav.prev` (-1) on the focused item at `item` in a viewport `viewHeight` tall (UI-370):
 * for an item taller than the viewport, the scroll that shows more of it while its far edge is out of view and the
 * item reaches into the viewport — a step of {@link PAGER_STEP} of the viewport, at most what remains plus
 * {@link PAGER_MARGIN_PX} —; `undefined` once that edge shows, or for an item that fits: the command then moves to the
 * next or previous item.
 */
export function pagerScroll(direction: 1 | -1, item: ItemEdges, viewHeight: number): number | undefined {
  if (item.bottom - item.top <= viewHeight) {
    return undefined;
  }
  const step = PAGER_STEP * viewHeight;
  if (direction === 1) {
    const hidden = item.bottom - viewHeight;
    return hidden > EDGE_PX && item.top < viewHeight ? Math.min(step, hidden + PAGER_MARGIN_PX) : undefined;
  }
  const hidden = -item.top;
  return hidden > EDGE_PX && item.bottom > 0 ? -Math.min(step, hidden + PAGER_MARGIN_PX) : undefined;
}

/**
 * Where a move by `nav.next` (`direction` 1) or `nav.prev` (-1) brings the view for the item it moved to, at `item`
 * (UI-370): an item taller than the viewport shows its top going down and its end going up, so reading goes on where
 * the previous item ended; the scroll is 0 when that edge already shows. `undefined` for an item that fits: it is
 * scrolled into view as any other.
 */
export function arrivalScroll(direction: 1 | -1, item: ItemEdges, viewHeight: number): number | undefined {
  if (item.bottom - item.top <= viewHeight) {
    return undefined;
  }
  if (direction === 1) {
    return item.top >= 0 && item.top < viewHeight ? 0 : item.top;
  }
  return item.bottom > 0 && item.bottom <= viewHeight ? 0 : item.bottom - viewHeight;
}

/** The scroll position after `nav.halfDown` (`direction` 1) or `nav.halfUp` (-1): half a viewport, within the content
 * (UI-371). */
export function halfPageScrollTop(
  direction: 1 | -1,
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
): number {
  const bottom = Math.max(0, scrollHeight - clientHeight);
  return Math.min(bottom, Math.max(0, scrollTop + (direction * clientHeight) / 2));
}

/** An item's extent in the scroll content, in px. */
export interface ItemExtent {
  readonly start: number;
  readonly end: number;
}

/**
 * After a scroll by a paging key (UI-371), the item to focus instead of `current`: `undefined` while `current` still
 * shows in the view `[viewTop, viewTop + viewHeight)`, else the first item reaching below the view's top.
 */
export function refocusAfterScroll(
  current: number | undefined,
  items: readonly ItemExtent[],
  viewTop: number,
  viewHeight: number,
): number | undefined {
  const item = current === undefined ? undefined : items[current];
  if (item === undefined || (item.end > viewTop && item.start < viewTop + viewHeight)) {
    return undefined;
  }
  const top = items.findIndex((extent) => extent.end > viewTop);
  return top < 0 ? items.length - 1 : top;
}
