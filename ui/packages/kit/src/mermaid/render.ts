/**
 * The Mermaid chunk of `MermaidDiagram` (UI-981): draws a diagram's source as SVG with the theme's colors. Mermaid
 * keeps one global configuration, so each drawing sets it and draws in turn, never interleaved with another.
 */
import mermaid from "mermaid";

/** Mermaid's `base` theme variables: colors as `#rrggbb`, `darkMode`, `fontFamily`, `fontSize`. */
export interface ThemeVariables {
  readonly fontFamily: string;
  readonly [name: string]: string | boolean;
}

let pending: Promise<unknown> = Promise.resolve();
let drawn = 0;

/**
 * The SVG of the diagram `source`, drawn with `themeVariables`; rejects with Mermaid's error when the source is not a
 * diagram it can draw. `securityLevel: "strict"`: labels are text, `click` callbacks are off, and Mermaid passes the
 * SVG through DOMPurify before returning it.
 */
export function renderDiagram(source: string, themeVariables: ThemeVariables): Promise<string> {
  drawn += 1;
  const id = `omfp-mermaid-${drawn}`;
  const drawing = pending.then(async () => {
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      suppressErrorRendering: true,
      theme: "base",
      themeVariables,
      // Mermaid measures labels with this family and its stylesheet sets the theme's: they must agree.
      fontFamily: themeVariables.fontFamily,
    });
    const { svg } = await mermaid.render(id, source);
    return svg;
  });
  pending = drawing.catch(() => undefined);
  return drawing;
}
