/**
 * The view's `side` slot collapsed by this window (UI-1001): a view's `["tabs", { "id": "side" }, …]` hides and comes
 * back with the Hide panel / Show panel button at the mode line's end. Kept per workspace and view in `localStorage`, so
 * the next window starts the same; a phone shows the side area its own way (UI-204) and ignores it.
 */

import { windowKey } from "../sdk/window-store.ts";
import type { LayoutTree } from "./layout.ts";

/** The `tabs` id of the view's side panel. */
export const SIDE_SLOT = "side";

/** The collapsed side panels, by {@link sideKey}. */
export const SIDE_COLLAPSED = windowKey<readonly string[]>("sideCollapsed", [], (stored) =>
  Array.isArray(stored) ? stored.filter((key): key is string => typeof key === "string") : undefined,
);

/** The key of a view's side panel: its workspace (`-` without one) and its id. */
export function sideKey(workspaceId: string | undefined, viewId: string): string {
  return `${workspaceId ?? "-"}/${viewId}`;
}

/** `collapsed` with `key` in or out. */
export function withSide(collapsed: readonly string[], key: string, collapse: boolean): readonly string[] {
  const without = collapsed.filter((entry) => entry !== key);
  return collapse ? [...without, key] : without;
}

/** Whether `tree` is the side panel. */
export function isSideSlot(tree: LayoutTree): boolean {
  return tree.kind === "tabs" && tree.props.id === SIDE_SLOT;
}

/** Whether the view holds a side panel. */
export function hasSideSlot(tree: LayoutTree): boolean {
  return tree.kind !== "widget" && (isSideSlot(tree) || tree.children.some(hasSideSlot));
}
