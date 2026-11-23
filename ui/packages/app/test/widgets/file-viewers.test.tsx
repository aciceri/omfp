/** L4: `omfp:file`'s viewers of text formats over a fake client: Markdown preview (UI-560..562), table (UI-563..565),
 * tree (UI-567..569). */

import { afterAll, afterEach, beforeAll, describe, expect, test } from "bun:test";
import type { FileContent } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Markdown } from "omfp/kit";
import { type FilePoll, OPEN_FILES, SELECTED_THREAD, SUCCESSES } from "../../src/sdk/index.ts";
import { FileWidget } from "../../src/widgets/file/index.ts";
import { FileImage } from "../../src/widgets/file/markdown.tsx";
import { viewerOf } from "../../src/widgets/file/viewers.tsx";
import { repo, thread, workspace } from "../support/fixtures.ts";
import { FakeClient, renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

/** A poll that never runs: these tests read each file once. */
const NO_POLL: FilePoll = () => () => undefined;

function text(path: string, content: string, extra: Partial<FileContent> = {}): FileContent {
  return {
    path,
    kind: "text",
    size: content.length,
    hash: `h-${content.length}`,
    unchanged: false,
    content,
    truncated: false,
    repo: "fe",
    head: content,
    ...extra,
  };
}

/** `omfp:file` showing `path`, which holds `content` on disk (in the repository `fe`). */
function setup(path: string, content: string, extra: Partial<FileContent> = {}) {
  const base = testHost(new FakeClient(), { filePoll: NO_POLL });
  base.host.window.set(SELECTED_THREAD, "t1");
  base.client.answers.set("workspaces.read_file", () => text(path, content, extra));
  const test = renderWithHost(<FileWidget path={path} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1", [repo("w1", "fe", "feat")])],
    threads: [thread("t1", "one", "w1")],
  });
  test.flush();
  return { ...test, user: userEvent.setup() };
}

async function editorText(): Promise<string> {
  const editor = await screen.findByTestId("file-editor");
  let shown = "";
  await waitFor(() => {
    expect(editor.querySelector(".cm-content")).not.toBeNull();
    shown = [...editor.querySelectorAll(".cm-line")].map((line) => line.textContent).join("\n");
  });
  return shown;
}

const README =
  "# Guide\n\nRead [the next part](next.md#L3) and [the root](../README.md).\n\n<script>window.pwned = 1</script>\n";

