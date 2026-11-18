/** The gallery's design sheet (UI-173): the density strip, the logo, the glyphs and the token sheet with measured
 * contrast and the contrast guard's overrides. */
import { Button, cssColorReader, Icon, RowList, Select, TextField, ThreadRow } from "@omfp/kit";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { AA, type Adjustment, type ContrastPair, contrastRatio, parseCssColor } from "../theme/contrast.ts";
import { type FocusMeasure, FocusTable, measureFocus } from "./focus.tsx";
import { type KeyHintMeasure, KeyHints, measureKeyHints } from "./key-hints.tsx";
import { LogoSheet } from "./logo.tsx";
import { Label, LEVELS, Line, Section } from "./parts.tsx";

export function DensityStrip() {
  return (
    <Section title="Density" note="UI-127 · UI-128 · compact and normal" wide>
      {(["compact", "normal"] as const).map((density) => (
        <div key={density} className="omfp-gallery__density" data-density={density}>
          <Label>{density}</Label>
          <Button variant="primary">Send</Button>
          <Button size="sm">Small</Button>
          <TextField label="Filter" hideLabel placeholder="Filter" icon={<Icon name="search" />} />
          <Select label="Level" hideLabel items={LEVELS} selectedKey="high" onSelectionChange={() => undefined} />
          <RowList className="omfp-gallery__row">
            <ThreadRow state="running" title="A thread row" meta="now" onSelect={() => undefined} />
          </RowList>
        </div>
      ))}
    </Section>
  );
}

const GLYPHS: readonly (readonly [glyph: string, meaning: string, className: string])[] = [
  ["◆", "identity, attention, awaiting", "omfp-gallery__glyph--warn"],
  ["▸ ▾", "current choice, disclosure", "omfp-gallery__glyph--primary"],
  ["●", "finished tool call, unread", "omfp-gallery__glyph--ok"],
  ["○", "idle thread", "omfp-gallery__glyph--outline"],
  ["⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏", "running thread, header (activity spinner)", "omfp-gallery__glyph--primary"],
  ["⣾⣽⣻⢿⡿⣟⣯⣷", "running tool (status spinner)", "omfp-gallery__glyph--primary"],
  ["╭─", "titled group", "omfp-gallery__glyph--primary"],
  ["·", "mode-line separator", "omfp-gallery__glyph--rule"],
  ["@", "daemon host", "omfp-gallery__glyph--muted"],
];

export function Logo() {
  return (
    <Section title="Logo" note="UI-220 · UI-221 · 48/32/22/16 px" wide>
      <LogoSheet />
    </Section>
  );
}

export function Glyphs() {
  return (
    <Section title="Glyphs" note="UI-122">
      <dl className="omfp-gallery__glyphs">
        {GLYPHS.map(([glyph, meaning, className]) => (
          <div key={glyph} className="omfp-gallery__glyph-row">
            <dt className={className}>{glyph}</dt>
            <dd>{meaning}</dd>
          </div>
        ))}
      </dl>
    </Section>
  );
}

const TYPE_SCALE = ["2xs", "xs", "sm", "md", "lg", "xl", "code"] as const;
const SPACES = ["s1", "s2", "s3", "s4", "s5", "s6", "s8"] as const;
const RADII = ["r-chip", "r-ctl", "r-row", "r-card"] as const;

const ROLES = [
  "primary",
  "on-primary",
  "primary-container",
  "on-primary-container",
  "secondary",
  "on-secondary",
  "secondary-container",
  "on-secondary-container",
  "tertiary",
  "on-tertiary",
  "tertiary-container",
  "on-tertiary-container",
  "error",
  "on-error",
  "error-container",
  "on-error-container",
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
  "success",
  "on-success",
  "warning",
  "on-warning",
] as const;

const DERIVED = [
  "grad",
  "grad-loop",
  "grad-soft",
  "grad-end",
  "ok",
  "warn",
  "rule",
  "rule-strong",
  "hover",
  "press",
  "pop-bg",
  "pop-border",
  "code-bg",
  "side-bg",
  "kbd-bg",
  "search-bg",
  "search-current-bg",
] as const;

/**
 * Text roles, the derived status colors (`ok`, `warn`, UI-136), gradient stops and the conversation search's
 * highlights (UI-736) on the backgrounds they sit on (UI-174, UI-135): each must reach AA. Names are tokens without
 * `--omfp-`.
 */
const TEXT_PAIRS: readonly (readonly [text: string, background: string])[] = [
  ["on-surface", "surface"],
  ["on-surface", "surface-container-lowest"],
  ["on-surface", "surface-container-low"],
  ["on-surface", "surface-container"],
  ["on-surface", "surface-container-high"],
  ["on-surface", "surface-container-highest"],
  ["on-surface-variant", "surface"],
  ["on-surface-variant", "surface-container-lowest"],
  ["on-surface-variant", "surface-container-low"],
  ["on-surface-variant", "surface-container"],
  ["primary", "surface"],
  ["primary", "surface-container-lowest"],
  ["primary", "surface-container-low"],
  ["tertiary", "surface"],
  ["tertiary", "surface-container"],
  ["secondary", "surface"],
  ["secondary", "surface-container-low"],
  ["error", "surface"],
  ["error", "surface-container-low"],
  ["ok", "surface"],
  ["warn", "surface"],
  ["ok", "surface-container-lowest"],
  ["warn", "surface-container-lowest"],
  ["on-primary", "primary"],
  ["on-primary", "tertiary"],
  ["on-tertiary", "tertiary"],
  ["on-error", "error"],
  ["on-secondary-container", "secondary-container"],
  ["inverse-on-surface", "inverse-surface"],
  // Gradient text (UI-135): every stop of --omfp-grad on the app and side backgrounds.
  ["grad-end", "surface"],
  ["grad-end", "surface-container-lowest"],
  ["tertiary", "surface-container-lowest"],
  // The conversation search's occurrences and its current one (UI-736).
  ["search-fg", "search-bg"],
  ["search-current-fg", "search-current-bg"],
];

