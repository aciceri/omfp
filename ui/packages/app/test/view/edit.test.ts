/** L1: what the window writes into views.json itself (`:widget add`, UI-300) and the splitters' sizes (UI-206). */

import { describe, expect, test } from "bun:test";
import { addWidget, manifestWithoutTab, manifestWithWidget, RESIZE_STEP, resizeSizes } from "../../src/view/edit.ts";
import { keysAt, type LayoutNode, parseLayout } from "../../src/view/layout.ts";
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

/** A widget node as the renderer shows it. */
function widget(name: string, props: Record<string, unknown> = {}) {
  return { kind: "widget", name, props } as const;
}

describe("closing a layout tab", () => {
  // spec: UI-375
  test("its keys from the view's root are the identities of its node and its ancestors among their siblings", () => {
    const layout = ["row", {}, ["tabs", {}, "omfp:changes", "omfp:changes", ["x", { instance: "a" }]], "omfp:chat"];
    const parsed = parseLayout(layout);
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    expect(keysAt(parsed.value, "0.0.1")).toEqual(["tabs#0", "omfp:changes#1"]);
    expect(keysAt(parsed.value, "0.0.2")).toEqual(["tabs#0", "a#0"]);
    expect(keysAt(parsed.value, "0.3")).toBeUndefined();
    expect(keysAt(parsed.value, "1.0")).toBeUndefined();
  });

  // spec: UI-375
  test("without views.json: the default view minus that tab", () => {
    const changes = manifestWithoutTab(
      { manifest: null, error: null },
      "default",
      ["tabs#0", "omfp:changes#0"],
      widget("omfp:changes"),
    );
    expect(changes).toEqual({
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
              ["tabs", { id: "side" }, "omfp:workspace", "omfp:background", "omfp:mcp"],
            ],
          },
        },
      },
    });
  });

  // spec: UI-375
  test("with views.json: that tab and its weight go, everything else stays", () => {
    const manifest = {
      default: "work",
      comment: "kept",
      panels: { notes: "./widgets/notes" },
      views: {
        ops: { title: "Ops", view: "./views/ops.tsx" },
        work: {
          title: "Work",
          layout: [
            "row",
            { sizes: [2, 1] },
            ["tabs", { id: "main" }, "omfp:chat", ["./widgets/tests", { instance: "tests" }]],
            ["tabs", { id: "side", sizes: [1, 2, 3] }, "omfp:workspace", ["col", {}, "omfp:todos"], "omfp:tree"],
          ],
        },
      },
    };
    const tests = widget("./widgets/tests", { instance: "tests" });
    expect(manifestWithoutTab({ manifest }, "work", ["tabs#0", "tests#0"], tests)).toEqual({
      ok: true,
      value: {
        ...manifest,
        views: {
          ...manifest.views,
          work: {
            title: "Work",
            layout: [
              "row",
              { sizes: [2, 1] },
              ["tabs", { id: "main" }, "omfp:chat"],
              ["tabs", { id: "side", sizes: [1, 2, 3] }, "omfp:workspace", ["col", {}, "omfp:todos"], "omfp:tree"],
            ],
          },
        },
      },
    });
    const col = { kind: "col", props: {}, children: [widget("omfp:todos")] } as const;
    expect(manifestWithoutTab({ manifest }, "work", ["tabs#1", "col#0"], col)).toEqual({
      ok: true,
      value: {
        ...manifest,
        views: {
          ...manifest.views,
          work: {
            title: "Work",
            layout: [
              "row",
              { sizes: [2, 1] },
              ["tabs", { id: "main" }, "omfp:chat", ["./widgets/tests", { instance: "tests" }]],
              ["tabs", { id: "side", sizes: [1, 3] }, "omfp:workspace", "omfp:tree"],
            ],
          },
        },
      },
    });
  });

  // spec: UI-375
  test("refused: an invalid file, a view in code, no such tab, a tab alone in its container, another node", () => {
    const manifest = {
      views: {
        ops: { title: "Ops", view: "./views/ops.tsx" },
        work: {
          layout: ["row", {}, ["tabs", {}, "omfp:chat"], ["tabs", {}, "omfp:workspace", ["omfp:todos", { a: 1 }]]],
        },
      },
    };
    const todos = widget("omfp:todos", { a: 1 });
    const invalid = {
      ok: false,
      error: "The layout cannot change while .omfp/views.json is invalid: fix the file first",
    } as const;
    expect(
      manifestWithoutTab({ manifest: null, error: "views: no view" }, "default", ["tabs#1", "omfp:todos#0"], todos),
    ).toEqual(invalid);
    expect(manifestWithoutTab({ manifest }, "ops", ["tabs#1", "omfp:todos#0"], todos)).toEqual({
      ok: false,
      error: "No view ops",
    });
    const gone = { ok: false, error: "View work no longer has that tab: .omfp/views.json changed meanwhile" } as const;
    // A row's cell is no tab, nor is a key no child has; the last tab of a container stays.
    expect(manifestWithoutTab({ manifest }, "work", ["tabs#1"], todos)).toEqual(gone);
    expect(manifestWithoutTab({ manifest }, "work", ["tabs#1", "omfp:tree#0"], todos)).toEqual(gone);
    expect(manifestWithoutTab({ manifest }, "work", ["tabs#0", "omfp:chat#0"], widget("omfp:chat"))).toEqual(gone);
    // The node there is no longer the one the window showed.
    expect(manifestWithoutTab({ manifest }, "work", ["tabs#1", "omfp:todos#0"], widget("omfp:todos"))).toEqual(gone);
    expect(manifestWithoutTab({ manifest }, "work", ["tabs#1", "omfp:todos#0"], todos)).toMatchObject({ ok: true });
  });

  // spec: UI-375
  test("keys find the tab in the same container whatever an earlier close moved", () => {
    // The window showed [A, tabs(X, Y), tabs(X, Y, Z)] and closed A; Y of the first nested tabs is still found there.
    const layout = ["tabs", {}, ["tabs", {}, "x", "y"], ["tabs", {}, "x", "y", "z"]];
    const manifest = { views: { work: { layout } } };
    expect(manifestWithoutTab({ manifest }, "work", ["tabs#0", "y#0"], widget("y"))).toEqual({
      ok: true,
      value: { views: { work: { layout: ["tabs", {}, ["tabs", {}, "x"], ["tabs", {}, "x", "y", "z"]] } } },
    });
  });
});
