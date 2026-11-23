/**
 * The test themes as role → color maps, read from the CSS the app ships (Synth, kit/src/tokens.css) and the gallery's
 * fixtures, the way a browser resolves them: the fixture's roles over Synth's, `ok`/`warn` mixed in OKLab (UI-136).
 */

import type { BuiltinTheme } from "@omfp/protocol";
import builtins from "../../../kit/src/themes.css" with { type: "text" };
import tokens from "../../../kit/src/tokens.css" with { type: "text" };
import { findRule, parseCss, schemeRule } from "../../../kit/test/support/css.ts";
import dux from "../../src/gallery/dux-solarized.css" with { type: "text" };
import forest from "../../src/gallery/matugen-forest.css" with { type: "text" };
import { fromOklab, parseCssColor, type Rgb, toOklab } from "../../src/theme/contrast.ts";

export type Scheme = "light" | "dark";
export type ThemeName = "synth" | "matugen-forest" | "dux-solarized";

const FORCED_DARK = ':root[data-color-scheme="dark"]';

/** The `#rrggbb` roles of `css` in `scheme`: `:root`'s `light-dark()` sides, or a user theme's forced dark rule. */
function roles(css: string, scheme: Scheme): Map<string, Rgb> {
  const rules = parseCss(css);
  const forced = rules.filter(
    (rule) => rule.selector === FORCED_DARK && [...rule.declarations.keys()].some((name) => name.startsWith("--omfp-")),
  );
  const chosen = scheme === "dark" && forced.length > 0 ? forced : rules.filter((rule) => rule.selector === ":root");
  const map = new Map<string, Rgb>();
  for (const rule of chosen) {
    for (const [property, value] of schemeRule(rule, scheme).declarations) {
      const color = /^#[0-9a-fA-F]{6}$/.test(value) ? parseCssColor(value) : undefined;
      if (property.startsWith("--omfp-") && color !== undefined) {
        map.set(property.slice("--omfp-".length), color);
      }
    }
  }
  return map;
}

function mixOklab(a: Rgb, b: Rgb, share: number): Rgb {
  const [x, y] = [toOklab(a), toOklab(b)];
  return fromOklab([
    x[0] * share + y[0] * (1 - share),
    x[1] * share + y[1] * (1 - share),
    x[2] * share + y[2] * (1 - share),
  ]);
}

const SOURCES: Record<ThemeName, string | null> = {
  synth: null,
  "matugen-forest": forest as unknown as string,
  "dux-solarized": dux as unknown as string,
};

/** The colors of `theme` in `scheme`, by token name without `--omfp-` (roles, `ok`, `warn`). */
export function themeColors(theme: ThemeName, scheme: Scheme): Map<string, Rgb> {
  const map = roles(tokens as unknown as string, scheme);
  const source = SOURCES[theme];
  if (source !== null) {
    for (const [name, color] of roles(source, scheme)) {
      map.set(name, color);
    }
  }
  const surface = map.get("surface");
  for (const [derived, role] of [
    ["ok", "success"],
    ["warn", "warning"],
  ] as const) {
    const color = map.get(role);
    if (color !== undefined && surface !== undefined) {
      map.set(derived, mixOklab(color, surface, 0.85));
    }
  }
  return map;
}

/**
 * An `oklch(from var(--omfp-system-accent) L C H)` value of the accent theme (themes.css) for `accent`, as a browser
 * resolves it: `C` is `c`, `calc(c * k)` or `min(c[ * k], cap)`, `H` is `h` or `calc(h + d)`; out of sRGB the chroma
 * is reduced (fromOklab). `undefined` for anything else.
 */
function fromAccent(value: string, accent: Rgb): Rgb | undefined {
  const match =
    /^oklch\(from var\(--omfp-system-accent\) ([\d.]+) (?:c|min\(c(?: \* ([\d.]+))?, ([\d.]+)\)) (?:h|calc\(h \+ (\d+)\))\)$/.exec(
      value,
    );
  if (match === null) {
    return undefined;
  }
  const [, a, b] = toOklab(accent);
  const chroma = Math.min(Math.hypot(a, b) * Number(match[2] ?? 1), Number(match[3] ?? Number.POSITIVE_INFINITY));
  const hue = Math.atan2(b, a) + (Number(match[4] ?? 0) * Math.PI) / 180;
  return fromOklab([Number(match[1]), chroma * Math.cos(hue), chroma * Math.sin(hue)]);
}

/**
 * The colors of the built-in `theme` (UI-510) in `scheme` over Synth's, by token name without `--omfp-` (roles, `ok`,
 * `warn`), the way a browser resolves themes.css; `accent` derives from `systemAccent` (UI-512).
 */
export function builtinColors(
  theme: BuiltinTheme,
  scheme: Scheme,
  systemAccent: Rgb = [0, 122, 255],
): Map<string, Rgb> {
  const map = themeColors("synth", scheme);
  if (theme !== "synth") {
    const rule = schemeRule(
      findRule(parseCss(builtins as unknown as string), ["@layer omfp-tokens"], `:root[data-theme="${theme}"]`),
      scheme,
    );
    for (const [property, value] of rule.declarations) {
      const color = parseCssColor(value) ?? fromAccent(value, systemAccent);
      if (color === undefined) {
        throw new Error(`${theme} ${scheme}: cannot resolve ${property}: ${value}`);
      }
      map.set(property.slice("--omfp-".length), color);
    }
  }
  const surface = map.get("surface");
  for (const [derived, role] of [
    ["ok", "success"],
    ["warn", "warning"],
  ] as const) {
    const color = map.get(role);
    if (color !== undefined && surface !== undefined) {
      map.set(derived, mixOklab(color, surface, 0.85));
    }
  }
  return map;
}

/** The text of a gallery fixture theme (`null`: Synth, no user theme). */
export function themeCss(theme: ThemeName): string | null {
  return SOURCES[theme];
}