function measure(): ContrastPair[] {
  const read = cssColorReader(document, parseCssColor);
  return TEXT_PAIRS.flatMap(([text, background]) => {
    const fore = read(text);
    const back = read(background);
    return fore === undefined || back === undefined ? [] : [{ text, background, ratio: contrastRatio(fore, back) }];
  });
}

/** The roles the contrast guard (UI-196) overrode for this theme: the theme's color, the override, both ratios. */
function GuardTable({ adjustments }: { adjustments: readonly Adjustment[] }) {
  return (
    <Section title="Contrast guard" note="UI-196 · theme → override">
      {adjustments.length === 0 ? (
        <span className="omfp-gallery__note" data-testid="contrast-guard-none">
          Every guarded role passes; nothing is overridden.
        </span>
      ) : (
        <table className="omfp-gallery__contrast omfp-gallery__guard">
          <tbody>
            {adjustments.map((adjustment) => (
              <tr
                key={adjustment.role}
                data-testid="contrast-guard"
                data-role={adjustment.role}
                data-from={adjustment.from}
                data-to={adjustment.to}
              >
                <td>
                  <span className="omfp-gallery__sample" style={{ color: adjustment.from }}>
                    Aa
                  </span>
                  <span className="omfp-gallery__sample" style={{ color: `var(--omfp-${adjustment.role})` }}>
                    Aa
                  </span>
                </td>
                <td>{adjustment.role}</td>
                <td>
                  {adjustment.from} → {adjustment.to}
                </td>
                <td className="omfp-gallery__ratio">
                  {adjustment.before.toFixed(2)} → {adjustment.after.toFixed(2)} / {adjustment.threshold}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Section>
  );
}

export function TokenSheet({ guard, onMeasured }: { guard: readonly Adjustment[]; onMeasured: () => void }) {
  const [pairs, setPairs] = useState<readonly ContrastPair[] | undefined>();
  const [focus, setFocus] = useState<readonly FocusMeasure[]>([]);
  const [keyHints, setKeyHints] = useState<readonly KeyHintMeasure[]>([]);
  const measured = useRef(onMeasured);
  measured.current = onMeasured;
  useLayoutEffect(() => {
    setPairs(measure());
    setFocus(measureFocus());
    setKeyHints(measureKeyHints(document));
  }, []);
  useEffect(() => {
    if (pairs !== undefined) {
      measured.current();
    }
  }, [pairs]);
  return (
    <>
      <Section title="Type scale" note="UI-126 · rem × [ui] font_size">
        {TYPE_SCALE.map((size) => (
          <div key={size} className="omfp-gallery__type" style={{ fontSize: `var(--omfp-fs-${size})` }}>
            <Label>fs-{size}</Label> Crush chrome 0123
          </div>
        ))}
        <div className="omfp-gallery__type">
          <Label>fs-prose</Label>
          <span className="omfp-prose">
            Prose reads in IBM Plex Serif, <em>italic</em> and <strong>bold</strong>.
          </span>
        </div>
        <div className="omfp-gallery__type omfp-gallery__weights">
          <span style={{ fontWeight: "var(--omfp-fw-regular)" }}>400 regular</span>
          <span style={{ fontWeight: "var(--omfp-fw-medium)" }}>600 medium</span>
          <span style={{ fontWeight: "var(--omfp-fw-strong)" }}>700 strong</span>
        </div>
      </Section>
      <Section title="Spacing · radii" note="UI-127 · UI-129">
        {SPACES.map((space) => (
          <div key={space} className="omfp-gallery__space">
            <Label>{space}</Label>
            <span className="omfp-gallery__bar" style={{ inlineSize: `var(--omfp-${space})` }} />
          </div>
        ))}
        <Line>
          {RADII.map((radius) => (
            <span key={radius} className="omfp-gallery__radius" style={{ borderRadius: `var(--omfp-${radius})` }}>
              {radius}
            </span>
          ))}
          <span className="omfp-gallery__radius" style={{ borderRadius: "999px" }}>
            pill
          </span>
        </Line>
      </Section>
      <Section title="Colors" note="UI-134 · UI-135 · UI-136 · UI-139" wide>
        <div className="omfp-gallery__swatches">
          {[...ROLES, ...DERIVED].map((token) => (
            <div key={token} className="omfp-gallery__swatch">
              <span className="omfp-gallery__chip" style={{ background: `var(--omfp-${token})` }} />
              <span>{token}</span>
            </div>
          ))}
        </div>
      </Section>
      <Section title="Contrast" note={`UI-174 · AA ${AA}:1`}>
        <table className="omfp-gallery__contrast">
          <tbody>
            {(pairs ?? []).map((pair) => (
              <tr
                key={`${pair.text}/${pair.background}`}
                data-testid="contrast"
                data-text={pair.text}
                data-background={pair.background}
                data-ratio={pair.ratio.toFixed(2)}
                data-pass={pair.ratio >= AA}
              >
                <td>
                  <span
                    className="omfp-gallery__sample"
                    style={{ color: `var(--omfp-${pair.text})`, background: `var(--omfp-${pair.background})` }}
                  >
                    Aa
                  </span>
                </td>
                <td>
                  {pair.text} on {pair.background}
                </td>
                <td className="omfp-gallery__ratio">{pair.ratio.toFixed(2)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Section>
      <FocusTable measures={focus} />
      <KeyHints measures={keyHints} />
      <GuardTable adjustments={guard} />
    </>
  );
}
