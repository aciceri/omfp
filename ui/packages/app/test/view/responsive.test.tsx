/** L1/L4: window sizes of the splitters (UI-206, UI-207) and a view on a phone (UI-204). */

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, fireEvent, screen } from "@testing-library/react";
import { breakpointOf, stripThreads } from "../../src/frame/responsive.ts";
import { OmfpProvider, type WidgetComponent } from "../../src/sdk/index.ts";
import { type Area, chatCell, type LayoutNode, parseLayout } from "../../src/view/layout.ts";
import { ViewRenderer } from "../../src/view/render.tsx";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import {
  decodePaneSizes,
  effectiveSizes,
  LEFT_MIN_REM,
  PANE_SIZES,
  resizeLeft,
  sizesKey,
  withSizes,
} from "../../src/view/sizes.ts";
import { resolveViews } from "../../src/view/views.ts";
import { thread } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

describe("breakpoints", () => {
  // spec: UI-200
  test("the frame's own width picks wide, narrow (under 1100 px) or mobile (under 700 px); unmeasured stays wide", () => {
    expect([0, 1400, 1100, 1099, 900, 700, 699, 390].map(breakpointOf)).toEqual([
      "wide",
      "wide",
      "wide",
      "narrow",
      "narrow",
      "narrow",
      "mobile",
      "mobile",
    ]);
  });

  // spec: UI-201
  test("the strip lists the threads needing the user and the running ones, in index order, never archived ones", () => {
    const threads = [
      thread("idle", "idle", "w1"),
      thread("asks", "asks", "w1", { needsAttention: true }),
      thread("busy", "", "w1", { settled: false }),
      thread("broke", "broke", "w1", { process: { state: "failed", message: "x" } }),
      thread("old", "old", "w1", { needsAttention: true, archivedAt: 5 }),
      thread("off", "off", "w1", { process: { state: "stopped" } }),
    ];
    expect(stripThreads(threads).map((item) => [item.id, item.status])).toEqual([
      ["asks", "awaiting"],
      ["busy", "running"],
      ["broke", "failed"],
    ]);
    expect(stripThreads(threads)[1]?.title).toBe("New thread");
  });

  // spec: UI-204
  test("a root row's chat area is the cell with the main slot, else the chat, else the first", () => {
    const tree = (node: LayoutNode) => {
      const parsed = parseLayout(node);
      if (!parsed.ok) {
        throw new Error(parsed.error);
      }
      return parsed.value;
    };
    expect(
      chatCell(tree(["row", {}, "omfp:workspace", ["col", {}, "omfp:header", ["tabs", { id: "main" }, "omfp:chat"]]])),
    ).toBe(1);
    expect(chatCell(tree(["row", {}, "omfp:workspace", "omfp:chat"]))).toBe(1);
    expect(chatCell(tree(["row", {}, "omfp:workspace", "omfp:changes"]))).toBe(0);
    expect(chatCell(tree(["col", {}, "omfp:header", "omfp:chat"]))).toBeUndefined();
    expect(chatCell(tree(["row", {}, "omfp:chat"]))).toBeUndefined();
    expect(chatCell(tree("omfp:chat"))).toBeUndefined();
  });
});

