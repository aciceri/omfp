/** L4: a window opened on a workspace's view shows that view first, never the views standing in before it (UI-223). */

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { act, cleanup, screen } from "@testing-library/react";
import { Frame } from "../../src/frame/frame.tsx";
import type { WidgetComponent } from "../../src/sdk/index.ts";
import { applyLaunch } from "../../src/sdk/launch.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, testHost } from "../support/host.tsx";
import { installResizeObserver } from "../support/resize.ts";

let restore = () => {};
beforeEach(() => {
  restore = installResizeObserver();
});
afterEach(() => {
  cleanup();
  restore();
});

const stub =
  (name: string): WidgetComponent =>
  () => <p data-testid={`stub-${name}`}>{name}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", stub("chat")],
  ["omfp:workspace", stub("workspace")],
]);

const MANIFEST = {
  default: "chat",
  views: {
    chat: { title: "Chat", layout: ["tabs", { id: "main" }, "omfp:chat"] },
    web: { title: "Web", layout: ["tabs", { id: "main" }, "omfp:header"] },
  },
};

const launching = () => screen.getByTestId("app").dataset.launching !== undefined;
const shownView = () => screen.queryAllByTestId("view").map((view) => view.dataset.view);

describe("a window opened on a workspace's view", () => {
  // spec: UI-223
  test("hides its shell until it shows the view named, then shows it at once", () => {
    const base = testHost(undefined, {
      importer: () => new Promise<unknown>(() => {}),
      reportTimers: new ManualTimers(),
      noticeTimers: new ManualTimers(),
    });
    base.client.answers.set("widgets.report", () => ({}));
    applyLaunch(base.host, { workspace: "w1", view: "web" });
    const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
    test.flush();
    expect(launching()).toBe(true);
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")] });
    test.flush();
    // The draft thread is open, but its workspace's views have not arrived: the built-in view stands in, hidden.
    expect(launching()).toBe(true);
    test.client.views("w1", { manifest: MANIFEST });
    test.flush();
    expect([launching(), ...shownView()]).toEqual([false, "web"]);
    // Once only: switching views later never hides the shell.
    act(() => {
      test.host.commands.run("view.switch", ["chat"]);
    });
    expect([launching(), ...shownView()]).toEqual([false, "chat"]);
  });
});
