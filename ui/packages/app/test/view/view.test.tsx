/** L1/L4: view layouts, their validation and the renderer with its error boundaries (UI-024, AGT-010, AGT-011). */

import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { cleanup, fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect } from "react";
import type { WidgetComponent } from "../../src/sdk/index.ts";
import { containsWidget, parseLayout } from "../../src/view/layout.ts";
import { ViewRenderer } from "../../src/view/render.tsx";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { PANE_SIZES } from "../../src/view/sizes.ts";
import { DEFAULT_MANIFEST, DEFAULT_VIEW, type ResolvedView, resolveViews } from "../../src/view/views.ts";
import { renderWithHost } from "../support/host.tsx";

afterEach(cleanup);

const Named: WidgetComponent = (props) => <span data-testid="named">{String(props.label ?? "plain")}</span>;
/** A text field whose effect runs while it is shown. */
const running = new Set<string>();
const Draft: WidgetComponent = ({ label }) => {
  const name = String(label);
  useEffect(() => {
    running.add(name);
    return () => {
      running.delete(name);
    };
  }, [name]);
  return <input aria-label={name} data-testid={`draft-${name}`} />;
};
const Broken: WidgetComponent = () => {
  throw new Error("boom");
};

const registry: WidgetRegistry = new Map([
  ["omfp:named", Named],
  ["omfp:broken", Broken],
  ["omfp:draft", Draft],
]);

function widgets(): string[] {
  return screen.getAllByTestId("widget").map((element) => element.dataset.widget ?? "");
}

interface ManifestCase {
  readonly name: string;
  readonly manifest: unknown;
  readonly error: string | null;
}

// The shared fixture of the repository (`fixtures/views/cases.json`), whose shape the daemon's test checks too.
const FIXTURE = JSON.parse(readFileSync(join(import.meta.dir, "../../../../../fixtures/views/cases.json"), "utf8")) as {
  readonly cases: readonly ManifestCase[];
};
const CASES = FIXTURE.cases;

describe("layouts", () => {
  // spec: UI-024, AGT-010
  test("the default view is header over the main tabs with the chat, the side tabs on the right; the threads column is the frame's", () => {
    const { views, defaultId, error } = resolveViews(undefined);
    expect([defaultId, error, views.length]).toEqual(["default", undefined, 1]);
    expect(DEFAULT_VIEW.layout).toEqual([
      "row",
      { sizes: [3, 1] },
      ["col", {}, "omfp:header", ["tabs", { id: "main" }, "omfp:chat"]],
      ["tabs", { id: "side" }, "omfp:canvas"],
    ]);
    expect(DEFAULT_VIEW.title).toBe("Default");
    const tree = views[0]?.tree;
    expect(tree !== undefined && containsWidget(tree, ["omfp:chat"])).toBe(true);
    expect(tree !== undefined && containsWidget(tree, ["omfp:threads", "omfp:interactions"])).toBe(false);
  });

  // spec: AGT-010, DMN-088
  test("the default manifest is the daemon's, which it writes into every workspace", () => {
    const daemon: unknown = JSON.parse(
      readFileSync(join(import.meta.dir, "../../../../../crates/omfp-daemon/src/widgets/default-views.json"), "utf8"),
    );
    expect(daemon).toEqual(DEFAULT_MANIFEST);
    expect(resolveViews(DEFAULT_MANIFEST).views.map((view) => [view.id, view.title, view.layout])).toEqual([
      [DEFAULT_VIEW.id, DEFAULT_VIEW.title, DEFAULT_VIEW.layout],
    ]);
  });

  // spec: AGT-010, AGT-011
  test("every shared manifest case gets the daemon's error, views listed in id order", () => {
    expect(CASES.length).toBeGreaterThan(10);
    for (const { name, manifest, error } of CASES) {
      const resolved = resolveViews(manifest);
      expect([name, resolved.error ?? null]).toEqual([name, error]);
      if (error !== null) {
        expect([name, resolved.views.map((view) => view.id)]).toEqual([name, ["default"]]);
      }
    }
    const everything = resolveViews(CASES.find((entry) => entry.name === "everything")?.manifest);
    expect(everything.views.map((view) => [view.id, view.title])).toEqual([
      ["review", "Review"],
      ["work", "Work"],
    ]);
    expect(everything.defaultId).toBe("work");
    const ordered = resolveViews({ views: { zeta: { layout: "omfp:chat" }, alpha: { layout: "omfp:header" } } });
    expect([ordered.views.map((view) => view.id), ordered.defaultId]).toEqual([["alpha", "zeta"], "alpha"]);
    expect(resolveViews({ views: { work: { title: 3, layout: "omfp:chat" } } }).views[0]?.title).toBe("work");
  });

  // spec: UI-024
  test("widget references with props and nested containers parse; mistakes name their place", () => {
    expect(parseLayout(["omfp:changes", { repo: "x" }])).toEqual({
      ok: true,
      value: { kind: "widget", name: "omfp:changes", props: { repo: "x" } },
    });
    expect(parseLayout(["tabs", {}, "a", ["stack", {}, "b"]]).ok).toBe(true);
    expect(parseLayout(42)).toEqual({
      ok: false,
      error: "layout: expected a widget name, [widget, props] or [container, props, ...children]",
    });
    expect(parseLayout(["row", {}])).toEqual({ ok: false, error: "layout: row has no children" });
    expect(parseLayout(["row", "a"])).toEqual({ ok: false, error: "layout: row needs a props object" });
    expect(parseLayout(["row", { sizes: [1] }, "a", "b"])).toEqual({
      ok: false,
      error: "layout: sizes must be 2 positive numbers",
    });
    expect(parseLayout(["col", {}, "a", ["row", {}, ""]])).toEqual({
      ok: false,
      error: "layout[3][2]: a widget name is empty",
    });
    expect(parseLayout(["omfp:x", {}, "extra"])).toEqual({
      ok: false,
      error: "layout: a widget reference is [name, props]",
    });
  });

  // spec: UI-024
  test("an invalid manifest falls back to the default view with its error", () => {
    const invalid = resolveViews({ views: { work: { title: "Work", layout: ["row", {}] } } });
    expect([invalid.defaultId, invalid.views.map((view) => view.id), invalid.error]).toEqual([
      "default",
      ["default"],
      "views.work.layout: row has no children",
    ]);
    expect(resolveViews({ default: "nope", views: { a: { layout: "x" } } }).error).toBe("default: no view nope");
    const valid = resolveViews({ default: "b", views: { a: { title: "A", layout: "x" }, b: { layout: "y" } } });
    expect([valid.defaultId, valid.views.map((view) => view.title), valid.error]).toEqual(["b", ["A", "b"], undefined]);
  });
});

