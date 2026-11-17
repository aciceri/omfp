/** L4: the phone's commands sheet (UI-460..UI-464) in the frame over a fake client. */

import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import type { ThreadSummary } from "@omfp/protocol";
import { act, cleanup, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Frame } from "../../src/frame/frame.tsx";
import { SCOPE_ATTRIBUTE, useCommand, type WidgetComponent } from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";
import { installResizeObserver, resize } from "../support/resize.ts";

let restoreResize = () => {};
beforeEach(() => {
  restoreResize = installResizeObserver();
});
afterEach(() => {
  cleanup();
  restoreResize();
  document.head.innerHTML = "";
});

const stub =
  (name: string): WidgetComponent =>
  () => <p data-testid={`stub-${name}`}>{name}</p>;

/** What the widget's commands ran, in order. */
let ran: string[] = [];

/** A widget with a global command and one of its own scope, around a button that can hold the focus. */
const Demo: WidgetComponent = () => {
  useCommand({
    id: "demo.ping",
    title: "Ping the demo",
    description: "Pings it",
    run: () => {
      ran.push("ping");
    },
  });
  useCommand({
    id: "demo.scoped",
    title: "Scoped demo",
    scope: "demo",
    run: () => {
      ran.push("scoped");
    },
  });
  return (
    <div {...{ [SCOPE_ATTRIBUTE]: "demo" }}>
      <button type="button" data-testid="in-scope">
        in scope
      </button>
    </div>
  );
};

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", Demo],
  ["omfp:workspace", stub("workspace")],
  ["omfp:transcript", stub("transcript")],
]);

const threads: ThreadSummary[] = [thread("t1", "checkout", "w1")];

/** The frame on a phone (390 px). */
function setup() {
  ran = [];
  const base = testHost(undefined, { importer: () => new Promise<unknown>(() => {}) });
  for (const method of ["widgets.report", "settings.set"] as const satisfies readonly CallMethod[]) {
    base.client.answers.set(method, () => ({}));
  }
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads });
  test.flush();
  const app = screen.getByTestId("app");
  resize(app, 390);
  return { ...test, app, user: userEvent.setup() };
}

/** The sheet's rows, by command id. */
function rows(): (string | undefined)[] {
  return screen.queryAllByTestId("commands-row").map((row) => row.dataset.command);
}

function focused(): string | null {
  return document.activeElement?.getAttribute("data-testid") ?? null;
}

describe("commands sheet", () => {
  // spec: UI-460, UI-461
  test("the phone's Commands button opens the full sheet; Escape or its close button closes it, the focus back where it was", async () => {
    const { app, user } = setup();
    const button = screen.getByTestId("area-action");
    expect([button.textContent, button.dataset.action]).toEqual(["Commands", "commands"]);
    expect(screen.getAllByTestId("area-button").length).toBe(3);
    screen.getByTestId("in-scope").focus();
    await user.click(button);
    const sheet = screen.getByRole("dialog", { name: "Commands" });
    expect(sheet.querySelectorAll("[data-testid=commands-search]").length).toBe(1);
    await user.keyboard("{Escape}");
    expect(screen.queryAllByTestId("commands-sheet").length).toBe(0);
    expect(focused()).toBe("in-scope");
    await user.click(screen.getByTestId("area-action"));
    await user.click(screen.getByTestId("sheet-close"));
    expect(screen.queryAllByTestId("commands-sheet").length).toBe(0);
    // Desktop unchanged: no area bar, no button.
    resize(app, 1400);
    expect(screen.queryAllByTestId("area-action").length).toBe(0);
  });

  // spec: UI-461
  test("the browser's back (a phone's back gesture) closes it; closing it otherwise drops its history entry", async () => {
    const { user } = setup();
    const back = spyOn(window.history, "back").mockImplementation(() => {});
    try {
      await user.click(screen.getByTestId("area-action"));
      expect(screen.getAllByTestId("commands-sheet").length).toBe(1);
      act(() => {
        window.dispatchEvent(new PopStateEvent("popstate"));
      });
      expect(screen.queryAllByTestId("commands-sheet").length).toBe(0);
      expect(back.mock.calls.length).toBe(0);
      await user.click(screen.getByTestId("area-action"));
      await user.keyboard("{Escape}");
      expect(back.mock.calls.length).toBe(1);
    } finally {
      back.mockRestore();
    }
  });

  // spec: UI-462, UI-463
  test("grouped rows of what the `:` line runs; the search narrows them", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("area-action"));
    const groups = screen.getAllByTestId("commands-group");
    expect(groups[0]?.querySelector("h3")?.textContent).toBe("Threads");
    expect(groups.map((group) => group.dataset.group).includes("other")).toBe(true);
    expect([rows().includes("window.theme"), rows().includes("threads.toggle")]).toEqual([true, false]);
    const ping = screen.getAllByTestId("commands-row").find((row) => row.dataset.command === "demo.ping");
    expect([
      ping?.querySelector(".omfp-sheet__label")?.textContent,
      ping?.querySelector(".omfp-sheet__name")?.textContent,
    ]).toEqual(["Ping the demo", ":demo.ping"]);
    // The description is searched, not shown.
    expect(ping?.textContent?.includes("Pings it")).toBe(false);
    await user.type(screen.getByTestId("commands-search"), "PING");
    expect(rows()).toEqual(["demo.ping"]);
    await user.type(screen.getByTestId("commands-search"), " nothing");
    expect([rows().length, screen.getAllByTestId("commands-empty").length]).toEqual([0, 1]);
  });

  // spec: UI-462, UI-464
  test("a command runs in the scopes the focus was in, once the sheet is closed; other scopes' commands are not listed", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("area-action"));
    expect(rows().includes("demo.scoped")).toBe(false);
    await user.keyboard("{Escape}");
    screen.getByTestId("in-scope").focus();
    await user.click(screen.getByTestId("area-action"));
    expect(rows().includes("demo.scoped")).toBe(true);
    const scoped = screen.getAllByTestId("commands-row").find((row) => row.dataset.command === "demo.scoped");
    await user.click(scoped ?? document.body);
    expect(screen.queryAllByTestId("commands-sheet").length).toBe(0);
    expect(ran).toEqual(["scoped"]);
    expect(focused()).toBe("in-scope");
  });

  // spec: UI-464
  test("a command taking arguments opens the `:` line on its name, with their completions", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("area-action"));
    const theme = screen.getAllByTestId("commands-row").find((row) => row.dataset.command === "window.theme");
    expect(theme?.dataset.more).toBe("true");
    await user.click(theme ?? document.body);
    expect(screen.queryAllByTestId("commands-sheet").length).toBe(0);
    const input = screen.getByTestId("command-line-input");
    expect([(input as HTMLInputElement).value, focused()]).toEqual(["theme ", "command-line-input"]);
    expect(screen.getAllByTestId("command-option").map((option) => option.dataset.value)).toEqual([
      "theme system",
      "theme light",
      "theme dark",
      "theme synth",
      "theme catppuccin",
      "theme gruvbox",
      "theme tokyo-night",
      "theme nord",
      "theme solarized",
      "theme rose-pine",
      "theme graphite",
      "theme paper",
      "theme clay",
      "theme primer",
      "theme accent",
      "theme matugen",
    ]);
  });
});
