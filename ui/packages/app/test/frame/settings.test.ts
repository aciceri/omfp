/** L1: the daemon's keymap entries over the defaults and the `:set` line (UI-097). */

import { describe, expect, test } from "bun:test";
import { parseSet, parseTheme, SET_KEYS, themeArgs } from "../../src/frame/set.ts";
import { type Binding, DEFAULT_BINDINGS, keysFor, mergeBindings } from "../../src/keymap/keymap.ts";

describe("keymap overrides", () => {
  // spec: UI-097, UI-095
  test("entries replace, remove or add bindings one by one; the rest of the defaults stay", () => {
    const defaults: Binding[] = [
      { mode: "normal", keys: "j", command: "nav.next" },
      { mode: "normal", keys: "k", command: "nav.prev" },
      { mode: "insert", keys: "Esc", command: "focus.leave" },
    ];
    expect(
      mergeBindings(defaults, [
        { mode: "normal", keys: "j", command: "nav.prev" },
        { mode: "normal", keys: "k", command: "" },
        { mode: "normal", keys: "  Space   t ", command: "window.theme" },
        { mode: "insert", keys: "Ctrl-s", command: "composer.send" },
        { mode: "normal", keys: "Space t", command: "density.set" },
        { mode: "cmdline", keys: "nope", command: "" },
      ]),
    ).toEqual([
      { mode: "normal", keys: "j", command: "nav.prev" },
      { mode: "insert", keys: "Esc", command: "focus.leave" },
      { mode: "normal", keys: "Space t", command: "density.set" },
      { mode: "insert", keys: "Ctrl-s", command: "composer.send" },
    ]);
    expect(mergeBindings(DEFAULT_BINDINGS, [])).toEqual([...DEFAULT_BINDINGS]);
  });

  // spec: UI-097
  test("an override of a default key in another mode leaves the default alone", () => {
    const merged = mergeBindings(DEFAULT_BINDINGS, [{ mode: "insert", keys: "j", command: "nav.next" }]);
    expect(keysFor(merged, "nav.next")).toEqual(["j"]);
    expect(keysFor(merged, "nav.next", "insert")).toEqual(["j"]);
  });
});

