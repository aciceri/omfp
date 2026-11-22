/** L4: floating widget panels (UI-230, UI-231, AGT-060), props updates in place (UI-232) and `:widget save` (UI-233),
 * in the frame over a fake client. */

import { afterEach, describe, expect, spyOn, test } from "bun:test";
import type { ReportWidgetsParams, ViewsState, WidgetInfo } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { openingSlot, topBarsBottom } from "../../src/frame/floating-box.ts";
import { Frame } from "../../src/frame/frame.tsx";
import {
  chooseView,
  ERRORS,
  type KeyValueStorage,
  SELECTED_THREAD,
  type WidgetComponent,
} from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

/** A widget with component state: a click counter showing its `label` prop. */
const Counter: WidgetComponent = (props) => {
  const [count, setCount] = useState(0);
  return (
    <button type="button" data-testid="counter" onClick={() => setCount((now) => now + 1)}>
      {`${String(props.label ?? "")} ${count}`}
    </button>
  );
};

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", () => <p>header</p>],
  ["omfp:chat", () => <p data-testid="stub-chat">chat</p>],
  ["omfp:workspace", () => <p data-testid="stub-workspace">workspace</p>],
  ["omfp:counter", Counter],
]);

class MemoryStorage implements KeyValueStorage {
  readonly items = new Map<string, string>();
  getItem(key: string): string | null {
    return this.items.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.items.set(key, value);
  }
}

function folder(source: WidgetInfo["source"], name: string, version: string): WidgetInfo {
  return { source, name, version, entry: "index.tsx" };
}

/** In `w1` (thread `t1`) with `views`; module widgets import as {@link Counter}. */
function setup(views?: Partial<ViewsState>) {
  const storage = new MemoryStorage();
  const timers = new ManualTimers();
  const base = testHost(undefined, {
    importer: async () => ({ default: Counter }),
    reportTimers: timers,
    storage,
  });
  base.client.answers.set("widgets.report", () => ({}));
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1"), workspace("w2")],
    threads: [thread("t1", "one", "w1"), thread("t2", "two", "w2")],
  });
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  if (views !== undefined) {
    test.client.views("w1", views);
  }
  test.flush();
  return {
    ...test,
    storage,
    user: userEvent.setup(),
    /** Runs the report timers and gives the widgets of the last `widgets.report`. */
    report: async () => {
      await act(async () => {});
      timers.run();
      const reports = test.client.callsTo("widgets.report");
      return (reports.at(-1)?.params as ReportWidgetsParams | undefined)?.widgets ?? [];
    },
  };
}

function press(key: string): void {
  act(() => {
    fireEvent.keyDown(document.activeElement ?? document.body, { key });
  });
}

function panels(): HTMLElement[] {
  return screen.queryAllByTestId("floating-widget");
}

function panel(id: string): HTMLElement {
  const found = panels().find((element) => element.dataset.panel === id);
  if (found === undefined) {
    throw new Error(`no panel ${id}`);
  }
  return found;
}

const VIEWS = {
  work: { title: "Work", layout: ["row", {}, "omfp:chat", "omfp:workspace"] },
  other: { title: "Other", layout: "omfp:header" },
};

const MANIFEST = {
  default: "work",
  views: VIEWS,
  panels: {
    clock: ["omfp:counter", { instance: "clock", title: "Clock", label: "tick" }],
    board: ["col", {}, "omfp:workspace", ["omfp:counter", { label: "inner" }]],
  },
};

