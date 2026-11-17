/**
 * `:set <key>=<value>` (UI-097): one setting of the daemon's `[ui]` or `[keymap]`, sent with `settings.set`. Keys:
 * `density`, `font_size` (px, 10..=28), `motion`, `notifications`, `show_xdev_notices`, `theme` (UI-510), `skin`
 * (UI-794), `fonts.prose|ui|code`, `keymap.<keys>` (normal mode) and `keymap.<mode>.<keys>`; a keymap value is a command id, `""`
 * removes the binding.
 */

import type { BuiltinTheme, Density, Motion, NotificationMode, SetSettingsParams, Skin } from "@omfp/protocol";
import { type Mode, normalizeKeys } from "../keymap/keymap.ts";
import { type ColorScheme, FONT_SIZE_MAX, FONT_SIZE_MIN } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";

type FontRole = "prose" | "ui" | "code";

/** The window's color schemes (`:theme`, the gallery's `?scheme=`). */
export const SCHEMES: readonly ColorScheme[] = ["system", "light", "dark"];
export const DENSITIES: readonly Density[] = ["compact", "normal"];
export const MOTIONS: readonly Motion[] = ["full", "reduced", "none"];
const NOTIFICATIONS: readonly NotificationMode[] = ["background", "always", "never"];
/**
 * The themes (UI-510): Synth (the default), the six palettes, the four calm ones for the boring skin, the system
 * accent's and `matugen`, the user theme (DMN-075) over Synth.
 */
export const THEMES: readonly BuiltinTheme[] = [
  "synth",
  "catppuccin",
  "gruvbox",
  "tokyo-night",
  "nord",
  "solarized",
  "rose-pine",
  "graphite",
  "paper",
  "clay",
  "primer",
  "accent",
  "matugen",
];
/** The skins (UI-794): Crush (the default) and the opt-in boring one. */
export const SKINS: readonly Skin[] = ["crush", "boring"];
const FONT_ROLES: readonly FontRole[] = ["prose", "ui", "code"];
const MODES: readonly Mode[] = ["normal", "insert", "cmdline"];

/** Completions of the key part (`:set ` …). */
export const SET_KEYS: readonly string[] = [
  ...DENSITIES.map((density) => `density=${density}`),
  "font_size=",
  ...MOTIONS.map((motion) => `motion=${motion}`),
  ...NOTIFICATIONS.map((mode) => `notifications=${mode}`),
  "show_xdev_notices=false",
  "show_xdev_notices=true",
  ...THEMES.map((theme) => `theme=${theme}`),
  ...SKINS.map((skin) => `skin=${skin}`),
  ...FONT_ROLES.map((role) => `fonts.${role}=`),
  "keymap.",
];

/** `:theme`'s arguments (UI-513): a colour scheme (this window's) and a built-in theme (the daemon's), either or both. */
export type ParsedTheme =
  | { readonly ok: true; readonly scheme?: ColorScheme; readonly theme?: BuiltinTheme }
  | { readonly ok: false; readonly error: string };

/** The completions of `:theme`'s next argument after `before`: the schemes, then the themes, less a kind already given. */
export function themeArgs(before: readonly string[]): string[] {
  const has = (names: readonly string[]) => before.some((word) => names.includes(word));
  return [...(has(SCHEMES) ? [] : SCHEMES), ...(has(THEMES) ? [] : THEMES)];
}

/** `:theme`'s arguments as a scheme and a theme, or what is wrong with them (none, an unknown word, two of a kind). */
export function parseTheme(args: readonly string[]): ParsedTheme {
  const schemes = SCHEMES.filter((candidate) => args.includes(candidate));
  const themes = THEMES.filter((candidate) => args.includes(candidate));
  const unknown = args.find((word) => !schemes.some((s) => s === word) && !themes.some((t) => t === word));
  if (unknown !== undefined) {
    return { ok: false, error: text.unknownTheme(unknown) };
  }
  const [scheme] = schemes;
  const [theme] = themes;
  if (args.length === 0 || args.length > 2 || schemes.length > 1 || themes.length > 1) {
    return { ok: false, error: text.themeUsage };
  }
  return { ok: true, ...(scheme === undefined ? {} : { scheme }), ...(theme === undefined ? {} : { theme }) };
}

export type ParsedSet =
  | { readonly ok: true; readonly patch: SetSettingsParams }
  | { readonly ok: false; readonly error: string };

const text = strings.shell.settings;

/** The patch for `line` (everything after `:set`; a font value may contain spaces), or what is wrong with it. */
export function parseSet(line: string): ParsedSet {
  const equals = line.indexOf("=");
  if (equals < 0) {
    return { ok: false, error: text.setUsage };
  }
  const key = line.slice(0, equals).trim();
  const value = unquote(line.slice(equals + 1).trim());
  if (key === "density") {
    const density = DENSITIES.find((candidate) => candidate === value);
    return density === undefined ? invalid(key, value) : { ok: true, patch: { ui: { density } } };
  }
  if (key === "font_size") {
    const fontSize = /^\d+$/.test(value) ? Number(value) : Number.NaN;
    return fontSize >= FONT_SIZE_MIN && fontSize <= FONT_SIZE_MAX
      ? { ok: true, patch: { ui: { fontSize } } }
      : invalid(key, value);
  }
  if (key === "motion") {
    const motion = MOTIONS.find((candidate) => candidate === value);
    return motion === undefined ? invalid(key, value) : { ok: true, patch: { ui: { motion } } };
  }
  if (key === "notifications") {
    const notifications = NOTIFICATIONS.find((candidate) => candidate === value);
    return notifications === undefined ? invalid(key, value) : { ok: true, patch: { ui: { notifications } } };
  }
  if (key === "show_xdev_notices") {
    return value === "true" || value === "false"
      ? { ok: true, patch: { ui: { showXdevNotices: value === "true" } } }
      : invalid(key, value);
  }
  if (key === "theme") {
    const theme = THEMES.find((candidate) => candidate === value);
    return theme === undefined ? invalid(key, value) : { ok: true, patch: { ui: { theme } } };
  }
  if (key === "skin") {
    const skin = SKINS.find((candidate) => candidate === value);
    return skin === undefined ? invalid(key, value) : { ok: true, patch: { ui: { skin } } };
  }
  if (key.startsWith("fonts.")) {
    const role = FONT_ROLES.find((candidate) => `fonts.${candidate}` === key);
    return role === undefined || value === ""
      ? invalid(key, value)
      : { ok: true, patch: { ui: { fonts: { [role]: value } } } };
  }
  if (key.startsWith("keymap.")) {
    return parseKeymap(key.slice("keymap.".length), value);
  }
  return { ok: false, error: text.unknownKey(key) };
}

function parseKeymap(rest: string, command: string): ParsedSet {
  const dot = rest.indexOf(".");
  const named = MODES.find((mode) => dot > 0 && rest.slice(0, dot) === mode);
  const mode = named ?? "normal";
  const keys = normalizeKeys(named === undefined ? rest : rest.slice(dot + 1));
  if (keys === "") {
    return { ok: false, error: text.setUsage };
  }
  return { ok: true, patch: { keymap: [{ mode, keys, command }] } };
}

function invalid(key: string, value: string): ParsedSet {
  return { ok: false, error: text.invalidValue(key, value) };
}

/** `"…"` or `'…'` around a value are dropped, so `:set keymap.g x=""` unbinds `g x`. */
function unquote(value: string): string {
  const quoted = value.length >= 2 && (value[0] === '"' || value[0] === "'") && value.at(-1) === value[0];
  return quoted ? value.slice(1, -1) : value;
}
