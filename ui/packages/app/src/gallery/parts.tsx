/** The gallery's building blocks (UI-173): sections, state grids and labelled lines, the forced states, and the
 * model and thinking-level fixtures. */
import type { ReactNode } from "react";

/** The forced interaction states shown next to the live one. */
export type Forced = "live" | "hover" | "pressed" | "focus" | "disabled";

const FORCED: readonly Forced[] = ["live", "hover", "pressed", "focus", "disabled"];

/** The kit's forced state class of `state` (none for live and disabled). */
export function forcedClass(state: Forced): string {
  return state === "hover" ? "is-hover" : state === "pressed" ? "is-pressed" : state === "focus" ? "is-focus" : "";
}

export function Section({
  title,
  note,
  wide,
  full,
  children,
}: {
  title: string;
  note?: string;
  /** Two columns of three. */
  wide?: boolean;
  /** The whole row. */
  full?: boolean;
  children: ReactNode;
}) {
  return (
    <section
      className="omfp-gallery__section"
      data-wide={wide === true || undefined}
      data-full={full === true || undefined}
      data-testid="kit-section"
    >
      <header className="omfp-gallery__head">
        <h2 className="omfp-gallery__title omfp-grad-text omfp-grad-flow">{title}</h2>
        {note !== undefined && <span className="omfp-gallery__note">{note}</span>}
      </header>
      <div className="omfp-gallery__body">{children}</div>
    </section>
  );
}

export function Label({ children }: { children: ReactNode }) {
  return <span className="omfp-gallery__label">{children}</span>;
}

/** Rows of a component, one column per forced state. */
export function StateGrid({
  states = FORCED,
  rows,
}: {
  states?: readonly Forced[];
  rows: readonly { label: string; render: (state: Forced) => ReactNode }[];
}) {
  return (
    <div className="omfp-gallery__grid" style={{ gridTemplateColumns: `6rem repeat(${states.length}, max-content)` }}>
      <span />
      {states.map((state) => (
        <Label key={state}>{state}</Label>
      ))}
      {rows.map((row) => (
        <Row key={row.label} label={row.label} states={states} render={row.render} />
      ))}
    </div>
  );
}

function Row({
  label,
  states,
  render,
}: {
  label: string;
  states: readonly Forced[];
  render: (state: Forced) => ReactNode;
}) {
  return (
    <>
      <Label>{label}</Label>
      {states.map((state) => (
        <span key={state} className="omfp-gallery__cell">
          {render(state)}
        </span>
      ))}
    </>
  );
}

/** A labelled line of examples. */
export function Line({ label, children }: { label?: string; children: ReactNode }) {
  return (
    <div className="omfp-gallery__line">
      {label !== undefined && <Label>{label}</Label>}
      {children}
    </div>
  );
}

export const MODELS = [
  { id: "opus-4.5", label: "opus-4.5", meta: "anthropic" },
  { id: "sonnet-4.5", label: "sonnet-4.5", meta: "anthropic" },
  { id: "gpt-5.1-codex", label: "gpt-5.1-codex", meta: "openai" },
];

export const LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"].map((level) => ({
  id: level,
  label: level,
}));