describe("floating panels", () => {
  // spec: UI-230, AGT-060
  test("each panel floats over every view with its title, its node and an opening place", async () => {
    const test = setup({ manifest: MANIFEST });
    // Id order: board, clock.
    expect(panels().map((element) => element.dataset.panel)).toEqual(["board", "clock"]);
    expect(panels().map((element) => within(element).getByTestId("floating-widget-title").textContent)).toEqual([
      "board",
      "Clock",
    ]);
    expect(panels().map((element) => element.getAttribute("aria-label"))).toEqual(["board", "Clock"]);
    const place = (element: HTMLElement) =>
      ["--panel-top", "--panel-column", "--panel-row"].map((name) => element.style.getPropertyValue(name)).join(" ");
    // happy-dom lays nothing out (no offset parent to measure): the panels open side by side by their index.
    expect(panels().map(place)).toEqual(["0px 0 0", "0px 1 0"]);
    // A widget node renders in its slot with its props, a container node like a view's.
    const clock = within(panel("clock")).getByTestId("widget");
    expect([clock.dataset.widget, clock.dataset.instance, clock.dataset.path]).toEqual([
      "omfp:counter",
      "clock",
      "panel:clock",
    ]);
    expect(within(panel("clock")).getByTestId("counter").textContent).toBe("tick 0");
    expect(
      within(panel("board"))
        .getAllByTestId("widget")
        .map((element) => [element.dataset.widget, element.dataset.path]),
    ).toEqual([
      ["omfp:workspace", "panel:board.0"],
      ["omfp:counter", "panel:board.1"],
    ]);
    for (const part of ["move", "minimise", "close", "resize"]) {
      expect(within(panel("clock")).getAllByTestId(`floating-widget-${part}`).length).toBe(1);
    }
    // Over whatever view is shown.
    act(() => chooseView(test.host, "other"));
    expect(screen.getByTestId("view").dataset.view).toBe("other");
    expect(panels().map((element) => element.dataset.panel)).toEqual(["board", "clock"]);
  });

  // spec: UI-230, AGT-026
  test("a panel's module widget shows in place and is part of the window's report", async () => {
    const test = setup({
      manifest: { views: VIEWS, panels: { clock: ["user:clock", { instance: "clock" }] } },
      widgets: [folder("user", "clock", "v1")],
    });
    await waitFor(() => expect(within(panel("clock")).getByTestId("counter").textContent).toBe(" 0"));
    expect(within(panel("clock")).getByTestId("floating-widget-title").textContent).toBe("clock");
    expect(await test.report()).toMatchObject([
      { reference: "user:clock", instance: "clock", source: "user", name: "clock", version: "v1", state: "rendered" },
    ]);
  });

  // spec: UI-230
  test("a panel moves, resizes and minimises, kept per workspace and id in this window", async () => {
    const test = setup({ manifest: MANIFEST });
    act(() => within(panel("clock")).getByTestId("floating-widget-move").focus());
    press("ArrowRight");
    press("ArrowDown");
    act(() => within(panel("clock")).getByTestId("floating-widget-resize").focus());
    press("ArrowRight");
    // happy-dom lays nothing out: the panel starts as a 0×0 box at 0,0, and a resize keeps the minimum size.
    const clock = panel("clock");
    expect([clock.style.left, clock.style.top, clock.style.width, clock.style.height]).toEqual([
      "16px",
      "16px",
      "240px",
      "120px",
    ]);
    // The other panel stays where the stylesheet places it.
    expect([panel("board").style.left, panel("board").style.getPropertyValue("--panel-column")]).toEqual(["", "0"]);
    await test.user.click(within(panel("clock")).getByText("tick 0"));
    await test.user.click(within(panel("clock")).getByTestId("floating-widget-minimise"));
    expect(panel("clock").dataset.minimised).toBe("");
    expect(within(panel("clock")).queryAllByTestId("floating-widget-resize").length).toBe(0);
    expect(within(panel("clock")).getByTestId("counter").closest("[hidden]")).not.toBeNull();
    expect(JSON.parse(test.storage.getItem("omfp.floatingWidget/w1/clock") ?? "{}")).toEqual({
      box: { left: 16, top: 16, width: 240, height: 120 },
      minimised: true,
    });
    expect(test.storage.getItem("omfp.floatingWidget/w1/board")).toBeNull();
    // Restored, the widget kept its state.
    await test.user.click(within(panel("clock")).getByTestId("floating-widget-minimise"));
    expect(panel("clock").dataset.minimised).toBeUndefined();
    expect(within(panel("clock")).getByTestId("counter").textContent).toBe("tick 1");

    // The same id in another workspace is another panel.
    act(() => test.host.window.set(SELECTED_THREAD, "t2"));
    test.client.views("w2", { manifest: MANIFEST });
    test.flush();
    expect(panel("clock").style.left).toBe("");
    act(() => test.host.window.set(SELECTED_THREAD, "t1"));
    test.client.views("w1", { manifest: MANIFEST });
    test.flush();
    expect(panel("clock").style.left).toBe("16px");
  });

  // spec: UI-231
  test("closing a panel unmounts it from views.json; a failure lands in the error list", async () => {
    const test = setup({ manifest: MANIFEST });
    test.client.answers.set("views.unmount", () => ({ views: [], removedViews: [], panel: true }));
    await test.user.click(within(panel("clock")).getByTestId("floating-widget-close"));
    expect(test.client.callsTo("views.unmount").map((call) => call.params)).toEqual([
      { workspaceId: "w1", id: "clock" },
    ]);
    // The daemon publishes the new manifest; the panel goes with it.
    test.client.views("w1", { manifest: { ...MANIFEST, panels: { board: MANIFEST.panels.board } } });
    test.flush();
    expect(panels().map((element) => element.dataset.panel)).toEqual(["board"]);

    test.client.answers.delete("views.unmount");
    await test.user.click(within(panel("board")).getByTestId("floating-widget-close"));
    const [, failing] = test.client.callsTo("views.unmount");
    expect(failing?.params).toEqual({ workspaceId: "w1", id: "board" });
    await act(async () => failing?.reject(new Error("nothing is mounted as board")));
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["nothing is mounted as board"]);
    expect(panels().length).toBe(1);
  });

  // spec: AGT-060, AGT-011
  test("invalid panels show the view error banner and no panel", () => {
    setup({ manifest: { views: VIEWS, panels: ["omfp:counter"] } });
    expect(screen.getByTestId("view-error").textContent).toBe(
      "The view is invalid, showing the default view: panels: expected an object of panels",
    );
    expect(panels().length).toBe(0);
  });

  // spec: UI-454
  test("a new panel opens below the view's top bars, beside the panels before it while they fit, then cascaded", () => {
    // 1600 px across, panels of 384 px with 12 px gaps: four side by side, the fifth on the next row.
    expect([0, 3, 4, 5].map((index) => openingSlot(index, 1600, 384))).toEqual([
      { column: 0, row: 0 },
      { column: 3, row: 0 },
      { column: 0, row: 1 },
      { column: 1, row: 1 },
    ]);
    expect(openingSlot(1, 300, 384)).toEqual({ column: 0, row: 1 });

    const container = document.createElement("div");
    container.innerHTML = `<div data-testid="view">
      <div data-widget="omfp:header" data-top="40" data-bottom="120"></div>
      <div class="omfp-view__tabbar" data-top="40" data-bottom="74"></div>
      <div class="omfp-view__tabbar" data-top="120" data-bottom="154"></div>
    </div>`;
    container.dataset.top = "20";
    container.dataset.bottom = "900";
    container.querySelector<HTMLElement>('[data-testid="view"]')?.setAttribute("data-top", "40");
    const rect = spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
      const top = Number(this.getAttribute("data-top") ?? 0);
      const bottom = Number(this.getAttribute("data-bottom") ?? top);
      return { top, bottom, height: bottom - top, left: 0, right: 0, width: 0, x: 0, y: top } as DOMRect;
    });
    try {
      // The header ends lowest among the bars at the view's top; the `main` slot's bar under it does not count.
      expect(topBarsBottom(container)).toBe(100);
      container.innerHTML = "";
      expect(topBarsBottom(container)).toBe(0);
    } finally {
      rect.mockRestore();
    }
  });
});

