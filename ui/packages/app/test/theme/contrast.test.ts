/** L1: WCAG contrast (UI-174, UI-135) and the contrast guard's arithmetic (UI-196). */

import { describe, expect, test } from "bun:test";
import {
  AA,
  contrastAdjustments,
  contrastRatio,
  deltaE,
  GUARDED_ROLES,
  lowestRatio,
  NON_TEXT,
  parseCssColor,
  type Rgb,
  readableColor,
  relativeLuminance,
  toHex,
  toOklab,
} from "../../src/theme/contrast.ts";
import { type Scheme, type ThemeName, themeColors } from "./themes.ts";

const hueOf = (rgb: Rgb) => {
  const [, a, b] = toOklab(rgb);
  return (Math.atan2(b, a) * 180) / Math.PI;
};

describe("contrast", () => {
  // spec: UI-174, UI-196
  test("computed colors parse to sRGB channels in every notation browsers serialize to", () => {
    expect(parseCssColor("rgb(0, 104, 118)")).toEqual([0, 104, 118]);
    expect(parseCssColor("rgba(250, 248, 255, 1)")).toEqual([250, 248, 255]);
    expect(parseCssColor("rgb(0 104 118 / 0.5)")).toEqual([0, 104, 118]);
    expect(parseCssColor("#eee8d5")).toEqual([0xee, 0xe8, 0xd5]);
    expect(parseCssColor("color(srgb 1 0.5 0)")).toEqual([255, 128, 0]);
    expect(parseCssColor("oklab(1 0 0)")).toEqual([255, 255, 255]);
    expect(parseCssColor("oklab(0% 0 0)")).toEqual([0, 0, 0]);
    expect(parseCssColor("oklch(0.627955 0.257683 29.2339)")).toEqual([255, 0, 0]);
    expect(parseCssColor("oklch(62.7955% 64.42% 29.2339)")).toEqual([255, 0, 0]);
    expect(parseCssColor("color(display-p3 1 0 0)")).toBeUndefined();
    expect(parseCssColor("var(--omfp-primary)")).toBeUndefined();
  });

  // spec: UI-174
  test("WCAG relative luminance and contrast ratio", () => {
    expect(relativeLuminance([255, 255, 255])).toBeCloseTo(1, 5);
    expect(relativeLuminance([0, 0, 0])).toBe(0);
    expect(contrastRatio([0, 0, 0], [255, 255, 255])).toBeCloseTo(21, 5);
    expect(contrastRatio([255, 255, 255], [0, 0, 0])).toBeCloseTo(21, 5);
    // Synth light: primary on surface.
    expect(contrastRatio([0, 0x68, 0x76], [0xfa, 0xf8, 0xff])).toBeCloseTo(6.15, 1);
  });

  // spec: UI-132
  test("ΔEok is the OKLab distance × 100: zero for one color, 100 from black to white", () => {
    expect(deltaE([12, 34, 56], [12, 34, 56])).toBe(0);
    expect(deltaE([0, 0, 0], [255, 255, 255])).toBeCloseTo(100, 3);
    expect(deltaE([255, 255, 255], [0, 0, 0])).toBeCloseTo(100, 3);
    // Synth light: primary at 12 % over the surface (composited in sRGB) against the surface.
    expect(deltaE([0xdc, 0xe7, 0xef], [0xfa, 0xf8, 0xff])).toBeGreaterThan(3);
  });
});

