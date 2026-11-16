/** The token names of the kit contract (without the `--omfp-` prefix): M3 roles (UI-091) and Crush (09-design). */

const ROLE_GROUPS = ["primary", "secondary", "tertiary", "error"] as const;

/** The 34 Material 3 roles of the Synth table (UI-139), in the table's order. */
export const ROLE_TOKENS: readonly string[] = [
  ...ROLE_GROUPS.flatMap((role) => [role, `on-${role}`, `${role}-container`, `on-${role}-container`]),
  "surface",
  "on-surface",
  "surface-variant",
  "on-surface-variant",
  "surface-container-lowest",
  "surface-container-low",
  "surface-container",
  "surface-container-high",
  "surface-container-highest",
  "surface-dim",
  "surface-bright",
  "inverse-surface",
  "inverse-on-surface",
  "inverse-primary",
  "outline",
  "outline-variant",
  "shadow",
  "scrim",
];

/** omfp's status roles (UI-136): no M3 role, themes MAY override them. */
export const STATUS_TOKENS: readonly string[] = ["success", "on-success", "warning", "on-warning"];

export const COLOR_TOKENS: readonly string[] = [...ROLE_TOKENS, ...STATUS_TOKENS];

export const FONT_TOKENS: readonly string[] = ["font-prose", "font-ui", "font-code"];

/** The type scale (UI-126): rem values, the same in both densities. */
export const TYPE_SCALE: Readonly<Record<string, string>> = {
  "fs-2xs": "0.6875rem",
  "fs-xs": "0.75rem",
  "fs-sm": "0.78125rem",
  "fs-md": "0.8125rem",
  "fs-lg": "0.9375rem",
  "fs-xl": "1.125rem",
  "fs-prose": "1rem",
  "fs-code": "0.78125rem",
  "fw-regular": "400",
  "fw-medium": "600",
  "fw-strong": "700",
  "lh-ui": "1.4",
  "lh-prose": "1.5",
  "lh-code": "1.5",
  "label-case": "none",
  "label-track": "0",
};

/** Spacing steps (UI-127): `unit × n × density`. */
export const SPACE_STEPS: readonly number[] = [1, 2, 3, 4, 5, 6, 8];

/** Text spacing (UI-127): paddings and margins in `em`/`ch` that scale with the text they sit in, not with density. */
export const TEXT_SPACING: Readonly<Record<string, string>> = {
  "kbd-pad": "0.36em 0.4em",
  "pill-pad": "0.1em 0.5em",
  "tag-pad": "0.1em 0.45em",
  "code-pad": "0.08em 0.32em",
  "md-block": "0.7em",
  "md-head": "1em",
  "md-head-after": "0.5em",
  "md-item": "0.2em",
  "md-indent": "1.4em",
  "glyph-gap": "0.1em",
  "mark-gap": "0.5ch",
};

/** Layout sizes (UI-128): those that scale with density, and the frame sizes that do not. */
export const DENSITY_SIZES: Readonly<Record<string, string>> = {
  "row-h": "1.5rem",
  "ctl-h": "1.75rem",
  "ctl-h-sm": "1.4375rem",
  "msg-gap": "1.25rem",
};
export const FRAME_SIZES: Readonly<Record<string, string>> = {
  "side-w": "17rem",
  "panel-w": "22rem",
  "bar-h": "2.125rem",
  "status-h": "1.625rem",
  gutter: "2rem",
  measure: "50rem",
  touch: "44px",
  "areabar-h": "max(var(--omfp-touch), 3rem)",
  "float-w": "24rem",
  "tip-w": "24rem",
  "toast-w": "22rem",
  "key-col": "13em",
};

