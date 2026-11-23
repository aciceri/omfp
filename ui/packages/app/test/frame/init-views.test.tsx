/** L4: views init files make in the window (UI-267, UI-300, UI-304, UI-375). */

import { afterEach, describe, expect, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import type { ReportWidgetsParams } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Frame } from "../../src/frame/frame.tsx";
import { initUrl } from "../../src/frame/init.tsx";
import {
  chooseView,
  ERRORS,
  type InitApi,
  SELECTED_THREAD,
  type WidgetComponent,
  type WidgetProps,
} from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { repo, thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, type TestHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

const stub =
  (name: string): WidgetComponent =>
  (props: WidgetProps) => <p data-testid="stub">{typeof props.repo === "string" ? `${name} ${props.repo}` : name}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>(
  ["header", "chat", "workspace", "changes", "background", "mcp", "todos", "file"].map((name) => [
    `omfp:${name}`,
    stub(name),
  ]),
);

const BASE = "/widgets/test";
const WORKSPACE_URL = initUrl(BASE, "w1", "v1");
const USER_URL = initUrl(BASE, "user", "u1");

const MANIFEST = {
  default: "work",
  views: {
    same: {
      title: "Same",
      layout: [
        "row",
        { sizes: [3, 2] },
        ["tabs", { id: "main" }, "omfp:chat"],
        ["col", { sizes: [1, 1] }, ["omfp:changes", { repo: "api" }], ["tabs", {}, "omfp:background", "omfp:mcp"]],
      ],
    },
    work: { title: "Work", layout: ["row", {}, ["tabs", { id: "main" }, "omfp:chat"], "omfp:workspace"] },
  },
};

// The style of the docs: the views change in place, then `use` shows them.
const workspaceInit = (omfp: InitApi) => {
  const views = omfp.views.load("views.json");
  views.add("review", {
    title: "Review",
    layout: ["row", {}, "omfp:chat", ...omfp.workspace.repos.map((r) => ["omfp:changes", { repo: r.name }] as const)],
  });
  omfp.views.use(views);
};

function setup(
  modules: Record<string, unknown>,
  { manifest = MANIFEST as unknown, user = null as string | null } = {},
) {
  const reportTimers = new ManualTimers();
  const base = testHost(undefined, {
    importer: (url) => (url in modules ? Promise.resolve(modules[url]) : new Promise<unknown>(() => {})),
    reportTimers,
  });
  for (const method of ["widgets.report", "views.set"] as const satisfies readonly CallMethod[]) {
    base.client.answers.set(method, () => ({}));
  }
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1", [repo("w1", "api", "main")])],
    threads: [thread("t1", "c", "w1")],
    init: user,
  });
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  test.client.views("w1", { manifest });
  test.flush();
  return {
    ...test,
    /** Runs the report timers and gives the `fileErrors` of the last `widgets.report` (UI-304). */
    fileErrors: () => {
      reportTimers.run();
      return (test.client.callsTo("widgets.report").at(-1)?.params as ReportWidgetsParams | undefined)?.fileErrors;
    },
  };
}

/** Lets imports, init runs and the renders they cause settle. */
async function settle(test: TestHost): Promise<void> {
  for (let round = 0; round < 4; round += 1) {
    await act(async () => {
      await test.host.init.settled();
      const { promise, resolve } = Promise.withResolvers<void>();
      setTimeout(resolve, 0);
      await promise;
    });
    test.flush();
  }
}

function show(test: TestHost, id: string): void {
  act(() => chooseView(test.host, id));
}

/** Runs `line` on the `:` command line. */
async function command(line: string): Promise<void> {
  act(() => {
    fireEvent.keyDown(document.body, { key: ":" });
  });
  await userEvent.setup().type(screen.getByTestId("command-line-input"), `${line}{Enter}`);
}

function viewTitles(): string[] {
  return screen.getAllByTestId("mode-view").map((tab) => tab.textContent ?? "");
}

function written(test: TestHost): unknown[] {
  return test.client.callsTo("views.set").map((call) => call.params);
}

/** The titles of the view's tabs that close as `views.json` edits (UI-375). */
function closable(): string[] {
  return within(screen.getByTestId("view"))
    .queryAllByTestId("layout-tab-close")
    .map((mark) => mark.closest('[role="tab"]')?.textContent ?? "");
}

describe("init file errors reported to the daemon", () => {
  // spec: UI-304
  test("a failing init file is reported with its file until it runs cleanly", async () => {
    const test = setup(
      {
        [WORKSPACE_URL]: {
          default: () => {
            throw new Error("no such view");
          },
        },
        [USER_URL]: { compileError: { file: "init.ts", line: 2, column: 1, message: "Unexpected }" } },
      },
      { user: "u1" },
    );
    test.client.viewsEvents("w1", { kind: "views_init", init: "v1" });
    test.flush();
    await settle(test);
    expect(test.fileErrors()).toEqual([
      { file: "user", message: "init.ts:2:1: Unexpected }" },
      { file: ".omfp/init.ts", message: "no such view" },
    ]);
  });
});