describe("readable color", () => {
  const surface: Rgb = [0xfd, 0xf6, 0xe3];
  const onSurface: Rgb = [0x00, 0x2b, 0x36];

  // spec: UI-196
  test("a color that already passes is left alone", () => {
    expect(readableColor([0x2c, 0x4a, 0x56], onSurface, [surface], AA)).toBeUndefined();
  });

  // spec: UI-196
  test("a failing color moves in lightness toward on-surface just far enough, keeping its hue", () => {
    const tertiary: Rgb = [0xee, 0xe8, 0xd5];
    const adjusted = readableColor(tertiary, onSurface, [surface], AA);
    expect(adjusted).toBeDefined();
    const color = adjusted ?? tertiary;
    const ratio = contrastRatio(color, surface);
    expect(ratio).toBeGreaterThanOrEqual(AA);
    // Just enough: not more than a few hundredths above the threshold.
    expect(ratio).toBeLessThan(AA + 0.1);
    // Darker than the theme's color, with its hue.
    expect(toOklab(color)[0]).toBeLessThan(toOklab(tertiary)[0]);
    expect(Math.abs(hueOf(color) - hueOf(tertiary))).toBeLessThan(3);
  });

  // spec: UI-196
  test("on a dark surface it lightens, and it must pass on every background", () => {
    const backgrounds: Rgb[] = [
      [0x12, 0x26, 0x2b],
      [0x0d, 0x1b, 0x1f],
      [0x1f, 0x3e, 0x45],
    ];
    const tertiary: Rgb = [0x33, 0x64, 0x6e];
    const adjusted = readableColor(tertiary, [0xeb, 0xe6, 0xc2], backgrounds, AA) ?? tertiary;
    expect(lowestRatio(adjusted, backgrounds)).toBeGreaterThanOrEqual(AA);
    expect(contrastRatio(adjusted, [0x1f, 0x3e, 0x45])).toBeLessThan(AA + 0.1);
    expect(toOklab(adjusted)[0]).toBeGreaterThan(toOklab(tertiary)[0]);
    expect(Math.abs(hueOf(adjusted) - hueOf(tertiary))).toBeLessThan(3);
  });

  // spec: UI-196
  test("the non-text threshold is 3:1", () => {
    const outline: Rgb = [0x23, 0x4e, 0x57];
    const adjusted = readableColor(outline, [0xeb, 0xe6, 0xc2], [[0x12, 0x26, 0x2b]], NON_TEXT) ?? outline;
    const ratio = contrastRatio(adjusted, [0x12, 0x26, 0x2b]);
    expect(ratio).toBeGreaterThanOrEqual(NON_TEXT);
    expect(ratio).toBeLessThan(AA);
  });

  // spec: UI-196
  test("when on-surface's lightness is not enough the color mixes into on-surface itself", () => {
    // A saturated red against mid grey: at the lightness of near-black text it still needs on-surface's own color.
    const background: Rgb = [0x76, 0x76, 0x76];
    const adjusted = readableColor([0xff, 0x00, 0x00], [0, 0, 0], [background], AA);
    expect(adjusted).toBeDefined();
    expect(contrastRatio(adjusted ?? [0, 0, 0], background)).toBeGreaterThanOrEqual(AA);
  });
});

describe("contrast adjustments", () => {
  const reader = (theme: ThemeName, scheme: Scheme) => {
    const colors = themeColors(theme, scheme);
    return (token: string) => colors.get(token);
  };

  // spec: UI-196, UI-139, UI-140
  test("Synth and the matugen fixture pass as they are: nothing is overridden", () => {
    for (const theme of ["synth", "matugen-forest"] as const) {
      for (const scheme of ["light", "dark"] as const) {
        expect(contrastAdjustments(reader(theme, scheme))).toEqual([]);
      }
    }
  });

  // spec: UI-196
  test("the Dux Solarized preset gets readable text roles, then readable labels on the fills it changed", () => {
    const light = contrastAdjustments(reader("dux-solarized", "light"));
    // Its light surface containers are all one color: the `NORMAL` pill's fill is set apart from the mode line.
    expect(light.map(({ role }) => role)).toEqual([
      "tertiary",
      "secondary",
      "error",
      "on-primary",
      "on-tertiary",
      "surface-container-highest",
    ]);
    const dark = contrastAdjustments(reader("dux-solarized", "dark"));
    expect(dark.map(({ role }) => role)).toEqual(["tertiary", "secondary", "error", "outline", "on-error"]);
    for (const scheme of ["light", "dark"] as const) {
      const colors = themeColors("dux-solarized", scheme);
      const adjustments = scheme === "light" ? light : dark;
      const rendered = (token: string): Rgb =>
        parseCssColor(adjustments.find(({ role }) => role === token)?.to ?? "") ?? colors.get(token) ?? [0, 0, 0];
      for (const adjustment of adjustments) {
        const backgrounds = (GUARDED_ROLES.find(({ role }) => role === adjustment.role)?.on ?? []).map(rendered);
        expect(adjustment.from).toBe(toHex(colors.get(adjustment.role) ?? [0, 0, 0]));
        expect(adjustment.before).toBeLessThan(adjustment.threshold);
        expect(adjustment.after).toBeGreaterThanOrEqual(adjustment.threshold);
        expect(lowestRatio(rendered(adjustment.role), backgrounds)).toBe(adjustment.after);
      }
    }
    // Labels move away from their fills: light text gets lighter, dark text darker.
    const onPrimary = light.find(({ role }) => role === "on-primary");
    expect(toOklab(parseCssColor(onPrimary?.to ?? "") ?? [0, 0, 0])[0]).toBeGreaterThan(
      toOklab(parseCssColor(onPrimary?.from ?? "") ?? [0, 0, 0])[0],
    );
    const onError = dark.find(({ role }) => role === "on-error");
    expect(toOklab(parseCssColor(onError?.to ?? "") ?? [0, 0, 0])[0]).toBeLessThan(
      toOklab(parseCssColor(onError?.from ?? "") ?? [0, 0, 0])[0],
    );
  });

  // spec: UI-196
  test("roles the page cannot resolve are skipped, and nothing is guarded without on-surface", () => {
    const colors = themeColors("dux-solarized", "light");
    expect(
      contrastAdjustments((token) => (token === "tertiary" ? undefined : colors.get(token))).map(({ role }) => role),
    ).toEqual(["secondary", "error", "surface-container-highest"]);
    expect(contrastAdjustments((token) => (token === "on-surface" ? undefined : colors.get(token)))).toEqual([]);
  });
});
