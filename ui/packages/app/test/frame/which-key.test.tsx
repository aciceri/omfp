/** L4: the which-key panel and the Space leader menu in the window (UI-320..UI-326). */

import { afterEach, describe, expect, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import type { Settings } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, within } from "@testing-library/react";
import { Frame } from "../../src/frame/frame.tsx";
import { initUrl } from "../../src/frame/init.tsx";
import { type InitApi, useCommand, type WidgetComponent } from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

/** A widget registering its own command (AGT-016). */
const Pinging: WidgetComponent = () => {
  useCommand({ id: "demo.ping", title: "Ping the demo", run: () => {} });
  return <p>ping</p>;
};

const stub =
  (name: string): WidgetComponent =>
  () => <p>{name}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", Pinging],
  ["omfp:workspace", stub("workspace")],
  ["omfp:transcript", stub("transcript")],
]);

const USER_URL = initUrl("/widgets/test", "user", "u1");

let log: string[] = [];

const userInit = (omfp: InitApi) => {
  omfp.command({
    id: "my.hello",
    title: "Say hello",
    run: () => {
      log.push("hello");
    },
  });
  omfp.bind([
    { keys: "g h", command: "my.hello" },
    { keys: "Space o h", command: "my.hello" },
  ]);
};

/** The window with the user's init file run. */
async function setup() {
  log = [];
  const base = testHost(undefined, {
    importer: (url) => (url === USER_URL ? Promise.resolve({ default: userInit }) : new Promise<unknown>(() => {})),
  });
  for (const method of ["widgets.report", "settings.set"] as const satisfies readonly CallMethod[]) {
    base.client.answers.set(method, () => ({}));
  }
  const timers = new ManualTimers();
  const test = renderWithHost(<Frame registry={registry} now={() => 0} keyTimers={timers} />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "checkout", "w1")], init: "u1" });
  test.flush();
  const keymap = (entries: Settings["keymap"]) => {
    test.client.indexEvents({ kind: "settings_changed", settings: { ui: {}, keymap: entries, error: null } });
    test.flush();
  };
  await act(async () => {
    await test.host.init.settled();
  });
  test.flush();
  const runTimers = () => act(() => timers.run());
  return { ...test, timers, runTimers, keymap };
}

/** Types `sequence` (key names, ` ` for Space) in normal mode. */
function keys(...sequence: string[]): void {
  for (const key of sequence) {
    act(() => {
      fireEvent.keyDown(document.activeElement ?? document.body, { key });
    });
  }
}

/** How many which-key panels show (0 or 1); never pass the element itself to `expect`. */
function open(): number {
  return screen.queryAllByTestId("which-key").length;
}

function panel(): HTMLElement | null {
  return screen.queryByTestId("which-key");
}

/** The panel's entries: `[group, key chip, kind, description]`. */
function entries(): string[][] {
  return screen.queryAllByTestId("which-key-group").flatMap((group) =>
    within(group)
      .queryAllByTestId("which-key-entry")
      .map((entry) => [
        group.getAttribute("data-group") ?? "",
        entry.querySelector("kbd")?.textContent ?? "",
        entry.getAttribute("data-kind") ?? "",
        entry.querySelector(".omfp-whichkey__description")?.textContent ?? "",
      ]),
  );
}

