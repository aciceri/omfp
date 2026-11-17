/**
 * Theme, skin, density, motion and fonts on the document (UI-091, UI-094, UI-097, UI-145, UI-794). The built-in light and dark
 * tokens follow `prefers-color-scheme` unless `<html data-color-scheme="light|dark">` (window-local) overrides it;
 * `data-theme` is the daemon's `[ui] theme` (UI-510), a built-in theme of `@omfp/kit/themes.css` over Synth's tokens
 * (`accent` only where the host gives the system accent, UI-512, else Synth; `matugen` is Synth under the user theme);
 * `data-skin` is the daemon's `[ui] skin` (UI-794: `crush` or `boring`, whose stylesheets override Crush's under
 * `[data-skin="boring"]`, whatever the theme);
 * `data-density` (the daemon's `[ui] density`) picks compact or normal spacing; `data-motion` is the effective motion level, the daemon's `[ui] motion` capped at `none` while the system asks for reduced motion, so stylesheets and
 * script effects read one attribute; `[ui] font_size` is `<html>`'s inline `font-size` (so every rem-based token
 * scales); `[ui.fonts]` set `--omfp-font-prose|ui|code` inline on `<html>`, which beats any stylesheet, user theme
 * included. The user theme is the daemon's (global, `theme.get`/`theme.set`) and shows only while `[ui] theme` is
 * `matugen`: the page loads it first through `<link rel="stylesheet" href="/theme.css" data-omfp-theme>` (so it applies
 * before the socket connects), then once the `index` topic is live `<style data-omfp-theme>` replaces that link in the
 * same place, holding the user theme while the theme is `matugen` and nothing otherwise, and follows both
 * (`theme_changed`, `settings_changed`) without asking the daemon again. The tokens and the built-in themes live in
 * `@layer omfp-tokens`, so the unlayered user theme always wins over them. The contrast guard (guard.ts, UI-196) then
 * keeps the theme's text roles readable: it runs with the appearance.
 */

import {
  type BuiltinTheme,
  type Density,
  type FontSettings,
  INDEX_TOPIC,
  type Motion,
  type Skin,
} from "@omfp/protocol";
import { useEffect, useSyncExternalStore } from "react";
import {
  COLOR_SCHEME,
  type ColorScheme,
  DEFAULT_DENSITY,
  DEFAULT_FONT_SIZE,
  DEFAULT_MOTION,
  DEFAULT_SKIN,
  DEFAULT_THEME,
  useSettings,
  useSnapshot,
  useWindowState,
} from "../sdk/index.ts";
import { installContrastGuard } from "./guard.ts";

export const THEME_ATTRIBUTE = "data-omfp-theme";
/**
 * The custom property the host sets inline on `<html>` to the system accent colour (`#rrggbb`) before the page loads
 * (UI-512): the macOS desktop window, from AppKit's `controlAccentColor`. Browsers do not expose it (WebKit answers a
 * fixed blue for `-apple-system-control-accent` to web pages, against fingerprinting), so elsewhere it is absent.
 */
export const SYSTEM_ACCENT = "--omfp-system-accent";
/** What `accent` derives its roles with (themes.css): relative colour syntax. */
const RELATIVE_COLOR = "oklch(from #000000 l c h)";

const FONT_ROLES = ["prose", "ui", "code"] as const;
const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

export interface Appearance {
  readonly scheme: ColorScheme;
  readonly theme: BuiltinTheme;
  readonly skin: Skin;
  readonly density: Density;
  readonly fontSize: number;
  /** The effective level (UI-145): the setting, or `none` while the system asks for reduced motion. */
  readonly motion: Motion;
  readonly fonts: FontSettings | null | undefined;
}

/**
 * The built-in theme `root` shows for the setting `theme` (UI-510, UI-512): `accent` only with a system accent from the
 * host and a browser that derives colours from it, else Synth; `matugen` Synth, under the user theme (`userTheme`);
 * any other theme as it is.
 */
export function effectiveTheme(root: HTMLElement, theme: BuiltinTheme): BuiltinTheme {
  if (theme === "matugen") {
    return DEFAULT_THEME;
  }
  if (theme !== "accent") {
    return theme;
  }
  const accent = root.style.getPropertyValue(SYSTEM_ACCENT).trim();
  return accent !== "" && CSS.supports("color", RELATIVE_COLOR) ? theme : DEFAULT_THEME;
}

