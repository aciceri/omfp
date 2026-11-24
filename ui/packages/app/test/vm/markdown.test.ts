import { describe, expect, test } from "bun:test";
import { markdownChunks } from "../../src/vm/markdown.ts";

describe("streaming Markdown chunks", () => {
  // spec: UI-191
  test("top-level blocks after a blank line start chunks; joined, the chunks are the text", () => {
    const text = "# Title\n\nFirst paragraph\nstill first.\n\nSecond paragraph.\n\n```ts\nconst a = 1;\n```\n\nEnd";
    const chunks = markdownChunks(text);
    expect(chunks).toEqual([
      "# Title\n\n",
      "First paragraph\nstill first.\n\n",
      "Second paragraph.\n\n",
      "```ts\nconst a = 1;\n```\n\n",
      "End",
    ]);
    expect(chunks.join("")).toBe(text);
  });

  // spec: UI-191
  test("blank lines inside a fence, a list or a block quote, and indented lines, do not split", () => {
    const fenced = "Intro\n\n~~~~\nfn a() {}\n\nfn b() {}\n~~~\nstill code\n~~~~\n\nAfter";
    expect(markdownChunks(fenced)).toEqual([
      "Intro\n\n",
      "~~~~\nfn a() {}\n\nfn b() {}\n~~~\nstill code\n~~~~\n\n",
      "After",
    ]);
    const list = "- one\n\n- two\n\n  more of two\n\n1. three\n\n> quote\n\n> more\n\n    indented code";
    expect(markdownChunks(list)).toEqual([list]);
  });

  // spec: UI-191, UI-850
  test("blank lines inside a `$$` math block do not split; a `$$x$$` line opens no block", () => {
    const math = "Intro\n\n$$\na\n\nb\n$$\n\nAfter";
    expect(markdownChunks(math)).toEqual(["Intro\n\n", "$$\na\n\nb\n$$\n\n", "After"]);
    expect(markdownChunks("$$x$$\n\nNext")).toEqual(["$$x$$\n\n", "Next"]);
  });

  // spec: UI-191
  test("a text with a link reference definition stays whole", () => {
    const text = "See [the docs][docs].\n\nMore.\n\n[docs]: https://example.com";
    expect(markdownChunks(text)).toEqual([text]);
  });
});
