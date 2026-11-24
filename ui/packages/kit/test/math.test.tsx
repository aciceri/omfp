/** L4: TeX math in the kit's Markdown (UI-850..UI-854), typeset by KaTeX itself (it needs no layout). */

import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { Markdown } from "../src/index.ts";
import { findRule, KIT_SRC, parseCss } from "./support/css.ts";

afterEach(cleanup);

function renderMarkdown(text: string): HTMLElement {
  render(<Markdown text={text} data-testid="md" />);
  return screen.getByTestId("md");
}

/** The formulas of `root`, once `count` of them are typeset. */
async function typeset(root: HTMLElement, count = 1): Promise<HTMLElement[]> {
  await waitFor(() => expect(root.querySelectorAll('.omfp-math[data-state="typeset"]').length).toBe(count));
  return [...root.querySelectorAll<HTMLElement>(".omfp-math")];
}

describe("math in Markdown", () => {
  // spec: UI-850
  test("`$…$` is inline math in its paragraph, with `_`, `*` and `\\,` left to TeX", async () => {
    const root = renderMarkdown("Euler: $e^{i\\pi} + 1 = 0$, then $x_1*y*z_1\\,dx$.");
    const [euler, product] = await typeset(root, 2);
    expect(euler?.parentElement?.tagName).toBe("P");
    expect(euler?.querySelector(".katex")).not.toBeNull();
    expect(euler?.querySelector(".katex-display")).toBeNull();
    expect(euler?.textContent).toContain("e^{i\\pi} + 1 = 0");
    expect(product?.textContent).toContain("x_1*y*z_1\\,dx");
    expect(root.querySelector("em, strong")).toBeNull();
    expect(root.textContent).toContain("Euler:");
  });

  // spec: UI-850
  test.each([
    ["`$$…$$` in a paragraph", "Then $$\\sum_{n=1}^{\\infty} n$$ diverges.", "\\sum_{n=1}^{\\infty} n"],
    ["a block between `$$` lines", "Then:\n$$\n\\int_0^1 x\\,dx\n$$\nDone.", "\\int_0^1 x\\,dx"],
    ["a longer closing line", "$$\na^2\n$$$", "a^2"],
    ["a `math` fence", "```math\n\\frac{a}{b}\n```", "\\frac{a}{b}"],
    ["a block in a list item", "- see:\n\n  $$\n  x^2\n  $$", "x^2"],
    ["a block in a block quote", "> $$\n> y^2\n> $$", "y^2"],
  ])("%s is display math", async (_name, text, source) => {
    const root = renderMarkdown(text);
    const [formula] = await typeset(root);
    expect(formula?.classList.contains("omfp-math--display")).toBe(true);
    expect(formula?.querySelector(".katex-display")).not.toBeNull();
    expect(formula?.textContent).toContain(source);
  });

  // spec: UI-850
  test.each([
    ["a `$$` block", "Here:\n\n$$\n\\int_0^1 f", "$$"],
    ["a `math` fence", "Here:\n\n```math\n\\int_0^1 f", "```"],
  ])("an open %s stays code while the reply streams, then becomes math", async (_name, open, close) => {
    const { rerender } = render(<Markdown text={open} data-testid="md" />);
    await act(async () => {});
    const root = screen.getByTestId("md");
    expect(root.querySelector(".omfp-math")).toBeNull();
    expect(root.querySelector('pre[data-lang="math"]')?.textContent?.trim()).toBe("\\int_0^1 f");

    rerender(<Markdown text={`${open}\n${close}`} data-testid="md" />);
    const [formula] = await typeset(root);
    expect(formula?.textContent).toContain("\\int_0^1 f");
    expect(root.querySelector("pre")).toBeNull();
  });

  // spec: UI-850
  test("with file paths opening, inline code stays a path and math is still typeset", async () => {
    render(<Markdown text="Open `src/main.rs` for $n^2$." onOpenPath={() => {}} data-testid="md" />);
    const root = screen.getByTestId("md");
    const [formula] = await typeset(root);
    expect(formula?.textContent).toContain("n^2");
    expect(root.querySelector(".omfp-path-link")?.textContent).toBe("src/main.rs");
  });

  // spec: UI-851
  test.each([
    ["prices", "It costs $5 and $10 a month.", "It costs $5 and $10 a month."],
    ["a range", "From $5-$10 each.", "From $5-$10 each."],
    ["a space after the opening `$`", "Pay $ 5 now $", "Pay $ 5 now $"],
    ["a space before the closing `$`", "Between $a and b $ here.", "Between $a and b $ here."],
    ["a digit after the closing `$`", "From $x$5 on.", "From $x$5 on."],
    ["escaped dollars", "Not \\$x\\$ math.", "Not $x$ math."],
    ["a code span", "Run `echo $x$` here.", "Run echo $x$ here."],
    ["a code fence", "```sh\necho $x$\n```", "echo $x$"],
  ])("%s stay text", async (_name, text, shown) => {
    const root = renderMarkdown(text);
    await act(async () => {});
    expect(root.querySelector(".omfp-math")).toBeNull();
    expect(root.textContent?.trim()).toBe(shown);
  });

  // spec: UI-851
  test("next to a price, `$…$` is math, and `\\$` inside it a dollar sign", async () => {
    const root = renderMarkdown("It costs $5, or $\\$5 + x$ with tax, and $y$.");
    const [taxed, y] = await typeset(root, 2);
    expect(taxed?.textContent).toContain("\\$5 + x");
    expect(y?.textContent).toContain("y");
    expect(root.textContent).toContain("It costs $5, or ");
  });

  // spec: UI-852
  test("colors work, while `\\href`, `\\url` and `\\includegraphics` stay off", async () => {
    const root = renderMarkdown(
      "$\\textcolor{red}{a} + \\colorbox{yellow}{b} + \\fcolorbox{blue}{white}{c}$ and " +
        "$\\href{https://example.com}{d}$ $\\url{https://example.com}$ $\\includegraphics{https://example.com/x.png}$",
    );
    const [colors, ...untrusted] = await typeset(root, 4);
    expect(colors?.querySelector('[style*="color:red"]')).not.toBeNull();
    expect(colors?.querySelector('[style*="background-color:yellow"]')).not.toBeNull();
    expect(colors?.querySelector('[style*="border-color:blue"]')).not.toBeNull();
    expect(untrusted.length).toBe(3);
    for (const formula of untrusted) {
      expect(formula.querySelector('[style*="color:var(--omfp-error)"]')).not.toBeNull();
    }
    expect(root.querySelector("a, img, [href], [src]")).toBeNull();
  });

  // spec: UI-853
  test("a formula KaTeX cannot parse shows its source in the error color, KaTeX's reason as its title", async () => {
    const root = renderMarkdown("Before $\\frac{1}{$ after, and $x^2$.");
    const [broken, fine] = await typeset(root, 2);
    const error = broken?.querySelector(".katex-error");
    expect(error?.textContent).toBe("\\frac{1}{");
    expect(error?.getAttribute("title")).toContain("KaTeX parse error");
    expect(error?.getAttribute("style")).toContain("color:var(--omfp-error)");
    expect(fine?.querySelector(".katex")).not.toBeNull();
    expect(root.textContent).toContain("Before ");
    expect(root.textContent).toContain(" after, and ");
  });

  // spec: UI-854
  test("display math is a focusable region that scrolls sideways; math sets no color of its own", async () => {
    const root = renderMarkdown("$$\n\\sum_{k=0}^{n} k\n$$\n\nand $k$.");
    const [display, inline] = await typeset(root, 2);
    expect(display?.getAttribute("tabindex")).toBe("0");
    expect(inline?.hasAttribute("tabindex")).toBe(false);
    expect(root.querySelector(".omfp-math [style*='color']")).toBeNull();

    const rules = parseCss(readFileSync(join(KIT_SRC, "kit.css"), "utf8"));
    const region = findRule(rules, ["@layer omfp-kit"], ".omfp-math--display");
    expect(region.declarations.get("overflow-x")).toBe("auto");
    expect(region.declarations.get("max-inline-size")).toBe("100%");
    expect(region.declarations.get("text-align")).toBe("center");
    expect(findRule(rules, ["@layer omfp-kit"], ".omfp-math").declarations.has("color")).toBe(false);
  });

  // spec: UI-854
  test("KaTeX's stylesheet comes once, under the kit's layer, its fonts from the bundle", async () => {
    const root = renderMarkdown("$a$ and $b$");
    await typeset(root, 2);
    const styles = document.head.querySelectorAll("style[data-omfp-katex]");
    expect(styles.length).toBe(1);
    const css = styles[0]?.textContent ?? "";
    expect(css.startsWith("@layer omfp-kit.katex {")).toBe(true);
    expect(css).toContain("url(/katex/fonts/KaTeX_Main-Regular.woff2)");
    expect(css.match(/url\((?!\/katex\/fonts\/)/g)).toBeNull();
  });

  // spec: UI-854
  test("the vendored stylesheet is the installed KaTeX's, each face loading only its woff2 from the bundle", () => {
    const katex = dirname(Bun.resolveSync("katex/package.json", import.meta.dir));
    const upstream = readFileSync(join(katex, "dist/katex.css"), "utf8");
    const expected = upstream.replace(
      /url\(fonts\/([^)]+\.woff2)\) format\("woff2"\)(?:, url\(fonts\/[^)]+\) format\("[^"]+"\))*/g,
      'url(/katex/fonts/$1) format("woff2")',
    );
    const vendored = readFileSync(join(KIT_SRC, "math/katex.css"), "utf8");
    expect(expected).not.toBe(upstream);
    expect(vendored.endsWith(expected)).toBe(true);
  });
});