export function applyAppearance(
  root: HTMLElement,
  { scheme, theme, skin, density, fontSize, motion, fonts }: Appearance,
): void {
  root.dataset.colorScheme = scheme;
  root.dataset.theme = effectiveTheme(root, theme);
  root.dataset.skin = skin;
  root.dataset.density = density;
  root.dataset.motion = motion;
  root.style.fontSize = `${fontSize}px`;
  for (const role of FONT_ROLES) {
    const family = fonts?.[role];
    if (family === undefined || family === null || family === "") {
      root.style.removeProperty(`--omfp-font-${role}`);
    } else {
      root.style.setProperty(`--omfp-font-${role}`, family);
    }
  }
}

/**
 * The user theme a window shows for the setting `theme` (UI-510, DMN-240): the daemon's `css` with `matugen`, none with
 * any other theme, while the daemon keeps it.
 */
export function userTheme(theme: BuiltinTheme, css: string | null): string | null {
  return theme === "matugen" ? css : null;
}

/** Puts `css` (`null`: no user theme) in the page's `<style data-omfp-theme>`, which replaces the bundle's `<link>`. */
export function applyUserTheme(document: Document, css: string | null): void {
  const current = document.querySelector(`[${THEME_ATTRIBUTE}]`);
  if (current instanceof HTMLStyleElement) {
    if (current.textContent !== (css ?? "")) {
      current.textContent = css ?? "";
    }
    return;
  }
  const style = document.createElement("style");
  style.setAttribute(THEME_ATTRIBUTE, "");
  style.textContent = css ?? "";
  if (current === null) {
    document.head.append(style);
  } else {
    current.replaceWith(style);
  }
}

/** Whether the system asks for reduced motion (`prefers-reduced-motion: reduce`), live. */
function useSystemReducedMotion(): boolean {
  return useSyncExternalStore(
    (changed) => {
      const query = window.matchMedia(REDUCED_MOTION_QUERY);
      query.addEventListener("change", changed);
      return () => query.removeEventListener("change", changed);
    },
    () => window.matchMedia(REDUCED_MOTION_QUERY).matches,
  );
}

/**
 * Keeps `<html>`'s colour scheme (window state), built-in theme, skin, density, motion, font size and fonts (daemon settings)
 * current, and
 * the contrast guard (UI-196) on it.
 */
export function useAppearance(root: HTMLElement = document.documentElement): void {
  const [scheme] = useWindowState(COLOR_SCHEME);
  const { ui } = useSettings();
  const systemReduced = useSystemReducedMotion();
  const theme = ui.theme ?? DEFAULT_THEME;
  const skin = ui.skin ?? DEFAULT_SKIN;
  const density = ui.density ?? DEFAULT_DENSITY;
  const fontSize = ui.fontSize ?? DEFAULT_FONT_SIZE;
  const motion = systemReduced ? "none" : (ui.motion ?? DEFAULT_MOTION);
  const { fonts } = ui;
  useEffect(
    () => applyAppearance(root, { scheme, theme, skin, density, fontSize, motion, fonts }),
    [root, scheme, theme, skin, density, fontSize, motion, fonts],
  );
  useEffect(() => installContrastGuard(root.ownerDocument, root), [root]);
}

/**
 * Applies the daemon's user theme once the `index` topic is live, while `[ui] theme` is `matugen` (`userTheme`), and
 * every change after: of the user theme (`theme_changed`) and of the theme (`settings_changed`).
 */
export function useUserTheme(): void {
  const live = useSnapshot((snapshot) =>
    snapshot.topics.some((topic) => topic.topic === INDEX_TOPIC && topic.sync.kind === "live"),
  );
  const theme = useSettings().ui.theme ?? DEFAULT_THEME;
  const stored = useSnapshot((snapshot) => snapshot.index.theme.css);
  const css = userTheme(theme, stored);
  useEffect(() => {
    if (live) {
      applyUserTheme(document, css);
    }
  }, [live, css]);
}