describe("props updates", () => {
  // spec: UI-232, AGT-062
  test("a widget whose props change re-renders in place, keeping its state, in a view and in a panel", async () => {
    const manifest = (label: string) => ({
      views: { work: { layout: ["row", {}, "omfp:chat", ["omfp:counter", { instance: "c", label }]] } },
      panels: { meter: ["./widgets/meter", { instance: "meter", label }] },
    });
    const test = setup({ manifest: manifest("a"), widgets: [folder("workspace", "meter", "v1")] });
    await waitFor(() => expect(within(panel("meter")).getByTestId("counter").textContent).toBe("a 0"));
    const counters = () => screen.getAllByTestId("counter");
    for (const counter of counters()) {
      await test.user.click(counter);
    }
    expect(counters().map((counter) => counter.textContent)).toEqual(["a 1", "a 1"]);

    test.client.views("w1", { manifest: manifest("b"), widgets: [folder("workspace", "meter", "v1")] });
    test.flush();
    // Right away: no placeholder, the same component state with the new props.
    expect(screen.queryAllByTestId("widget-loading").length).toBe(0);
    expect(counters().map((counter) => counter.textContent)).toEqual(["b 1", "b 1"]);
  });
});

describe("widget save", () => {
  // spec: UI-233, AGT-064, UI-520
  test("`:widget save <widget> [<name>]` saves to the user library, a success toast says user:<name>; save! replaces", async () => {
    const test = setup();
    test.client.answers.set("widgets.save", (params) => {
      const { name } = params as { name: string };
      return { name, reference: `user:${name}` };
    });
    press(":");
    await test.user.type(screen.getByTestId("command-line-input"), "widget save ./widgets/board{Enter}");
    expect(test.client.callsTo("widgets.save").map((call) => call.params)).toEqual([
      { workspaceId: "w1", widget: "./widgets/board", name: "board", replace: false },
    ]);
    await waitFor(() =>
      expect(screen.getByTestId("live-region").textContent).toBe("Saved to the user library as user:board"),
    );
    press(":");
    await test.user.type(screen.getByTestId("command-line-input"), "widget save! board dash{Enter}");
    expect(test.client.callsTo("widgets.save").at(-1)?.params).toEqual({
      workspaceId: "w1",
      widget: "board",
      name: "dash",
      replace: true,
    });
    await waitFor(() =>
      expect(screen.getByTestId("live-region").textContent).toBe("Saved to the user library as user:dash"),
    );
    expect(screen.getAllByTestId("success").map((success) => success.textContent)).toEqual([
      "Saved to the user library as user:board",
      "Saved to the user library as user:dash",
    ]);

    test.client.answers.delete("widgets.save");
    press(":");
    await test.user.type(screen.getByTestId("command-line-input"), "widget save clock{Enter}");
    const failing = test.client.callsTo("widgets.save").at(-1);
    expect(failing?.params).toEqual({ workspaceId: "w1", widget: "clock", name: "clock", replace: false });
    await act(async () => failing?.reject(new Error("user:clock already exists")));
    press(":");
    await test.user.type(screen.getByTestId("command-line-input"), "widget save{Enter}");
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
      "user:clock already exists",
      "Usage: :widget add <reference> | :widget copy <reference> [<name>] | :widget save[!] <widget> [<name>]",
    ]);
  });
});
