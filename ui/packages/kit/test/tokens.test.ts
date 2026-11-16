import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { type CssRule, cssFiles, findRule, KIT_SRC, parseCss } from "./support/css.ts";
import {
  COLOR_TOKENS,
  DENSITY_SIZES,
  DERIVED_TOKENS,
  FONT_FILES,
  FONT_TOKENS,
  FRAME_SIZES,
  prop,
  SEARCH_TOKENS,
  SHAPE_TOKENS,
  SPACE_STEPS,
  SURFACE_TOKENS,
  SYNTH,
  TEXT_PAIRS,
  TEXT_SPACING,
  TYPE_SCALE,
} from "./support/tokens.ts";

const LAYER = "@layer omfp-tokens";
const DARK_MEDIA = "@media (prefers-color-scheme: dark)";
const TOKENS_CSS = join(KIT_SRC, "tokens.css");
const rules = parseCss(readFileSync(TOKENS_CSS, "utf8"));

const light = findRule(rules, [LAYER], ":root");
const darkBySystem = findRule(rules, [LAYER, DARK_MEDIA], ':root:not([data-color-scheme="light"])');
const darkForced = findRule(rules, [LAYER], ':root[data-color-scheme="dark"]');
const scaled = findRule(rules, [LAYER], ":root, [data-density]");
const compact = findRule(rules, [LAYER], '[data-density="compact"]');
const normal = findRule(rules, [LAYER], '[data-density="normal"]');
const fontFaces = rules.filter((rule) => rule.selector === "@font-face");

/** The `/kit` gallery's matugen fixture (UI-173), which defines no status roles. */
const forest = parseCss(readFileSync(join(KIT_SRC, "..", "..", "app", "src", "gallery", "matugen-forest.css"), "utf8"));

/** Name, omfp's base tokens, the theme's tokens (UI-136). */
const STATUS_THEMES: readonly (readonly [string, CssRule, CssRule])[] = [
  ["Synth light", light, light],
  ["Synth dark", darkForced, darkForced],
  ["matugen forest light", light, findRule(forest, [], ":root")],
  ["matugen forest dark", darkForced, findRule(forest, [], ':root[data-color-scheme="dark"]')],
];

/** Where status text sits: the app, the left column and mode line, code and popovers (UI-130). */
const STATUS_BACKGROUNDS = ["surface", "surface-container-lowest", "surface-container-low", "surface-container"];

function missing(rule: CssRule, tokens: readonly string[]): string[] {
  return tokens.filter((token) => !rule.declarations.has(prop(token)));
}

/** Whitespace inside parentheses does not matter (multi-line values). */
function tidy(text: string): string {
  return text.replace(/\(\s+/g, "(").replace(/\s+\)/g, ")");
}

function value(rule: CssRule, token: string): string {
  const found = rule.declarations.get(prop(token));
  if (found === undefined) throw new Error(`${prop(token)} missing in ${rule.selector}`);
  return tidy(found);
}

/** The tokens of `expected` whose value in `rule` differs, with the value found (`toEqual` ignores `undefined`). */
function differing(rule: CssRule, expected: Readonly<Record<string, string>>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(expected)
      .filter(([token, wanted]) => {
        const found = rule.declarations.get(prop(token));
        return found === undefined || tidy(found) !== wanted;
      })
      .map(([token]) => [token, rule.declarations.get(prop(token)) ?? "(missing)"]),
  );
}

