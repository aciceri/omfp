/**
 * The contrast guard (UI-196): whenever the effective theme changes — initial load, the user theme's `<link>` loading
 * or its `<style>` being replaced (UI-091), `data-color-scheme` or the built-in theme's `data-theme` (UI-510) changing, the system's `prefers-color-scheme`
 * changing — it removes its overrides, measures the guarded roles as rendered (contrast.ts) and sets an inline
 * `--omfp-<role>` on `<html>` for each one under its threshold, like `[ui.fonts]` does (theme.ts). Derived tokens
 * (gradients, hover, selection, diff colors) follow because they are mixed from the roles.
 */

import { cssColorReader, watchTheme } from "@omfp/kit";
import { type Adjustment, contrastAdjustments, GUARDED_ROLES, parseCssColor, type Rgb } from "./contrast.ts";

/** Reads a color token (name without `--omfp-`) as rendered; `undefined` when it cannot be resolved. */
export type ColorReader = (token: string) => Rgb | undefined;

/** `<html>`'s attribute listing the roles the guard overrides (space-separated), absent when none. */
export const GUARD_ATTRIBUTE = "data-contrast-guard";

/**
 * Removes the previous overrides from `root`, measures with `read` and sets the overrides the theme needs. Returns
 * them (empty: the theme passes as it is).
 */
export function applyContrastGuard(root: HTMLElement, read: ColorReader): Adjustment[] {
  for (const { role } of GUARDED_ROLES) {
    root.style.removeProperty(`--omfp-${role}`);
  }
  const adjustments = contrastAdjustments(read);
  for (const { role, to } of adjustments) {
    root.style.setProperty(`--omfp-${role}`, to);
  }
  if (adjustments.length === 0) {
    root.removeAttribute(GUARD_ATTRIBUTE);
  } else {
    root.setAttribute(GUARD_ATTRIBUTE, adjustments.map(({ role }) => role).join(" "));
  }
  return adjustments;
}

/** Guards `root` now and after every theme change until the returned function is called (which also clears it). */
export function installContrastGuard(
  document: Document,
  root: HTMLElement = document.documentElement,
  read: ColorReader = cssColorReader(document, parseCssColor),
): () => void {
  const guard = () => applyContrastGuard(root, read);
  guard();
  // Not `<html>`'s `style`: the guard writes its overrides there.
  const unwatch = watchTheme(document, guard, ["data-color-scheme", "data-theme"]);
  return () => {
    unwatch();
    applyContrastGuard(root, () => undefined);
  };
}
