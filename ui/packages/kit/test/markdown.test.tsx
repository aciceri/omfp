import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import { Code, Markdown, type MarkdownThreadLinkProps, threadLinkId, threadLinkUrl } from "../src/index.ts";

afterEach(cleanup);

function renderMarkdown(text: string): HTMLElement {
  render(<Markdown text={text} data-testid="md" />);
  return screen.getByTestId("md");
}

describe("Markdown", () => {
  // spec: UI-040
  test.each([
    ["script", "Hello <script>window.pwned = true</script> world", "script"],
    ["img onerror", 'Look <img src="x" onerror="window.pwned = true"> here', "img"],
    ["iframe", '<iframe src="https://example.com/"></iframe>', "iframe"],
    ["block html", '<div onclick="window.pwned = true">\n\n*text*\n\n</div>', "[onclick]"],
    ["inline style", '<span style="color: red">red</span>', "span"],
  ])("raw HTML (%s) never becomes DOM", (_name, text, selector) => {
    const root = renderMarkdown(text);
    expect(root.querySelector(selector)).toBeNull();
    expect(root.querySelector("[onerror], [onclick], script, iframe")).toBeNull();
    expect(Reflect.get(window, "pwned")).toBeUndefined();
  });

  // spec: UI-040
  test("javascript: links lose their URL", () => {
    const root = renderMarkdown("[click](javascript:alert(1))");
    expect(root.querySelector("a")?.getAttribute("href") ?? "").not.toContain("javascript:");
  });

  // spec: UI-040
  test("remote images load and code-language class names are kept", () => {
    const root = renderMarkdown("![chart](https://example.com/chart.png)\n\n```rust\nfn main() {}\n```");
    const image = root.querySelector("img");
    expect(image?.getAttribute("src")).toBe("https://example.com/chart.png");
    expect(image?.getAttribute("alt")).toBe("chart");
    expect(root.querySelector("pre code")?.className).toContain("language-rust");
  });

  // spec: UI-040
  test("links open in a new tab without a referrer", () => {
    const root = renderMarkdown("See [docs](https://example.com/docs).");
    const link = root.querySelector("a");
    expect(link?.getAttribute("href")).toBe("https://example.com/docs");
    expect(link?.getAttribute("target")).toBe("_blank");
    expect(link?.getAttribute("rel")).toBe("noreferrer");
  });

  // spec: UI-040
  test("GitHub-flavoured Markdown renders tables, task lists and strikethrough", () => {
    const root = renderMarkdown("| a | b |\n| - | - |\n| 1 | 2 |\n\n- [x] done\n\n~~old~~ https://example.com");
    expect(root.querySelectorAll("table td")).toHaveLength(2);
    expect(root.querySelector('input[type="checkbox"]')?.hasAttribute("checked")).toBe(true);
    expect(root.querySelector("del")?.textContent).toBe("old");
    expect(root.querySelector('a[href="https://example.com"]')).not.toBeNull();
  });

  // spec: UI-369
  test("a table sits in its own sideways-scrolling region, reachable by keyboard", () => {
    const root = renderMarkdown("| # | Ticket |\n| - | - |\n| A | PSE-240 |");
    const region = root.querySelector(".omfp-markdown__table");
    // A labelled section is a region landmark.
    expect(region?.tagName).toBe("SECTION");
    expect(region?.firstElementChild?.tagName).toBe("TABLE");
    expect(region?.getAttribute("tabindex")).toBe("0");
    expect(region?.getAttribute("aria-label")).toBe("Table");
  });

  // spec: UI-094
  test("prose gets the prose class, code blocks the code class", () => {
    const root = renderMarkdown("Some *prose* and `inline`.\n\n```\nblock\n```");
    expect(root.classList.contains("omfp-prose")).toBe(true);
    expect(root.querySelector("pre")?.classList.contains("omfp-code")).toBe(true);
    expect(root.querySelector("p code")?.textContent).toBe("inline");
  });
});

describe("Code", () => {
  // spec: UI-094
  test("renders inline code or a pre block with the code class", () => {
    render(
      <>
        <Code code="ls -la" data-testid="inline" />
        <Code code={"a\n  b"} block data-testid="block" />
      </>,
    );
    const inline = screen.getByTestId("inline");
    expect(inline.tagName).toBe("CODE");
    expect(inline.classList.contains("omfp-code")).toBe(true);
    const block = screen.getByTestId("block");
    expect(block.tagName).toBe("PRE");
    expect(block.querySelector("code")?.textContent).toBe("a\n  b");
  });
});

/** Shows what the kit hands a thread link: its id, whether its text is bare, and that text. */
function ThreadProbe({ threadId, bare, children }: MarkdownThreadLinkProps) {
  return (
    <span data-testid="thread-probe" data-thread-id={threadId} data-bare={String(bare)}>
      {children}
    </span>
  );
}

describe("thread links", () => {
  // spec: UI-045
  test("omfp://thread/<id> names a thread, its id decoded; nothing else does", () => {
    expect(threadLinkId("omfp://thread/01ABC")).toBe("01ABC");
    expect(threadLinkId("OMFP://Thread/01ABC/")).toBe("01ABC");
    expect(threadLinkId("omfp://thread/01ABC#prompt-2")).toBe("01ABC");
    expect(threadLinkId("omfp://thread/a%20b")).toBe("a b");
    expect(threadLinkId(threadLinkUrl("t/1#x"))).toBe("t/1#x");
    for (const href of [
      undefined,
      "",
      "omfp://thread/",
      "omfp://thread/%E0",
      "omfp://threads/01ABC",
      "omfp://docs/widgets",
      "#thread=01ABC",
      "https://example.com/omfp://thread/01ABC",
      "docs/a.md",
    ]) {
      expect([href, threadLinkId(href)]).toEqual([href, undefined]);
    }
  });

  // spec: UI-045
  test("with threadLink, a link to a thread renders through it; other links and paths keep their own rendering", () => {
    render(
      <Markdown
        data-testid="md"
        threadLink={ThreadProbe}
        onOpenPath={() => {}}
        text={
          "See [the parser fix](omfp://thread/01FIX), <omfp://thread/01AUTO>, [01BARE](omfp://thread/01BARE), " +
          "[docs](https://example.com/) and [notes](docs/notes.md)."
        }
      />,
    );
    const probes = screen.getAllByTestId("thread-probe");
    expect(probes.map((probe) => [probe.dataset.threadId, probe.dataset.bare, probe.textContent])).toEqual([
      ["01FIX", "false", "the parser fix"],
      ["01AUTO", "true", "omfp://thread/01AUTO"],
      ["01BARE", "true", "01BARE"],
    ]);
    const root = screen.getByTestId("md");
    expect([...root.querySelectorAll("a")].map((link) => [link.getAttribute("href"), link.target])).toEqual([
      ["https://example.com/", "_blank"],
    ]);
    expect(screen.getAllByTestId("path-link").map((link) => link.dataset.omfpPath)).toEqual(["docs/notes.md"]);
  });

  // spec: UI-046
  test("without threadLink, a link to a thread is its text, never a dead link", () => {
    const root = renderMarkdown("Done in [the fix](omfp://thread/01FIX).");
    expect(root.querySelector("a")).toBeNull();
    expect(screen.getByTestId("thread-link-text").textContent).toBe("the fix");
    expect(root.textContent).toBe("Done in the fix.");
  });
});
