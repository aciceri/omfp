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
 * Whether this window's wide left column is collapsed to nothing (UI-985); narrow and phone frames ignore it. The last
 * choice of the window, which a view nobody collapsed or expanded the column on follows (UI-224). Kept across
 * reloads, like the column's width (UI-988, UI-207).
 */
export const LEFT_COLLAPSED = windowKey<boolean>("leftCollapsed", false, (stored) =>
  typeof stored === "boolean" ? stored : undefined,
);

/** A view of a workspace (UI-224). */
export interface WorkspaceView {
  readonly workspaceId: string;
  readonly viewId: string;
}

/** The wide column's state last chosen on one view of a workspace (UI-224). */
export interface ViewColumn extends WorkspaceView {
  readonly collapsed: boolean;
}

/** How many views keep the column's state; the least recently chosen goes first. */
export const VIEW_COLUMNS_KEPT = 200;

/** The wide column's state per view (UI-224), oldest first; kept across reloads (UI-988). */
export const VIEW_COLUMNS = windowKey<readonly ViewColumn[]>("leftCollapsedViews", [], (stored) =>
  Array.isArray(stored)
    ? stored
        .flatMap((entry: unknown): ViewColumn[] => {
          if (typeof entry !== "object" || entry === null) {
            return [];
          }
          const { workspaceId, viewId, collapsed } = entry as Partial<Record<keyof ViewColumn, unknown>>;
          return typeof workspaceId === "string" && typeof viewId === "string" && typeof collapsed === "boolean"
            ? [{ workspaceId, viewId, collapsed }]
            : [];
        })
        .slice(-VIEW_COLUMNS_KEPT)
    : undefined,
);

/** Whether the wide column is collapsed on `shown` (none: no workspace's view yet): its own state, else `fallback`. */
export function collapsedOn(
  columns: readonly ViewColumn[],
  fallback: boolean,
  shown: WorkspaceView | undefined,
): boolean {
  if (shown === undefined) {
    return fallback;
  }
  return (
    columns.find((column) => column.workspaceId === shown.workspaceId && column.viewId === shown.viewId)?.collapsed ??
    fallback
  );
}

/** `columns` with the column `collapsed` on `shown`, as the most recent choice; the oldest over the bound go. */
export function withViewColumn(
  columns: readonly ViewColumn[],
  shown: WorkspaceView,
  collapsed: boolean,
): readonly ViewColumn[] {
  return [
    ...columns.filter((column) => column.workspaceId !== shown.workspaceId || column.viewId !== shown.viewId),
    { ...shown, collapsed },
  ].slice(-VIEW_COLUMNS_KEPT);
}

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
