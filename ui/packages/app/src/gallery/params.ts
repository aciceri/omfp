/**
 * The `/kit` gallery's parameters (UI-173, UI-514): `?scheme=light|dark`, `?theme=` a theme (UI-510: `synth`,
 * `catppuccin`, …, `accent`, and `matugen` with the matugen-forest fixture as the user theme) or the fixture user theme
 * `dux-solarized`, `?accent=` the system accent `accent` derives from (`#rrggbb`, default macOS's blue), `?density=`,
 * `?motion=` and `?fs=` (the root size in px). Unknown values keep the app's defaults.
 */
import type { BuiltinTheme, Density, Motion } from "@omfp/protocol";
import { DENSITIES, MOTIONS, SCHEMES, THEMES } from "../frame/set.ts";
import { type ColorScheme, DEFAULT_DENSITY, DEFAULT_FONT_SIZE, DEFAULT_MOTION, DEFAULT_THEME } from "../sdk/index.ts";

/** The fixture user theme over Synth besides `matugen`'s: the user preset the contrast guard fixes (UI-196). */
export type GalleryFixture = "dux-solarized";

/** A theme of the setting, or the fixture user theme. */
export type GalleryTheme = BuiltinTheme | GalleryFixture;

export interface GalleryParams {
  readonly scheme: ColorScheme;
  readonly theme: GalleryTheme;
  /** The system accent `accent` derives from (`#rrggbb`). */
  readonly accent: string;
  readonly density: Density;
  readonly motion: Motion;
  readonly fontSize: number;
}

const FIXTURES: readonly GalleryFixture[] = ["dux-solarized"];
/** Every theme the gallery shows, its picker's order: the themes of the setting, then the fixture. */
export const GALLERY_THEMES: readonly GalleryTheme[] = [...THEMES, ...FIXTURES];
/** macOS's default accent, blue (`NSColor.controlAccentColor` with the multicolour setting). */
export const DEFAULT_ACCENT = "#007aff";
/** Root sizes the gallery accepts (px), the `[ui] font_size` range. */
const MIN_FONT_SIZE = 10;
const MAX_FONT_SIZE = 32;

function pick<T extends string>(value: string | null, allowed: readonly T[], fallback: T): T {
  return allowed.find((candidate) => candidate === value) ?? fallback;
}

/** The theme of the setting `theme` is, or `undefined` for the fixture user theme. */
export function builtinTheme(theme: GalleryTheme): BuiltinTheme | undefined {
  return THEMES.find((candidate) => candidate === theme);
}

/** Whether `pathname` is the gallery's. */
export function isGalleryPath(pathname: string): boolean {
  return pathname === "/kit" || pathname === "/kit/";
}

/** The parameters of a gallery URL's query string. */
export function galleryParams(search: string): GalleryParams {
  const query = new URLSearchParams(search);
  const size = Number(query.get("fs"));
  const accent = query.get("accent")?.toLowerCase() ?? "";
  return {
    scheme: pick(query.get("scheme"), SCHEMES, "system"),
    theme: pick(query.get("theme"), GALLERY_THEMES, DEFAULT_THEME),
    accent: /^#[0-9a-f]{6}$/.test(accent) ? accent : DEFAULT_ACCENT,
    density: pick(query.get("density"), DENSITIES, DEFAULT_DENSITY),
    motion: pick(query.get("motion"), MOTIONS, DEFAULT_MOTION),
    fontSize: Number.isInteger(size) && size >= MIN_FONT_SIZE && size <= MAX_FONT_SIZE ? size : DEFAULT_FONT_SIZE,
  };
}

/** The gallery's query string for `params` with `change` applied (the picker's links keep the other parameters). */
export function gallerySearch(params: GalleryParams, change: Partial<GalleryParams>): string {
  const next = { ...params, ...change };
  const query = new URLSearchParams({
    scheme: next.scheme,
    theme: next.theme,
    density: next.density,
    motion: next.motion,
    fs: String(next.fontSize),
  });
  if (next.theme === "accent") {
    query.set("accent", next.accent);
  }
  return `?${query.toString()}`;
}
