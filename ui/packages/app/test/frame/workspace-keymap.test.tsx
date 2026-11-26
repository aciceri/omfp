/** L4: the current workspace's `.omfp/keymap.json` in the window (UI-210..212) over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import type { Settings, ThreadSummary, ViewsState, WorkspaceBinding } from "@omfp/protocol";
import { act, cleanup, fireEvent, renderHook, screen, within } from "@testing-library/react";
import { Frame } from "../../src/frame/frame.tsx";
import { type Binding, NO_INIT_BINDINGS } from "../../src/keymap/keymap.ts";
import { useBindings } from "../../src/keymap/workspace.tsx";
import { type HostOptions, SELECTED_THREAD, useCommand, type WidgetComponent } from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

/** The arguments of every `demo.ping` run. */
let pings: string[][] = [];

/** A widget registering the command the workspace binds (AGT-016, AGT-030). */
const Pinging: WidgetComponent = () => {
  useCommand({
    id: "demo.ping",
    title: "Ping the demo",
    run: (args) => {
      pings.push([...args]);
    },
  });
  return <p>ping</p>;
};

const stub =
  (name: string): WidgetComponent =>
  () => <p>{name}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", Pinging],
  ["omfp:workspace", stub("workspace")],
  ["omfp:changes", stub("changes")],
]);

const threads: ThreadSummary[] = [thread("t1", "checkout", "w1"), thread("t2", "other", "w2")];

const W1_KEYMAP: WorkspaceBinding[] = [
  { mode: "normal", keys: "g p", command: "demo.ping", args: ["one"] },
  { mode: "insert", keys: "Ctrl-t", command: "demo.ping" },
];

/** The frame on `t1` (workspace `w1`, whose keymap is `keymap`), with `global` as the daemon's `[keymap]`. */
function setup(keymap: WorkspaceBinding[], global: Settings["keymap"] = [], importer?: HostOptions["importer"]) {
  pings = [];
  const base = testHost(undefined, { importer: importer ?? (() => new Promise<unknown>(() => {})) });
  for (const method of ["widgets.report", "settings.set"] as const satisfies readonly CallMethod[]) {
    base.client.answers.set(method, () => ({}));
  }
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1"), workspace("w2")],
    threads,
    settings: { ui: {}, keymap: global, error: null },
  });
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  test.client.views("w1", { keymap });
  test.flush();
  return test;
}

function keys(...sequence: string[]): void {
  for (const key of sequence) {
    act(() => {
      fireEvent.keyDown(document.activeElement ?? document.body, { key });
    });
  }
}

/** The rows of a help section: `[keys, args, description]`. */
function rows(section: string): string[][] {
  const element = screen.getByTestId("help").querySelector(`[data-section="${section}"]`);
  if (!(element instanceof HTMLElement)) {
    return [];
  }
  return within(element)
    .queryAllByTestId("help-row")
    .map((row) => [
      [...row.querySelectorAll("kbd")].map((chip) => chip.textContent).join(", "),
      row.querySelector(".omfp-help__args")?.textContent ?? "",
      row.querySelector(".omfp-help__description")?.textContent ?? "",
    ]);
}

/** The window's bindings as the frame merges them, without init files: `keymap` is the workspace's file. */
function bindings(defaults: readonly Binding[], global: Settings["keymap"], keymap?: WorkspaceBinding[]) {
  const views: ViewsState | undefined = keymap === undefined ? undefined : { widgets: [], stored: {}, keymap };
  const { result, unmount } = renderHook(() => useBindings(defaults, global, views, NO_INIT_BINDINGS));
  unmount();
  return result.current;
}