/** WCAG relative luminance of a `#rrggbb` color. */
function luminance(hex: string): number {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (match === null) throw new Error(`not a #rrggbb color: ${hex}`);
  const [r, g, b] = match.slice(1).map((channel) => {
    const c = Number.parseInt(channel, 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * (r ?? 0) + 0.7152 * (g ?? 0) + 0.0722 * (b ?? 0);
}

function contrast(a: string, b: string): number {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return ((high ?? 0) + 0.05) / ((low ?? 0) + 0.05);
}

/** sRGB channel (0–1) to linear light and back. */
function linear(c: number): number {
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function gamma(c: number): number {
  return Math.min(1, Math.max(0, c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055));
}

/** `color-mix(in oklab, a p, b)` of two `#rrggbb` colors, as `#rrggbb`. */
function mixOklab(a: string, b: string, p: number): string {
  const toOklab = (hex: string) => {
    const [r, g, bl] = [1, 3, 5].map((at) => linear(Number.parseInt(hex.slice(at, at + 2), 16) / 255)) as [
      number,
      number,
      number,
    ];
    const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * bl);
    const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * bl);
    const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * bl);
    return [
      0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
      1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
      0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ];
  };
  const [x, y] = [toOklab(a), toOklab(b)];
  const [L, A, B] = x.map((channel, at) => p * channel + (1 - p) * (y[at] ?? 0)) as [number, number, number];
  const l = (L + 0.3963377774 * A + 0.2158037573 * B) ** 3;
  const m = (L - 0.1055613458 * A - 0.0638541728 * B) ** 3;
  const s = (L - 0.0894841775 * A - 1.291485548 * B) ** 3;
  const rgb = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
  return `#${rgb
    .map((c) =>
      Math.round(gamma(c) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

describe("tokens.css", () => {
  // spec: UI-091, UI-125
  test("every rule lives in the omfp-tokens layer", () => {
    expect(rules.filter((rule) => rule.at[0] !== LAYER)).toEqual([]);
  });

  // spec: UI-091, UI-125
  test("light (:root) defines every color and font token", () => {
    expect(missing(light, [...COLOR_TOKENS, ...FONT_TOKENS])).toEqual([]);
    expect(light.declarations.get("color-scheme")).toBe("light");
  });

  // spec: UI-139
  test("the built-in palette is Synth, light on :root", () => {
    expect(differing(light, Object.fromEntries(COLOR_TOKENS.map((token) => [token, SYNTH[token]?.[0] ?? ""])))).toEqual(
      {},
    );
  });

  // spec: UI-091, UI-139
  test("dark is Synth dark, by prefers-color-scheme and by the data-color-scheme override alike", () => {
    expect(
      differing(darkForced, Object.fromEntries(COLOR_TOKENS.map((token) => [token, SYNTH[token]?.[1] ?? ""]))),
    ).toEqual({});
    expect([...darkBySystem.declarations]).toEqual([...darkForced.declarations]);
    expect(darkForced.declarations.get("color-scheme")).toBe("dark");
  });

  // spec: UI-374
  test("native controls follow the effective scheme: color-scheme per scheme, accent-color the primary role", () => {
    expect(light.declarations.get("color-scheme")).toBe("light");
    expect(light.declarations.get("accent-color")).toBe("var(--omfp-primary)");
    expect(darkBySystem.declarations.get("color-scheme")).toBe("dark");
    expect(darkForced.declarations.get("color-scheme")).toBe("dark");
    // The forced light scheme keeps `:root`'s light even when the system is dark.
    expect(darkBySystem.selector).toBe(':root:not([data-color-scheme="light"])');
    // Nothing below the root resets them, in the kit or the app.
    const others = [...cssFiles(KIT_SRC), ...cssFiles(join(KIT_SRC, "..", "..", "app", "src"))].filter(
      (file) => file !== TOKENS_CSS,
    );
    expect(others.length).toBeGreaterThan(2);
    const resetting = others.flatMap((file) =>
      parseCss(readFileSync(file, "utf8"))
        .filter((rule) => rule.declarations.has("color-scheme") || rule.declarations.has("accent-color"))
        .map((rule) => `${file}: ${rule.selector}`),
    );
    expect(resetting).toEqual([]);
  });

  // spec: UI-091, UI-139
  test.each(["light", "dark"] as const)("text roles reach 4.5:1 contrast in %s", (scheme) => {
    const rule = scheme === "light" ? light : darkForced;
    const failing = TEXT_PAIRS.map(([text, background]) => ({
      pair: `${text} on ${background}`,
      ratio: contrast(value(rule, text), value(rule, background)),
    })).filter(({ ratio }) => ratio < 4.5);
    expect(failing).toEqual([]);
  });

  // spec: UI-136, UI-137
  test.each(STATUS_THEMES)(
    "ok and warn reach 4.5:1 as text on the surfaces they sit on in %s",
    (_name, base, theme) => {
      // A theme without status roles keeps omfp's (UI-136).
      const color = (token: string) => value(theme.declarations.has(prop(token)) ? theme : base, token);
      const failing = ["success", "warning"].flatMap((status) => {
        const text = mixOklab(color(status), color("surface"), 0.85);
        return STATUS_BACKGROUNDS.map((background) => ({
          pair: `${status} 85% on ${background}`,
          ratio: contrast(text, color(background)),
        })).filter(({ ratio }) => ratio < 4.5);
      });
      expect(failing).toEqual([]);
    },
  );

  // spec: UI-121, UI-142
  test("prose is IBM Plex Serif, code Commit Mono, and the interface follows the code family", () => {
    expect(value(light, "font-prose")).toBe('"IBM Plex Serif", ui-serif, Georgia, serif');
    expect(value(light, "font-code")).toBe('"Commit Mono", ui-monospace, monospace');
    expect(value(light, "font-ui")).toBe("var(--omfp-font-code)");
  });

  // spec: UI-141, UI-142
  test("one @font-face per bundled file, swapping, with the file's weight and style; the files and licences ship", () => {
    expect(
      fontFaces.map((rule) => [
        rule.declarations.get("font-family"),
        rule.declarations.get("src"),
        rule.declarations.get("font-weight"),
        rule.declarations.get("font-style"),
        rule.declarations.get("font-display"),
      ]),
    ).toEqual(
      FONT_FILES.map(([family, stem, weight, style]) => [
        `"${family}"`,
        `url("/fonts/${stem}.woff2") format("woff2")`,
        weight,
        style,
        "swap",
      ]),
    );
    const fonts = join(KIT_SRC, "..", "fonts");
    expect(FONT_FILES.filter(([, stem]) => !existsSync(join(fonts, `${stem}.woff2`)))).toEqual([]);
    expect(
      ["ibm-plex-serif-OFL.txt", "commit-mono-OFL.txt"].filter(
        (licence) => !readFileSync(join(fonts, licence), "utf8").includes("SIL Open Font License, Version 1.1"),
      ),
    ).toEqual([]);
  });

  // spec: UI-126
  test("the type scale, weights and line heights are fixed rem values, the same in both densities", () => {
    expect(differing(light, TYPE_SCALE)).toEqual({});
  });

  // spec: UI-127, UI-094, UI-173
  test("spacing steps are unit × n × density, compact by default and normal on data-density, also in a subtree", () => {
    expect(value(light, "unit")).toBe("0.25rem");
    expect(value(light, "density")).toBe("1");
    expect(value(normal, "density")).toBe("1.3");
    expect([...normal.declarations.keys()]).toEqual([prop("density")]);
    expect(value(compact, "density")).toBe("1");
    expect([...compact.declarations.keys()]).toEqual([prop("density")]);
    expect(
      differing(
        scaled,
        Object.fromEntries(
          SPACE_STEPS.map((n) => [`s${n}`, `calc(var(--omfp-unit) * ${n} * var(--omfp-density))`] as const),
        ),
      ),
    ).toEqual({});
  });

  // spec: UI-127
  test("text spacing is in em/ch, so chips, inline code and Markdown flow scale with their text, not with density", () => {
    expect(differing(light, TEXT_SPACING)).toEqual({});
  });

  // spec: UI-128
  test("row, control and message-gap sizes scale with density; the frame sizes do not", () => {
    expect(
      differing(
        scaled,
        Object.fromEntries(
          Object.entries(DENSITY_SIZES).map(([token, size]) => [token, `calc(${size} * var(--omfp-density))`]),
        ),
      ),
    ).toEqual({});
    expect(differing(light, FRAME_SIZES)).toEqual({});
  });

  // spec: UI-129, UI-130, UI-131, UI-132, UI-133
  test("radii, rules, surfaces, icons, focus and interactive states", () => {
    expect(differing(light, SHAPE_TOKENS)).toEqual({});
    expect(differing(light, SURFACE_TOKENS)).toEqual({});
    expect(differing(darkForced, { "pop-bg": "var(--omfp-surface-container)" })).toEqual({});
  });

  // spec: UI-134, UI-135, UI-136
  test("gradients, ok and warn derive from the roles, with the end stop pulled toward on-surface", () => {
    expect(differing(light, DERIVED_TOKENS)).toEqual({});
    const derived = Object.keys(DERIVED_TOKENS).map(prop);
    expect([...darkForced.declarations.keys()].filter((name) => derived.includes(name))).toEqual([]);
  });

  // spec: UI-736
  test("the search highlights derive from role pairs that reach AA, the same in light and dark", () => {
    expect(differing(light, SEARCH_TOKENS)).toEqual({});
    const search = Object.keys(SEARCH_TOKENS).map(prop);
    expect([...darkForced.declarations.keys()].filter((name) => search.includes(name))).toEqual([]);
    const pairs = TEXT_PAIRS.map(([text, background]) => `${text} on ${background}`);
    expect(
      ["on-tertiary-container on tertiary-container", "on-primary on primary"].filter((pair) => !pairs.includes(pair)),
    ).toEqual([]);
  });
});
