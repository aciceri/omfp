/**
 * The KaTeX chunk of `MathFormula` (UI-850, UI-852..UI-854): typesets a formula's TeX as HTML and adds KaTeX's
 * stylesheet (katex.css, vendored) to the page once. The stylesheet sits in a sublayer of the kit's layer, so kit.css
 * and the themes win over it, and loads KaTeX's fonts from the bundle's `/katex/fonts/` (build.ts), never from a CDN.
 */
import katex from "katex";
import katexCss from "./katex.css" with { type: "text" };

const STYLESHEET = `@layer omfp-kit.katex {\n${String(katexCss)}\n}\n`;

/** Adds KaTeX's stylesheet to `document` unless it is there already. */
export function addStylesheet(document: Document): void {
  if (document.head.querySelector("style[data-omfp-katex]") !== null) {
    return;
  }
  const style = document.createElement("style");
  style.dataset.omfpKatex = "";
  style.textContent = STYLESHEET;
  document.head.append(style);
}

/**
 * The HTML of the formula `source`, display or inline. `trust: false` keeps `\href`, `\url`, `\includegraphics` and
 * `\html…` off (KaTeX shows them in the error color), while colors (`\textcolor`, `\colorbox`, `\fcolorbox`) work. A
 * formula KaTeX cannot parse comes back as its source in the theme's error color, KaTeX's message as its `title`.
 */
export function renderMath(source: string, display: boolean): string {
  return katex.renderToString(source, {
    displayMode: display,
    throwOnError: false,
    errorColor: "var(--omfp-error)",
    trust: false,
    strict: "ignore",
  });
}
