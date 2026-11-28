/** L4: the left column collapsed to zero width in a wide frame, and back from the mode line (UI-985..UI-988). */

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import { act, cleanup, fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Frame } from "../../src/frame/frame.tsx";
import { commandGroup } from "../../src/keymap/which-key.ts";
import { type KeyValueStorage, SELECTED_THREAD, type WidgetComponent } from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, type TestHost, testHost } from "../support/host.tsx";
import { installResizeObserver, resize } from "../support/resize.ts";

let restore = () => {};
beforeEach(() => {
  restore = installResizeObserver();
});
afterEach(() => {
  cleanup();
  restore();
  delete document.documentElement.dataset.skin;
});

const stub =
  (name: string): WidgetComponent =>
  () => <p data-testid={`stub-${name}`}>{name}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", stub("chat")],
  ["omfp:workspace", stub("workspace")],
]);

/** A storage the test keeps across mounts: a reload of the window. */
function memoryStorage(): KeyValueStorage & { readonly stored: Map<string, string> } {
  const stored = new Map<string, string>();
  return {
    stored,
    getItem: (key) => stored.get(key) ?? null,
    setItem: (key, value) => {
      stored.set(key, value);
    },
  };
}

function setup(storage: KeyValueStorage = memoryStorage()) {
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: new ManualTimers(),
    noticeTimers: new ManualTimers(),
    storage,
  });
  for (const method of ["settings.set", "widgets.report"] as const satisfies readonly CallMethod[]) {
    base.client.answers.set(method, () => ({}));
  }
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1")],
    threads: [thread("t1", "checkout", "w1"), thread("t2", "flaky", "w1")],
  });
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  test.client.views("w1", {});
  test.flush();
  return { ...test, user: userEvent.setup() };
}

function press(key: string): void {
  act(() => {
    fireEvent.keyDown(document.activeElement ?? document.body, { key });
  });
}

const column = () => screen.getByTestId("left-column");
const collapsed = () => column().hidden;
const expandButtons = () => screen.queryAllByTestId("left-expand");
const focused = () => document.activeElement?.getAttribute("data-testid");