function only(manifest: unknown): ResolvedView {
  const view = resolveViews(manifest).views[0];
  if (view === undefined) {
    throw new Error("fixture");
  }
  return view;
}

/** Renders `layout` as the view `v`. */
function show(layout: unknown) {
  return renderWithHost(<ViewRenderer view={only({ views: { v: { layout } } })} registry={registry} />);
}

describe("renderer", () => {
  // spec: UI-024, AGT-010
  test("containers render their widgets in order with their props, sizes as flex weights", () => {
    show(["row", { sizes: [2, 1] }, ["omfp:named", { label: "first" }], ["col", {}, "omfp:named"]]);
    expect(screen.getByTestId("view").dataset.view).toBe("v");
    expect(widgets()).toEqual(["omfp:named", "omfp:named"]);
    expect(screen.getAllByTestId("widget").map((element) => element.dataset.path)).toEqual(["0.0", "0.1.0"]);
    expect(screen.getAllByTestId("named").map((element) => element.textContent)).toEqual(["first", "plain"]);
    const cells = screen
      .getByTestId("view")
      .querySelectorAll<HTMLElement>(":scope > [data-layout=row] > .omfp-view__cell");
    expect([...cells].map((cell) => cell.style.flexGrow)).toEqual(["2", "1"]);
    // Splitters are always there (UI-206).
    expect(screen.getAllByTestId("splitter").map((splitter) => splitter.dataset.path)).toEqual(["0"]);
  });

  // spec: UI-024, UI-092
  test("a failing or unknown widget shows its error in place; the others still render", () => {
    const quiet = console.error;
    console.error = () => {};
    try {
      show(["col", {}, "omfp:broken", "omfp:missing", "omfp:named"]);
    } finally {
      console.error = quiet;
    }
    const [broken, missing] = screen.getAllByTestId("widget");
    expect(within(broken ?? document.body).getByTestId("widget-error").textContent).toContain("boom");
    expect(within(missing ?? document.body).getByTestId("widget-error").textContent).toBe(
      "Unknown widget omfp:missing",
    );
    expect(screen.getByTestId("named").textContent).toBe("plain");
  });

  // spec: UI-024, UI-092
  test("tabs show one widget at a time and switch from the keyboard", async () => {
    show(["tabs", {}, ["omfp:named", { title: "One", label: "1" }], ["omfp:named", { label: "2" }]]);
    const tabs = screen.getAllByTestId("view-tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["One", "omfp:named"]);
    const shown = () =>
      screen
        .getAllByTestId("named")
        .filter((named) => named.closest("[hidden]") === null)
        .map((named) => named.textContent);
    expect(shown()).toEqual(["1"]);
    const user = userEvent.setup();
    await user.click(tabs[0] ?? document.body);
    await user.keyboard("{ArrowRight}");
    expect(shown()).toEqual(["2"]);
  });

  // spec: UI-400
  test("a tab not shown stays mounted, hidden and inert, its effects paused; it shows again as it was", async () => {
    show(["tabs", {}, ["omfp:draft", { label: "one" }], ["omfp:draft", { label: "two" }]]);
    const [one, two] = screen.getAllByTestId("view-tab");
    const panel = (name: string) => screen.getByTestId(`draft-${name}`).closest(".omfp-view__panel") as HTMLElement;
    expect([panel("one").hidden, panel("two").hidden]).toEqual([false, true]);
    expect([...running]).toEqual(["one"]);
    const user = userEvent.setup();
    await user.type(screen.getByTestId("draft-one"), "typed");
    await user.click(two ?? document.body);
    expect([panel("one").hidden, panel("one").hasAttribute("inert"), panel("two").hidden]).toEqual([true, true, false]);
    expect([...running]).toEqual(["two"]);
    await user.click(one ?? document.body);
    expect((screen.getByTestId("draft-one") as HTMLInputElement).value).toBe("typed");
    expect([...running]).toEqual(["one"]);
  });

  // spec: AGT-010, UI-080
  test("a built-in without a title prop is labelled with its own name", () => {
    show(["tabs", {}, "omfp:workspace", "omfp:changes", ["omfp:changes", { title: "Diffs" }]]);
    expect(screen.getAllByTestId("view-tab").map((tab) => tab.textContent)).toEqual(["Workspace", "Changes", "Diffs"]);
  });

  // spec: AGT-010
  test("a tabs container with one child shows no tab bar, only that child", () => {
    show(["tabs", { id: "main" }, ["omfp:named", { label: "chat" }]]);
    expect(screen.queryAllByTestId("view-tab").length).toBe(0);
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.getByTestId("named").textContent).toBe("chat");
    const tabs = document.querySelector("[data-layout=tabs]");
    expect(tabs?.getAttribute("data-path")).toBe("0");
    expect(screen.getByTestId("widget").dataset.path).toBe("0.0");
  });
});

