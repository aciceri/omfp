/**
 * L1: the built-in themes (UI-510..UI-512) as a browser resolves them over Synth's tokens: every text role, label and
 * status colour passes the contrast guard's thresholds as it is (UI-196: the guard overrides nothing) and the kit's
 * text pairs reach AA, in light and dark; `accent` does for every macOS accent colour; and `<html>` shows `accent`
 * only with a system accent from the host; `matugen` is Synth under the user theme, which only it shows.
 */

import { afterEach, describe, expect, test } from "bun:test";
import type { BuiltinTheme } from "@omfp/protocol";
import { TEXT_PAIRS } from "../../../kit/test/support/tokens.ts";
import { THEMES } from "../../src/frame/set.ts";
import { contrastAdjustments, contrastRatio, parseCssColor, type Rgb } from "../../src/theme/contrast.ts";
import { applyAppearance, effectiveTheme, SYSTEM_ACCENT, userTheme } from "../../src/theme/theme.ts";
import { builtinColors, type Scheme } from "./themes.ts";

const SCHEMES: readonly Scheme[] = ["light", "dark"];

/** macOS's accent colours (System Settings → Appearance), `NSColor.controlAccentColor` in sRGB. */
const MACOS_ACCENTS: Readonly<Record<string, string>> = {
  blue: "#007aff",
  purple: "#a550a7",
  pink: "#f74f9e",
  red: "#ff5257",
  orange: "#f7821b",
  yellow: "#ffc600",
  green: "#62ba46",
  graphite: "#8c8c8c",
};

/** The palettes of themes.css: every theme but `matugen`, the user theme over Synth. */
const PALETTES = THEMES.filter((theme) => theme !== "matugen");

/** What the guard would override and the kit's text pairs under AA, for one theme's colours. */
function problems(colors: ReadonlyMap<string, Rgb>): string[] {
  const read = (token: string) => colors.get(token);
  const overrides = contrastAdjustments(read).map(({ role, before }) => `guard: ${role} ${before.toFixed(2)}`);
  const pairs = TEXT_PAIRS.flatMap(([text, background]) => {
    const [a, b] = [read(text), read(background)];
    const ratio = a === undefined || b === undefined ? 0 : contrastRatio(a, b);
    return ratio < 4.5 ? [`${text} on ${background} ${ratio.toFixed(2)}`] : [];
  });
  return [...overrides, ...pairs];
}

describe("built-in themes", () => {
  // spec: UI-511, UI-510, UI-196
  test.each(
    PALETTES.filter((theme) => theme !== "accent").flatMap((theme) => SCHEMES.map((scheme) => [theme, scheme])),
  )("%s in %s passes the guard as it is and its text pairs reach AA", (theme, scheme) => {
    expect(problems(builtinColors(theme as BuiltinTheme, scheme as Scheme))).toEqual([]);
  });

  // spec: UI-512, UI-196
  test.each(Object.entries(MACOS_ACCENTS).flatMap(([name, hex]) => SCHEMES.map((scheme) => [name, hex, scheme])))(
    "accent from macOS %s (%s) in %s passes the guard as it is and its text pairs reach AA",
    (_name, hex, scheme) => {
      const accent = parseCssColor(hex);
      if (accent === undefined) throw new Error(hex);
      expect(problems(builtinColors("accent", scheme as Scheme, accent))).toEqual([]);
    },
  );

  // spec: UI-511
  test("each palette has its own colours, light and dark apart", () => {
    const surfaces = PALETTES.flatMap((theme) =>
      SCHEMES.map((scheme) => builtinColors(theme, scheme).get("surface")?.join(",")),
    );
    expect(new Set(surfaces).size).toBe(surfaces.length);
  });
});

describe("the accent theme on <html>", () => {
  const root = document.documentElement;
  const appearance = {
    scheme: "system",
    density: "compact",
    fontSize: 17,
    motion: "full",
    fonts: null,
  } as const;

  afterEach(() => {
    root.style.removeProperty(SYSTEM_ACCENT);
    delete root.dataset.theme;
  });

  // spec: UI-512, UI-510
  test("shows only with a system accent from the host and relative colours; otherwise it is Synth", () => {
    applyAppearance(root, { ...appearance, theme: "accent" });
    expect(root.dataset.theme).toBe("synth");
    root.style.setProperty(SYSTEM_ACCENT, "#a550a7");
    applyAppearance(root, { ...appearance, theme: "accent" });
    expect(root.dataset.theme).toBe("accent");
    // A browser without relative colour syntax cannot derive the roles.
    const css = Object.getOwnPropertyDescriptor(globalThis, "CSS");
    Object.defineProperty(globalThis, "CSS", { value: { supports: () => false }, configurable: true });
    try {
      expect(effectiveTheme(root, "accent")).toBe("synth");
      expect(effectiveTheme(root, "nord")).toBe("nord");
    } finally {
      if (css !== undefined) {
        Object.defineProperty(globalThis, "CSS", css);
      }
    }
    applyAppearance(root, { ...appearance, theme: "rose-pine" });
    expect(root.dataset.theme).toBe("rose-pine");
  });
});

describe("the matugen theme", () => {
  const root = document.documentElement;

  afterEach(() => {
    delete root.dataset.theme;
  });

  // spec: UI-510, DMN-240
  test("is Synth under the user theme, which no other theme shows", () => {
    applyAppearance(root, {
      scheme: "system",
      theme: "matugen",
      density: "compact",
      fontSize: 17,
      motion: "full",
      fonts: null,
    });
    expect(root.dataset.theme).toBe("synth");
    const css = ":root { --omfp-primary: #6750a4; }";
    expect(THEMES.map((theme) => [theme, userTheme(theme, css)]).filter(([, shown]) => shown !== null)).toEqual([
      ["matugen", css],
    ]);
    expect(userTheme("matugen", null)).toBeNull();
  });
});