describe("Markdown preview", () => {
  // spec: UI-560
  test("Markdown files read-only show the preview; the toggle shows the source and back; other files have none", async () => {
    const test = setup("fe/docs/guide.md", README);
    const preview = await screen.findByTestId("file-viewer");
    expect(preview.dataset.viewer).toBe("markdown");
    expect(preview.querySelector("h1")?.textContent).toBe("Guide");
    // No raw HTML (UI-040).
    expect(preview.querySelector("script")).toBeNull();
    expect(screen.queryAllByTestId("file-editor").length).toBe(0);
    const toggle = screen.getByTestId("file-view-toggle");
    expect([toggle.textContent, toggle.getAttribute("aria-pressed")]).toEqual(["Preview", "true"]);
    await test.user.click(toggle);
    expect(await editorText()).toBe(README);
    expect(screen.queryAllByTestId("file-viewer").length).toBe(0);
    expect(screen.getByTestId("file-view-toggle").getAttribute("aria-pressed")).toBe("false");
    await test.user.click(screen.getByTestId("file-view-toggle"));
    expect((await screen.findByTestId("file-viewer")).dataset.viewer).toBe("markdown");
    cleanup();
    for (const path of ["notes.markdown", "page.mdx", "README.MD"]) {
      setup(path, "# Title\n");
      expect([path, (await screen.findByTestId("file-viewer")).dataset.viewer]).toEqual([path, "markdown"]);
      cleanup();
    }
    setup("src/lib.rs", "fn one() {}\n");
    expect(await editorText()).toBe("fn one() {}\n");
    expect(screen.queryAllByTestId("file-view-toggle").length).toBe(0);
  });

  // spec: UI-560
  test("editing shows the source and Done the preview again; the diff takes the preview's place while it is on", async () => {
    const test = setup("fe/docs/guide.md", README);
    await screen.findByTestId("file-viewer");
    await test.user.click(screen.getByTestId("file-edit"));
    expect(await editorText()).toBe(README);
    expect(screen.getByTestId("file-editor").dataset.readOnly).toBe("false");
    expect(screen.queryAllByTestId("file-view-toggle").length).toBe(0);
    await test.user.click(screen.getByTestId("file-done"));
    expect((await screen.findByTestId("file-viewer")).dataset.viewer).toBe("markdown");
    await test.user.click(screen.getByTestId("file-diff"));
    await screen.findByTestId("file-head-diff");
    expect(screen.queryAllByTestId("file-viewer").length).toBe(0);
    expect(screen.getByTestId("file-view-toggle").getAttribute("aria-pressed")).toBe("false");
    await test.user.click(screen.getByTestId("file-view-toggle"));
    expect(screen.queryAllByTestId("file-head-diff").length).toBe(0);
    expect(screen.getByTestId("file-diff").getAttribute("aria-pressed")).toBe("false");
    await screen.findByTestId("file-viewer");
  });

  // spec: UI-561
  test("relative links open the file they name relative to the Markdown file's folder", async () => {
    const test = setup("fe/docs/guide.md", README);
    await screen.findByTestId("file-viewer");
    const links = screen.getAllByTestId("path-link");
    expect(links.map((link) => link.dataset.omfpPath)).toEqual(["fe/docs/next.md", "fe/README.md"]);
    await test.user.click(links[0] ?? document.body);
    expect(test.host.window.get(OPEN_FILES).w1?.tabs.map(({ path, line }) => ({ path, line }))).toEqual([
      { path: "fe/docs/next.md", line: 3 },
    ]);
  });

  // spec: UI-562
  test("relative images load from the SDK's file URL, resolved against the Markdown file's folder", async () => {
    setup("fe/docs/guide.md", "![the chart](img/chart.png)\n\n![remote](https://example.com/r.png)\n");
    const preview = await screen.findByTestId("file-viewer");
    expect(screen.queryAllByTestId("file-image-alt").length).toBe(0);
    const sources = [...preview.querySelectorAll("img")].map((img) => img.getAttribute("src") ?? "");
    expect(sources.length).toBe(2);
    expect(sources[0]).toEndWith("/w1/fe/docs/img/chart.png");
    expect(sources[1]).toBe("https://example.com/r.png");
  });

  // spec: UI-562
  test("relative images show their alt text until the file's URL is known", () => {
    const test = renderWithHost(
      <Markdown data-testid="md" linkBase="docs" image={FileImage} text={"![chart](img/chart.png)"} />,
    );
    expect(screen.getByTestId("file-image-alt").textContent).toBe("chart");
    act(() => {
      test.client.connected();
      test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "one", "w1")] });
      test.host.window.set(SELECTED_THREAD, "t1");
      test.flush();
    });
    const image = screen.getByTestId("md").querySelector("img");
    expect([image?.getAttribute("src"), image?.getAttribute("alt")]).toEqual([
      "/files/test/w1/docs/img/chart.png",
      "chart",
    ]);
    expect(screen.queryAllByTestId("file-image-alt").length).toBe(0);
  });

  // spec: UI-560, UI-563, UI-567
  test("the viewer is chosen by the file's extension, case aside", () => {
    const paths = ["a.md", "b/c.MARKDOWN", "d.mdx", "e.rs", "md", "f.md.bak", "g.csv", "h.Tsv", "csv"];
    expect(paths.map((path) => viewerOf(path)?.name)).toEqual([
      "markdown",
      "markdown",
      "markdown",
      undefined,
      undefined,
      undefined,
      "table",
      "table",
      undefined,
    ]);
    expect(["a.json", "b.JSONC", "c.yaml", "d.yml"].map((path) => viewerOf(path)?.name)).toEqual([
      "tree",
      "tree",
      "tree",
      "tree",
    ]);
  });
});

/** The cells of the table viewer's rendered body rows. */
function bodyRows(): string[][] {
  return [...screen.getByTestId("file-table").querySelectorAll("tbody tr[data-index]")].map((row) =>
    [...row.querySelectorAll("td")].map((cell) => cell.textContent ?? ""),
  );
}

const CSV = 'name,size,note\nbeta,10,"two, words"\nalpha,9,\ngamma,100,"line\nbreak"\n';

