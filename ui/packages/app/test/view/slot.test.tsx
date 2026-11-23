/** L4: the widget slot — reference resolution, module loading by versioned URL, and errors in place. */

import { afterEach, describe, expect, test } from "bun:test";
import type { ReportWidgetsParams, ViewsState, WidgetInfo, WidgetReport } from "@omfp/protocol";
import { act, cleanup, screen } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { OmfpProvider, useStored, useWidget, type WidgetComponent, type WidgetProps } from "../../src/sdk/index.ts";
import { resolveWidget, type WidgetRegistry, widgetUrl } from "../../src/view/resolve.ts";
import { WidgetSlot } from "../../src/view/slot.tsx";
import { FakeClient, ManualTimers, renderWithHost, type TestHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

const WS = "01W";
const BASE = "/widgets/test";

function Builtin(props: WidgetProps): ReactNode {
  const { instance } = useWidget();
  return <span data-testid="builtin">{`${instance} ${String(props.label)}`}</span>;
}

const REGISTRY: WidgetRegistry = new Map<string, WidgetComponent>([["omfp:header", Builtin]]);

function folder(source: WidgetInfo["source"], name: string, version: string, entry: string | null = "index.tsx") {
  return { source, name, version, entry };
}

function views(...widgets: WidgetInfo[]): ViewsState {
  return { manifest: null, error: null, widgets, stored: {} };
}

interface Deferred {
  readonly promise: Promise<unknown>;
  resolve(value: unknown): void;
  reject(error: unknown): void;
}

/** A fake `import()`: every URL waits until the test settles it. */
class Imports {
  readonly urls: string[] = [];
  readonly #pending = new Map<string, Deferred>();

  readonly importer = (url: string): Promise<unknown> => {
    this.urls.push(url);
    const deferred = Promise.withResolvers<unknown>();
    this.#pending.set(url, deferred);
    return deferred.promise;
  };

  async resolve(url: string, module: unknown): Promise<void> {
    await act(async () => this.#deferred(url).resolve(module));
  }

  async reject(url: string, error: Error): Promise<void> {
    await act(async () => this.#deferred(url).reject(error));
  }

  #deferred(url: string): Deferred {
    const deferred = this.#pending.get(url);
    if (deferred === undefined) {
      throw new Error(`${url} was not imported; imported: ${this.urls.join(", ")}`);
    }
    return deferred;
  }
}

interface Slot {
  readonly reference: string;
  readonly props?: WidgetProps;
  readonly path?: string;
}

function Slots({ slots, state }: { slots: readonly Slot[]; state: ViewsState | undefined }): ReactNode {
  return slots.map(({ reference, props = {}, path = "0" }) => (
    <WidgetSlot
      key={path}
      reference={reference}
      props={props}
      path={path}
      workspaceId={WS}
      views={state}
      registry={REGISTRY}
    />
  ));
}

interface Rendered extends TestHost {
  readonly imports: Imports;
  readonly timers: ManualTimers;
  rerender(slots: readonly Slot[], state: ViewsState | undefined): void;
  unmount(): void;
  /** Runs the report timers (after pending DOM observations) and gives the widgets of the last report. */
  report(): Promise<WidgetReport[]>;
}

function renderSlots(slots: readonly Slot[], state: ViewsState | undefined): Rendered {
  const imports = new Imports();
  const timers = new ManualTimers();
  const client = new FakeClient();
  client.connected();
  client.answers.set("widgets.report", () => ({}));
  const test = testHost(client, { importer: imports.importer, reportTimers: timers });
  test.host.frames.commit();
  test.host.reports.show(WS, { epoch: "e1", seq: 1 });
  const { view } = renderWithHost(<Slots slots={slots} state={state} />, test);
  return {
    ...test,
    imports,
    timers,
    rerender: (next, nextState) =>
      view.rerender(
        <OmfpProvider host={test.host}>
          <Slots slots={next} state={nextState} />
        </OmfpProvider>,
      ),
    unmount: () => view.unmount(),
    report: async () => {
      await act(async () => {});
      timers.run();
      const reports = client.callsTo("widgets.report");
      return (reports.at(-1)?.params as ReportWidgetsParams | undefined)?.widgets ?? [];
    },
  };
}

function widget(): HTMLElement {
  return screen.getByTestId("widget");
}

function errorOf(): { kind: string | null; text: string | null } {
  const error = screen.getByTestId("widget-error");
  return { kind: error.getAttribute("data-error-kind"), text: error.textContent };
}

function Hello(props: WidgetProps): ReactNode {
  return <span data-testid="module">{`hello ${String(props.who)}`}</span>;
}

function Bye(props: WidgetProps): ReactNode {
  return <span data-testid="module">{`bye ${String(props.who)}`}</span>;
}

function Broken(): ReactNode {
  throw new Error("boom");
}

function Count(): ReactNode {
  const [count] = useStored("count", 0);
  return <span data-testid="module">{`count ${count}`}</span>;
}

describe("resolveWidget", () => {
  // spec: AGT-022
  test("resolves built-ins, workspace overrides and workspace widgets", () => {
    const tests = folder("workspace", "tests", "w1");
    const header = folder("workspace", "header", "h1");
    const state = views(tests, header);
    expect(resolveWidget("omfp:header", WS, undefined, REGISTRY)).toEqual({ kind: "builtin", component: Builtin });
    expect(resolveWidget("omfp:header", WS, views(tests), REGISTRY)).toEqual({ kind: "builtin", component: Builtin });
    expect(resolveWidget("omfp:header", WS, state, REGISTRY)).toEqual({ kind: "module", widget: header });
    expect(resolveWidget("./widgets/tests", WS, state, REGISTRY)).toEqual({ kind: "module", widget: tests });
    expect(resolveWidget("./widgets/tests", WS, undefined, REGISTRY)).toEqual({ kind: "pending" });
    for (const reference of ["omfp:nope", "./widgets/clock", "./widgets/", "tests"]) {
      expect({ reference, resolved: resolveWidget(reference, WS, state, REGISTRY) }).toEqual({
        reference,
        resolved: { kind: "missing" },
      });
    }
    expect(resolveWidget("./widgets/tests", undefined, undefined, REGISTRY)).toEqual({ kind: "missing" });
  });

  // spec: UI-105
  test("widget URLs follow the daemon's scheme", () => {
    expect(widgetUrl(BASE, WS, folder("workspace", "tests", "v1"), "index.tsx")).toBe(
      "/widgets/test/01W/tests/v1/index.tsx",
    );
    expect(widgetUrl(BASE, WS, folder("workspace", "clock", "u 2"), "index.js")).toBe(
      "/widgets/test/01W/clock/u%202/index.js",
    );
  });
});

describe("widget slot", () => {
  // spec: AGT-010, AGT-016
  test("a built-in renders with its props, instance, reference and path", () => {
    renderSlots([{ reference: "omfp:header", props: { label: "x", instance: "top" }, path: "0.1" }], undefined);
    expect(screen.getByTestId("builtin").textContent).toBe("top x");
    expect(widget().dataset).toMatchObject({ widget: "omfp:header", instance: "top", path: "0.1" });
    expect(widget().hasAttribute("data-version")).toBe(false);
  });

  // spec: AGT-010, AGT-014
  test("an unknown reference shows a missing error in place", () => {
    renderSlots([{ reference: "omfp:nope" }], views());
    expect(errorOf()).toEqual({ kind: "missing", text: "Unknown widget omfp:nope" });
    expect(widget().dataset.instance).toBe("omfp:nope");
  });

  // spec: UI-105, AGT-013
  test("a workspace widget loads from its versioned URL, then renders with its props", async () => {
    const { imports } = renderSlots(
      [{ reference: "./widgets/hello", props: { who: "you" } }],
      views(folder("workspace", "hello", "v1")),
    );
    expect(screen.getByTestId("widget-loading")).toBeDefined();
    expect(imports.urls).toEqual(["/widgets/test/01W/hello/v1/index.tsx"]);
    expect(widget().dataset.source).toBe("workspace");

    await imports.resolve("/widgets/test/01W/hello/v1/index.tsx", { default: Hello });

    expect(screen.getByTestId("module").textContent).toBe("hello you");
    expect(widget().dataset.version).toBe("v1");
  });

  // spec: AGT-022
  test("a workspace folder named like a built-in replaces it in that workspace", async () => {
    const { imports } = renderSlots(
      [{ reference: "omfp:header", props: { who: "copy" } }],
      views(folder("workspace", "header", "h1")),
    );
    expect(screen.queryByTestId("builtin")).toBeNull();
    await imports.resolve("/widgets/test/01W/header/h1/index.tsx", { default: Hello });
    expect(screen.getByTestId("module").textContent).toBe("hello copy");
    expect(widget().dataset).toMatchObject({ widget: "omfp:header", source: "workspace", version: "h1" });
  });

  // spec: AGT-013, UI-105
  test("a new version is imported again and replaces only that widget; modules are kept per URL", async () => {
    const slots = [
      { reference: "./widgets/hello", props: { who: "a" }, path: "0.0" },
      { reference: "./widgets/other", props: { who: "b" }, path: "0.1" },
      { reference: "./widgets/hello", props: { who: "c", instance: "second" }, path: "0.2" },
    ];
    const other = folder("workspace", "other", "o1");
    const { imports, rerender } = renderSlots(slots, views(folder("workspace", "hello", "v1"), other));
    // Two slots of the same widget share one import.
    expect(imports.urls).toEqual(["/widgets/test/01W/hello/v1/index.tsx", "/widgets/test/01W/other/o1/index.tsx"]);
    await imports.resolve("/widgets/test/01W/hello/v1/index.tsx", { default: Hello });
    await imports.resolve("/widgets/test/01W/other/o1/index.tsx", { default: Hello });

    rerender(slots, views(folder("workspace", "hello", "v2"), other));
    expect(imports.urls.slice(2)).toEqual(["/widgets/test/01W/hello/v2/index.tsx"]);
    // The previous version stays until the new one is loaded.
    expect(screen.getAllByTestId("module").map((node) => node.textContent)).toEqual(["hello a", "hello b", "hello c"]);
    await imports.resolve("/widgets/test/01W/hello/v2/index.tsx", { default: Bye });

    expect(screen.getAllByTestId("module").map((node) => node.textContent)).toEqual(["bye a", "hello b", "bye c"]);
    expect(screen.getAllByTestId("widget").map((node) => node.dataset.version)).toEqual(["v2", "o1", "v2"]);
    expect(imports.urls.length).toBe(3);
  });

  // spec: AGT-014, UI-105
  test("a compile error shows the file, line, column and message in place", async () => {
    const { imports } = renderSlots([{ reference: "./widgets/bad" }], views(folder("workspace", "bad", "b1")));
    const compileError = { file: "parts/view.tsx", line: 3, column: 7, message: "Unexpected token" };
    await imports.resolve("/widgets/test/01W/bad/b1/index.tsx", {
      compileError,
      default: () => createElement("pre", null, "stand-in"),
    });
    expect(errorOf().kind).toBe("compile");
    expect(screen.getByTestId("widget-error-location").textContent).toBe("parts/view.tsx:3:7");
    expect(errorOf().text).toContain("Unexpected token");
    expect(widget().dataset.version).toBe("b1");
  });

  // spec: AGT-014, UI-105
  test("a failed import, a missing default export and a folder without entry are load errors", async () => {
    const state = views(
      folder("workspace", "a", "1"),
      folder("workspace", "b", "1"),
      folder("workspace", "c", "1", null),
    );
    const { imports } = renderSlots(
      [
        { reference: "./widgets/a", path: "0.0" },
        { reference: "./widgets/b", path: "0.1" },
        { reference: "./widgets/c", path: "0.2" },
      ],
      state,
    );
    expect(imports.urls).toEqual(["/widgets/test/01W/a/1/index.tsx", "/widgets/test/01W/b/1/index.tsx"]);
    await imports.reject("/widgets/test/01W/a/1/index.tsx", new Error("Failed to fetch"));
    await imports.resolve("/widgets/test/01W/b/1/index.tsx", { named: Hello });

    const errors = screen.getAllByTestId("widget-error");
    expect(errors.map((error) => error.getAttribute("data-error-kind"))).toEqual(["load", "load", "load"]);
    expect(errors.map((error) => error.querySelector("pre")?.textContent)).toEqual([
      "Failed to fetch",
      "The widget's entry has no default export (a React component).",
      "The widget folder has no index.tsx, index.ts, index.jsx or index.js.",
    ]);
  });

  // spec: AGT-014, AGT-013
  test("a render error shows in place, the rest keeps working, and the next version recovers", async () => {
    const slots = [
      { reference: "./widgets/hello", path: "0.0" },
      { reference: "omfp:header", props: { label: "fine" }, path: "0.1" },
    ];
    const { imports, rerender } = renderSlots(slots, views(folder("workspace", "hello", "v1")));
    await imports.resolve("/widgets/test/01W/hello/v1/index.tsx", { default: Broken });
    expect(errorOf()).toEqual({ kind: "render", text: "Widget ./widgets/hello failedboom" });
    expect(screen.getByTestId("builtin").textContent).toBe("omfp:header fine");

    rerender(slots, views(folder("workspace", "hello", "v2")));
    await imports.resolve("/widgets/test/01W/hello/v2/index.tsx", { default: Hello });
    expect(screen.queryByTestId("widget-error")).toBeNull();
    expect(screen.getByTestId("module").textContent).toBe("hello undefined");
  });

  // spec: AGT-013, AGT-018
  test("stored state is the instance's and survives a reload of the module", async () => {
    const state = { ...views(folder("workspace", "hello", "v1")), stored: { mine: { count: 4 } } };
    const { client, flush, imports, rerender } = renderSlots(
      [{ reference: "./widgets/hello", props: { instance: "mine" } }],
      state,
    );
    client.views(WS, state);
    flush();
    await imports.resolve("/widgets/test/01W/hello/v1/index.tsx", { default: Count });
    expect(screen.getByTestId("module").textContent).toBe("count 4");

    rerender([{ reference: "./widgets/hello", props: { instance: "mine" } }], {
      ...state,
      widgets: [folder("workspace", "hello", "v2")],
    });
    await imports.resolve("/widgets/test/01W/hello/v2/index.tsx", { default: Count });
    expect(screen.getByTestId("module").textContent).toBe("count 4");
    expect(widget().dataset.version).toBe("v2");
  });

  // spec: UI-105
  test("workspace widgets wait for the views state", () => {
    const { imports } = renderSlots([{ reference: "./widgets/hello" }], undefined);
    expect(screen.getByTestId("widget-loading")).toBeDefined();
    expect(imports.urls).toEqual([]);
  });
});

describe("slot reports", () => {
  // spec: AGT-026, PRT-043
  test("a module widget reports its import, version and rendered text; built-ins are not reported", async () => {
    const { imports, report, rerender } = renderSlots(
      [
        { reference: "./widgets/hello", props: { who: "you" }, path: "0.0" },
        { reference: "omfp:header", props: { label: "fine" }, path: "0.1" },
      ],
      views(folder("workspace", "hello", "v1")),
    );
    expect(await report()).toMatchObject([
      { reference: "./widgets/hello", instance: "./widgets/hello", source: "workspace", name: "hello" },
    ]);
    expect((await report())[0]).toMatchObject({ state: "loading", version: null, errors: [], jobs: [] });

    await imports.resolve("/widgets/test/01W/hello/v1/index.tsx", { default: Hello });
    expect(await report()).toEqual([
      {
        reference: "./widgets/hello",
        instance: "./widgets/hello",
        source: "workspace",
        name: "hello",
        version: "v1",
        state: "rendered",
        errors: [],
        console: [],
        text: "hello you",
        jobs: [],
      },
    ]);

    // The text follows what the widget renders.
    rerender(
      [
        { reference: "./widgets/hello", props: { who: "me" }, path: "0.0" },
        { reference: "omfp:header", props: { label: "fine" }, path: "0.1" },
      ],
      views(folder("workspace", "hello", "v1")),
    );
    expect((await report())[0]?.text).toBe("hello me");

    // While a new version loads, the previous one stays shown and the instance is loading.
    rerender(
      [{ reference: "./widgets/hello", props: { who: "me" }, path: "0.0" }],
      views(folder("workspace", "hello", "v2")),
    );
    expect((await report())[0]).toMatchObject({ state: "loading", version: "v1" });
    await imports.resolve("/widgets/test/01W/hello/v2/index.tsx", { default: Bye });
    expect((await report())[0]).toMatchObject({ state: "rendered", version: "v2", text: "bye me" });
  });

  // spec: AGT-026, PRT-043
  test("compile, load and render errors are reported with their messages", async () => {
    const { imports, report } = renderSlots(
      [
        { reference: "./widgets/bad", path: "0.0" },
        { reference: "./widgets/gone", path: "0.1" },
        { reference: "./widgets/broken", path: "0.2" },
      ],
      views(folder("workspace", "bad", "b1"), folder("workspace", "gone", "g1"), folder("workspace", "broken", "r1")),
    );
    await imports.resolve("/widgets/test/01W/bad/b1/index.tsx", {
      compileError: { file: "parts/view.tsx", line: 3, column: 7, message: "Unexpected token" },
      default: () => createElement("pre", null, "stand-in"),
    });
    await imports.reject("/widgets/test/01W/gone/g1/index.tsx", new Error("Failed to fetch"));
    await imports.resolve("/widgets/test/01W/broken/r1/index.tsx", { default: Broken });

    expect((await report()).map(({ name, state, version, errors }) => ({ name, state, version, errors }))).toEqual([
      { name: "bad", state: "compile_error", version: "b1", errors: ["parts/view.tsx:3:7: Unexpected token"] },
      { name: "gone", state: "load_error", version: "g1", errors: ["Failed to fetch"] },
      { name: "broken", state: "render_error", version: "r1", errors: ["boom"] },
    ]);
  });

  // spec: AGT-026
  test("an unmounted slot leaves the report", async () => {
    const { imports, report, unmount } = renderSlots(
      [{ reference: "./widgets/hello" }],
      views(folder("workspace", "hello", "v1")),
    );
    await imports.resolve("/widgets/test/01W/hello/v1/index.tsx", { default: Hello });
    expect((await report()).length).toBe(1);
    unmount();
    expect(await report()).toEqual([]);
  });
});