describe("which-key panel", () => {
  // spec: UI-320, UI-321, UI-323
  test("a prefix shows the keys completing it after the delay, in a labelled region, announced", async () => {
    const test = await setup();
    document.body.focus();
    keys("g");
    expect(open()).toBe(0);
    expect(test.timers.pending).toBe(1);
    test.runTimers();
    const shown = panel();
    expect(shown?.getAttribute("data-keys")).toBe("g");
    expect(shown?.tagName).toBe("SECTION");
    expect(shown?.getAttribute("aria-label")).toBe("Keys after g");
    expect(shown?.classList.contains("omfp-popover")).toBe(true);
    expect(shown?.closest('[data-testid="mode-bar"]') !== null).toBe(true);
    // No workspace is current and no notice waits: their commands do not answer now (UI-323).
    expect(entries()).toEqual([
      ["views", "v", "command", "Pick a view (opens :view with the views to complete) (not available now)"],
      ["files", "f", "command", "Open a file (:open <path>[:<line>]), or the focused path (g f) (not available now)"],
      ["navigation", "g", "command", "First thread"],
      ["navigation", "n", "command", "Open the newest notice (not available now)"],
      ["other", "h", "command", "Say hello"],
    ]);
    expect(test.host.announcer.getSnapshot().message).toBe("g: 5 keys, Escape cancels");
    // The panel never takes the focus: the next key completes the sequence and closes it.
    expect(document.activeElement === document.body).toBe(true);
    keys("h");
    expect(log).toEqual(["hello"]);
    expect(open()).toBe(0);
  });

  // spec: UI-321
  test("the next key typed before the delay never shows it", async () => {
    const test = await setup();
    document.body.focus();
    keys("g", "h");
    test.runTimers();
    expect(open()).toBe(0);
    expect(log).toEqual(["hello"]);
  });

  // spec: UI-320, UI-262
  test("it lists the live bindings: init files', `[keymap]` entries with a widget's command, removed defaults gone", async () => {
    const test = await setup();
    test.keymap([
      { mode: "normal", keys: "g p", command: "demo.ping" },
      { mode: "normal", keys: "g f", command: "" },
    ]);
    document.body.focus();
    keys("g");
    test.runTimers();
    expect(entries()).toEqual([
      ["views", "v", "command", "Pick a view (opens :view with the views to complete) (not available now)"],
      ["navigation", "g", "command", "First thread"],
      ["navigation", "n", "command", "Open the newest notice (not available now)"],
      ["other", "p", "command", "Ping the demo"],
      ["other", "h", "command", "Say hello"],
    ]);
    keys("h");
    expect(log).toEqual(["hello"]);
    expect(open()).toBe(0);
  });

  // spec: UI-322
  test("Escape closes it and cancels the prefix; so does a click", async () => {
    const test = await setup();
    document.body.focus();
    keys("g");
    test.runTimers();
    expect(open()).toBe(1);
    keys("Escape");
    expect(open()).toBe(0);
    // The next `g` starts a sequence again instead of completing `g g`.
    keys("g");
    test.runTimers();
    expect(panel()?.getAttribute("data-keys")).toBe("g");
    act(() => {
      fireEvent.pointerDown(document.body);
    });
    expect(open()).toBe(0);
  });
});

describe("Space leader menu", () => {
  // spec: UI-324, UI-326
  test("Space in normal mode opens the leader menu at once, grouped, with the leader's groups as prefixes", async () => {
    const test = await setup();
    document.body.focus();
    keys(" ");
    expect(test.timers.pending).toBe(0);
    expect(panel()?.getAttribute("data-keys")).toBe("Space");
    expect(panel()?.getAttribute("aria-label")).toBe("Keys after spc");
    expect(
      screen
        .queryAllByTestId("which-key-group")
        .map((group) => [group.getAttribute("data-group"), group.firstChild?.textContent]),
    ).toEqual([
      ["threads", "threads"],
      ["views", "views"],
      ["workspace", "workspace"],
      ["files", "files"],
      ["help", "help"],
      ["commands", "command line"],
      ["other", "other"],
    ]);
    const shown = entries();
    expect(shown.filter(([, , kind]) => kind === "prefix")).toEqual([
      ["views", "v", "prefix", "views…"],
      ["workspace", "p", "prefix", "workspace…"],
      ["files", "f", "prefix", "files…"],
      ["help", "h", "prefix", "help…"],
      ["other", "o", "prefix", "1 binding…"],
    ]);
    expect(shown.filter(([group]) => group === "commands")).toEqual([
      ["commands", ":", "command", "Open the command line"],
      ["commands", "spc", "command", "Open the command line"],
    ]);
    // Without a selected thread, its commands do not answer now.
    const archive = screen
      .getAllByTestId("which-key-entry")
      .find((entry) => entry.dataset.command === "thread.archive");
    expect(archive?.hasAttribute("data-disabled")).toBe(true);
  });

  // spec: UI-321, UI-326
  test("the menu follows the sequence as it grows and runs the binding picked", async () => {
    await setup();
    document.body.focus();
    keys(" ", "h");
    expect(panel()?.getAttribute("data-keys")).toBe("Space h");
    expect(entries()).toEqual([
      ["help", "h", "command", "Show every key and command (this help)"],
      ["help", "k", "command", "Describe a key: what it runs and where it is bound"],
      ["help", "e", "command", "Show the notices and toasts that went by (newest first)"],
    ]);
    keys("k");
    expect(open()).toBe(0);
    expect(screen.queryAllByTestId("describe").length).toBe(1);
  });

  // spec: UI-325
  test("Space Space and Space : open the command line, which `:` still opens", async () => {
    await setup();
    document.body.focus();
    keys(" ", " ");
    expect(open()).toBe(0);
    expect(document.activeElement === screen.getByTestId("command-line-input")).toBe(true);
    keys("Escape");
    expect(screen.queryByTestId("command-line")).toBeNull();
    document.body.focus();
    keys(" ", ":");
    expect(document.activeElement === screen.getByTestId("command-line-input")).toBe(true);
    keys("Escape");
    document.body.focus();
    keys(":");
    expect(document.activeElement === screen.getByTestId("command-line-input")).toBe(true);
  });
});
