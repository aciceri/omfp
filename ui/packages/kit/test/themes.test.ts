import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { findRule, KIT_SRC, parseCss, schemeRule } from "./support/css.ts";
import { COLOR_TOKENS, prop, STATUS_TOKENS } from "./support/tokens.ts";

/** The built-in themes besides Synth (UI-510): the six palettes and the four calm ones, then the system accent's. */
const PALETTES = [
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
];
const LAYER = "@layer omfp-tokens";
const rules = parseCss(readFileSync(join(KIT_SRC, "themes.css"), "utf8"));

/** A theme's one rule, as the light and the dark scheme resolve its `light-dark()` values (tokens.css). */
function themeRules(name: string) {
  const rule = findRule(rules, [LAYER], `:root[data-theme="${name}"]`);
  return { light: schemeRule(rule, "light"), dark: schemeRule(rule, "dark") };
}

describe("themes.css", () => {
  // spec: UI-510
  test("every rule lives in the omfp-tokens layer, so a user theme wins over any built-in theme", () => {
    expect(rules.filter((rule) => rule.at[0] !== LAYER)).toEqual([]);
    expect(rules.length).toBe(PALETTES.length + 1);
  });

  // spec: UI-511
  test.each(PALETTES)("%s sets every colour role as #rrggbb in light and in dark", (name) => {
    const { light, dark } = themeRules(name);
    for (const rule of [light, dark]) {
      expect(COLOR_TOKENS.filter((token) => !/^#[0-9a-f]{6}$/.test(rule.declarations.get(prop(token)) ?? ""))).toEqual(
        [],
      );
      expect([...rule.declarations.keys()]).toEqual(COLOR_TOKENS.map(prop));
    }
    expect([...light.declarations]).not.toEqual([...dark.declarations]);
  });

  // spec: UI-512
  test("accent derives every role it sets from the system accent and keeps Synth's error and status roles", () => {
    const { light, dark } = themeRules("accent");
    expect([...light.declarations]).not.toEqual([...dark.declarations]);
    for (const rule of [light, dark]) {
      const derived = [...rule.declarations].filter(([, value]) => value.includes("var(--omfp-system-accent)"));
      const fixed = [...rule.declarations].filter(([, value]) => !value.includes("var(--omfp-system-accent)"));
      expect(derived.length).toBeGreaterThan(20);
      expect(fixed.filter(([, value]) => !["#ffffff", "#000000"].includes(value))).toEqual([]);
      const kept = [...STATUS_TOKENS, "error", "on-error", "error-container", "on-error-container"].map(prop);
      expect([...rule.declarations.keys()].filter((name) => kept.includes(name))).toEqual([]);
    }
  });
});
