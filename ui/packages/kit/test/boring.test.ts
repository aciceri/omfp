/** The boring skin's stylesheets (UI-794, UI-795): scoped, token-driven overrides of Crush. */
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { findRule, KIT_SRC, parseCss } from "./support/css.ts";

const SKIN = ':root[data-skin="boring"]';
const FULL = ':root[data-motion="full"]';
const APP_SRC = join(KIT_SRC, "..", "..", "app", "src");
const kitRules = parseCss(readFileSync(join(KIT_SRC, "kit.css"), "utf8"));
const tokenRules = parseCss(readFileSync(join(KIT_SRC, "tokens.css"), "utf8"));
const boringRules = parseCss(readFileSync(join(KIT_SRC, "boring.css"), "utf8"));
const appBoringRules = parseCss(readFileSync(join(APP_SRC, "boring.css"), "utf8"));

/** The selectors of a selector list, split at its top-level commas. */
function selectors(list: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < list.length; index += 1) {
    const char = list[index];
    if (char === "(") depth += 1;
    else if (char === ")") depth -= 1;
    else if (char === "," && depth === 0) {
      parts.push(list.slice(start, index).trim());
      start = index + 1;
    }
  }
  parts.push(list.slice(start).trim());
  return parts;
}

describe("the boring skin", () => {
  // spec: UI-794, UI-795
  test.each([
    ["kit", boringRules, ["@layer omfp-tokens", "@layer omfp-kit"]],
    ["app", appBoringRules, ["@layer omfp-widgets"]],
  ] as const)("every %s rule is scoped under data-skin=boring, in the layers it overrides", (_name, rules, layers) => {
    expect(rules.length).toBeGreaterThan(0);
    expect(rules.flatMap((rule) => selectors(rule.selector)).filter((selector) => !selector.startsWith(SKIN))).toEqual(
      [],
    );
    expect(rules.filter((rule) => !layers.some((layer) => rule.at[0] === layer)).map((rule) => rule.selector)).toEqual(
      [],
    );
  });

  // spec: UI-795, UI-121
  test("chrome and prose take the system sans-serif, code keeps the code family; [ui] fonts still win (inline)", () => {
    const tokens = findRule(boringRules, ["@layer omfp-tokens"], SKIN).declarations;
    expect([tokens.get("--omfp-font-ui"), tokens.get("--omfp-font-prose"), tokens.get("--omfp-font-code")]).toEqual([
      "var(--omfp-font-sans)",
      "var(--omfp-font-sans)",
      undefined,
    ]);
    const sans = findRule(tokenRules, ["@layer omfp-tokens"], ":root").declarations.get("--omfp-font-sans") ?? "";
    expect(sans.startsWith("system-ui,") && sans.endsWith("sans-serif")).toBe(true);
    // Its own families: the interface one, or the code one for what shows code (commands, outputs, paths).
    const families = [...boringRules, ...appBoringRules].flatMap((rule) =>
      rule.declarations.has("font-family") ? [[rule.selector, rule.declarations.get("font-family")]] : [],
    );
    expect(
      families.filter(([selector, family]) =>
        family === "var(--omfp-font-code)"
          ? !/__(?:action|output|command|path)\b/.test(selector ?? "")
          : family !== "var(--omfp-font-ui)",
      ),
    ).toEqual([]);
  });

  // spec: UI-795, UI-123
  test("no gradients: the gradient tokens are one solid role and no rule draws a gradient of its own", () => {
    const tokens = findRule(boringRules, ["@layer omfp-tokens"], SKIN).declarations;
    expect(["--omfp-grad", "--omfp-grad-loop", "--omfp-grad-soft"].map((token) => tokens.get(token))).toEqual([
      "linear-gradient(var(--omfp-primary), var(--omfp-primary))",
      "linear-gradient(var(--omfp-primary), var(--omfp-primary))",
      "linear-gradient(var(--omfp-sel-bg), var(--omfp-sel-bg))",
    ]);
    expect(["--omfp-hover", "--omfp-sel-bg", "--omfp-focus-bg"].map((token) => tokens.get(token))).toEqual([
      "color-mix(in oklab, var(--omfp-on-surface) 4%, transparent)",
      "color-mix(in oklab, var(--omfp-on-surface) 6%, transparent)",
      "color-mix(in oklab, var(--omfp-on-surface) 12%, transparent)",
    ]);
    const drawing = [...boringRules, ...appBoringRules]
      .filter((rule) => rule.at[0] !== "@layer omfp-tokens")
      .filter((rule) => [...rule.declarations.values()].some((value) => /gradient\(|var\(--omfp-grad/.test(value)));
    expect(drawing.map((rule) => rule.selector)).toEqual([]);
    // What Crush draws as gradient text is plain text.
    const text = findRule(boringRules, ["@layer omfp-kit"], `${SKIN} .omfp-grad-text`).declarations;
    expect([text.get("color"), text.get("background"), text.get("background-clip")]).toEqual([
      "var(--omfp-on-surface)",
      "none",
      "border-box",
    ]);
  });

  // spec: UI-795, UI-149, UI-171
  test("neutral chrome: an inverted primary button and pills, a gray user bubble, the accent kept off fills", () => {
    const rule = (selector: string) => findRule(boringRules, ["@layer omfp-kit"], `${SKIN} ${selector}`).declarations;
    const primary = rule('.omfp-button[data-variant="primary"]');
    expect([primary.get("color"), primary.get("background")]).toEqual([
      "var(--omfp-inverse-on-surface)",
      "var(--omfp-inverse-surface)",
    ]);
    const insert = rule('.omfp-mode-pill[data-mode="insert"]');
    expect([insert.get("color"), insert.get("background")]).toEqual([
      "var(--omfp-inverse-on-surface)",
      "var(--omfp-inverse-surface)",
    ]);
    const bubble = rule('.omfp-msg[data-author="user"] .omfp-msg__body');
    expect([bubble.get("background"), bubble.get("border"), bubble.get("border-radius")]).toEqual([
      "var(--omfp-surface-container-high)",
      "0",
      "calc(var(--omfp-unit) * 3)",
    ]);
    // The accent (primary, tertiary) never fills a background in the kit's boring rules.
    const filled = boringRules
      .filter((rule) => /var\(--omfp-(?:primary|tertiary)\)/.test(rule.declarations.get("background") ?? ""))
      .map((rule) => rule.selector);
    expect(filled).toEqual([`${SKIN} .omfp-voice-meter[data-voice="user"] .omfp-voice-meter__fill`]);
  });

  // spec: UI-795, UI-796, UI-187
  test("every decorative glyph of kit.css has a boring form: plain chevrons, no `╭─`, `▸` or name glyphs, a ring spinner", () => {
    // A glyph of kit.css (the state marks' `◆` and `●` stay: they carry the state) is covered by a boring rule on the
    // same element's `::before`.
    const boringBefore = boringRules
      .filter((rule) => rule.declarations.has("content") || rule.declarations.has("transform"))
      .flatMap((rule) => selectors(rule.selector))
      .filter((selector) => selector.endsWith("::before"));
    const glyphs = kitRules
      .filter((rule) => /"[^"]*[▸▾╭◆][^"]*"/.test(rule.declarations.get("content") ?? ""))
      .flatMap((rule) => selectors(rule.selector))
      .filter((selector) => !selector.includes("state-mark"));
    expect(glyphs.length).toBeGreaterThan(10);
    const crushOnly = glyphs.filter((selector) => {
      const element = selector.match(/\.[\w-]+/g)?.at(-1) ?? selector;
      return !boringBefore.some((boring) => new RegExp(`${element.replace(".", "\\.")}(?![\\w-])`).test(boring));
    });
    expect(crushOnly).toEqual([]);
    const running = findRule(
      boringRules,
      ["@layer omfp-kit"],
      `${SKIN} .omfp-state-mark[data-state="running"]::before, ${SKIN} .omfp-state-mark[data-state="running"][data-spinner="status"]::before`,
    ).declarations;
    expect([running.get("content"), running.get("border-radius"), running.get("animation")]).toEqual([
      '""',
      "50%",
      "omfp-spin 0.8s linear infinite",
    ]);
  });

  // spec: UI-795, UI-146, UI-145
  test("no shimmer, breathe, bounce, flow or entrance: each motion-full effect but the caret is undone, the indeterminate bar still travels", () => {
    const effects = kitRules.flatMap((rule) =>
      (rule.declarations.get("animation") ?? "").startsWith("omfp-blink")
        ? []
        : selectors(rule.selector)
            .filter((selector) => selector.startsWith(FULL) && rule.declarations.has("animation"))
            .map((selector) => selector.replace(FULL, SKIN)),
    );
    expect(effects.length).toBeGreaterThan(10);
    const kept = effects.filter((selector) =>
      boringRules
        .filter((rule) => selectors(rule.selector).includes(selector))
        .every((rule) => {
          const animation = rule.declarations.get("animation");
          return animation === undefined || (animation !== "none" && animation.includes("omfp-flow"));
        }),
    );
    expect(kept).toEqual([]);
    const indeterminate = findRule(
      boringRules,
      ["@layer omfp-kit"],
      `${SKIN} .omfp-progress:not([aria-valuenow]) .omfp-progress__fill`,
    );
    expect(indeterminate.declarations.get("animation")).toBe("omfp-indeterminate 1.4s var(--omfp-ease) infinite");
    // The entrances outside the motion-full block: popovers, dialogs, toasts, sheets, chips, the mode line's pops.
    const ENTRANCE = /^omfp-(?:pop|fade|toast-in|slide-in)\b/;
    const entrances = kitRules
      .filter((rule) => !rule.selector.startsWith(FULL) && ENTRANCE.test(rule.declarations.get("animation") ?? ""))
      .map((rule) => (rule.selector.match(/^\.[\w-]+/) ?? [""])[0]);
    const undone = boringRules
      .filter((rule) => rule.declarations.get("animation") === "none")
      .map((rule) => rule.selector)
      .join(" ");
    expect(entrances.filter((base) => !new RegExp(`${base.replace(".", "\\.")}\\b`).test(undone))).toEqual([]);
  });

  // spec: UI-984, UI-150, UI-172
  test("the focused composer shows one ring: its box's, not the text area's inside it", () => {
    const ring = "0 0 0 3px color-mix(in oklab, var(--omfp-primary) 22%, transparent)";
    const field = findRule(
      boringRules,
      ["@layer omfp-kit"],
      `${SKIN} :is(.omfp-input-box:focus-within, .omfp-field.is-focus .omfp-input-box):not(.omfp-compose__field *)`,
    );
    expect(field.declarations.get("box-shadow")).toBe(ring);
    const leaks = boringRules.filter(
      (rule) => rule.declarations.has("box-shadow") && rule.selector.includes("input-box") && rule !== field,
    );
    expect(leaks.map((rule) => rule.selector)).toEqual([]);
    const compose = findRule(boringRules, ["@layer omfp-kit"], `${SKIN} .omfp-compose:focus-within`);
    expect(Object.fromEntries(compose.declarations)).toEqual({ "box-shadow": ring });
    // Crush is unchanged: its composer text area draws no ring of its own either.
    const crush = parseCss(readFileSync(join(APP_SRC, "widgets/chat/chat.css"), "utf8")).filter((rule) =>
      rule.selector.includes(".omfp-compose__field .omfp-input-box"),
    );
    expect(crush.some((rule) => rule.declarations.has("box-shadow"))).toBe(false);
  });
});
