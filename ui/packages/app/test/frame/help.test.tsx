/** L4: the help panel (UI-198) over a fake client: `?` and `:help`, the effective keymap, widget commands, the filter. */

import { afterEach, describe, expect, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import type { Settings, ThreadSummary } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Frame } from "../../src/frame/frame.tsx";
import { builtinDescription } from "../../src/frame/help.ts";
import { DEFAULT_BINDINGS } from "../../src/keymap/keymap.ts";
import { useCommand, type WidgetComponent } from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

const stub =
  (name: string): WidgetComponent =>
  () => <p data-testid={`stub-${name}`}>{name}</p>;

/** A widget registering its own command, with arguments (AGT-016). */
const Pinging: WidgetComponent = () => {
  useCommand({
    id: "demo.ping",
    title: "Ping the demo",
    aliases: ["ping"],
    args: () => ["once", "twice"],
    run: () => {},
  });
  return <p>ping</p>;
};

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", Pinging],
  ["omfp:workspace", stub("workspace")],
  ["omfp:transcript", stub("transcript")],
]);

const threads: ThreadSummary[] = [thread("t1", "checkout", "w1"), thread("t2", "flaky", "w1")];

function setup() {
  const base = testHost(undefined, { importer: () => new Promise<unknown>(() => {}) });
  for (const method of ["widgets.report", "settings.set"] as const satisfies readonly CallMethod[]) {
    base.client.answers.set(method, () => ({}));
  }
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads });
  test.flush();
  const keymap = (entries: Settings["keymap"]) => {
    test.client.indexEvents({ kind: "settings_changed", settings: { ui: {}, keymap: entries, error: null } });
    test.flush();
  };
  return { ...test, keymap, user: userEvent.setup() };
}

function press(key: string, init: KeyboardEventInit = {}): void {
  act(() => {
    fireEvent.keyDown(document.activeElement ?? document.body, { key, ...init });
  });
}

/** The rows of a section: `[keys, args, description]`. */
function rows(section: string, command?: string): string[][] {
  const element = screen.getByTestId("help").querySelector(`[data-section="${section}"]`);
  if (!(element instanceof HTMLElement)) {
    return [];
  }
  return within(element)
    .queryAllByTestId("help-row")
    .filter((row) => command === undefined || row.dataset.command === command)
    .map((row) => [
      [...row.querySelectorAll("kbd")].map((chip) => chip.textContent).join(", "),
      row.querySelector(".omfp-help__args")?.textContent ?? "",
      row.querySelector(".omfp-help__description")?.textContent ?? "",
    ]);
}

describe("help panel", () => {
  // spec: UI-198
  test("every default binding has a human description", () => {
    expect(DEFAULT_BINDINGS.filter((binding) => builtinDescription(binding.command) === undefined)).toEqual([]);
  });

  // spec: UI-198, UI-095, UI-187
  test("`?` opens it on the default bindings by mode; Escape closes it and gives the focus back", async () => {
    const { user } = setup();
    const row = screen.getAllByTestId("thread-row")[0];
    row?.focus();
    press("?", { shiftKey: true });
    const help = screen.getByTestId("help");
    expect(help.querySelector(".omfp-bbox__label")?.textContent).toBe("help");
    expect(document.activeElement === screen.getByTestId("help-filter")).toBe(true);
    expect([...help.querySelectorAll("[data-section]")].map((section) => section.getAttribute("data-section"))).toEqual(
      ["normal", "insert", "transcript", "cmdline", "editor", "commands"],
    );
    expect(rows("normal", "nav.next")).toEqual([["j", "", "Next thread"]]);
    expect(rows("normal", "thread.nextAttention")).toEqual([
      ["spc a, ] a, alt+a", "", "Next thread needing attention"],
    ]);
    expect(rows("normal", "help.open")).toEqual([["?, spc h h", "", "Show every key and command (this help)"]]);
    expect(rows("transcript", "nav.next")).toEqual([
      ["j", "", "Next message (a long one scrolls down to its end first)"],
    ]);
    expect(rows("transcript", "nav.halfDown")).toEqual([["ctrl+d", "", "Scroll half a page down"]]);
    expect(rows("insert", "focus.leave")).toEqual([["esc", "", "Leave the composer"]]);
    expect(rows("commands", "window.theme")).toEqual([
      [
        ":theme",
        "system | light | dark | synth | catppuccin | gruvbox | tokyo-night | nord | solarized | rose-pine | graphite | paper | clay | primer | accent | matugen",
        "Colour scheme (system, light, dark) and theme (synth, catppuccin, gruvbox, …; matugen: the user theme file, Synth without one)",
      ],
    ]);
    await user.keyboard("{Escape}");
    expect(screen.queryByTestId("help")).toBeNull();
    expect(document.activeElement === row).toBe(true);
  });

  // spec: UI-198, UI-097
  test("`:help` opens it; `[keymap]` entries show live, removed defaults are gone", async () => {
    const { keymap, user } = setup();
    keymap([
      { mode: "normal", keys: "Space n", command: "" },
      { mode: "normal", keys: "Space h", command: "thread.new" },
    ]);
    document.body.focus();
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "help{Enter}");
    expect(screen.queryByTestId("command-line")).toBeNull();
    expect(rows("normal", "thread.new")).toEqual([["alt+n, spc h", "", "New thread in the current workspace"]]);
    keymap([{ mode: "normal", keys: "g h", command: "help.open" }]);
    expect(rows("normal", "help.open")).toEqual([["?, spc h h, g h", "", "Show every key and command (this help)"]]);
    expect(rows("normal", "thread.new")).toEqual([["spc n, alt+n", "", "New thread in the current workspace"]]);
  });

  // spec: UI-198, UI-095
  test("a widget's command shows with its arguments, and under the keys a `[keymap]` entry binds to it", () => {
    const { keymap } = setup();
    keymap([{ mode: "normal", keys: "g p", command: "demo.ping" }]);
    press("?");
    expect(rows("commands", "demo.ping")).toEqual([[":ping", "once | twice", "Ping the demo"]]);
    expect(rows("normal", "demo.ping")).toEqual([["g p", "", "Ping the demo"]]);
  });

  // spec: UI-198
  test("the filter keeps the rows matching a key or a description; `?` closes only while it is empty", async () => {
    const { user } = setup();
    press("?");
    const filter = screen.getByTestId("help-filter");
    await user.type(filter, "ctrl+c");
    expect(rows("normal")).toEqual([["ctrl+c", "", "Interrupt the agent"]]);
    expect(rows("insert")).toEqual([["ctrl+c", "", "Clear the draft, or abort when it is empty (Ctrl-C)"]]);
    expect(screen.getByTestId("help").querySelector('[data-section="commands"]')).toBeNull();
    await user.clear(filter);
    await user.type(filter, "ARCHIVE");
    expect(rows("normal")).toEqual([
      ["spc x", "", "Archive the thread"],
      ["spc u", "", "Unarchive the thread"],
    ]);
    // `:set` matches through its `delete_archived_after=` argument.
    expect(rows("commands").map(([keys]) => keys)).toEqual([":archive", ":unarchive", ":set"]);
    await user.type(filter, "?");
    expect((filter as HTMLInputElement).value).toBe("ARCHIVE?");
    expect(screen.getByTestId("help-empty").textContent).toBe("No key or command matches");
    await user.clear(filter);
    press("?");
    expect(screen.queryByTestId("help")).toBeNull();
  });
});
