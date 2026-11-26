/**
 * View mode's focus (UI-095, UI-098): the panes are the widgets of the shown view and of the floating panels (each
 * `.omfp-view__item`, focusable itself). `v` focuses the view where it was focused last outside a text field, else its
 * transcript, else its first pane ({@link focusView}); `Ctrl-w h/j/k/l` move to the pane on that side, as vim's
 * windows ({@link focusPaneToward}, {@link paneToward}); `g t`/`g T` step the tabs of the innermost tabs container
 * around the focus ({@link stepTab}, which the container answers through {@link TAB_STEP_EVENT}).
 */

import { SCOPE_ATTRIBUTE } from "../sdk/index.ts";
import { modeAt, SHOWN_VIEW } from "./handler.ts";

/** One widget of a view or floating panel. */
export const PANE = ".omfp-view__item";

/**
 * The event {@link stepTab} sends from the focused element, bubbling: the innermost tabs container with a tab bar
 * around it selects its next (`detail` 1) or previous (-1) tab, focuses its pane and cancels the event.
 */
export const TAB_STEP_EVENT = "omfp-tab-step";

/** A side of a pane, as `Ctrl-w h/j/k/l`. */
export type Direction = "left" | "down" | "up" | "right";

/** A pane's place on screen. */
export interface Box {
  readonly left: number;
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
}

/** The element focused last in View mode, per view (or floating panel) and per pane. */
const lastFocused = new WeakMap<Element, HTMLElement>();

/** Shown: in the page, not in a hidden tab panel, a hidden view (UI-390) or another hidden part. */
function shown(element: Element): boolean {
  return element.isConnected && element.closest('[hidden], [inert], [data-testid="kept-view"]') === null;
}

/** Keeps `element`, just focused, as the last focus of its view and pane when it puts the window in View mode. */
export function rememberFocus(element: Element): void {
  if (!(element instanceof HTMLElement) || modeAt(element) !== "view") {
    return;
  }
  for (const holder of [element.closest(".omfp-view"), element.closest(PANE)]) {
    if (holder !== null) {
      lastFocused.set(holder, element);
    }
  }
}

/** The element `holder` had the focus on last, while it is still shown inside it. */
function remembered(holder: Element): HTMLElement | undefined {
  const element = lastFocused.get(holder);
  return element !== undefined && shown(element) && holder.contains(element) ? element : undefined;
}

/**
 * Focuses `pane`: where its focus was last, else its first focusable widget with a command scope (the transcript), else
 * the pane itself, so no button takes the keys by surprise.
 */
export function focusPane(pane: HTMLElement): void {
  const scoped = [...pane.querySelectorAll<HTMLElement>(`[${SCOPE_ATTRIBUTE}][tabindex]`)].find(
    (element) => element.tabIndex >= 0 && shown(element) && modeAt(element) === "view",
  );
  (remembered(pane) ?? scoped ?? pane).focus({ preventScroll: true });
}

/** `view.focus` (`v`, `Enter` in Threads mode): focuses the shown view; `false` when the window shows none. */
export function focusView(): boolean {
  const view = document.querySelector<HTMLElement>(SHOWN_VIEW);
  if (view === null) {
    return false;
  }
  const last = remembered(view);
  if (last !== undefined) {
    last.focus({ preventScroll: true });
    return true;
  }
  const transcript = view.querySelector<HTMLElement>(`[${SCOPE_ATTRIBUTE}="transcript"]`);
  // A transcript in a hidden (kept-mounted) tab cannot take the focus: the first shown pane does.
  const pane =
    (transcript === null || !shown(transcript) ? null : transcript.closest<HTMLElement>(PANE)) ?? shownPanes(view)[0];
  if (pane !== undefined) {
    focusPane(pane);
  }
  return true;
}

/** `view.leave` (`Esc` in View mode): the focus leaves the view for nothing, so the window is in Threads mode. */
export function leaveView(): void {
  const focused = document.activeElement;
  if (focused instanceof HTMLElement && modeAt(focused) === "view") {
    focused.blur();
  }
}

/** The panes inside `within` that show, in document order. */
export function shownPanes(within: ParentNode): HTMLElement[] {
  return [...within.querySelectorAll<HTMLElement>(PANE)].filter(shown);
}

