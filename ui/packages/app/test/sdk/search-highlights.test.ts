/** The conversation search's shared highlights (UI-736): every transcript's marks, one owner's repaint keeps the rest. */

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
  paintSearchHighlights,
  SEARCH_CURRENT_HIGHLIGHT,
  SEARCH_MATCH_HIGHLIGHT,
} from "../../src/sdk/search-highlights.ts";

/** A stand-in for `Highlight`, which happy-dom lacks: a set of ranges. */
class FakeHighlight extends Set<Range> {
  priority = 0;
  constructor(...ranges: Range[]) {
    super(ranges);
  }
}

let saved: PropertyDescriptor | undefined;
beforeEach(() => {
  saved = Object.getOwnPropertyDescriptor(globalThis, "Highlight");
  Object.defineProperty(globalThis, "Highlight", { configurable: true, value: FakeHighlight });
});
afterEach(() => {
  if (saved === undefined) {
    Reflect.deleteProperty(globalThis, "Highlight");
  } else {
    Object.defineProperty(globalThis, "Highlight", saved);
  }
});

/** A range over `text` in a fresh text node. */
function rangeOver(text: string): Range {
  const range = document.createRange();
  range.selectNodeContents(document.createTextNode(text));
  return range;
}

describe("search highlights", () => {
  // spec: UI-736
  test("transcripts share the two highlights: each repaint keeps the others' marks, closing one leaves the rest", () => {
    const registry = new Map<string, FakeHighlight>();
    const highlights = registry as unknown as HighlightRegistry;
    const marked = (name: string) => [...(registry.get(name) ?? [])].map(String).sort();
    const builtIn = {};
    const copy = {};
    paintSearchHighlights(highlights, builtIn, { matches: [rangeOver("a1")], current: rangeOver("a0") });
    paintSearchHighlights(highlights, copy, { matches: [rangeOver("b1"), rangeOver("b2")], current: undefined });
    expect([marked(SEARCH_MATCH_HIGHLIGHT), marked(SEARCH_CURRENT_HIGHLIGHT)]).toEqual([["a1", "b1", "b2"], ["a0"]]);
    expect([registry.get(SEARCH_MATCH_HIGHLIGHT)?.priority, registry.get(SEARCH_CURRENT_HIGHLIGHT)?.priority]).toEqual([
      0, 1,
    ]);
    // The built-in transcript repaints, then closes: the copy's marks stay.
    paintSearchHighlights(highlights, builtIn, { matches: [], current: rangeOver("a3") });
    expect([marked(SEARCH_MATCH_HIGHLIGHT), marked(SEARCH_CURRENT_HIGHLIGHT)]).toEqual([["b1", "b2"], ["a3"]]);
    paintSearchHighlights(highlights, builtIn, undefined);
    expect([marked(SEARCH_MATCH_HIGHLIGHT), registry.has(SEARCH_CURRENT_HIGHLIGHT)]).toEqual([["b1", "b2"], false]);
    paintSearchHighlights(highlights, copy, undefined);
    expect([...registry.keys()]).toEqual([]);
  });
});