/** Radii, borders and rules (UI-129), icons (UI-131), focus (UI-132) and interactive states (UI-133). */
export const SHAPE_TOKENS: Readonly<Record<string, string>> = {
  "r-ctl": "0.375rem",
  "r-row": "0.375rem",
  "r-chip": "0.3125rem",
  "r-card": "0.625rem",
  bw: "1px",
  rule: "color-mix(in oklab, var(--omfp-outline-variant) 80%, transparent)",
  "rule-strong": "var(--omfp-outline-variant)",
  ic: "0.875rem",
  "ic-sm": "0.8125rem",
  "ic-stroke": "2",
  "focus-w": "1px",
  "focus-off": "1px",
  "focus-color": "var(--omfp-primary)",
  "focus-bg": "color-mix(in oklab, var(--omfp-primary) 12%, transparent)",
  "focus-bar": "var(--omfp-primary)",
  hover: "color-mix(in oklab, var(--omfp-primary) 8%, transparent)",
  press: "color-mix(in oklab, var(--omfp-primary) 15%, transparent)",
  "sel-bg": "var(--omfp-grad-soft)",
  "sel-fg": "var(--omfp-on-surface)",
  disabled: "0.42",
  "dur-1": "110ms",
  "dur-2": "220ms",
  ease: "cubic-bezier(0.2, 0.7, 0.2, 1)",
};

/** Surfaces (UI-130), in light; dark changes only the popover surface. */
export const SURFACE_TOKENS: Readonly<Record<string, string>> = {
  "app-bg": "var(--omfp-surface)",
  "side-bg": "var(--omfp-surface-container-lowest)",
  "bar-bg": "var(--omfp-surface-container-lowest)",
  "card-bg": "transparent",
  "card-border": "var(--omfp-rule)",
  "field-bg": "transparent",
  "field-border": "var(--omfp-rule-strong)",
  "ctl-bg": "transparent",
  "code-bg": "var(--omfp-surface-container-low)",
  "pop-bg": "var(--omfp-surface-container-low)",
  "pop-border": "color-mix(in oklab, var(--omfp-primary) 45%, var(--omfp-outline-variant))",
  "pop-shadow":
    "0 0 0 1px color-mix(in oklab, var(--omfp-primary) 8%, transparent), 0 0.75rem 2rem color-mix(in oklab, var(--omfp-primary) 14%, transparent)",
  "kbd-bg": "var(--omfp-surface-container)",
};

/** Colors derived from the roles (UI-135, UI-136): no theme needs to define them. */
export const DERIVED_TOKENS: Readonly<Record<string, string>> = {
  "grad-end": "color-mix(in oklab, var(--omfp-tertiary) 68%, var(--omfp-on-surface))",
  grad: "linear-gradient(90deg, var(--omfp-primary), var(--omfp-tertiary) 62%, var(--omfp-grad-end))",
  "grad-loop":
    "linear-gradient(90deg, var(--omfp-primary), var(--omfp-tertiary) 30%, var(--omfp-grad-end) 50%, var(--omfp-tertiary) 70%, var(--omfp-primary))",
  "grad-soft":
    "linear-gradient(90deg, color-mix(in oklab, var(--omfp-primary) 16%, transparent), color-mix(in oklab, var(--omfp-tertiary) 10%, transparent))",
  ok: "color-mix(in oklab, var(--omfp-success) 85%, var(--omfp-surface))",
  warn: "color-mix(in oklab, var(--omfp-warning) 85%, var(--omfp-surface))",
};

