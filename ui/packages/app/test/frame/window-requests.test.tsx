/** L4: the window state agents read and the daemon's requests to the window (UI-350..UI-354, PRT-122, PRT-124). */

import { afterEach, describe, expect, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import type { ReportWidgetsParams, WidgetInfo, WindowRequest } from "@omfp/protocol";
import { act, cleanup, screen } from "@testing-library/react";
import { Frame } from "../../src/frame/frame.tsx";
import { SETTLE_MS } from "../../src/frame/window-requests.tsx";
import { SELECTED_THREAD, useCommand, type WidgetComponent } from "../../src/sdk/index.ts";
import { type WidgetRegistry, widgetUrl } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, type TestHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

const stub =
  (name: string): WidgetComponent =>
  () => <p>{name}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>(
  ["header", "chat", "workspace", "changes", "background"].map((name) => [`omfp:${name}`, stub(name)]),
);

const BASE = "/widgets/test";
const widget = (name: string): WidgetInfo => ({ name, source: "workspace", version: "v1", entry: "index.tsx" });
const WIDGETS = [widget("tests"), widget("broken"), widget("slow")];

/** A widget registering a command (UI-351). */
function Tests() {
  useCommand({ id: "tests.run", title: "Run the tests", run: () => {} });
  return <p>Tests 3/3</p>;
}

function Broken(): never {
  throw new Error("x is not defined");
}

const MANIFEST = {
  default: "work",
  views: {
    ops: { title: "Ops", layout: ["row", {}, "omfp:chat", "./widgets/tests"] },
    work: { title: "Work", layout: ["row", {}, ["tabs", { id: "main" }, "omfp:chat"], "omfp:workspace"] },
  },
};

function setup({ select = true } = {}) {
  const reportTimers = new ManualTimers();
  const checkTimers = new ManualTimers();
  const modules: Record<string, unknown> = {
    [widgetUrl(BASE, "w1", widget("tests"), "index.tsx")]: { default: Tests },
    [widgetUrl(BASE, "w1", widget("broken"), "index.tsx")]: { default: Broken },
  };
  const base = testHost(undefined, {
    importer: (wanted) => (wanted in modules ? Promise.resolve(modules[wanted]) : new Promise<unknown>(() => {})),
    reportTimers,
  });
  for (const method of ["widgets.report", "windows.answer"] as const satisfies readonly CallMethod[]) {
    base.client.answers.set(method, () => ({}));
  }
  const test = renderWithHost(
    <Frame registry={registry} now={() => 0} hasFocus={() => true} checkTimers={checkTimers} />,
    base,
  );
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "checkout", "w1")] });
  if (select) {
    act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  }
  test.client.views("w1", { manifest: MANIFEST, widgets: WIDGETS });
  test.flush();
  return {
    ...test,
    reportTimers,
    checkTimers,
    /** The last `widgets.report`, after the report timers ran. */
    report: () => {
      reportTimers.run();
      return test.client.callsTo("widgets.report").at(-1)?.params as ReportWidgetsParams | undefined;
    },
    /** The `windows.answer` params sent so far. */
    answers: () => test.client.callsTo("windows.answer").map((call) => call.params),
    request: (id: string, request: WindowRequest) =>
      act(() => test.client.windowRequest({ type: "window_request", id, request })),
  };
}

/** Lets imports and the renders they cause settle. */
async function settle(test: TestHost): Promise<void> {
  for (let round = 0; round < 3; round += 1) {
    await act(async () => {
      const { promise, resolve } = Promise.withResolvers<void>();
      setTimeout(resolve, 0);
      await promise;
    });
    test.flush();
  }
}

describe("the window state", () => {
  // spec: UI-350, UI-351, PRT-122
  test("the report carries the thread, the focus, the views with their widgets and a widget's command", async () => {
    const test = setup();
    expect(test.report()?.window).toEqual({
      threadId: "t1",
      focused: true,
      activeView: "work",
      defaultView: "work",
      views: [
        { id: "ops", title: "Ops", source: "views_json", widgets: ["omfp:chat", "./widgets/tests"] },
        { id: "work", title: "Work", source: "views_json", widgets: ["omfp:chat", "omfp:workspace"] },
      ],
      commands: [],
    });

    test.request("w1", { kind: "run_command", command: "view", args: ["ops"] });
    await settle(test);
    const window = test.report()?.window;
    expect(window?.activeView).toBe("ops");
    expect(window?.commands).toEqual([
      {
        id: "tests.run",
        title: "Run the tests",
        aliases: [],
        source: "widget",
        widget: "./widgets/tests",
        enabled: true,
        bindings: [],
      },
    ]);
  });

  // spec: UI-350
  test("a window without a workspace reports no views", () => {
    const test = setup({ select: false });
    expect(test.report()?.window).toEqual({
      threadId: null,
      focused: true,
      activeView: null,
      defaultView: null,
      views: [],
      commands: [],
    });
  });
});