describe("window sizes", () => {
  // spec: UI-206
  test("stored sizes keep only lists of positive weights; anything else is no sizes", () => {
    expect(decodePaneSizes({ a: [1, 2], b: [0, 1], c: "x", d: [], e: [1, Number.NaN] })).toEqual({ a: [1, 2] });
    expect(decodePaneSizes([1, 2])).toBeUndefined();
    expect(decodePaneSizes(null)).toBeUndefined();
  });

  // spec: UI-206
  test("a window's sizes apply while the container has one per cell; else the view's defaults", () => {
    expect(sizesKey("w1", "default", "0.1")).toBe("w1/default/0.1");
    expect(sizesKey(undefined, "default", "0")).toBe("-/default/0");
    expect(effectiveSizes([3, 1], [1, 1], 2)).toEqual([1, 1]);
    expect(effectiveSizes([3, 1], [1, 1, 1], 2)).toEqual([3, 1]);
    expect(effectiveSizes(undefined, undefined, 2)).toBeUndefined();
    expect(effectiveSizes(undefined, [2, 1], 2)).toEqual([2, 1]);
  });

  // spec: UI-206
  test("setting and dropping one container's sizes leaves the others", () => {
    const sizes = withSizes({ a: [1, 2] }, "b", [3, 1]);
    expect(sizes).toEqual({ a: [1, 2], b: [3, 1] });
    expect(withSizes(sizes, "a", undefined)).toEqual({ b: [3, 1] });
    expect(withSizes(sizes, "z", undefined)).toBe(sizes);
  });

  // spec: UI-207
  test("the left column moves by shares of the frame, between 12rem and half the frame", () => {
    expect(resizeLeft(17, 0.05, 80)).toBe(21);
    expect(resizeLeft(17, -0.5, 80)).toBe(LEFT_MIN_REM);
    expect(resizeLeft(17, 0.5, 80)).toBe(40);
    expect(resizeLeft(17, 0.5, 20)).toBe(LEFT_MIN_REM);
  });
});

const Named: WidgetComponent = (props) => (
  <span data-testid={`named-${String(props.label)}`}>{String(props.label)}</span>
);
const registry: WidgetRegistry = new Map([
  ["omfp:named", Named],
  ["omfp:header", () => <header data-testid="named-header">header</header>],
  ["omfp:chat", () => <p data-testid="named-chat">chat</p>],
]);

const named = (label: string): LayoutNode => ["omfp:named", { label, title: label }];

function show(layout: LayoutNode, options: { area?: Area; workspaceId?: string } = {}) {
  const resolved = resolveViews({ views: { v: { title: "V", layout } } }).views[0];
  if (resolved === undefined) {
    throw new Error("no view");
  }
  const base = testHost();
  const { area, workspaceId = "w1" } = options;
  const ui = (view = resolved) => (
    <ViewRenderer view={view} registry={registry} workspaceId={workspaceId} area={area} />
  );
  return { ...renderWithHost(ui(), base), ui };
}

function grows(path: string): string[] {
  const container = document.querySelector(`[data-path="${path}"]`);
  return [...(container?.querySelectorAll<HTMLElement>(":scope > .omfp-view__cell") ?? [])].map(
    (cell) => cell.style.flexGrow,
  );
}

function splitter(path: string): HTMLElement {
  const found = screen.getAllByTestId("splitter").find((element) => element.dataset.path === path);
  if (found === undefined) {
    throw new Error(`no splitter at ${path}`);
  }
  return found;
}

