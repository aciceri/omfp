/** L1: what the window writes into views.json itself (`:widget add`, UI-300) and the splitters' sizes (UI-206). */

import { describe, expect, test } from "bun:test";
import { addWidget, manifestWithWidget, RESIZE_STEP, resizeSizes } from "../../src/view/edit.ts";
import type { LayoutNode } from "../../src/view/layout.ts";
import { DEFAULT_VIEW } from "../../src/view/views.ts";

/** The default view's `main` tabs, holding the chat. */
const MAIN_TABS: LayoutNode = ["tabs", { id: "main" }, "omfp:chat"];

describe("resize", () => {
  // spec: UI-206
  test("a keyboard step moves 5 % of the container between two adjacent cells, each keeping at least 5 %", () => {
    expect(resizeSizes([3, 1], 1, RESIZE_STEP)).toEqual([3.2, 0.8]);
    expect(resizeSizes([1, 1, 1], 2, -RESIZE_STEP)).toEqual([1, 0.85, 1.15]);
    expect(resizeSizes([1, 1], 1, 0.25)).toEqual([1.5, 0.5]);
    expect(resizeSizes([1, 1], 1, 2)).toEqual([1.9, 0.1]);
  });
});

describe("add", () => {
  // spec: UI-300
  test("as the last tab of the side tabs, else at the end of the root container; a root widget becomes a row", () => {
    expect(addWidget(DEFAULT_VIEW.layout, "user:charts")).toEqual([
      "row",
      { sizes: [3, 1] },
      ["col", {}, "omfp:header", MAIN_TABS],
      ["tabs", { id: "side" }, "omfp:workspace", "omfp:changes", "omfp:background", "omfp:mcp", "user:charts"],
    ]);
    expect(addWidget(["row", { sizes: [1, 3] }, "a", "b"], "./widgets/tests")).toEqual([
      "row",
      { sizes: [1, 3, 2] },
      "a",
      "b",
      "./widgets/tests",
    ]);
    expect(addWidget(["tabs", {}, "a"], "./widgets/tests")).toEqual(["tabs", {}, "a", "./widgets/tests"]);
    expect(addWidget(["omfp:chat", { instance: "x" }], "omfp:header")).toEqual([
      "row",
      {},
      ["omfp:chat", { instance: "x" }],
      "omfp:header",
    ]);
  });
});

describe("manifest to write", () => {
  // spec: UI-300
  test("without views.json: a new manifest holding the default view with the widget", () => {
    expect(manifestWithWidget({ manifest: null, error: null }, "default", "omfp:transcript")).toEqual({
      ok: true,
      value: {
        $schema: "omfp://schemas/views.json",
        default: "default",
        views: {
          default: {
            title: "Default",
            layout: [
              "row",
              { sizes: [3, 1] },
              ["col", {}, "omfp:header", MAIN_TABS],
              [
                "tabs",
                { id: "side" },
                "omfp:workspace",
                "omfp:changes",
                "omfp:background",
                "omfp:mcp",
                "omfp:transcript",
              ],
            ],
          },
        },
      },
    });
  });

  // spec: UI-300
  test("with views.json: that view's layout changes, everything else stays; never an invalid file or a TSX view", () => {
    const manifest = {
      default: "work",
      comment: "kept",
      views: {
        ops: { title: "Ops", view: "./views/ops.tsx" },
        work: { title: "Work", layout: ["row", {}, "omfp:chat", "omfp:workspace"] },
      },
    };
    expect(manifestWithWidget({ manifest }, "work", "omfp:todos")).toEqual({
      ok: true,
      value: {
        ...manifest,
        views: {
          ...manifest.views,
          work: { title: "Work", layout: ["row", {}, "omfp:chat", "omfp:workspace", "omfp:todos"] },
        },
      },
    });
    expect(manifestWithWidget({ manifest }, "gone", "omfp:todos")).toEqual({ ok: false, error: "No view gone" });
    expect(manifestWithWidget({ manifest }, "ops", "omfp:todos")).toEqual({ ok: false, error: "No view ops" });
    expect(manifestWithWidget({ manifest: null, error: "views: no view" }, "default", "omfp:chat")).toEqual({
      ok: false,
      error: "The layout cannot change while .omfp/views.json is invalid: fix the file first",
    });
  });
});