describe("table viewer", () => {
  // happy-dom lays nothing out: the table's scroll box gets a 600 px view, the height the virtualizer reads.
  const clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
  beforeAll(() => {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList.contains("omfp-file__table") ? 600 : 0;
      },
    });
  });
  afterAll(() => {
    if (clientHeight === undefined) {
      Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
    } else {
      Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
    }
  });

  // spec: UI-563, UI-566
  test("CSV and TSV files show as a table under their first record; the toggle shows the source", async () => {
    const test = setup("fe/data/sizes.csv", CSV);
    const viewer = await screen.findByTestId("file-viewer");
    expect(viewer.dataset.viewer).toBe("table");
    expect([...viewer.querySelectorAll("thead th")].map((cell) => cell.textContent)).toEqual(["name", "size", "note"]);
    expect(bodyRows()).toEqual([
      ["beta", "10", "two, words"],
      ["alpha", "9", ""],
      ["gamma", "100", "line\nbreak"],
    ]);
    expect(screen.getByTestId("file-table-summary").textContent).toBe("3 rows, 3 columns");
    const toggle = screen.getByTestId("file-view-toggle");
    expect([toggle.textContent, toggle.getAttribute("aria-pressed")]).toEqual(["Table", "true"]);
    await test.user.click(toggle);
    expect((await editorText()).split("\n")[0]).toBe("name,size,note");
    cleanup();
    setup("fe/data/sizes.TSV", "a\tb\n1\t2\n");
    await screen.findByTestId("file-viewer");
    expect(bodyRows()).toEqual([["1", "2"]]);
  });

  // spec: UI-564
  test("a click on a header sorts by its column, numbers by value, ascending then descending", async () => {
    const test = setup("fe/data/sizes.csv", CSV);
    await screen.findByTestId("file-viewer");
    const header = () => screen.getAllByTestId("file-table-sort")[1] ?? document.body;
    const sorted = () => screen.getByTestId("file-table").querySelectorAll("th")[1]?.getAttribute("aria-sort");
    expect(sorted()).toBe("none");
    await test.user.click(header());
    expect([sorted(), bodyRows().map((row) => row[0])]).toEqual(["ascending", ["alpha", "beta", "gamma"]]);
    await test.user.click(header());
    expect([sorted(), bodyRows().map((row) => row[0])]).toEqual(["descending", ["gamma", "beta", "alpha"]]);
    // Another column sorts ascending first; empty cells go last.
    await test.user.click(screen.getAllByTestId("file-table-sort")[2] ?? document.body);
    expect(bodyRows().map((row) => row[0])).toEqual(["gamma", "beta", "alpha"]);
    expect(sorted()).toBe("none");
  });

  // spec: UI-565
  test("only the rows in view are rendered; a file the daemon cut says so and drops its incomplete last row", async () => {
    const rows = Array.from({ length: 5000 }, (_, index) => `row ${index},${index}`);
    setup("big.csv", `name,n\n${rows.join("\n")}\n`);
    await screen.findByTestId("file-viewer");
    expect(screen.getByTestId("file-table-summary").textContent).toBe("5000 rows, 2 columns");
    const rendered = bodyRows().length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(100);
    expect(screen.getByTestId("file-table").querySelector("table")?.getAttribute("aria-rowcount")).toBe("5001");
    // Scrolled to the 3000th row, the rows around it replace the first ones.
    const scroller = screen.getByTestId("file-table");
    Object.defineProperty(scroller, "scrollTop", { configurable: true, value: 3000 * 28 });
    act(() => {
      fireEvent.scroll(scroller);
    });
    await waitFor(() => expect(bodyRows().some((row) => row[0] === "row 3000")).toBe(true));
    expect(bodyRows().some((row) => row[0] === "row 0")).toBe(false);
    expect(bodyRows().length).toBeLessThan(100);
    cleanup();
    setup("cut.csv", 'name,n\na,1\nb,"2', { truncated: true, size: 4 * 1024 * 1024 });
    await screen.findByTestId("file-viewer");
    expect(bodyRows()).toEqual([["a", "1"]]);
    expect(screen.getByTestId("file-table-summary").textContent).toBe(
      "1 row, 2 columns: the first rows of this 4.0 MiB file, which is too large to read whole",
    );
  });
});

/** The tree's rendered nodes as `path kind summary`. */
function nodes(): string[] {
  return screen
    .getAllByTestId("file-tree-node")
    .map(
      (node) => `${node.dataset.path} ${node.dataset.kind} ${node.querySelector(".omfp-file__summary")?.textContent}`,
    );
}

/** The tree node at `path`. */
function node(path: string): HTMLElement {
  const found = screen.getAllByTestId("file-tree-node").find((element) => element.dataset.path === path);
  if (found === undefined) {
    throw new Error(`no node ${path}`);
  }
  return found;
}

/** The node's own control (not its children's) with `testId`. */
function control(path: string, testId: string): HTMLElement {
  const found = node(path).querySelector(`:scope > .omfp-file__node [data-testid="${testId}"]`);
  if (!(found instanceof HTMLElement)) {
    throw new Error(`no ${testId} on ${path}`);
  }
  return found;
}