describe("views of init files in the window", () => {
  // spec: UI-267, UI-300, UI-301
  test("a workspace's init file uses views.json's views plus a computed one; `:widget add` writes only views.json's", async () => {
    const test = setup({ [WORKSPACE_URL]: { default: workspaceInit } });
    test.client.viewsEvents("w1", { kind: "views_init", init: "v1" });
    test.flush();
    await settle(test);
    expect(viewTitles()).toEqual(["Same", "Work", "Review"]);

    // views.json's view takes the widget in views.json.
    await command("widget add omfp:mcp");
    expect(written(test)).toEqual([
      {
        workspaceId: "w1",
        manifest: {
          ...MANIFEST,
          views: {
            ...MANIFEST.views,
            work: {
              title: "Work",
              layout: ["row", {}, ["tabs", { id: "main" }, "omfp:chat"], "omfp:workspace", "omfp:mcp"],
            },
          },
        },
      },
    ]);

    // The computed view says where it comes from instead of writing a file the window ignores.
    show(test, "review");
    test.flush();
    expect(
      within(screen.getByTestId("view"))
        .getAllByTestId("stub")
        .map((element) => element.textContent),
    ).toEqual(["chat", "changes api"]);
    await command("widget add omfp:mcp");
    expect(written(test).length).toBe(1);
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
      "View review comes from the workspace's .omfp/init.ts: add the widget there (layout.addTab), :widget add writes only views.json",
    ]);

    // A new repository runs the file again: the computed view follows.
    test.client.indexEvents({
      kind: "workspace_upserted",
      workspace: workspace("w1", [repo("w1", "api", "main"), repo("w1", "web", "main")]),
    });
    test.flush();
    await settle(test);
    expect(
      within(screen.getByTestId("view"))
        .getAllByTestId("stub")
        .map((element) => element.textContent),
    ).toEqual(["chat", "changes api", "changes web"]);

    // A new views.json runs it again with the new file.
    test.client.viewsEvents("w1", {
      kind: "views_manifest",
      manifest: { ...MANIFEST, views: { ...MANIFEST.views, extra: { title: "Extra", layout: "omfp:chat" } } },
      error: null,
    });
    test.flush();
    await settle(test);
    expect(viewTitles()).toContain("Extra");
    expect(test.host.init.getSnapshot().views?.views.views.map((view) => view.id)).toEqual([
      "extra",
      "same",
      "work",
      "review",
    ]);
  });

  const DEFAULT_LAYOUT = [
    "row",
    {},
    ["col", {}, "omfp:header", ["tabs", { id: "main" }, "omfp:chat"]],
    ["tabs", { id: "side" }, "omfp:workspace", "omfp:changes", "omfp:background"],
  ];
  const EDITABLE = { default: "default", views: { default: { title: "Default", layout: DEFAULT_LAYOUT } } };

  /** The user's init file of the docs: the todos as the last side tab of the default view. */
  const userInit = (omfp: InitApi) => {
    omfp.views.transform((views) => {
      views.update("default", (layout) => {
        layout.addTab("side", "omfp:todos");
      });
    });
  };

  // spec: UI-267, UI-300
  test("a views.json view an init file transforms takes `:widget add` in views.json; the transform applies to the new file", async () => {
    const test = setup({ [USER_URL]: { default: userInit } }, { manifest: EDITABLE, user: "u1" });
    await settle(test);
    const tabs = () => screen.getAllByTestId("view-tab").map((tab) => tab.textContent);
    expect(tabs().slice(-4)).toEqual(["Workspace", "Changes", "Background", "Todos"]);

    // The widget goes to views.json's side tabs, without the tab the user's file added.
    await command("widget add omfp:header");
    const added = [
      "row",
      {},
      ["col", {}, "omfp:header", ["tabs", { id: "main" }, "omfp:chat"]],
      ["tabs", { id: "side" }, "omfp:workspace", "omfp:changes", "omfp:background", "omfp:header"],
    ];
    const pushed = { ...EDITABLE, views: { default: { title: "Default", layout: added } } };
    expect(written(test)).toEqual([{ workspaceId: "w1", manifest: pushed }]);

    // The daemon pushes the new file back: the transform applies to it again.
    test.client.viewsEvents("w1", { kind: "views_manifest", manifest: pushed, error: null });
    test.flush();
    await settle(test);
    expect(test.host.init.getSnapshot().views?.manifest).toBe(pushed);
    expect(tabs().slice(-5)).toEqual(["Workspace", "Changes", "Background", "Header", "Todos"]);
  });

  // spec: UI-267, UI-375
  test("in a views.json view an init file transforms, the file's own tabs close, never the tab the init file added", async () => {
    const test = setup({ [USER_URL]: { default: userInit } }, { manifest: EDITABLE, user: "u1" });
    await settle(test);
    expect(closable()).toEqual(["Workspace", "Changes", "Background"]);
  });
});
