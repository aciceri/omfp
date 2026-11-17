import { describe, expect, test } from "bun:test";
import { compactPrompt, parseCompact, tokensBefore } from "../../src/vm/compaction.ts";

describe("compaction arguments", () => {
  // spec: UI-910, UI-913
  test("a first word naming a mode, in any case, is the mode; the rest is the focus", () => {
    expect(parseCompact("")).toEqual({ mode: undefined, focus: "" });
    expect(parseCompact("  keep the API  ")).toEqual({ mode: undefined, focus: "keep the API" });
    expect(parseCompact("REMOTE keep the API")).toEqual({ mode: "remote", focus: "keep the API" });
    expect(parseCompact("snapcompact")).toEqual({ mode: "snapcompact", focus: "" });
    expect(parseCompact("softly please")).toEqual({ mode: undefined, focus: "softly please" });
    expect([compactPrompt("soft", ""), compactPrompt("remote", "the plan")]).toEqual([
      "/compact soft",
      "/compact remote the plan",
    ]);
  });

  // spec: UI-911
  test("the size before is omp's positive tokensBefore", () => {
    expect([tokensBefore({ tokensBefore: 1200 }), tokensBefore({ tokensBefore: 0 }), tokensBefore(null)]).toEqual([
      1200,
      undefined,
      undefined,
    ]);
  });
});
