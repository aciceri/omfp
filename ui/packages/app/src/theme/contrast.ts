/**
 * WCAG 2 contrast and the contrast guard's arithmetic (UI-196, UI-174, UI-135). Pure: colors are sRGB channels
 * (0–255); the guard (guard.ts) reads them from the page and writes the overrides.
 *
 * The guard keeps the roles omfp draws as text or marks readable on the surfaces they sit on: a role under its
 * threshold on any of those backgrounds is moved in OKLab lightness toward `on-surface`, just far enough, keeping its
 * hue and as much chroma as sRGB allows. A role that passes is left alone.
 */

export type Rgb = readonly [red: number, green: number, blue: number];

/** One measured pair: a text color on a background. */
export interface ContrastPair {
  readonly text: string;
  readonly background: string;
  readonly ratio: number;
}

/** AA for normal text. */
export const AA = 4.5;
/** AA for non-text marks and borders (WCAG 1.4.11). */
export const NON_TEXT = 3;

/**
 * The backgrounds text roles sit on (UI-130): `surface` (app), `surface-container-lowest` (left column, mode line),
 * `surface-container-low` (code, light popovers) and `surface-container` (key chips, dark popovers).
 */
const GUARD_BACKGROUNDS: readonly string[] = [
  "surface",
  "surface-container-lowest",
  "surface-container-low",
  "surface-container",
];

/** The role a failing text role moves toward. */
const GUARD_TARGET = "on-surface";

/** A role the guard keeps readable, by token name without `--omfp-`. */
export interface GuardedRole {
  readonly role: string;
  /** The ratio it must reach on every background. */
  readonly threshold: number;
  /** The tokens it is drawn on. */
  readonly on: readonly string[];
  /**
   * Where it moves: toward `on-surface` (text roles on the surfaces), or `away` from its fills (white when it is lighter
   * than them, else black: the `on-` roles of filled controls).
   */
  readonly toward: "on-surface" | "away";
}

/** The least a fill must stand out from the surface it sits on to read as a shape (Synth's lowest is 1.29:1). */
const FILL = 1.1;

/**
 * Every role the kit and the shell use as a text (or mark) color (UI-134, UI-137, UI-138), on the surfaces: normal
 * text at AA; `outline` draws the idle mark, hover borders and syntax punctuation, so it needs the non-text 3:1 (Synth's
 * own light outline is 4.26:1). `on-surface` is the target, so it is not guarded. Then the labels of filled controls
 * on their fills, measured after the fills were guarded: `on-primary` on the primary button and the `INSERT` pill
 * (primary → tertiary), `on-tertiary` on the `COMMAND` pill (the reversed gradient), `on-error` on the danger button,
 * `on-secondary-container` on counts. Last the `NORMAL` pill's fill, `surface-container-highest`, on the mode line's
 * `surface-container-lowest`: a theme that makes them one color hides the pill.
 */
export const GUARDED_ROLES: readonly GuardedRole[] = [
  ...["primary", "tertiary", "secondary", "error", "on-surface-variant", "ok", "warn"].map(
    (role): GuardedRole => ({ role, threshold: AA, on: GUARD_BACKGROUNDS, toward: "on-surface" }),
  ),
  { role: "outline", threshold: NON_TEXT, on: GUARD_BACKGROUNDS, toward: "on-surface" },
  { role: "on-primary", threshold: AA, on: ["primary", "tertiary"], toward: "away" },
  { role: "on-tertiary", threshold: AA, on: ["tertiary", "primary"], toward: "away" },
  { role: "on-error", threshold: AA, on: ["error"], toward: "away" },
  { role: "on-secondary-container", threshold: AA, on: ["secondary-container"], toward: "away" },
  { role: "surface-container-highest", threshold: FILL, on: ["surface-container-lowest"], toward: "on-surface" },
];

const WHITE: Rgb = [255, 255, 255];
const BLACK: Rgb = [0, 0, 0];

/** One role the guard changed. */
export interface Adjustment {
  readonly role: string;
  readonly threshold: number;
  /** The theme's color. */
  readonly from: string;
  /** The override, `#rrggbb`. */
  readonly to: string;
  /** The lowest ratio over its backgrounds, before and after. */
  readonly before: number;
  readonly after: number;
}