const PACKAGE = '{"name": "omfp", "version": 1.10, "private": true, "files": ["a", "b c"], "bin": null, "odd key": {}}';

describe("tree viewer", () => {
  // spec: UI-567
  test("JSON shows as a tree: the root open, keys with their types and counts, containers opened on click", async () => {
    const test = setup("fe/package.json", PACKAGE);
    expect((await screen.findByTestId("file-viewer")).dataset.viewer).toBe("tree");
    expect(nodes()).toEqual([
      ". object 6 keys",
      ".name string string",
      ".version number number",
      ".private boolean boolean",
      ".files array 2 items",
      ".bin null null",
      '.["odd key"] object 0 keys',
    ]);
    expect(node(".name").textContent).toContain('"omfp"');
    expect(node(".version").textContent).toContain("1.10");
    expect(control(".files", "file-tree-toggle").getAttribute("aria-expanded")).toBe("false");
    await test.user.click(control(".files", "file-tree-toggle"));
    expect(control(".files", "file-tree-toggle").getAttribute("aria-expanded")).toBe("true");
    expect(nodes().slice(5, 7)).toEqual([".files[0] string string", ".files[1] string string"]);
    await test.user.click(control(".", "file-tree-toggle"));
    expect(nodes()).toEqual([". object 6 keys"]);
    const toggle = screen.getByTestId("file-view-toggle");
    expect([toggle.textContent, toggle.getAttribute("aria-pressed")]).toEqual(["Tree", "true"]);
    await test.user.click(toggle);
    expect((await editorText()).startsWith('{"name"')).toBe(true);
  });

  // spec: UI-568
  test("each node copies its path or its value, with a success toast", async () => {
    const test = setup("fe/package.json", PACKAGE);
    await screen.findByTestId("file-viewer");
    await test.user.click(control('.["odd key"]', "file-tree-copy-path"));
    expect(await navigator.clipboard.readText()).toBe('.["odd key"]');
    await test.user.click(control(".name", "file-tree-copy-value"));
    expect(await navigator.clipboard.readText()).toBe("omfp");
    await test.user.click(control(".files", "file-tree-copy-value"));
    expect(await navigator.clipboard.readText()).toBe('[\n  "a",\n  "b c"\n]');
    await test.user.click(control(".version", "file-tree-copy-value"));
    expect(await navigator.clipboard.readText()).toBe("1.10");
    await waitFor(() =>
      expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual([
        'Copied the path .["odd key"]',
        "Copied the value of .name",
        "Copied the value of .files",
        "Copied the value of .version",
      ]),
    );
  });

  // spec: UI-567, UI-570
  test("JSONC takes comments and trailing commas; YAML shows its documents", async () => {
    setup("fe/tsconfig.json", '{\n  // strict\n  "compilerOptions": { "strict": true, },\n}\n');
    await screen.findByTestId("file-viewer");
    expect(nodes()).toEqual([". object 1 key", ".compilerOptions object 1 key"]);
    cleanup();
    setup("fe/ci.yml", "on: push\njobs:\n  test:\n    steps: [a, b]\n");
    await waitFor(() => expect(nodes()).toEqual([". object 2 keys", ".on string string", ".jobs object 1 key"]));
    cleanup();
    setup("fe/k8s.yaml", "kind: A\n---\nkind: B\n");
    await waitFor(() => expect(nodes()).toEqual([". array 2 documents", ".[0] object 1 key", ".[1] object 1 key"]));
  });

  // spec: UI-569
  test("an invalid file says where it fails and shows its source instead; the toggle tries again", async () => {
    const test = setup("fe/data.json", '{\n  "a": 1,\n  "b" 2\n}\n');
    const error = await screen.findByTestId("file-viewer-error");
    expect(error.textContent).toBe("Cannot show this file as a tree: Expected `:` (line 3, column 7).");
    expect((await editorText()).split("\n")[2]).toBe('  "b" 2');
    expect(screen.queryAllByTestId("file-viewer").length).toBe(0);
    expect(screen.getByTestId("file-view-toggle").getAttribute("aria-pressed")).toBe("false");
    // The toggle tries again, and fails again.
    await test.user.click(screen.getByTestId("file-view-toggle"));
    expect((await screen.findByTestId("file-viewer-error")).textContent).toContain("line 3, column 7");
    cleanup();
    setup("fe/ci.yaml", "a: [1, 2\nb: 3\n");
    expect((await screen.findByTestId("file-viewer-error")).textContent).toMatch(/\(line \d+, column \d+\)\.$/);
  });
});
