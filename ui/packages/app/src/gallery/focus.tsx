/**
 * The gallery's focus sheet (UI-132, UI-174): items and controls focused on each surface they sit on, and how visible
 * the item indicator is there. The page paints the tokens' colors the way it draws them (each layer over the surface,
 * in sRGB) and measures the 2 px bar's contrast against what it touches and how far the tint is from the other states.
 */
import { Button, Checkbox, Icon, RowList, TextField, ThreadRow } from "@omfp/kit";
import { contrastRatio, deltaE, NON_TEXT, type Rgb } from "../theme/contrast.ts";
import { Label, Section } from "./parts.tsx";

/** The surfaces items sit on: the transcript, the left column and mode line, menus and popovers. */
const SURFACES = ["surface", "surface-container-lowest", "surface-container-low"] as const;

/** The least ΔEok of the tint from the unfocused item, and of a selected item when focused (a clear step). */
export const TINT_FROM_PLAIN = 3;
/** The least ΔEok of the tint from the hover and selected backgrounds: not the same color (the bar tells them apart). */
export const TINT_FROM_STATE = 1.5;

/** One measure: the bar's contrast on a background, or the ΔEok between two backgrounds. */
export interface FocusMeasure {
  readonly kind: "bar" | "tint";
  readonly what: string;
  readonly surface: string;
  readonly value: number;
  readonly threshold: number;
}

/** The computed color of a token (`color`), as the page resolves it. */
export function tokenColor(token: string): string {
  const probe = document.createElement("span");
  probe.hidden = true;
  probe.style.color = `var(--omfp-${token})`;
  document.body.append(probe);
  const color = getComputedStyle(probe).color;
  probe.remove();
  return color;
}

/** The arguments of a CSS function, split at its top-level commas. */
export function functionArguments(value: string): string[] {
  const inner = value.slice(value.indexOf("(") + 1, value.lastIndexOf(")"));
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < inner.length; index += 1) {
    const char = inner[index];
    depth += char === "(" ? 1 : char === ")" ? -1 : 0;
    if (char === "," && depth === 0) {
      parts.push(inner.slice(start, index).trim());
      start = index + 1;
    }
  }
  parts.push(inner.slice(start).trim());
  return parts;
}

/** The color at the inline-start edge of a background token (the first stop of a gradient), where the bar sits. */
function startColor(token: string): string | undefined {
  const probe = document.createElement("span");
  probe.hidden = true;
  probe.style.background = `var(--omfp-${token})`;
  document.body.append(probe);
  const { backgroundColor, backgroundImage } = getComputedStyle(probe);
  probe.remove();
  if (backgroundImage === "none" || backgroundImage === "") {
    return backgroundColor;
  }
  return functionArguments(backgroundImage)
    .map((stop) => stop.replace(/(?:\s+-?[\d.]+(?:%|px))+$/, ""))
    .find((stop) => CSS.supports("color", stop));
}

/** Paints layers of CSS colors over one another on a 1 px canvas and reads the result, or `undefined` if one is not a color the canvas takes. */
export function painter(): (layers: readonly string[]) => Rgb | undefined {
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  return (layers) => {
    if (context === null) {
      return undefined;
    }
    context.clearRect(0, 0, 1, 1);
    for (const layer of layers) {
      // An unparsable color leaves the previous fill style: start from a sentinel to notice.
      context.fillStyle = "#000000";
      context.fillStyle = layer;
      if (context.fillStyle === "#000000" && !CSS.supports("color", layer)) {
        return undefined;
      }
      context.fillRect(0, 0, 1, 1);
    }
    const [red = 0, green = 0, blue = 0] = context.getImageData(0, 0, 1, 1).data;
    return [red, green, blue];
  };
}