const NUMBER = String.raw`([+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?)(%?)`;
const SEPARATOR = String.raw`\s*[,\s]\s*`;
const FUNCTION = new RegExp(
  String.raw`^(rgba?|oklab|oklch|color)\(\s*(?:(srgb)\s+)?${NUMBER}${SEPARATOR}${NUMBER}${SEPARATOR}${NUMBER}`,
);

/**
 * The sRGB channels of a computed color in the notations browsers serialize to (`#rrggbb`, `rgb()`/`rgba()`,
 * `color(srgb …)`, `oklab()`, `oklch()`); alpha is ignored. `undefined` for anything else (e.g. `color(display-p3 …)`).
 */
export function parseCssColor(color: string): Rgb | undefined {
  const text = color.trim().toLowerCase();
  const hex = /^#([0-9a-f]{6})$/.exec(text);
  if (hex !== null) {
    const value = Number.parseInt(hex[1] ?? "0", 16);
    return [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff];
  }
  const match = FUNCTION.exec(text);
  if (match === null) {
    return undefined;
  }
  const [, name, space] = match;
  const value = (index: number, percentScale: number) => {
    const number = Number(match[index]);
    return match[index + 1] === "%" ? (number / 100) * percentScale : number;
  };
  if (name === "rgb" || name === "rgba") {
    return clampRgb([value(3, 255), value(5, 255), value(7, 255)]);
  }
  if (name === "color") {
    return space === "srgb" ? clampRgb([value(3, 1) * 255, value(5, 1) * 255, value(7, 1) * 255]) : undefined;
  }
  if (name === "oklab") {
    return fromOklab([value(3, 1), value(5, 0.4), value(7, 0.4)]);
  }
  const chroma = value(5, 0.4);
  const hue = (value(7, 1) * Math.PI) / 180;
  return fromOklab([value(3, 1), chroma * Math.cos(hue), chroma * Math.sin(hue)]);
}

function clampRgb(rgb: readonly number[]): Rgb {
  const channel = (index: number) => Math.min(255, Math.max(0, Math.round(rgb[index] ?? 0)));
  return [channel(0), channel(1), channel(2)];
}

/** `#rrggbb`. */
export function toHex(rgb: Rgb): string {
  return `#${clampRgb(rgb)
    .map((channel) => channel.toString(16).padStart(2, "0"))
    .join("")}`;
}

/** WCAG relative luminance of an sRGB color (channels 0–255). */
export function relativeLuminance([red, green, blue]: Rgb): number {
  return 0.2126 * toLinear(red) + 0.7152 * toLinear(green) + 0.0722 * toLinear(blue);
}

/** The contrast ratio of two colors, 1 to 21, whichever is lighter. */
export function contrastRatio(a: Rgb, b: Rgb): number {
  const [high, low] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
  return ((high ?? 0) + 0.05) / ((low ?? 0) + 0.05);
}

/**
 * How far apart two colors look: the OKLab distance × 100 (ΔEok; about 2 is the least step people notice). For
 * backgrounds too close in luminance for a contrast ratio to say anything, such as a tint and its surface (UI-132).
 */
export function deltaE(a: Rgb, b: Rgb): number {
  const [x, y] = [toOklab(a), toOklab(b)];
  return Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]) * 100;
}

type Lab = readonly [lightness: number, a: number, b: number];

