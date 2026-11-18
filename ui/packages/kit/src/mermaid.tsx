/**
 * Mermaid diagrams (UI-981..UI-983): the source of a diagram drawn as SVG by Mermaid, in a chunk of the bundle loaded
 * with the first diagram (mermaid/render.ts), colored and set in the theme's tokens and drawn again when the theme
 * changes; its source one press away. A source Mermaid cannot draw shows as code with Mermaid's reason.
 */
import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button } from "./button.tsx";
import type { ThemeVariables } from "./mermaid/render.ts";
import { cssColorReader } from "./theme-colors.ts";
import { watchTheme } from "./theme-watch.ts";

/** Mermaid's color variables (theme `base`, which derives the rest from them) and the tokens they take. */
const COLORS: Readonly<Record<string, string>> = {
  background: "code-bg",
  primaryColor: "primary-container",
  primaryTextColor: "on-primary-container",
  primaryBorderColor: "primary",
  secondaryColor: "secondary-container",
  secondaryTextColor: "on-secondary-container",
  secondaryBorderColor: "secondary",
  tertiaryColor: "tertiary-container",
  tertiaryTextColor: "on-tertiary-container",
  tertiaryBorderColor: "tertiary",
  lineColor: "on-surface-variant",
  edgeLabelBackground: "code-bg",
  textColor: "on-surface",
  noteBkgColor: "surface-container-highest",
  noteTextColor: "on-surface",
  noteBorderColor: "outline",
  errorBkgColor: "error-container",
  errorTextColor: "on-error-container",
};

/** Whether `#rrggbb` is darker than mid-gray: Mermaid then derives its other colors for a dark background. */
function dark(color: string): boolean {
  const value = /^#([0-9a-f]{6})$/i.exec(color)?.[1];
  if (value === undefined) {
    return false;
  }
  const rgb = Number.parseInt(value, 16);
  return 0.2126 * ((rgb >> 16) & 0xff) + 0.7152 * ((rgb >> 8) & 0xff) + 0.0722 * (rgb & 0xff) < 128;
}

/** The theme's tokens as rendered in `document`, read through a hidden probe. */
function readPalette(document: Document): ThemeVariables {
  const read = cssColorReader(document, (color) => (/^#[0-9a-f]{6}$/i.test(color) ? color : undefined));
  const colors: Record<string, string> = {};
  for (const [name, token] of Object.entries(COLORS)) {
    const color = read(token);
    if (color !== undefined) {
      colors[name] = color;
    }
  }
  const probe = document.createElement("span");
  probe.hidden = true;
  document.body.append(probe);
  probe.style.fontFamily = "var(--omfp-font-ui)";
  probe.style.fontSize = "var(--omfp-fs-code)";
  const font = document.defaultView?.getComputedStyle(probe);
  const variables: ThemeVariables = {
    ...colors,
    darkMode: dark(colors.background ?? ""),
    fontFamily: font?.fontFamily ?? "",
    fontSize: font?.fontSize ?? "",
  };
  probe.remove();
  return variables;
}

/** The first line of why Mermaid could not draw, without the colon that announces its details. */
function reason(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return (message.split("\n")[0] ?? "").trim().replace(/:$/, "");
}

export interface MermaidDiagramProps {
  /** The diagram's Mermaid source. */
  readonly source: string;
}

/**
 * A Mermaid diagram: its source as code until it is drawn, then the drawing with a toggle back to the source. While
 * the theme changes it keeps the drawing it has until the new one is ready. Links in the drawing open in a new tab.
 */
export function MermaidDiagram({ source }: MermaidDiagramProps): ReactNode {
  const [result, setResult] = useState<{ source: string; svg: string } | { source: string; error: string }>();
  const drawing = result?.source === source ? result : undefined;
  const [showSource, setShowSource] = useState(false);
  useEffect(() => {
    let live = true;
    let paletteKey: string | undefined;
    let revision = 0;
    const draw = () => {
      const palette = readPalette(document);
      const key = JSON.stringify(palette);
      if (key === paletteKey) {
        return;
      }
      paletteKey = key;
      const current = ++revision;
      void import("./mermaid/render.ts")
        .then(({ renderDiagram }) => renderDiagram(source, palette))
        .then(
          (svg) => live && current === revision && setResult({ source, svg }),
          (error: unknown) => live && current === revision && setResult({ source, error: reason(error) }),
        );
    };
    draw();
    const unwatch = watchTheme(document, draw);
    return () => {
      live = false;
      unwatch();
    };
  }, [source]);

  const svg = drawing !== undefined && "svg" in drawing ? drawing.svg : undefined;
  const error = drawing !== undefined && "error" in drawing ? drawing.error : undefined;
  const shown = showSource ? undefined : svg;
  const drawingRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (shown === undefined) {
      return;
    }
    for (const link of drawingRef.current?.querySelectorAll("a") ?? []) {
      link.setAttribute("target", "_blank");
      link.setAttribute("rel", "noreferrer");
    }
  }, [shown]);

  return (
    <figure
      className="omfp-diagram"
      data-testid="mermaid-diagram"
      data-state={drawing === undefined ? "drawing" : svg === undefined ? "failed" : "drawn"}
    >
      {shown === undefined ? (
        <pre className="omfp-code omfp-code--block" data-lang="mermaid">
          <code className="language-mermaid">{source}</code>
        </pre>
      ) : (
        <div
          ref={drawingRef}
          className="omfp-diagram__drawing"
          // biome-ignore lint/security/noDangerouslySetInnerHtml: Mermaid's SVG, sanitized by its DOMPurify pass (securityLevel "strict", UI-982).
          dangerouslySetInnerHTML={{ __html: shown }}
        />
      )}
      {drawing !== undefined && (
        <figcaption className="omfp-diagram__foot">
          {error !== undefined && <span className="omfp-diagram__error">Diagram not drawn: {error}</span>}
          {svg !== undefined && (
            <Button size="sm" variant="quiet" aria-pressed={showSource} onPress={() => setShowSource((on) => !on)}>
              Source
            </Button>
          )}
        </figcaption>
      )}
    </figure>
  );
}
