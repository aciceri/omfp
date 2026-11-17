/**
 * The gallery's key hints in buttons (UI-148, UI-149, UI-174): a button of every variant with its key chip, and the
 * chip's contrast on the button's own fill (every gradient stop of it, painted over the surface), at least 3:1.
 */
import { Button, type ButtonVariant } from "@omfp/kit";
import { contrastRatio, NON_TEXT, type Rgb } from "../theme/contrast.ts";
import { functionArguments, painter, tokenColor } from "./focus.tsx";
import { Line, Section } from "./parts.tsx";

const VARIANTS: readonly ButtonVariant[] = ["primary", "secondary", "quiet", "danger"];

/** One measure: the key chip of a button variant on the button's fill. */
export interface KeyHintMeasure {
  readonly variant: string;
  readonly ratio: number;
}

/** The layers a background paints, one list per point that matters: its color, then each stop of its gradient. */
function fills({ backgroundColor, backgroundImage }: CSSStyleDeclaration): string[][] {
  if (backgroundImage === "none" || backgroundImage === "") {
    return [[backgroundColor]];
  }
  return functionArguments(backgroundImage)
    .map((stop) => stop.replace(/(?:\s+-?[\d.]+(?:%|px|deg))+$/, ""))
    .filter((stop) => CSS.supports("color", stop))
    .map((stop) => [backgroundColor, stop]);
}

/** `ink` at `opacity` over `under`. */
function blend(ink: Rgb, under: Rgb, opacity: number): Rgb {
  const channel = (index: 0 | 1 | 2) => Math.round(ink[index] * opacity + under[index] * (1 - opacity));
  return [channel(0), channel(1), channel(2)];
}

/** The lowest contrast of each sample button's key chip on its fill, over the surface the samples sit on. */
export function measureKeyHints(root: ParentNode): KeyHintMeasure[] {
  const paint = painter();
  const surface = tokenColor("surface");
  return [...root.querySelectorAll<HTMLElement>("[data-testid=key-hint-sample]")].flatMap((button) => {
    const kbd = button.querySelector(".omfp-kbd");
    if (kbd === null) {
      return [];
    }
    const chip = getComputedStyle(kbd);
    const opacity = Number(chip.opacity);
    const ratios = fills(getComputedStyle(button)).flatMap((fill) => {
      const layers = [surface, ...fill];
      const under = paint(layers);
      const over = paint([...layers, chip.backgroundColor]);
      if (under === undefined || over === undefined) {
        return [];
      }
      const ink = paint([...layers, chip.backgroundColor, chip.color]);
      if (ink === undefined) {
        return [];
      }
      // The chip's opacity blends it, ink and fill, with the button under it.
      return [contrastRatio(blend(ink, under, opacity), blend(over, under, opacity))];
    });
    return ratios.length === 0 ? [] : [{ variant: button.dataset.variant ?? "", ratio: Math.min(...ratios) }];
  });
}

/** The sample buttons and their measures, one row each. */
export function KeyHints({ measures }: { measures: readonly KeyHintMeasure[] }) {
  return (
    <Section title="Key hints in buttons" note={`UI-148 · UI-149 · ≥ ${NON_TEXT}:1 on the button`}>
      <Line>
        {VARIANTS.map((variant) => (
          <Button key={variant} variant={variant} kbd="enter" data-testid="key-hint-sample">
            {variant}
          </Button>
        ))}
      </Line>
      <table className="omfp-gallery__contrast">
        <tbody>
          {measures.map((measure) => (
            <tr
              key={measure.variant}
              data-testid="key-hint-contrast"
              data-variant={measure.variant}
              data-ratio={measure.ratio.toFixed(2)}
              data-pass={measure.ratio >= NON_TEXT}
            >
              <td>enter in {measure.variant}</td>
              <td className="omfp-gallery__ratio">
                {measure.ratio.toFixed(2)} / {NON_TEXT}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Section>
  );
}