describe("commands the daemon runs in the window", () => {
  // spec: UI-352, PRT-124
  test("a command runs by id or alias in the global scope; unknown, disabled, scoped and failing ones say why", async () => {
    const test = setup();
    test.request("r1", { kind: "run_command", command: "view", args: ["ops"] });
    await settle(test);
    expect(screen.getByTestId("view").dataset.view).toBe("ops");
    test.request("r2", { kind: "run_command", command: "view.switch", args: ["nope"] });
    test.request("r3", { kind: "run_command", command: "eval", args: ["alert(1)"] });
    test.request("r4", { kind: "run_command", command: "thread.unarchive", args: [] });
    test.request("r5", { kind: "run_command", command: "nav.toggleSection", args: [] });
    await settle(test);
    // The async view.switch answers last: compare by request.
    const byRequest = (test.answers() as { requestId: string; answer: unknown }[]).toSorted((a, b) =>
      a.requestId.localeCompare(b.requestId),
    );
    expect(byRequest).toEqual([
      { requestId: "r1", answer: { kind: "ran", command: "view.switch" } },
      { requestId: "r2", answer: { kind: "not_run", reason: "failed", message: "No view nope" } },
      {
        requestId: "r3",
        answer: {
          kind: "not_run",
          reason: "unknown",
          message:
            'no command "eval": omfp_windows lists the commands init files and widgets add, omfp://keymap the built-ins',
        },
      },
      { requestId: "r4", answer: { kind: "not_run", reason: "disabled", message: "thread.unarchive is disabled now" } },
      {
        requestId: "r5",
        answer: {
          kind: "not_run",
          reason: "unknown",
          message: '"nav.toggleSection" runs only with the focus in the threads scope; run a global command',
        },
      },
    ]);
  });

  // spec: UI-525
  test("a short name runs the command it is an alias of", async () => {
    const test = setup();
    test.request("r1", { kind: "run_command", command: "theme", args: ["light"] });
    await settle(test);
    expect(test.answers()).toEqual([{ requestId: "r1", answer: { kind: "ran", command: "window.theme" } }]);
    expect(document.documentElement.dataset.colorScheme).toBe("light");
  });
});

describe("widgets checked out of sight", () => {
  // spec: UI-353, PRT-124
  test("a widget no view shows renders in a hidden container, is reported back and unmounted", async () => {
    const test = setup();
    test.request("c1", { kind: "check_widget", reference: "./widgets/tests", version: "v1", timeoutMs: 4000 });
    const container = screen.getByTestId("offscreen-check");
    expect(container.getAttribute("aria-hidden")).toBe("true");
    expect(container.hasAttribute("inert")).toBe(true);
    expect(container.closest("[data-testid=view]")).toBeNull();
    await settle(test);
    expect(container.textContent).toBe("Tests 3/3");
    test.reportTimers.run();
    // Answered only once it settled: no answer before the settle delay.
    expect(test.answers()).toEqual([]);
    expect(test.checkTimers.pending).toBe(2);
    act(() => test.checkTimers.run());
    expect(test.answers()).toEqual([
      {
        requestId: "c1",
        answer: {
          kind: "checked",
          settled: true,
          report: {
            reference: "./widgets/tests",
            instance: "./widgets/tests@check",
            source: "workspace",
            name: "tests",
            version: "v1",
            state: "rendered",
            errors: [],
            console: [],
            text: "Tests 3/3",
            jobs: [],
          },
        },
      },
    ]);
    expect(screen.queryAllByTestId("offscreen-check").length).toBe(0);
    // The check is never part of the window's own report.
    expect(test.report()?.widgets.map((report) => report.instance)).toEqual([]);
    expect(SETTLE_MS).toBe(500);
  });

  // spec: UI-353
  test("a throwing widget is a render error; one that never loads gives up unsettled; no workspace, no check", async () => {
    const test = setup();
    test.request("c1", { kind: "check_widget", reference: "./widgets/broken", version: "v1", timeoutMs: 4000 });
    test.request("c2", { kind: "check_widget", reference: "./widgets/slow", version: "v1", timeoutMs: 4000 });
    await settle(test);
    test.reportTimers.run();
    act(() => test.checkTimers.run());
    const answers = test.answers() as {
      requestId: string;
      answer: { kind: string; settled?: boolean; report?: { state: string; errors: string[] } };
    }[];
    const broken = answers.find((answer) => answer.requestId === "c1")?.answer;
    expect(broken?.settled).toBe(true);
    expect(broken?.report?.state).toBe("render_error");
    expect(broken?.report?.errors).toEqual(["x is not defined"]);
    const slow = answers.find((answer) => answer.requestId === "c2")?.answer;
    expect(slow?.settled).toBe(false);
    expect(slow?.report?.state).toBe("loading");
    expect(screen.queryAllByTestId("offscreen-check").length).toBe(0);

    const idle = setup({ select: false });
    idle.request("c3", { kind: "check_widget", reference: "./widgets/tests", version: null, timeoutMs: 4000 });
    expect(idle.answers()).toEqual([
      {
        requestId: "c3",
        answer: {
          kind: "not_run",
          reason: "failed",
          message: "this window shows no workspace, so it renders no workspace or user widget",
        },
      },
    ]);
  });
});