/** The bar's contrast and the tint's distances on every surface, from the page's tokens. */
export function measureFocus(): FocusMeasure[] {
  const paint = painter();
  const focusBg = tokenColor("focus-bg");
  const hover = tokenColor("hover");
  const bar = tokenColor("focus-bar");
  const selected = startColor("sel-bg");
  return SURFACES.flatMap((surface) => {
    const base = tokenColor(surface);
    const plain = paint([base]);
    const focused = paint([base, focusBg]);
    const hovered = paint([base, hover]);
    const chosen = selected === undefined ? undefined : paint([base, selected]);
    const both = selected === undefined ? undefined : paint([base, selected, focusBg]);
    const edge = paint([base, bar]);
    const measures: FocusMeasure[] = [];
    const add = (kind: FocusMeasure["kind"], what: string, threshold: number, a?: Rgb, b?: Rgb) => {
      if (a !== undefined && b !== undefined) {
        const value = kind === "bar" ? contrastRatio(a, b) : deltaE(a, b);
        measures.push({ kind, what, surface, value, threshold });
      }
    };
    add("bar", "focus-bar on the surface", NON_TEXT, edge, plain);
    add("bar", "focus-bar on focus-bg", NON_TEXT, edge, focused);
    add("bar", "focus-bar on sel-bg + focus-bg", NON_TEXT, edge, both);
    add("tint", "focused vs unfocused", TINT_FROM_PLAIN, focused, plain);
    add("tint", "selected + focused vs selected", TINT_FROM_PLAIN, both, chosen);
    add("tint", "focused vs hovered", TINT_FROM_STATE, focused, hovered);
    add("tint", "focused vs selected", TINT_FROM_STATE, focused, chosen);
    return measures;
  });
}

/** The measures, one row each: what, on which surface, the value and its threshold. */
export function FocusTable({ measures }: { measures: readonly FocusMeasure[] }) {
  return (
    <Section title="Focus contrast" note={`UI-132 · bar ≥ ${NON_TEXT}:1 · tint ΔEok`}>
      <table className="omfp-gallery__contrast">
        <tbody>
          {measures.map((measure) => (
            <tr
              key={`${measure.surface}/${measure.what}`}
              data-testid="focus-contrast"
              data-kind={measure.kind}
              data-what={measure.what}
              data-surface={measure.surface}
              data-value={measure.value.toFixed(2)}
              data-threshold={measure.threshold}
              data-pass={measure.value >= measure.threshold}
            >
              <td>{measure.what}</td>
              <td>{measure.surface}</td>
              <td className="omfp-gallery__ratio">
                {measure.kind === "bar" ? `${measure.value.toFixed(2)}:1` : `ΔE ${measure.value.toFixed(1)}`} /{" "}
                {measure.threshold}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Section>
  );
}

/** Items and controls in every focus-related state, on each surface they sit on. */
export function FocusStates() {
  return (
    <Section title="Focus" note="UI-132 · items: tint + bar · controls: quiet ring" full>
      <div className="omfp-gallery__focus">
        {SURFACES.map((surface) => (
          <div
            key={surface}
            className="omfp-gallery__focus-surface"
            style={{ background: `var(--omfp-${surface})` }}
            data-testid="focus-surface"
          >
            <Label>{surface}</Label>
            <RowList>
              <ThreadRow state="idle" title="Unfocused" meta="3d" onSelect={() => undefined} />
              <ThreadRow state="idle" title="Hovered" meta="3d" className="is-hover" onSelect={() => undefined} />
              <ThreadRow state="idle" title="Focused" meta="3d" className="is-focus" onSelect={() => undefined} />
              <ThreadRow state="idle" title="Selected" meta="3d" selected onSelect={() => undefined} />
              <ThreadRow
                state="idle"
                title="Selected, focused"
                meta="3d"
                selected
                className="is-focus"
                onSelect={() => undefined}
              />
            </RowList>
            <div className="omfp-gallery__line">
              <Button className="is-focus" icon={<Icon name="check" />}>
                Button
              </Button>
              <Checkbox defaultSelected className="is-focus">
                Box
              </Checkbox>
            </div>
            <TextField label="Field" hideLabel placeholder="Focused field" className="is-focus" />
          </div>
        ))}
      </div>
    </Section>
  );
}