/** The Synth palette (UI-139) and omfp's status values: [light, dark]. */
export const SYNTH: Readonly<Record<string, readonly [light: string, dark: string]>> = {
  primary: ["#006876", "#44d8f0"],
  "on-primary": ["#ffffff", "#00363e"],
  "primary-container": ["#9fefff", "#004e59"],
  "on-primary-container": ["#004e59", "#9fefff"],
  secondary: ["#4d616c", "#b4cad6"],
  "on-secondary": ["#ffffff", "#1f333d"],
  "secondary-container": ["#d0e6f3", "#364954"],
  "on-secondary-container": ["#364954", "#d0e6f3"],
  tertiary: ["#584aca", "#c6c0ff"],
  "on-tertiary": ["#ffffff", "#28069c"],
  "tertiary-container": ["#e4dfff", "#402eb2"],
  "on-tertiary-container": ["#402eb2", "#e4dfff"],
  error: ["#ba1a1a", "#ffb4ab"],
  "on-error": ["#ffffff", "#690005"],
  "error-container": ["#ffdad6", "#93000a"],
  "on-error-container": ["#93000a", "#ffdad6"],
  surface: ["#faf8ff", "#12131a"],
  "on-surface": ["#1a1b23", "#e2e1ec"],
  "surface-variant": ["#dce2f2", "#404753"],
  "on-surface-variant": ["#404753", "#c0c7d5"],
  "surface-container-lowest": ["#ffffff", "#0c0e15"],
  "surface-container-low": ["#f3f2fd", "#1a1b23"],
  "surface-container": ["#eeedf8", "#1e1f27"],
  "surface-container-high": ["#e8e7f2", "#282a31"],
  "surface-container-highest": ["#e2e1ec", "#33343c"],
  "surface-dim": ["#dad9e4", "#12131a"],
  "surface-bright": ["#faf8ff", "#373941"],
  "inverse-surface": ["#2f3038", "#e2e1ec"],
  "inverse-on-surface": ["#f1f0fa", "#2f3038"],
  "inverse-primary": ["#44d8f0", "#006876"],
  outline: ["#717785", "#8a919f"],
  "outline-variant": ["#c0c7d5", "#404753"],
  shadow: ["#000000", "#000000"],
  scrim: ["#000000", "#000000"],
  success: ["#225f23", "#8fd88a"],
  "on-success": ["#ffffff", "#00390a"],
  warning: ["#7e4600", "#ffb868"],
  "on-warning": ["#ffffff", "#4a2800"],
};

/** The bundled font files (UI-141): family, file stem, weight, style. */
export const FONT_FILES: readonly (readonly [family: string, stem: string, weight: string, style: string])[] = [
  "IBM Plex Serif",
  "Commit Mono",
].flatMap((family) => {
  const stem = family === "IBM Plex Serif" ? "ibm-plex-serif" : "commit-mono";
  return [
    [family, `${stem}-latin-400-normal`, "400", "normal"] as const,
    [family, `${stem}-latin-400-italic`, "400", "italic"] as const,
    [family, `${stem}-latin-600-normal`, "600", "normal"] as const,
    [family, `${stem}-latin-700-normal`, "700", "normal"] as const,
  ];
});

/** Text color over background color pairs that must reach WCAG AA (4.5:1) in every scheme. */
export const TEXT_PAIRS: readonly (readonly [text: string, background: string])[] = [
  ...ROLE_GROUPS.flatMap((role) => [
    [`on-${role}`, role] as const,
    [`on-${role}-container`, `${role}-container`] as const,
  ]),
  ["on-success", "success"],
  ["on-warning", "warning"],
  ["on-surface", "surface"],
  ["on-surface", "surface-container-lowest"],
  ["on-surface", "surface-container-low"],
  ["on-surface", "surface-container"],
  ["on-surface", "surface-container-high"],
  ["on-surface", "surface-container-highest"],
  ["on-surface-variant", "surface"],
  ["on-surface-variant", "surface-variant"],
  ["on-surface-variant", "surface-container-highest"],
  ["inverse-on-surface", "inverse-surface"],
  ["primary", "surface"],
  ["tertiary", "surface"],
  ["error", "surface"],
  ["success", "surface"],
  ["warning", "surface"],
];

/** The conversation search's highlights (UI-736): role pairs Material keeps readable (both in {@link TEXT_PAIRS}), so
 * every theme follows; the veils mark occurrences where the browser has no custom highlights. */
export const SEARCH_TOKENS: Readonly<Record<string, string>> = {
  "search-bg": "var(--omfp-tertiary-container)",
  "search-fg": "var(--omfp-on-tertiary-container)",
  "search-current-bg": "var(--omfp-primary)",
  "search-current-fg": "var(--omfp-on-primary)",
  "search-veil": "color-mix(in oklab, var(--omfp-tertiary) 30%, transparent)",
  "search-current-veil": "color-mix(in oklab, var(--omfp-primary) 45%, transparent)",
};

/** The kit's earlier ad-hoc tokens, replaced by the ones above with no aliases (09-design, Tokens). */
export const RETIRED_TOKEN = /--omfp-(?:radius-|font-size-|row-height|line-height|space-)/;

/** The token name as a custom property. */
export function prop(token: string): string {
  return `--omfp-${token}`;
}
