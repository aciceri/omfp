/** L4: views init files make and TSX views in the window (UI-267, UI-268, UI-300, UI-304). */

import { afterEach, describe, expect, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import type { ReportWidgetsParams } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Frame } from "../../src/frame/frame.tsx";
import { initUrl } from "../../src/frame/init.tsx";
import {
  Col,
  chooseView,
  ERRORS,
  type InitApi,
  openFile,
  Row,
  SELECTED_THREAD,
  Tabs,
  Widget,
  type WidgetComponent,
  type WidgetProps,
} from "../../src/sdk/index.ts";
import { viewFileUrl } from "../../src/view/file-view.tsx";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { PANE_SIZES } from "../../src/view/sizes.ts";
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
const opsUrl = (version: string) => viewFileUrl(BASE, "w1", version, "./views/ops.tsx");

/** The example of the docs (UI-268): the same layout as {@link SAME}. */
function Ops() {
  return (
    <Row sizes={[3, 2]}>
      <Tabs id="main">
        <Widget use="omfp:chat" />
      </Tabs>
      <Col sizes={[1, 1]}>
        <Widget use="omfp:changes" props={{ repo: "api" }} />
        <Tabs>
          <Widget use="omfp:background" />
          <Widget use="omfp:mcp" />
        </Tabs>
      </Col>
    </Row>
  );
}

const SAME = [
  "row",
  { sizes: [3, 2] },
  ["tabs", { id: "main" }, "omfp:chat"],
  ["col", { sizes: [1, 1] }, ["omfp:changes", { repo: "api" }], ["tabs", {}, "omfp:background", "omfp:mcp"]],
];

