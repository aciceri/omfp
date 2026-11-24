/**
 * TeX math (UI-850, UI-852..UI-854): a formula typeset by KaTeX in a chunk of the bundle loaded with the first formula
 * (math/render.ts), in the color of the text around it. Until the chunk is there the formula shows as its source; once
 * it is, every later formula is typeset in its first render. Display math is a centered block that scrolls sideways,
 * focusable so the keyboard can scroll it, when it is wider than the text (like tables, UI-369).
 */
import { type ReactNode, useEffect, useMemo, useState } from "react";

let renderMath: ((source: string, display: boolean) => string) | undefined;
let loading: Promise<void> | undefined;

/** Loads the KaTeX chunk once; after a failed load the next formula asks for it again. */
function loadRenderer(): Promise<void> {
  loading ??= import("./math/render.ts").then(
    (chunk) => {
      chunk.addStylesheet(document);
      renderMath = chunk.renderMath;
    },
    () => {
      loading = undefined;
    },
  );
  return loading;
}

export interface MathFormulaProps {
  /** The formula's TeX, without its delimiters. */
  readonly source: string;
  /** Display math (a centered block) instead of inline math. */
  readonly display?: boolean | undefined;
}

/** A formula typeset by KaTeX; its source as code until KaTeX is loaded, or if KaTeX throws on it. */
export function MathFormula({ source, display = false }: MathFormulaProps): ReactNode {
  const [ready, setReady] = useState(renderMath !== undefined);
  useEffect(() => {
    let live = true;
    if (!ready) {
      void loadRenderer().then(() => live && renderMath !== undefined && setReady(true));
    }
    return () => {
      live = false;
    };
  }, [ready]);
  const html = useMemo(() => {
    if (!ready || renderMath === undefined) {
      return undefined;
    }
    try {
      return renderMath(source, display);
    } catch {
      // Not a parse error (KaTeX reports those in its markup): the source stays, never a broken row.
      return undefined;
    }
  }, [ready, source, display]);

  const className = display ? "omfp-math omfp-math--display" : "omfp-math";
  if (html === undefined) {
    return (
      <span className={className} data-state={ready ? "failed" : "loading"}>
        <code className="omfp-math__source">{source}</code>
      </span>
    );
  }
  if (display) {
    return (
      <span
        className={className}
        data-state="typeset"
        // biome-ignore lint/a11y/noNoninteractiveTabindex: a wide formula scrolls, so the keyboard must reach it.
        tabIndex={0}
        // biome-ignore lint/security/noDangerouslySetInnerHtml: KaTeX's markup, untrusted commands off (UI-852).
        dangerouslySetInnerHTML={{ __html: html }}
      />
    );
  }
  return (
    <span
      className={className}
      data-state="typeset"
      // biome-ignore lint/security/noDangerouslySetInnerHtml: KaTeX's markup, untrusted commands off (UI-852).
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
