/** L4: Mermaid diagrams in the kit's Markdown (UI-981..UI-983) over a fake Mermaid. */

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Markdown } from "../src/index.ts";
import { fakeMermaid, themeOf } from "./support/mermaid.ts";

beforeEach(() => fakeMermaid.reset());

afterEach(() => {
  cleanup();
  document.documentElement.removeAttribute("style");
});

/** The only diagram, once it says `state`. */
async function diagram(state: "drawn" | "failed"): Promise<HTMLElement> {
  await waitFor(() => expect(screen.getByTestId("mermaid-diagram").dataset.state).toBe(state));
  return screen.getByTestId("mermaid-diagram");
}

describe("Mermaid diagrams", () => {
  // spec: UI-981
  test("a closed mermaid fence is drawn in place of its code, its source one toggle away", async () => {
    render(<Markdown text={"Before:\n\n```mermaid\ngraph TD\n  A --> B\n```\n\nAfter."} />);
    const figure = await diagram("drawn");

    expect(fakeMermaid.drawings.map(({ source }) => source)).toEqual(["graph TD\n  A --> B"]);
    expect(figure.querySelector(".omfp-diagram__drawing svg text")?.textContent).toBe("graph TD\n  A --> B");
    expect(figure.querySelector("pre")).toBeNull();
    expect(figure.previousElementSibling?.textContent).toBe("Before:");
    expect(figure.nextElementSibling?.textContent).toBe("After.");

    const toggle = screen.getByRole("button", { name: "Source" });
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    await userEvent.click(toggle);
    expect(figure.querySelector("svg")).toBeNull();
    expect(figure.querySelector('pre[data-lang="mermaid"] code.language-mermaid')?.textContent).toBe(
      "graph TD\n  A --> B",
    );
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    await userEvent.click(toggle);
    expect(figure.querySelector(".omfp-diagram__drawing svg")).not.toBeNull();
    expect(fakeMermaid.drawings.length).toBe(1);
  });

  // spec: UI-981
  test.each([
    ["no closing fence yet", "```mermaid\ngraph LR\n  open1 --> B", false],
    ["a shorter closing fence", "````mermaid\ngraph LR\n  open2 --> B\n```", false],
    ["another fence character", "```mermaid\ngraph LR\n  open3 --> B\n~~~", false],
    ["a four-space-indented apparent closer", "```mermaid\ngraph LR\n  open4 --> B\n    ```", false],
    ["an indented apparent closer with CRLF", "```mermaid\r\ngraph LR\r\n  open5 --> B\r\n    ```", false],
    ["an indented apparent closer in a quote", "> ```mermaid\n> graph LR\n>   open6 --> B\n>     ```", false],
    ["an indented apparent closer in a list", "- see:\n\n  ```mermaid\n  graph LR\n    open7 --> B\n      ```", false],
    ["a longer closing fence", "```mermaid\ngraph LR\n  closed1 --> B\n````", true],
    ["a tilde fence", "~~~mermaid\ngraph LR\n  closed2 --> B\n~~~", true],
    ["a fence in a block quote", "> ```mermaid\n> graph LR\n>   closed3 --> B\n> ```", true],
    ["a fence in a list item", "- see:\n\n  ```Mermaid\n  graph LR\n    closed4 --> B\n  ```", true],
    ["a closer indented three spaces", "```mermaid\ngraph LR\n  closed5 --> B\n   ```", true],
    ["a closing fence with trailing whitespace", "```mermaid\ngraph LR\n  closed6 --> B\n``` \t", true],
  ])("%s: drawn %p", async (_name, text, drawn) => {
    render(<Markdown text={text} data-testid="md" />);
    if (drawn) {
      await diagram("drawn");
    } else {
      // Nothing is drawn: the block stays the highlighted code it is while the reply streams.
      await act(async () => {});
      expect(screen.queryByTestId("mermaid-diagram")).toBeNull();
      expect(screen.getByTestId("md").querySelector('pre[data-lang="mermaid"]')).not.toBeNull();
    }
    expect(fakeMermaid.drawings.length).toBe(drawn ? 1 : 0);
  });

  // spec: UI-981
  test("a streamed block is drawn once its closing fence arrives", async () => {
    const open = "Here:\n\n```mermaid\nsequenceDiagram\n  A->>B: hi";
    const { rerender } = render(<Markdown text={open} />);
    await act(async () => {});
    expect(screen.queryByTestId("mermaid-diagram")).toBeNull();
    rerender(<Markdown text={`${open}\n\`\`\``} />);
    await diagram("drawn");
    expect(fakeMermaid.drawings.map(({ source }) => source)).toEqual(["sequenceDiagram\n  A->>B: hi"]);
  });

  // spec: UI-981
  test("identical diagrams mounted together have independent SVG IDs", async () => {
    const text = "```mermaid\ngraph TD\n  again --> B\n```";
    const { container } = render(
      <>
        <Markdown text={text} />
        <Markdown text={text} />
      </>,
    );
    await waitFor(() => expect(container.querySelectorAll("svg").length).toBe(2));
    const ids = [...container.querySelectorAll("svg")].map((svg) => svg.id);
    expect(ids.every((id) => id !== "") && new Set(ids).size === ids.length).toBe(true);
  });

  // spec: UI-981
  test("its colors and font are the theme's tokens, and a theme change draws it again", async () => {
    const root = document.documentElement;
    root.style.setProperty("--omfp-code-bg", "#101014");
    root.style.setProperty("--omfp-primary-container", "#223344");
    root.style.setProperty("--omfp-on-surface", "#eeeeee");
    root.style.setProperty("--omfp-font-ui", '"Commit Mono", monospace');
    root.style.setProperty("--omfp-fs-code", "13px");
    render(<Markdown text={"```mermaid\ngraph TD\n  themed --> B\n```"} />);
    const figure = await diagram("drawn");
    expect(fakeMermaid.drawings[0]?.config).toMatchObject({ theme: "base", fontFamily: '"Commit Mono", monospace' });
    expect(themeOf(0)).toMatchObject({
      background: "#101014",
      primaryColor: "#223344",
      textColor: "#eeeeee",
      darkMode: true,
      fontFamily: '"Commit Mono", monospace',
      fontSize: "13px",
    });

    root.style.setProperty("--omfp-code-bg", "#fafafa");
    root.style.setProperty("--omfp-primary-container", "#ddeeff");
    await waitFor(() => expect(fakeMermaid.drawings.length).toBe(2));
    expect(themeOf(1)).toMatchObject({ background: "#fafafa", primaryColor: "#ddeeff", darkMode: false });
    // The drawing stays while the new one is drawn, then gives way to it.
    expect(figure.querySelector("svg")).not.toBeNull();
    const second = fakeMermaid.drawings[1]?.id;
    await waitFor(() => expect(figure.querySelector("svg")?.id).toBe(second ?? "none"));

    // A change of `<html>` that leaves the tokens as they are draws nothing.
    root.style.setProperty("--vvh", "500px");
    await act(async () => {});
    expect(fakeMermaid.drawings.length).toBe(2);
  });

  // spec: UI-982
  test("Mermaid draws at its strict security level, and the drawing's links open in a new tab", async () => {
    render(<Markdown text={"```mermaid\ngraph TD\n  linked --> B\n```"} />);
    const figure = await diagram("drawn");
    expect(fakeMermaid.drawings[0]?.config).toMatchObject({ securityLevel: "strict", startOnLoad: false });
    const link = figure.querySelector("svg a");
    expect(link?.getAttribute("href")).toBe("https://example.com/docs");
    expect(link?.getAttribute("target")).toBe("_blank");
    expect(link?.getAttribute("rel")).toBe("noreferrer");
  });

  // spec: UI-983
  test("a source Mermaid cannot draw stays code, with Mermaid's reason under it", async () => {
    render(<Markdown text={"```mermaid\nfail TD\n  A -->\n```"} />);
    const figure = await diagram("failed");
    expect(figure.querySelector('pre[data-lang="mermaid"] code')?.textContent).toBe("fail TD\n  A -->");
    expect(figure.querySelector(".omfp-diagram__error")?.textContent).toBe("Diagram not drawn: Parse error on line 1");
    expect(screen.queryByRole("button", { name: "Source" })).toBeNull();
    expect(fakeMermaid.config).toMatchObject({ suppressErrorRendering: true });
  });
});
