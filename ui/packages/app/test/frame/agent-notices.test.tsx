/** L4: the notifications an agent sends with `omfp_notify` (UI-215, DMN-125), over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, screen } from "@testing-library/react";
import { Frame, type FrameProps } from "../../src/frame/frame.tsx";
import type { NotificationPort } from "../../src/frame/notifications.ts";
import { NOTICES, type Notice, SELECTED_THREAD, type WidgetComponent } from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

const stub =
  (name: string): WidgetComponent =>
  () => <p data-testid={`stub-${name}`}>{name}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", stub("chat")],
  ["omfp:workspace", stub("workspace")],
]);

/** A frame on connection 1 (the fake `system.hello`) with `t1` selected and both threads followed. */
function setup(props: FrameProps) {
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: new ManualTimers(),
    noticeTimers: new ManualTimers(),
  });
  base.client.answers.set("widgets.report", () => ({}));
  const test = renderWithHost(<Frame registry={registry} now={() => 0} {...props} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1")],
    threads: [thread("t1", "checkout", "w1"), thread("t2", "flaky", "w1")],
  });
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  test.client.thread("t1", []);
  test.client.thread("t2", []);
  test.flush();
  return test;
}

/** The mode-line notice as `[tone, thread, text]`, or `undefined` without one. */
function modeNotice(): [string | undefined, string | undefined, string | null] | undefined {
  const notice = screen.queryByTestId("mode-notice");
  return notice === null ? undefined : [notice.dataset.tone, notice.dataset.thread, notice.textContent];
}

function grantedPort(shown: Array<{ notice: Notice; click: () => void }>): NotificationPort {
  return {
    permission: () => "granted",
    requestPermission: async () => "granted",
    show: (notice, click) => shown.push({ notice, click }),
  };
}

describe("notifications the agent sends", () => {
  // spec: UI-215, PRT-065
  test("with focus, the addressed window shows an agent notification once; clicking opens its thread", () => {
    const shown: Array<{ notice: Notice; click: () => void }> = [];
    const test = setup({ notifications: grantedPort(shown), hasFocus: () => true });

    test.client.threadEvents("t2", { kind: "notify", id: "n1", connection: 2, title: "Tests", body: "All green" });
    test.flush();
    expect(modeNotice()).toBeUndefined();

    const request = { kind: "notify", id: "n2", connection: 1, title: "Tests", body: "All green" } as const;
    test.client.threadEvents("t2", request);
    test.flush();
    expect(modeNotice()?.slice(0, 2)).toEqual(["info", "t2"]);
    expect(screen.getByTestId("mode-notice").textContent).toContain("All green");
    expect(shown.length).toBe(0);
    expect(screen.getByTestId("live-region").textContent).toBe("Tests All green");

    // The thread's snapshot still carries the request (followed again, a resync): it is not shown twice.
    test.client.thread("t2", [request]);
    test.flush();
    expect(test.host.window.get(NOTICES).length).toBe(1);

    act(() => {
      screen.getByTestId("mode-notice").click();
    });
    expect(test.host.window.get(SELECTED_THREAD)).toBe("t2");
    expect(modeNotice()).toBeUndefined();
  });

  // spec: UI-215
  test("on the thread the user is on, with focus, the agent's notification still shows in the mode line", () => {
    const shown: Array<{ notice: Notice; click: () => void }> = [];
    const test = setup({ notifications: grantedPort(shown), hasFocus: () => true });

    // A thread change on the selected thread stays silent (UI-192)…
    test.client.indexEvents({
      kind: "thread_upserted",
      thread: thread("t1", "checkout", "w1", { needsAttention: true }),
    });
    test.flush();
    expect(modeNotice()).toBeUndefined();

    // …but the agent asked explicitly.
    test.client.threadEvents("t1", { kind: "notify", id: "n1", connection: 1, title: "Deploy", body: "Needs your OK" });
    test.flush();
    expect(modeNotice()?.slice(0, 2)).toEqual(["info", "t1"]);
    expect(screen.getByTestId("mode-notice").textContent).toContain("Needs your OK");
    expect(shown.length).toBe(0);
  });

  // spec: UI-215, UI-096
  test("without focus it is a system notification whose click focuses the window on the thread", () => {
    const shown: Array<{ notice: Notice; click: () => void }> = [];
    let focusedWindow = 0;
    const test = setup({
      notifications: grantedPort(shown),
      hasFocus: () => false,
      focusWindow: () => {
        focusedWindow += 1;
      },
    });

    test.client.threadEvents("t2", { kind: "notify", id: "n1", connection: 1, title: "Tests", body: "All green" });
    test.flush();
    expect(shown.map(({ notice }) => [notice.title, notice.body, notice.threadId, notice.tone])).toEqual([
      ["Tests", "All green", "t2", "info"],
    ]);
    expect(modeNotice()).toBeUndefined();

    act(() => shown[0]?.click());
    expect(focusedWindow).toBe(1);
    expect(test.host.window.get(SELECTED_THREAD)).toBe("t2");
  });
});
