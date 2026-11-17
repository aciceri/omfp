/** L4: the toasts of the URLs omp opened in a browser (UI-346), over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Frame } from "../../src/frame/frame.tsx";
import { OPENED_URL_MS } from "../../src/frame/opened-urls.tsx";
import { ERRORS, SELECTED_THREAD, type WidgetComponent } from "../../src/sdk/index.ts";
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

const LOGIN = "https://login.example.com/device?code=1";

const openUrl = (id: string, url: string): ThreadEvent =>
  ({ kind: "omp", frame: { type: "extension_ui_request", method: "open_url", id, url } }) as ThreadEvent;

/** A frame following `t1` (selected) and `t2`; `t1` already has an opened URL when the window first sees it. */
function setup() {
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: new ManualTimers(),
    noticeTimers: new ManualTimers(),
  });
  base.client.answers.set("widgets.report", () => ({}));
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1")],
    threads: [thread("t1", "checkout", "w1"), thread("t2", "flaky", "w1")],
  });
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  test.client.thread("t1", [openUrl("old", "https://old.example/")]);
  test.client.thread("t2", []);
  test.flush();
  return { ...test, user: userEvent.setup() };
}

const toasts = () => screen.queryAllByTestId("opened-url-toast");

describe("opened URLs", () => {
  // spec: UI-346, UI-520
  test("a URL a followed thread opens later shows once as a toast with Open and Copy; earlier ones do not", async () => {
    const test = setup();
    expect(toasts().length).toBe(0);

    test.client.threadEvents("t2", openUrl("n1", LOGIN));
    test.flush();
    const [toast] = toasts();
    if (toast === undefined) {
      throw new Error("no toast");
    }
    expect([toast.textContent?.startsWith("omp opened login.example.com in the browser"), toast.dataset.url]).toEqual([
      true,
      LOGIN,
    ]);

    const opened: unknown[][] = [];
    const copied: string[] = [];
    const open = window.open;
    window.open = (...args: unknown[]) => {
      opened.push(args);
      return null;
    };
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: async (text: string) => void copied.push(text) },
    });
    try {
      await test.user.click(within(toast).getByTestId("opened-url-open"));
      expect(opened).toEqual([[LOGIN, "_blank", "noopener"]]);
      await test.user.click(within(toast).getByTestId("opened-url-copy"));
      expect(copied).toEqual([LOGIN]);
      // Copying is a user action: its outcome is a success toast (UI-520).
      await waitFor(() =>
        expect(screen.getAllByTestId("success").map((success) => success.textContent)).toEqual(["Copied the URL"]),
      );
    } finally {
      window.open = open;
      Reflect.deleteProperty(navigator, "clipboard");
    }

    // A resync carrying the same URL does not show it again; dismissing removes it.
    test.client.thread("t2", [openUrl("n1", LOGIN)]);
    test.flush();
    expect(toasts().length).toBe(1);
    await test.user.click(within(toast).getByTestId("toast-dismiss"));
    expect(toasts().length).toBe(0);
  });

  // spec: UI-346, UI-160
  test("the toasts share the error toasts' region and stay as long as omp's OAuth wait", () => {
    const test = setup();
    act(() => test.host.window.set(ERRORS, ["boom"]));
    test.client.threadEvents("t1", openUrl("n2", LOGIN));
    test.flush();
    const regions = screen.getAllByTestId("toasts");
    expect(regions.length).toBe(1);
    expect([...(regions[0]?.children ?? [])].map((child) => child.getAttribute("data-testid"))).toEqual([
      "error",
      "opened-url-toast",
    ]);
    expect(OPENED_URL_MS).toBeGreaterThanOrEqual(60_000);
  });

  // spec: UI-761
  test("a toast whose sign-in returns to omp's loopback listener relays the pasted address and goes once omp took it", async () => {
    const test = setup();
    test.client.threadEvents("t2", openUrl("n1", LOGIN));
    const reauth = `https://auth.example.com/authorize?redirect_uri=${encodeURIComponent("http://127.0.0.1:1455/cb")}`;
    test.client.threadEvents("t2", openUrl("n2", reauth));
    test.flush();
    const [device, oauth] = toasts();
    if (device === undefined || oauth === undefined) {
      throw new Error("no toasts");
    }
    // A device page returns nowhere: nothing to paste.
    expect(within(device).queryAllByTestId("oauth-callback").length).toBe(0);
    const pasted = "http://127.0.0.1:1455/cb?code=c0de&state=s1";
    await test.user.type(within(oauth).getByTestId("oauth-callback-input"), `${pasted}{Enter}`);
    const [relay] = test.client.callsTo("oauth.callback");
    expect(relay?.params).toEqual({ threadId: "t2", url: pasted });
    await act(async () => {
      relay?.resolve({ status: 200 });
      await Promise.resolve();
    });
    expect(toasts().map((toast) => toast.dataset.url)).toEqual([LOGIN]);
    await waitFor(() =>
      expect(screen.getAllByTestId("success").map((success) => success.textContent)).toEqual(["omp got the sign-in"]),
    );
  });
});