describe("always-active splitters", () => {
  const LAYOUT: LayoutNode = ["row", { sizes: [3, 1] }, ["col", {}, "omfp:header", "omfp:chat"], named("side")];

  // spec: UI-206
  test("a splitter resizes the cells for this window only, kept by workspace, view and path", () => {
    const { host } = show(LAYOUT);
    // None under the header (as tall as its content); the row's has the cells' share.
    expect(screen.getAllByTestId("splitter").map((element) => element.dataset.path)).toEqual(["0"]);
    expect(splitter("0").getAttribute("aria-valuenow")).toBe("75");
    fireEvent.keyDown(splitter("0"), { key: "ArrowRight" });
    expect(host.window.get(PANE_SIZES)).toEqual({ "w1/v/0": [3.2, 0.8] });
    expect(grows("0")).toEqual(["3.2", "0.8"]);
    expect(splitter("0").getAttribute("aria-valuenow")).toBe("80");
    fireEvent.keyDown(splitter("0"), { key: "ArrowLeft" });
    fireEvent.keyDown(splitter("0"), { key: "ArrowLeft" });
    expect(grows("0")).toEqual(["2.8", "1.2"]);
  });

  // spec: UI-206
  test("a double click goes back to the view's sizes", () => {
    const { host } = show(LAYOUT);
    act(() => host.window.set(PANE_SIZES, { "w1/v/0": [1, 1], "w2/v/0": [2, 2] }));
    expect(grows("0")).toEqual(["1", "1"]);
    fireEvent.doubleClick(splitter("0"));
    expect(grows("0")).toEqual(["3", "1"]);
    expect(host.window.get(PANE_SIZES)).toEqual({ "w2/v/0": [2, 2] });
  });

  // spec: UI-206
  test("a pointer drag previews the move and keeps it for the window once released", () => {
    const { host } = show(LAYOUT);
    const bar = splitter("0");
    const container = bar.parentElement ?? document.body;
    container.getBoundingClientRect = () => DOMRect.fromRect({ x: 0, y: 0, width: 400, height: 300 });
    bar.setPointerCapture = () => {};
    fireEvent.pointerDown(bar, { pointerId: 1, clientX: 300, clientY: 0, button: 0 });
    expect(bar.dataset.dragging).toBe("true");
    fireEvent.pointerMove(bar, { pointerId: 1, clientX: 260, clientY: 0 });
    expect(grows("0")).toEqual(["2.6", "1.4"]);
    expect(host.window.get(PANE_SIZES)).toEqual({});
    fireEvent.pointerUp(bar, { pointerId: 1, clientX: 260, clientY: 0 });
    expect(host.window.get(PANE_SIZES)).toEqual({ "w1/v/0": [2.6, 1.4] });
    expect(grows("0")).toEqual(["2.6", "1.4"]);
    expect(bar.dataset.dragging).toBeUndefined();
  });

  // spec: UI-206
  test("a window's sizes give way when the container gets another number of cells", () => {
    const { host, view, ui } = show(LAYOUT);
    act(() => host.window.set(PANE_SIZES, { "w1/v/0": [1, 1] }));
    expect(grows("0")).toEqual(["1", "1"]);
    const [three] = resolveViews({
      views: { v: { title: "V", layout: ["row", { sizes: [1, 2, 1] }, named("a"), named("b"), named("c")] } },
    }).views;
    if (three === undefined) {
      throw new Error("no view");
    }
    view.rerender(<OmfpProvider host={host}>{ui(three)}</OmfpProvider>);
    expect(grows("0")).toEqual(["1", "2", "1"]);
  });
});

describe("a view on a phone", () => {
  const LAYOUT: LayoutNode = [
    "row",
    { sizes: [3, 1, 1] },
    ["col", {}, "omfp:header", ["tabs", { id: "main" }, "omfp:chat"]],
    ["col", {}, named("repos"), named("changes")],
    named("todos"),
  ];

  const shown = () =>
    [...document.querySelectorAll<HTMLElement>("[data-testid^=named-]")]
      .filter((element) => element.closest("[hidden]") === null)
      .map((element) => element.textContent);

  // spec: UI-204
  test("the chat area alone, header over chat, without splitters; every widget stays mounted", () => {
    show(LAYOUT, { area: "chat" });
    expect(shown()).toEqual(["header", "chat"]);
    // Compare counts and strings, never elements: Bun's diff of a React-rendered element prints its whole fiber graph
    // (gigabytes) when the expectation fails.
    expect(screen.queryAllByTestId("splitter").length).toBe(0);
    // The side area keeps its column's tabs mounted, hidden with it.
    expect(screen.queryAllByTestId("cell-tab").filter((tab) => tab.closest("[hidden]") === null).length).toBe(0);
    expect(document.querySelectorAll("[data-testid^=named-]").length).toBe(5);
    expect(screen.getByTestId("view").dataset.area).toBe("chat");
  });

  // spec: UI-204
  test("the side area shows the other cells as tabs, and a column of boxes as tabs too", () => {
    show(LAYOUT, { area: "side" });
    const tabs = () => screen.getAllByTestId("cell-tab");
    expect(tabs().map((tab) => [tab.textContent, tab.getAttribute("aria-selected")])).toEqual([
      ["col", "true"],
      ["todos", "false"],
      ["repos", "true"],
      ["changes", "false"],
    ]);
    expect(shown()).toEqual(["repos"]);
    act(() => (tabs()[3] ?? document.body).click());
    expect(shown()).toEqual(["changes"]);
    act(() => tabs()[0]?.focus());
    fireEvent.keyDown(tabs()[0] ?? document.body, { key: "ArrowRight" });
    expect(shown()).toEqual(["todos"]);
    expect(document.activeElement === tabs()[1]).toBe(true);
  });
});
