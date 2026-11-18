/**
 * A fake Mermaid for the component tests of diagrams (kit's `MermaidDiagram`, the app's transcript): importing this
 * module replaces `mermaid` for the diagram's chunk. happy-dom lays nothing out, so Mermaid cannot draw there; the fake
 * draws a source as an SVG titled with its first line, with a link, the source as its label and a stylesheet, fails on
 * a source starting with `fail` as Mermaid's parser does, and records each drawing with the configuration it was drawn
 * with.
 */
import { mock } from "bun:test";

export interface FakeDrawing {
  readonly id: string;
  readonly source: string;
  readonly config: Readonly<Record<string, unknown>>;
}

export const fakeMermaid = {
  /** Every drawing asked for, in order. */
  drawings: [] as FakeDrawing[],
  config: {} as Readonly<Record<string, unknown>>,
  reset(): void {
    this.drawings.length = 0;
    this.config = {};
  },
};

/** The theme variables of drawing `index`. */
export function themeOf(index: number): Readonly<Record<string, unknown>> | undefined {
  return fakeMermaid.drawings[index]?.config.themeVariables as Readonly<Record<string, unknown>> | undefined;
}

mock.module("mermaid", () => ({
  default: {
    initialize: (config: Record<string, unknown>) => {
      fakeMermaid.config = config;
    },
    render: async (id: string, source: string) => {
      fakeMermaid.drawings.push({ id, source, config: fakeMermaid.config });
      if (source.startsWith("fail")) {
        throw new Error("Parse error on line 1:\nfail\n^\nExpecting 'NEWLINE', got 'EOF'");
      }
      const text = source.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
      return {
        diagramType: "flowchart-v2",
        // Mermaid puts its stylesheet first; happy-dom's parser drops whatever follows a `<style>` in an `<svg>`, and
        // the stylesheet's text. The title is what Mermaid makes of an `accTitle`.
        svg:
          `<svg id="${id}" viewBox="0 0 100 50"><title>${text.split("\n")[0]}</title>` +
          `<a href="https://example.com/docs"><g class="label"><text>${text}</text></g></a>` +
          `<style>#${id} .label { fill: red; }</style></svg>`,
      };
    },
  },
}));
