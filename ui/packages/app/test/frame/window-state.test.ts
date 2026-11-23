/** L1: the window state the window reports for agents (UI-350, UI-351, PRT-122). */

import { describe, expect, test } from "bun:test";
import { widgetsOf, windowReport } from "../../src/frame/window-state.ts";
import type { Binding } from "../../src/keymap/keymap.ts";
import type { Command } from "../../src/sdk/index.ts";
import { parseLayout } from "../../src/view/layout.ts";
import { BUILTIN_VIEW, type ResolvedView, type Views } from "../../src/view/views.ts";

function resolved(id: string, layout: unknown, extra: Partial<ResolvedView> = {}): ResolvedView {
  const tree = parseLayout(layout);
  if (!tree.ok) {
    throw new Error(tree.error);
  }
  return { id, title: id.toUpperCase(), layout: layout as ResolvedView["layout"], tree: tree.value, ...extra };
}

const work = resolved("work", ["row", {}, ["tabs", { id: "main" }, "omfp:chat"], "./widgets/tests", "omfp:chat"]);
const solo = resolved("solo", "omfp:chat", { init: "workspace" });
const ops = resolved("ops", ["row", {}, "omfp:chat", "./widgets/tests"], { init: "user" });
const views: Views = { views: [ops, solo, work], defaultId: "work", panels: [], error: undefined };

const noop = () => {};
const commands: Command[] = [
  { id: "view.switch", title: "Switch view", aliases: ["view"], run: noop },
  { id: "demo.solo", title: "Show solo", init: "workspace", aliases: ["solo"], run: noop },
  { id: "tests.run", title: "Run tests", widget: "./widgets/tests", enabled: () => false, run: noop },
];
const bindings: Binding[] = [
  { mode: "normal", keys: "g v", command: "view.pick" },
  { mode: "normal", keys: "g h", command: "demo.solo", source: "init:workspace" },
  { mode: "insert", keys: "Ctrl-t", command: "tests.run", args: ["unit"], source: "workspace" },
];

describe("window state", () => {
  // spec: UI-350
  test("each view says where it comes from and its widgets", () => {
    const report = windowReport({
      workspaceId: "w1",
      threadId: "t1",
      focused: true,
      views,
      shown: work,
      commands: [],
      bindings,
    });
    expect(report).toEqual({
      threadId: "t1",
      focused: true,
      activeView: "work",
      defaultView: "work",
      views: [
        { id: "ops", title: "OPS", source: "user_init", widgets: ["omfp:chat", "./widgets/tests"] },
        { id: "solo", title: "SOLO", source: "workspace_init", widgets: ["omfp:chat"] },
        { id: "work", title: "WORK", source: "views_json", widgets: ["omfp:chat", "./widgets/tests"] },
      ],
      commands: [],
    });

    const builtin: Views = { views: [BUILTIN_VIEW], defaultId: "default", panels: [], error: undefined };
    const fallback = windowReport({
      workspaceId: "w1",
      threadId: undefined,
      focused: false,
      views: builtin,
      shown: BUILTIN_VIEW,
      commands: [],
      bindings,
    });
    expect(fallback.views?.map((view) => view.source)).toEqual(["builtin"]);
  });

  // spec: UI-351
  test("only commands of init files and widgets are reported, with who registered them and their keys", () => {
    const report = windowReport({
      workspaceId: "w1",
      threadId: undefined,
      focused: false,
      views,
      shown: work,
      commands,
      bindings,
    });
    expect(report.commands).toEqual([
      {
        id: "demo.solo",
        title: "Show solo",
        aliases: ["solo"],
        source: "workspace_init",
        widget: null,
        enabled: true,
        bindings: [{ mode: "normal", keys: "g h", args: [] }],
      },
      {
        id: "tests.run",
        title: "Run tests",
        aliases: [],
        source: "widget",
        widget: "./widgets/tests",
        enabled: false,
        bindings: [{ mode: "insert", keys: "Ctrl-t", args: ["unit"] }],
      },
    ]);
  });

  // spec: UI-350
  test("a window without a workspace reports no views", () => {
    expect(
      windowReport({
        workspaceId: undefined,
        threadId: undefined,
        focused: true,
        views,
        shown: work,
        commands: [],
        bindings,
      }),
    ).toEqual({ threadId: null, focused: true, activeView: null, defaultView: null, views: [], commands: [] });
  });

  // spec: UI-350
  test("a layout's widgets are listed once each, in layout order", () => {
    expect(widgetsOf(work.tree)).toEqual(["omfp:chat", "./widgets/tests"]);
  });
});
