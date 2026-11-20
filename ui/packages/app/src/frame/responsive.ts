/**
 * The frame's breakpoints (UI-200): measured on the frame itself, not the viewport, so half a desktop screen narrows
 * the window like a phone does. `wide`: the left column beside the view; `narrow` (under 1100 px): a thin strip whose
 * column opens over the content (UI-201); `mobile` (under 700 px): one area at a time from a bottom bar (UI-204). A
 * short touch screen (a phone held sideways) is `mobile` whatever its width (UI-414).
 */

import type { ThreadSummary } from "@omfp/protocol";
import { windowKey } from "../sdk/window-store.ts";
import type { Area } from "../view/layout.ts";
import { displayTitle, isArchived, type ThreadStatus, threadStatus } from "../vm/threads.ts";

export type Breakpoint = "wide" | "narrow" | "mobile";

/** Frames narrower than this get the strip (px). */
const NARROW_BELOW = 1100;
/** Frames narrower than this get one area at a time (px). */
const MOBILE_BELOW = 700;

/** The breakpoint of a frame `width` px wide; an unmeasured frame (0) stays wide. */
export function breakpointOf(width: number): Breakpoint {
  if (width <= 0 || width >= NARROW_BELOW) {
    return "wide";
  }
  return width < MOBILE_BELOW ? "mobile" : "narrow";
}

/** A touch screen (coarse primary pointer): touch sizes, the visual viewport followed (UI-413, UI-414). */
export const COARSE_POINTER = "(pointer: coarse)";
/** A touch screen too short for the strip and the side column: a phone held sideways (UI-414). */
export const SHORT_TOUCH_SCREEN = "(pointer: coarse) and (max-height: 500px)";

/** Whether the left column is open over the content (narrow) or as the threads area (mobile). */
export const LEFT_OPEN = windowKey<boolean>("leftOpen", false);

/**
 * Whether this window's wide left column is collapsed to nothing (UI-985); narrow and phone frames ignore it. Kept
 * across reloads, like the column's width (UI-988, UI-207).
 */
export const LEFT_COLLAPSED = windowKey<boolean>("leftCollapsed", false, (stored) =>
  typeof stored === "boolean" ? stored : undefined,
);

/** The area a phone shows when the threads are closed (UI-204). */
export const MOBILE_AREA = windowKey<Area>("mobileArea", "chat");

/** A thread the strip shows (UI-201). */
export interface StripThread {
  readonly id: string;
  readonly title: string;
  readonly status: ThreadStatus;
}

/** The threads the strip shows, in index order: those needing the user (awaiting, failed) and those running. */
export function stripThreads(threads: readonly ThreadSummary[]): StripThread[] {
  return threads.flatMap((thread) => {
    if (isArchived(thread)) {
      return [];
    }
    const status = threadStatus(thread);
    return status === "awaiting" || status === "failed" || status === "running" || status === "starting"
      ? [{ id: thread.id, title: displayTitle(thread), status }]
      : [];
  });
}