function toLinear(channel: number): number {
  const c = channel / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function fromLinear(c: number): number {
  return (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055) * 255;
}

/** OKLab of an sRGB color (Björn Ottosson's matrices). */
export function toOklab(rgb: Rgb): Lab {
  const [r, g, b] = [toLinear(rgb[0]), toLinear(rgb[1]), toLinear(rgb[2])];
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

/** Linear sRGB of an OKLab color, unclamped. */
function linearOf([lightness, a, b]: Lab): readonly [number, number, number] {
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

const GAMUT_EPSILON = 1e-6;

function inGamut(linear: readonly number[]): boolean {
  return linear.every((c) => c >= -GAMUT_EPSILON && c <= 1 + GAMUT_EPSILON);
}

/** sRGB of an OKLab color; out of gamut, chroma is reduced (lightness and hue kept) until it fits. */
export function fromOklab(lab: Lab): Rgb {
  const lightness = Math.min(1, Math.max(0, lab[0]));
  let linear = linearOf([lightness, lab[1], lab[2]]);
  if (!inGamut(linear)) {
    let low = 0;
    let high = 1;
    for (let step = 0; step < 24; step += 1) {
      const scale = (low + high) / 2;
      if (inGamut(linearOf([lightness, lab[1] * scale, lab[2] * scale]))) {
        low = scale;
      } else {
        high = scale;
      }
    }
    linear = linearOf([lightness, lab[1] * low, lab[2] * low]);
  }
  return clampRgb(linear.map((c) => fromLinear(Math.min(1, Math.max(0, c)))));
}

/** The lowest contrast of `color` over `backgrounds`. */
export function lowestRatio(color: Rgb, backgrounds: readonly Rgb[]): number {
  return Math.min(...backgrounds.map((background) => contrastRatio(color, background)));
}

const SEARCH_STEPS = 24;

/** The smallest `t` in (0, 1] for which `at(t)` passes, given that `at(1)` does; `at(0)` is known to fail. */
function smallestPassing(at: (t: number) => Rgb, passes: (color: Rgb) => boolean): Rgb {
  let low = 0;
  let high = 1;
  let best = at(1);
  for (let step = 0; step < SEARCH_STEPS; step += 1) {
    const middle = (low + high) / 2;
    const candidate = at(middle);
    if (passes(candidate)) {
      high = middle;
      best = candidate;
    } else {
      low = middle;
    }
  }
  return best;
}

/**
 * `color` moved toward `toward` just far enough to reach `threshold` on every background, or `undefined` when it
 * already does. First only the OKLab lightness moves (hue and chroma kept, chroma reduced only where sRGB requires);
 * if even `toward`'s lightness is not enough, the color is then mixed in OKLab into `toward` itself.
 */
export function readableColor(
  color: Rgb,
  toward: Rgb,
  backgrounds: readonly Rgb[],
  threshold: number,
): Rgb | undefined {
  const passes = (candidate: Rgb) => lowestRatio(candidate, backgrounds) >= threshold;
  if (backgrounds.length === 0 || passes(color)) {
    return undefined;
  }
  const from = toOklab(color);
  const target = toOklab(toward);
  const lightness = (t: number) => fromOklab([from[0] + t * (target[0] - from[0]), from[1], from[2]]);
  if (passes(lightness(1))) {
    return smallestPassing(lightness, passes);
  }
  const end = toOklab(lightness(1));
  const mixed = (t: number) =>
    fromOklab([
      end[0] + t * (target[0] - end[0]),
      end[1] + t * (target[1] - end[1]),
      end[2] + t * (target[2] - end[2]),
    ]);
  return passes(toward) ? smallestPassing(mixed, passes) : clampRgb(toward);
}

/**
 * The overrides the theme needs, reading each token's color through `read` (`undefined`: unknown, the role is
 * skipped). Roles that pass are not listed. Later roles are measured on the earlier ones' overrides.
 */
export function contrastAdjustments(read: (token: string) => Rgb | undefined): Adjustment[] {
  const target = read(GUARD_TARGET);
  if (target === undefined) {
    return [];
  }
  const overrides = new Map<string, Rgb>();
  const resolve = (token: string) => overrides.get(token) ?? read(token);
  return GUARDED_ROLES.flatMap(({ role, threshold, on, toward }) => {
    const color = read(role);
    const backgrounds = on.flatMap((token) => {
      const background = resolve(token);
      return background === undefined ? [] : [background];
    });
    if (color === undefined || backgrounds.length === 0) {
      return [];
    }
    const fills = backgrounds.reduce((sum, background) => sum + relativeLuminance(background), 0) / backgrounds.length;
    const lighter = relativeLuminance(color) >= fills;
    const destination = toward === "on-surface" ? target : lighter ? WHITE : BLACK;
    const adjusted = readableColor(color, destination, backgrounds, threshold);
    if (adjusted === undefined) {
      return [];
    }
    overrides.set(role, adjusted);
    return [
      {
        role,
        threshold,
        from: toHex(color),
        to: toHex(adjusted),
        before: lowestRatio(color, backgrounds),
        after: lowestRatio(adjusted, backgrounds),
      },
    ];
  });
}