describe("workspace keymap", () => {
  // spec: UI-210, UI-097
  test("the workspace's bindings win over the `[keymap]` entries, which win over the defaults", () => {
    const defaults: Binding[] = [
      { mode: "normal", keys: "g n", command: "notice.open" },
      { mode: "normal", keys: "?", command: "help.open" },
      { mode: "normal", keys: "j", command: "nav.next" },
    ];
    const merged = bindings(
      defaults,
      [
        { mode: "normal", keys: "g n", command: "thread.new" },
        { mode: "normal", keys: "g h", command: "help.open" },
        { mode: "normal", keys: "x", command: "thread.archive" },
      ],
      [
        { mode: "normal", keys: "g  n", command: "demo.ping", args: ["one"] },
        { mode: "normal", keys: "x", command: "" },
        { mode: "insert", keys: "j", command: "demo.ping" },
      ],
    );
    expect(merged).toEqual([
      { mode: "normal", keys: "g n", command: "demo.ping", args: ["one"], source: "workspace" },
      { mode: "normal", keys: "?", command: "help.open" },
      { mode: "normal", keys: "j", command: "nav.next" },
      { mode: "normal", keys: "g h", command: "help.open", source: "config" },
      { mode: "insert", keys: "j", command: "demo.ping", source: "workspace" },
    ]);
    expect(bindings(defaults, [])).toEqual(defaults);
  });

  // spec: UI-210, AGT-030
  test("its keys run a widget's command with their arguments only while that workspace is current", () => {
    const test = setup(W1_KEYMAP, [{ mode: "normal", keys: "g p", command: "help.open" }]);
    keys("g", "p");
    expect(pings).toEqual([["one"]]);
    expect(screen.queryByTestId("help")).toBeNull();

    // A live change of the file applies at once.
    test.client.viewsEvents("w1", {
      kind: "views_keymap",
      keymap: [{ mode: "normal", keys: "g p", command: "demo.ping", args: ["two"] }],
      error: null,
    });
    test.flush();
    keys("g", "p");
    expect(pings).toEqual([["one"], ["two"]]);

    // Another workspace: its own keymap (none), so the `[keymap]` entry applies again.
    act(() => test.host.window.set(SELECTED_THREAD, "t2"));
    test.client.views("w2");
    test.flush();
    keys("g", "p");
    expect(pings).toEqual([["one"], ["two"]]);
    expect(screen.getByTestId("help")).toBeTruthy();
  });

  // spec: UI-095, UI-210, UI-262, UI-322
  test.each([false, true])(
    "empty workspace/init updates preserve a pending leader (global override: %s)",
    async (configured) => {
      const test = setup([], configured ? [{ mode: "normal", keys: "g p", command: "demo.ping" }] : [], () =>
        Promise.resolve({ default: () => {} }),
      );
      keys(" ");
      test.client.viewsEvents("w1", { kind: "views_keymap", keymap: [], error: null });
      test.client.viewsEvents("w1", { kind: "views_init", init: "no-bindings" });
      test.flush();
      await act(async () => {
        await test.host.init.settled();
      });
      test.flush();
      keys("/");
      expect(screen.queryAllByTestId("thread-picker").length).toBe(1);
      expect(screen.queryAllByTestId("conversation-search").length).toBe(0);
    },
  );

  // spec: UI-210, UI-322
  test("a real binding change resets the pending leader and applies the new map", () => {
    const test = setup([], [{ mode: "normal", keys: "Space p", command: "demo.ping" }]);
    keys(" ");
    test.client.viewsEvents("w1", {
      kind: "views_keymap",
      keymap: [{ mode: "normal", keys: "Space p", command: "demo.ping", args: ["new"] }],
      error: null,
    });
    test.flush();
    keys("p");
    expect(pings).toEqual([]);
    keys(" ", "p");
    expect(pings).toEqual([["new"]]);
  });

  // spec: UI-212, AGT-011
  test("an invalid file shows its error in a banner while its last valid bindings stay", () => {
    const test = setup(W1_KEYMAP);
    expect(screen.queryByTestId("keymap-error")).toBeNull();
    test.client.viewsEvents("w1", {
      kind: "views_keymap",
      keymap: W1_KEYMAP,
      error: "bindings[2].keys: expected a key sequence",
    });
    test.flush();
    expect(screen.queryAllByTestId("keymap-error").length).toBe(1);
    keys("g", "p");
    expect(pings).toEqual([["one"]]);

    test.client.viewsEvents("w1", { kind: "views_keymap", keymap: W1_KEYMAP, error: null });
    test.flush();
    expect(screen.queryByTestId("keymap-error")).toBeNull();
  });

  // spec: UI-211, UI-198
  test("the help panel lists the workspace's bindings in a group of their own, first", () => {
    setup([
      ...W1_KEYMAP,
      { mode: "normal", keys: "?", command: "help.open" },
      { mode: "normal", keys: "j", command: "" },
    ]);
    keys("?");
    const help = screen.getByTestId("help");
    expect([...help.querySelectorAll("[data-section]")].map((section) => section.getAttribute("data-section"))).toEqual(
      ["workspace", "normal", "threads", "view", "insert", "transcript", "cmdline", "editor", "commands"],
    );
    // In keymap order: a binding that replaces a default keeps its place.
    expect(rows("workspace")).toEqual([
      ["?", "", "Show every key and command (this help)"],
      ["g p", "one", "Ping the demo"],
      ["ctrl+t", "", "Ping the demo (insert)"],
    ]);
    // A `normal` entry speaks for Threads and View mode both: `j` is gone from either.
    const chips = ["normal", "threads", "view"].flatMap((section) => rows(section).map(([key]) => key));
    expect(chips).not.toContain("?");
    expect(chips).not.toContain("j");
  });
});