describe("splitters", () => {
  const LAYOUT = [
    "row",
    { sizes: [3, 1] },
    ["col", {}, "omfp:named", ["omfp:named", { label: "x" }]],
    ["tabs", {}, "omfp:other", "omfp:named"],
  ];

  // spec: UI-206
  test("splitters between cells of rows and columns resize this window's sizes with the arrow keys in 5 % steps", () => {
    const { host } = show(LAYOUT);
    const splitters = screen.getAllByTestId("splitter");
    // Document order: the column's splitter sits inside the row's first cell.
    expect(splitters.map((splitter) => [splitter.dataset.path, splitter.dataset.index])).toEqual([
      ["0.0", "1"],
      ["0", "1"],
    ]);
    const [col, row] = splitters;
    expect(row?.getAttribute("role")).toBe("separator");
    expect(row?.getAttribute("aria-orientation")).toBe("vertical");
    expect(row?.getAttribute("aria-valuenow")).toBe("75");
    expect(col?.getAttribute("aria-orientation")).toBe("horizontal");
    expect(col?.getAttribute("aria-valuenow")).toBe("50");
    fireEvent.keyDown(row ?? document.body, { key: "ArrowRight" });
    expect(host.window.get(PANE_SIZES)).toEqual({ "-/v/0": [3.2, 0.8] });
    fireEvent.keyDown(row ?? document.body, { key: "ArrowLeft" });
    fireEvent.keyDown(col ?? document.body, { key: "ArrowDown" });
    fireEvent.keyDown(col ?? document.body, { key: "Enter" });
    expect(host.window.get(PANE_SIZES)).toEqual({ "-/v/0": [3, 1], "-/v/0.0": [1.1, 0.9] });
  });
});