const MANIFEST = {
  default: "work",
  views: {
    ops: { title: "Ops", view: "./views/ops.tsx" },
    same: { title: "Same", layout: SAME },
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
  test.client.views("w1", { manifest, viewFiles: "f1" });
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

/** The view as rendered: each laid-out element's path, kind and slot, then the widgets' text. */
function structure(): string[] {
  const view = screen.getByTestId("view");
  return [
    ...[...view.querySelectorAll<HTMLElement>("[data-path][data-layout]")].map(
      (element) => `${element.dataset.path} ${element.dataset.layout} ${element.dataset.slot ?? ""}`,
    ),
    ...within(view)
      .queryAllByTestId("stub")
      .map((element) => element.textContent ?? ""),
  ];
}

/** The splitter at `path` of the view shown (the views the frame keeps hidden have theirs, UI-390). */
function splitter(path: string): HTMLElement {
  const found = within(screen.getByTestId("view"))
    .getAllByTestId("splitter")
    .find((element) => element.dataset.path === path);
  if (found === undefined) {
    throw new Error(`no splitter at ${path}`);
  }
  return found;
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

function banner(): string | undefined {
  return screen.queryByTestId("view-file-error")?.textContent ?? undefined;
}

describe("TSX views", () => {
  // spec: UI-268
  test("a TSX view renders like the same layout in views.json: tree, main slot, splitters", async () => {
    const test = setup({ [opsUrl("f1")]: { default: Ops } });
    show(test, "same");
    await settle(test);
    const manifest = structure();
    show(test, "ops");
    await settle(test);
    expect(screen.getByTestId("view").dataset.view).toBe("ops");
    expect(structure()).toEqual(manifest);
    expect(
      within(screen.getByTestId("view"))
        .getAllByTestId("splitter")
        .map((element) => element.dataset.path),
    ).toEqual(["0", "0.1"]);
    // Files open as tabs of its `main` slot.
    act(() => openFile(test.host, "src/lib.rs"));
    test.flush();
    expect(within(screen.getByTestId("view")).getAllByTestId("file-tab").length).toBe(1);
    expect(banner()).toBeUndefined();
  });

  // spec: UI-268, UI-300
  test("its splitters keep this window's sizes by workspace, view and path; a double click resets; `:widget add` refuses it", async () => {
    const test = setup({ [opsUrl("f1")]: { default: Ops } });
    show(test, "ops");
    await settle(test);
    expect(splitter("0").getAttribute("aria-valuenow")).toBe("60");
    fireEvent.keyDown(splitter("0"), { key: "ArrowRight" });
    expect(Object.keys(test.host.window.get(PANE_SIZES))).toEqual(["w1/ops/0"]);
    expect(splitter("0").getAttribute("aria-valuenow")).not.toBe("60");
    fireEvent.doubleClick(splitter("0"));
    expect(test.host.window.get(PANE_SIZES)).toEqual({});
    expect(splitter("0").getAttribute("aria-valuenow")).toBe("60");

    // The view exists only in code: nothing writes a views.json the window would not show.
    await command("widget add omfp:mcp");
    expect(written(test)).toEqual([]);
    expect(test.host.window.get(ERRORS)).toEqual([
      'View ops is the TSX file .omfp/views/ops.tsx: add the widget there (<Widget use="…" />), :widget add writes only views.json',
    ]);
  });

  // spec: UI-268, UI-304
  test("a compile error, a throw or a node that is no layout shows a banner and the default view, reported to the daemon; the next save recovers", async () => {
    const throws = () => {
      throw new Error("kaboom");
    };
    const noLayout = () => (
      <Row>
        <div />
      </Row>
    );
    const test = setup({
      [opsUrl("f1")]: { compileError: { file: "ops.tsx", line: 3, column: 5, message: "Unexpected token" } },
      [opsUrl("f2")]: { default: throws },
      [opsUrl("f3")]: { default: noLayout },
      [opsUrl("f4")]: { default: Ops },
    });
    show(test, "ops");
    await settle(test);
    expect(banner()).toBe("View .omfp/views/ops.tsx failed, showing the default view: ops.tsx:3:5: Unexpected token");
    expect(screen.getByTestId("view").dataset.view).toBe("default");
    // The mode line still shows the failing view as the current one.
    expect(screen.getAllByTestId("mode-view").find((tab) => tab.dataset.current !== undefined)?.textContent).toBe(
      "Ops",
    );
    expect(test.fileErrors()).toEqual([
      { kind: "view", file: ".omfp/views/ops.tsx", message: "ops.tsx:3:5: Unexpected token" },
    ]);

    test.client.viewsEvents("w1", { kind: "views_files", viewFiles: "f2" });
    test.flush();
    await settle(test);
    expect(banner()).toBe("View .omfp/views/ops.tsx failed, showing the default view: kaboom");
    expect(screen.getByTestId("view").dataset.view).toBe("default");
    expect(test.fileErrors()).toEqual([{ kind: "view", file: ".omfp/views/ops.tsx", message: "kaboom" }]);

    test.client.viewsEvents("w1", { kind: "views_files", viewFiles: "f3" });
    test.flush();
    await settle(test);
    expect(banner()).toContain("<div> is not a layout component");

    test.client.viewsEvents("w1", { kind: "views_files", viewFiles: "f4" });
    test.flush();
    await settle(test);
    expect(banner()).toBeUndefined();
    expect(screen.getByTestId("view").dataset.view).toBe("ops");
    expect(test.fileErrors()).toEqual([]);
  });

  // spec: UI-268, UI-390
  test("a kept TSX view shows its last layout when it shows again while its new version loads", async () => {
    const opsTwoUrl = viewFileUrl(BASE, "w1", "f1", "./views/ops2.tsx");
    const manifest = { ...MANIFEST, views: { ...MANIFEST.views, ops2: { title: "Ops 2", view: "./views/ops2.tsx" } } };
    const test = setup({ [opsUrl("f1")]: { default: Ops }, [opsTwoUrl]: { default: Ops } }, { manifest });
    show(test, "ops");
    await settle(test);
    const layout = structure();
    show(test, "ops2");
    await settle(test);
    // A save of `.omfp/views/`: the new versions never finish loading here.
    test.client.viewsEvents("w1", { kind: "views_files", viewFiles: "f2" });
    test.flush();
    await settle(test);
    expect(screen.getByTestId("view").dataset.view).toBe("ops2");
    show(test, "ops");
    await settle(test);
    expect(screen.getByTestId("view").dataset.view).toBe("ops");
    expect(structure()).toEqual(layout);
  });
});

describe("init file errors reported to the daemon", () => {
  // spec: UI-304
  test("a failing init file is reported with its kind and file until it runs cleanly", async () => {
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
      { kind: "init", file: "user", message: "init.ts:2:1: Unexpected }" },
      { kind: "init", file: ".omfp/init.ts", message: "no such view" },
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
    expect(viewTitles()).toEqual(["Ops", "Same", "Work", "Review"]);

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
    expect(test.host.window.get(ERRORS)).toEqual([
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
      "ops",
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
});