describe(":set", () => {
  // spec: UI-097, UI-145
  test("ui keys become settings.set params", () => {
    expect(parseSet("density=normal")).toEqual({ ok: true, patch: { ui: { density: "normal" } } });
    expect(parseSet("font_size=18")).toEqual({ ok: true, patch: { ui: { fontSize: 18 } } });
    expect(parseSet("motion=reduced")).toEqual({ ok: true, patch: { ui: { motion: "reduced" } } });
    expect(parseSet("motion = none")).toEqual({ ok: true, patch: { ui: { motion: "none" } } });
    expect(parseSet(" notifications = never ")).toEqual({ ok: true, patch: { ui: { notifications: "never" } } });
    expect(parseSet("fonts.prose=Iowan Old Style, serif")).toEqual({
      ok: true,
      patch: { ui: { fonts: { prose: "Iowan Old Style, serif" } } },
    });
    expect(parseSet("fonts.code='JetBrains Mono'")).toEqual({
      ok: true,
      patch: { ui: { fonts: { code: "JetBrains Mono" } } },
    });
  });

  // spec: UI-513, UI-510
  test("`theme=` takes a built-in theme's name, completes every one and refuses others", () => {
    expect(parseSet("theme=rose-pine")).toEqual({ ok: true, patch: { ui: { theme: "rose-pine" } } });
    expect(parseSet("theme = synth")).toEqual({ ok: true, patch: { ui: { theme: "synth" } } });
    expect(parseSet("theme=matugen")).toEqual({ ok: true, patch: { ui: { theme: "matugen" } } });
    expect(parseSet("theme=dracula")).toEqual({ ok: false, error: "Invalid value for theme: dracula" });
    expect(SET_KEYS.filter((key) => key.startsWith("theme="))).toEqual([
      "theme=synth",
      "theme=catppuccin",
      "theme=gruvbox",
      "theme=tokyo-night",
      "theme=nord",
      "theme=solarized",
      "theme=rose-pine",
      "theme=accent",
      "theme=matugen",
    ]);
  });

  // spec: UI-513
  test("`:theme` takes a scheme, a theme or both, in any order, and completes the kind not given yet", () => {
    expect(parseTheme(["dark"])).toEqual({ ok: true, scheme: "dark" });
    expect(parseTheme(["nord"])).toEqual({ ok: true, theme: "nord" });
    expect(parseTheme(["tokyo-night", "light"])).toEqual({ ok: true, scheme: "light", theme: "tokyo-night" });
    expect(parseTheme(["system", "accent"])).toEqual({ ok: true, scheme: "system", theme: "accent" });
    expect(parseTheme(["matugen", "dark"])).toEqual({ ok: true, scheme: "dark", theme: "matugen" });
    expect(parseTheme(["dracula"])).toEqual({ ok: false, error: "Unknown colour scheme or theme: dracula" });
    expect(parseTheme([])).toEqual({ ok: false, error: expect.stringContaining("Usage: :theme") });
    expect(parseTheme(["dark", "light"])).toEqual({ ok: false, error: expect.stringContaining("Usage: :theme") });
    expect(parseTheme(["nord", "gruvbox"])).toEqual({ ok: false, error: expect.stringContaining("Usage: :theme") });
    expect(themeArgs([])).toEqual([
      "system",
      "light",
      "dark",
      "synth",
      "catppuccin",
      "gruvbox",
      "tokyo-night",
      "nord",
      "solarized",
      "rose-pine",
      "accent",
      "matugen",
    ]);
    expect(themeArgs(["catppuccin"])).toEqual(["system", "light", "dark"]);
    expect(themeArgs(["dark"])).toEqual([
      "synth",
      "catppuccin",
      "gruvbox",
      "tokyo-night",
      "nord",
      "solarized",
      "rose-pine",
      "accent",
      "matugen",
    ]);
    expect(themeArgs(["dark", "nord"])).toEqual([]);
  });

  // spec: UI-097
  test("keymap keys bind in normal mode by default, or in the named mode; an empty value unbinds", () => {
    expect(parseSet("keymap.Space t=window.theme")).toEqual({
      ok: true,
      patch: { keymap: [{ mode: "normal", keys: "Space t", command: "window.theme" }] },
    });
    expect(parseSet('keymap.insert.Ctrl-s=""')).toEqual({
      ok: true,
      patch: { keymap: [{ mode: "insert", keys: "Ctrl-s", command: "" }] },
    });
    expect(parseSet("keymap.g x=")).toEqual({
      ok: true,
      patch: { keymap: [{ mode: "normal", keys: "g x", command: "" }] },
    });
  });

  // spec: UI-097, UI-145
  test("unknown keys and invalid values are refused", () => {
    expect(parseSet("density")).toEqual({ ok: false, error: expect.stringContaining("Usage: :set") });
    expect(parseSet("density=huge")).toEqual({ ok: false, error: "Invalid value for density: huge" });
    expect(parseSet("motion=slow")).toEqual({ ok: false, error: "Invalid value for motion: slow" });
    expect(parseSet("fonts.title=x")).toEqual({ ok: false, error: "Invalid value for fonts.title: x" });
    expect(parseSet("fonts.ui=")).toEqual({ ok: false, error: "Invalid value for fonts.ui: (empty)" });
    expect(parseSet("font_size=9")).toEqual({ ok: false, error: "Invalid value for font_size: 9" });
    expect(parseSet("font_size=29")).toEqual({ ok: false, error: "Invalid value for font_size: 29" });
    expect(parseSet("font_size=16.5")).toEqual({ ok: false, error: "Invalid value for font_size: 16.5" });
    expect(parseSet("colour=red")).toEqual({ ok: false, error: "Unknown setting: colour" });
    expect(parseSet("keymap.=nav.next")).toEqual({ ok: false, error: expect.stringContaining("Usage: :set") });
  });
});