describe("collapsing the left column", () => {
  // spec: UI-985
  test("the column's Hide threads button collapses it to nothing: no strip, no splitter, the view fills the frame", async () => {
    const { user } = setup();
    expect([collapsed(), screen.queryAllByTestId("left-splitter").length, expandButtons().length]).toEqual([
      false,
      1,
      0,
    ]);
    const hide = screen.getByRole("button", { name: "Hide threads" });
    expect(hide.closest(".omfp-brand")).not.toBeNull();
    await user.click(hide);
    expect(collapsed()).toBe(true);
    expect(column().dataset.collapsed).toBe("true");
    expect([
      screen.queryAllByTestId("left-splitter").length,
      screen.queryAllByTestId("left-strip").length,
      screen.queryAllByRole("button", { name: "Hide threads" }).length,
    ]).toEqual([0, 0, 0]);
    // The view is the shell's one visible child: nothing else takes room beside it.
    const shown = [...screen.getByTestId("shell").children].filter((child) => !(child as HTMLElement).hidden);
    expect(shown.map((child) => child.className)).toEqual(["omfp-frame__main"]);
    // The widget stays mounted, hidden; the focus that was on the button goes to the mode line's, which stands for the
    // column: the window stays in Threads mode (UI-095).
    expect(screen.getByTestId("sidebar").isConnected).toBe(true);
    expect(focused()).toBe("left-expand");
    expect(screen.getByTestId("mode-line").dataset.mode).toBe("threads");
  });

  // spec: UI-985
  test("Space b and :threads.collapse hide and show the column; help and which-key list it under Views", async () => {
    const { host, user } = setup();
    press(" ");
    press("b");
    expect(collapsed()).toBe(true);
    press(" ");
    press("b");
    expect(collapsed()).toBe(false);
    act(() => {
      host.commands.run("threads.collapse");
    });
    expect(collapsed()).toBe(true);
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "threads.collapse{Enter}");
    expect(collapsed()).toBe(false);
    const command = host.commands.list().find((candidate) => candidate.id === "threads.collapse");
    expect([command?.title, commandGroup("threads.collapse")]).toEqual(["Hide or show the left column", "views"]);
    press("?");
    const help = (section: string) =>
      [
        ...(screen
          .getByTestId("help")
          .querySelector(`[data-section="${section}"]`)
          ?.querySelectorAll<HTMLElement>('[data-testid="help-row"][data-command="threads.collapse"]') ?? []),
      ].map((row) => [
        [...row.querySelectorAll("kbd")].map((chip) => chip.textContent).join(", "),
        row.querySelector(".omfp-help__description")?.textContent ?? "",
      ]);
    expect([help("normal"), help("commands")]).toEqual([
      [["spc b", "Hide or show the left column"]],
      [[":threads.collapse", "Hide or show the left column"]],
    ]);
  });

  // spec: UI-986
  test("the mode line starts with Show threads while collapsed; it brings the column back at its width", async () => {
    const { host, user } = setup();
    act(() => {
      host.commands.run("threads.collapse");
    });
    const [show] = expandButtons();
    const bar = screen.getByTestId("mode-bar");
    expect(bar.firstElementChild?.getAttribute("data-testid")).toBe("left-expand");
    expect(show?.getAttribute("aria-controls")).toBe("omfp-left-column");
    await user.click(show ?? document.body);
    expect([collapsed(), expandButtons().length, screen.queryAllByTestId("left-splitter").length]).toEqual([
      false,
      0,
      1,
    ]);
    expect(focused()).toBe("left-collapse");
  });

  // spec: UI-987
  test("collapsed, Space t shows the column and focuses it; the attention keys select without showing it", () => {
    const { client, flush, host } = setup();
    act(() => {
      host.commands.run("threads.collapse");
    });
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { needsAttention: true }) });
    flush();
    act(() => {
      host.commands.run("thread.nextAttention");
    });
    expect([host.window.get(SELECTED_THREAD), collapsed()]).toEqual(["t2", true]);
    act(() => (document.activeElement as HTMLElement | null)?.blur());
    press(" ");
    press("t");
    expect(collapsed()).toBe(false);
    expect(column().contains(document.activeElement)).toBe(true);
    expect(focused()).toBe("thread-row");
  });

  // spec: UI-987
  test("collapsed, Space w (workspace.new) shows the column and focuses its new-workspace field", () => {
    const { host } = setup();
    act(() => {
      host.commands.run("threads.collapse");
    });
    press(" ");
    press("w");
    expect([collapsed(), focused()]).toEqual([false, "new-workspace-input"]);
  });

  // spec: UI-987
  test("narrow and phone frames keep the strip and the drawer: the state waits for a wide frame", () => {
    const { host } = setup();
    act(() => {
      host.commands.run("threads.collapse");
    });
    const app = screen.getByTestId("app");
    resize(app, 900);
    expect([collapsed(), expandButtons().length, screen.queryAllByTestId("left-strip").length]).toEqual([false, 0, 1]);
    expect(
      host.commands
        .list()
        .find((command) => command.id === "threads.collapse")
        ?.enabled?.(),
    ).toBe(false);
    resize(app, 390);
    expect([collapsed(), expandButtons().length, screen.queryAllByTestId("area-bar").length]).toEqual([false, 0, 1]);
    resize(app, 1400);
    expect([collapsed(), expandButtons().length]).toEqual([true, 1]);
  });

  // spec: UI-988
  test("the collapsed state belongs to the window and survives a reload", () => {
    const storage = memoryStorage();
    const first = setup(storage);
    act(() => {
      first.host.commands.run("threads.collapse");
    });
    expect(storage.stored.get("omfp.leftCollapsed")).toBe("true");
    cleanup();
    setup(storage);
    expect([collapsed(), expandButtons().length]).toEqual([true, 1]);
  });

  // spec: UI-224, UI-988
  test("each view keeps the state chosen on it; a view never chosen on follows the window's last choice", () => {
    const storage = memoryStorage();
    const manifest = {
      default: "a",
      views: {
        a: { title: "A", layout: "omfp:chat" },
        b: { title: "B", layout: "omfp:header" },
        c: { title: "C", layout: "omfp:workspace" },
      },
    };
    const show = ({ host }: TestHost, view: string) =>
      act(() => {
        host.commands.run("view.switch", [view]);
      });
    const first = setup(storage);
    first.client.views("w1", { manifest });
    first.flush();
    act(() => {
      first.host.commands.run("threads.collapse");
    });
    show(first, "b");
    // Never chosen on b: the window's last choice.
    expect(collapsed()).toBe(true);
    act(() => {
      first.host.commands.run("threads.collapse");
    });
    expect(collapsed()).toBe(false);
    show(first, "a");
    expect(collapsed()).toBe(true);
    show(first, "c");
    expect(collapsed()).toBe(false);
    cleanup();
    // After a reload too.
    const second = setup(storage);
    second.client.views("w1", { manifest });
    second.flush();
    show(second, "a");
    expect(collapsed()).toBe(true);
    show(second, "b");
    expect(collapsed()).toBe(false);
  });

  // spec: UI-985, UI-986
  test("both skins render the buttons with their accessible names", () => {
    for (const skin of ["crush", "boring"] as const) {
      const { client, flush, host } = setup();
      client.indexEvents({ kind: "settings_changed", settings: { ui: { skin }, keymap: [], error: null } });
      flush();
      expect(document.documentElement.dataset.skin).toBe(skin);
      expect(screen.getAllByRole("button", { name: "Hide threads" }).length).toBe(1);
      act(() => {
        host.commands.run("threads.collapse");
      });
      expect(screen.getAllByRole("button", { name: "Show threads" }).map((button) => button.dataset.testid)).toEqual([
        "left-expand",
      ]);
      cleanup();
    }
  });
});