function box(element: Element): Box {
  const { left, top, right, bottom } = element.getBoundingClientRect();
  return { left, top, right, bottom };
}

/**
 * The index of the box of `candidates` next to `from` toward `direction`, as vim's `Ctrl-w h/j/k/l`: one wholly on
 * that side (1 px of overlap allowed), preferring the ones facing `from` (overlapping it across the move), then the
 * nearest, then the one whose middle is nearest to `from`'s. With none wholly on that side, one overlapping `from` (a
 * floating panel lies over the view's panes) whose middle is nearest past `from`'s middle that way; else `undefined`.
 */
export function paneToward(from: Box, candidates: readonly Box[], direction: Direction): number | undefined {
  const [start, end] =
    direction === "left" || direction === "right" ? (["top", "bottom"] as const) : (["left", "right"] as const);
  let best: { index: number; facing: boolean; distance: number; offset: number } | undefined;
  for (const [index, candidate] of candidates.entries()) {
    const gap = {
      left: from.left - candidate.right,
      right: candidate.left - from.right,
      up: from.top - candidate.bottom,
      down: candidate.top - from.bottom,
    }[direction];
    if (gap < -1) {
      continue;
    }
    const facing = Math.min(candidate[end], from[end]) > Math.max(candidate[start], from[start]);
    const distance = Math.max(gap, 0);
    const offset = Math.abs(candidate[start] + candidate[end] - from[start] - from[end]) / 2;
    const better =
      best === undefined ||
      (facing && !best.facing) ||
      (facing === best.facing && (distance < best.distance || (distance === best.distance && offset < best.offset)));
    if (better) {
      best = { index, facing, distance, offset };
    }
  }
  if (best !== undefined) {
    return best.index;
  }
  const middle = (box: Box) =>
    direction === "left" || direction === "right" ? (box.left + box.right) / 2 : (box.top + box.bottom) / 2;
  const sign = direction === "left" || direction === "up" ? -1 : 1;
  let nearest: { index: number; distance: number } | undefined;
  for (const [index, candidate] of candidates.entries()) {
    const overlaps =
      candidate.left < from.right &&
      from.left < candidate.right &&
      candidate.top < from.bottom &&
      from.top < candidate.bottom;
    const distance = (middle(candidate) - middle(from)) * sign;
    if (overlaps && distance > 0 && (nearest === undefined || distance < nearest.distance)) {
      nearest = { index, distance };
    }
  }
  return nearest?.index;
}

/**
 * `pane.left` … `pane.right` (`Ctrl-w h/j/k/l`): focuses the pane next to the focused one toward `direction`; with the
 * focus outside every pane, the view as {@link focusView} does. `false` when nothing changed.
 */
export function focusPaneToward(direction: Direction): boolean {
  const focused = document.activeElement;
  const from = focused === null ? null : focused.closest<HTMLElement>(PANE);
  if (from === null) {
    return focusView();
  }
  const others = shownPanes(document).filter((pane) => pane !== from && !pane.contains(from) && !from.contains(pane));
  const index = paneToward(box(from), others.map(box), direction);
  const target = index === undefined ? undefined : others[index];
  if (target === undefined) {
    return false;
  }
  focusPane(target);
  return true;
}

/**
 * `tab.next` / `tab.prev` (`g t` / `g T`): steps the tabs of the innermost tabs container with a tab bar around the
 * focused element, a pane or a tab bar (a floating panel's too); with the focus elsewhere, around where the view was
 * focused last. `false` when no container took it.
 */
export function stepTab(step: 1 | -1): boolean {
  const focused = document.activeElement;
  const view = document.querySelector<HTMLElement>(SHOWN_VIEW);
  const from =
    focused !== null &&
    modeAt(focused) === "view" &&
    (focused.closest(PANE) !== null || focused.closest('[role="tablist"]') !== null)
      ? focused
      : view === null
        ? undefined
        : (remembered(view) ?? view.querySelector(`[${SCOPE_ATTRIBUTE}="transcript"]`) ?? undefined);
  if (from === undefined) {
    return false;
  }
  return !from.dispatchEvent(new CustomEvent(TAB_STEP_EVENT, { bubbles: true, cancelable: true, detail: step }));
}
