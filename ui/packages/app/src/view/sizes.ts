/**
 * This window's splitter sizes (UI-206, UI-207): the always-active splitters resize the cells of a view's `row` and
 * `col` containers and the left column for this window only, kept in `localStorage` so the next window starts from
 * them. `views.json` keeps the defaults; a window's sizes apply on top of them while
 * the container still has as many cells, and a double click drops them.
 */

import { isObject } from "../sdk/json.ts";
import { windowKey } from "../sdk/window-store.ts";

/** Window sizes by {@link sizesKey}: flex weights, one per cell. */
export type PaneSizes = Readonly<Record<string, readonly number[]>>;

function isWeight(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value > 0;
}

/** Keeps the entries that are lists of positive weights; `undefined` when `stored` is not an object. */
export function decodePaneSizes(stored: unknown): PaneSizes | undefined {
  if (!isObject(stored)) {
    return undefined;
  }
  const sizes: Record<string, readonly number[]> = {};
  for (const [key, value] of Object.entries(stored)) {
    if (Array.isArray(value) && value.length > 0 && value.every(isWeight)) {
      sizes[key] = value;
    }
  }
  return sizes;
}

export const PANE_SIZES = windowKey<PaneSizes>("paneSizes", {}, decodePaneSizes);

/** The key of a container's sizes: workspace (`-` without one), view and the container's layout path. */
export function sizesKey(workspaceId: string | undefined, viewId: string, path: string): string {
  return `${workspaceId ?? "-"}/${viewId}/${path}`;
}

/**
 * The weights a container shows: this window's when it has one per cell, else the view's (`undefined`: equal).
 */
export function effectiveSizes(
  defaults: readonly number[] | undefined,
  own: readonly number[] | undefined,
  cells: number,
): readonly number[] | undefined {
  return own !== undefined && own.length === cells && own.every(isWeight) ? own : defaults;
}

/** `sizes` with the entry of `key` set, or removed when `next` is `undefined`. */
export function withSizes(sizes: PaneSizes, key: string, next: readonly number[] | undefined): PaneSizes {
  if (next === undefined) {
    if (!Object.hasOwn(sizes, key)) {
      return sizes;
    }
    const { [key]: _dropped, ...rest } = sizes;
    return rest;
  }
  return { ...sizes, [key]: next };
}

/** The left column's width in this window, in `rem`; `undefined`: the stylesheet's default (UI-207, UI-486). */
export const LEFT_WIDTH = windowKey<number | undefined>("leftWidth", undefined, (stored) =>
  isWeight(stored) ? stored : undefined,
);

/** The narrowest left column, in `rem`. */
export const LEFT_MIN_REM = 12;
/** The widest left column, as a share of the frame. */
const LEFT_MAX_SHARE = 0.5;

/**
 * The left column's width after moving its splitter by `delta` (a share of the frame's width `frameRem`, in `rem`),
 * between {@link LEFT_MIN_REM} and half the frame; rounded to a hundredth of a `rem`.
 */
export function resizeLeft(current: number, delta: number, frameRem: number): number {
  const widest = Math.max(LEFT_MIN_REM, frameRem * LEFT_MAX_SHARE);
  const next = Math.min(Math.max(current + delta * frameRem, LEFT_MIN_REM), widest);
  return Math.round(next * 100) / 100;
}
