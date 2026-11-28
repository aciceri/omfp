/** L4: closing the tabs of a view's layout, which `views.json` holds (UI-375..UI-377), over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import type { ViewsState } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { Frame } from "../../src/frame/frame.tsx";
import { ERRORS, openFile, SELECTED_THREAD, SUCCESSES, type WidgetComponent } from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { type FakeClient, ManualTimers, renderWithHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

const stub =
  (name: string): WidgetComponent =>
  () => <p>{name}</p>;

/** A widget counting its clicks in React state, which a remount loses. */
function Counter() {
  const [clicks, setClicks] = useState(0);
  return (
    <button type="button" data-testid="counter" onClick={() => setClicks((n) => n + 1)}>
      {clicks}
    </button>
  );
}

const STUBS: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", stub("chat")],
  ["omfp:workspace", stub("workspace")],
  ["omfp:changes", stub("changes")],
  ["omfp:activity", stub("activity")],
  ["omfp:todos", Counter],
  ["omfp:file", stub("file")],
]);

/** A view whose `main` slot holds the chat and a mounted widget, and whose side tabs hold three widgets. */
const MANIFEST = {
  default: "work",
  views: {
    work: {
      title: "Work",
      layout: [
        "row",
        { sizes: [3, 1] },
        ["col", {}, "omfp:header", ["tabs", { id: "main" }, "omfp:chat", ["omfp:activity", { instance: "act" }]]],
        ["tabs", { id: "side" }, "omfp:workspace", "omfp:changes", "omfp:todos"],
      ],
    },
  },
};

/** {@link MANIFEST} without its side tab `index`. */
function withoutSide(index: number) {
  const side = ["omfp:workspace", "omfp:changes", "omfp:todos"].toSpliced(index, 1);
  const work = MANIFEST.views.work;
  const [row, sizes, main] = work.layout;
  return { ...MANIFEST, views: { work: { ...work, layout: [row, sizes, main, ["tabs", { id: "side" }, ...side]] } } };
}

function setup(views: Partial<ViewsState> = { manifest: MANIFEST }, { answer = true } = {}) {
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: new ManualTimers(),
  });
  base.client.answers.set("widgets.report", () => ({}));
  if (answer) {
    base.client.answers.set("views.set", () => ({}));
  }
  const test = renderWithHost(<Frame registry={STUBS} now={() => 0} />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "one", "w1")] });
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  test.client.views("w1", views);
  test.flush();
  return { ...test, user: userEvent.setup() };
}

/** What the daemon pushes after `views.set` wrote `manifest`. */
function pushed(test: { client: FakeClient; flush(): void }, manifest: unknown): void {
  act(() => test.client.viewsEvents("w1", { kind: "views_manifest", manifest, error: null }));
  test.flush();
}

function tabElements(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('[data-testid="view"] [role="tab"]')];
}

function tabs(): string[] {
  return tabElements().map((tab) => tab.textContent ?? "");
}

function tab(title: string): HTMLElement {
  return tabElements().find((element) => element.textContent === title) as HTMLElement;
}

function closable(): string[] {
  return tabElements()
    .filter((element) => within(element).queryAllByTestId("layout-tab-close").length > 0)
    .map((element) => element.textContent ?? "");
}

/** The `views.set` calls so far. */
function written(client: FakeClient): unknown[] {
  return client.callsTo("views.set").map((call) => call.params);
}

/** The params of `views.set` writing `manifest` into the test's workspace while it holds `expected` (DMN-364). */
function set(manifest: unknown, expected: unknown) {
  return { workspaceId: "w1", manifest, expected };
}

describe("closing layout tabs", () => {
  // spec: UI-375, UI-377
  test("a layout tab has a × unless it is its container's last or holds the conversation; opened tabs are italic", () => {
    const test = setup();
    act(() => openFile(test.host, "fe/src/lib.rs"));
    expect(tabs()).toEqual(["Chat", "Activity", "lib.rs", "Workspace", "Changes", "Todos"]);
    expect(closable()).toEqual(["Activity", "Workspace", "Changes", "Todos"]);
    // The opened file tab keeps its own ×, and only it shows as opened.
    expect(within(tab("lib.rs")).getAllByTestId("file-tab-close").length).toBe(1);
    expect(
      [...document.querySelectorAll('[data-testid="view"] [data-opened]')].map((element) => element.textContent),
    ).toEqual(["lib.rs"]);
  });

  // spec: UI-375
  test("the default view's chat stays without ×, and so does a view while views.json is invalid", () => {
    const test = setup({ manifest: null, error: "views: no view" });
    // The default view's slots hold one tab each (the chat, the canvas): no tab bar until a file opens beside the chat.
    expect(tabs()).toEqual([]);
    act(() => openFile(test.host, "fe/src/lib.rs"));
    expect(tabs()).toEqual(["Chat", "lib.rs"]);
    expect(closable()).toEqual([]);
  });

  // spec: UI-375, UI-376
  test("the × writes views.json without the tab; the toast's Undo writes it back", async () => {
    const test = setup();
    await test.user.click(within(tab("Changes")).getByTestId("layout-tab-close"));
    await waitFor(() => expect(written(test.client)).toEqual([set(withoutSide(1), MANIFEST)]));
    pushed(test, withoutSide(1));
    expect(tabs()).toEqual(["Chat", "Activity", "Workspace", "Todos"]);
    await waitFor(() =>
      expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual(["Closed Changes"]),
    );

    await test.user.click(screen.getByTestId("success-undo"));
    await waitFor(() =>
      expect(written(test.client)).toEqual([set(withoutSide(1), MANIFEST), set(MANIFEST, withoutSide(1))]),
    );
    expect(test.host.window.get(SUCCESSES)).toEqual([]);
    pushed(test, MANIFEST);
    expect(tabs()).toEqual(["Chat", "Activity", "Workspace", "Changes", "Todos"]);
  });

  // spec: UI-376
  test("Undo writes nothing once views.json changed since the close", async () => {
    const test = setup();
    await test.user.click(within(tab("Activity")).getByTestId("layout-tab-close"));
    await waitFor(() => expect(written(test.client).length).toBe(1));
    const [row, sizes, , side] = MANIFEST.views.work.layout;
    const closed = {
      ...MANIFEST,
      views: {
        work: {
          ...MANIFEST.views.work,
          layout: [row, sizes, ["col", {}, "omfp:header", ["tabs", { id: "main" }, "omfp:chat"]], side],
        },
      },
    };
    expect(written(test.client)).toEqual([set(closed, MANIFEST)]);
    pushed(test, closed);
    // Another window (or an agent) changes the file before the Undo.
    pushed(test, withoutSide(0));
    await waitFor(() => expect(screen.queryAllByTestId("success-undo").length).toBe(1));
    await test.user.click(screen.getByTestId("success-undo"));
    await waitFor(() =>
      expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
        "Nothing restored: .omfp/views.json changed since Activity was closed",
      ]),
    );
    expect(written(test.client).length).toBe(1);
  });

  // spec: UI-375
  test("a middle click closes a layout tab before the selected one, which stays selected", async () => {
    const test = setup();
    await test.user.click(tab("Todos"));
    expect(tab("Todos").getAttribute("aria-selected")).toBe("true");
    expect(fireEvent.mouseDown(tab("Workspace"), { button: 1 })).toBe(false);
    fireEvent(tab("Workspace"), new MouseEvent("auxclick", { button: 1, bubbles: true, cancelable: true }));
    await waitFor(() => expect(written(test.client)).toEqual([set(withoutSide(0), MANIFEST)]));
    pushed(test, withoutSide(0));
    expect(tabs()).toEqual(["Chat", "Activity", "Changes", "Todos"]);
    // The tab keeps its id, so the selection stays with it.
    expect(tab("Todos").getAttribute("aria-selected")).toBe("true");
  });

  // spec: UI-375
  test("Delete closes the focused layout tab; when it was selected, its neighbour is", async () => {
    const test = setup();
    await test.user.click(tab("Changes"));
    act(() => tab("Changes").focus());
    await test.user.keyboard("{Delete}");
    await waitFor(() => expect(written(test.client)).toEqual([set(withoutSide(1), MANIFEST)]));
    pushed(test, withoutSide(1));
    expect(tabs()).toEqual(["Chat", "Activity", "Workspace", "Todos"]);
    await waitFor(() => expect(tab("Todos").getAttribute("aria-selected")).toBe("true"));
  });

  // spec: UI-375
  test("the tabs after a closed one keep their state: they are not mounted again", async () => {
    const test = setup();
    await test.user.click(tab("Todos"));
    await test.user.click(screen.getByTestId("counter"));
    await test.user.click(screen.getByTestId("counter"));
    expect(screen.getByTestId("counter").textContent).toBe("2");
    await test.user.click(within(tab("Workspace")).getByTestId("layout-tab-close"));
    await waitFor(() => expect(written(test.client).length).toBe(1));
    pushed(test, withoutSide(0));
    expect(tabs()).toEqual(["Chat", "Activity", "Changes", "Todos"]);
    expect(screen.getByTestId("counter").textContent).toBe("2");
  });

  // spec: UI-375
  test("a second close waits for the first write, so it never brings the first tab back", async () => {
    const test = setup({ manifest: MANIFEST }, { answer: false });
    await test.user.click(within(tab("Workspace")).getByTestId("layout-tab-close"));
    await test.user.click(within(tab("Todos")).getByTestId("layout-tab-close"));
    await waitFor(() => expect(written(test.client)).toEqual([set(withoutSide(0), MANIFEST)]));

    // The daemon publishes the file, then answers; only then does the second close write, without both tabs.
    act(() => test.client.viewsEvents("w1", { kind: "views_manifest", manifest: withoutSide(0), error: null }));
    await act(async () => test.client.callsTo("views.set")[0]?.resolve({}));
    test.flush();
    const [row, sizes, main] = MANIFEST.views.work.layout;
    const both = {
      ...MANIFEST,
      views: { work: { ...MANIFEST.views.work, layout: [row, sizes, main, ["tabs", { id: "side" }, "omfp:changes"]] } },
    };
    await waitFor(() =>
      expect(written(test.client)).toEqual([set(withoutSide(0), MANIFEST), set(both, withoutSide(0))]),
    );
  });

  // spec: UI-375, UI-376
  test("a close right after an Undo waits for the restored file and starts from it", async () => {
    const test = setup();
    await test.user.click(within(tab("Workspace")).getByTestId("layout-tab-close"));
    await waitFor(() => expect(written(test.client).length).toBe(1));
    pushed(test, withoutSide(0));
    await waitFor(() => expect(screen.queryAllByTestId("success-undo").length).toBe(1));
    await test.user.click(screen.getByTestId("success-undo"));
    await waitFor(() => expect(written(test.client).length).toBe(2));
    // Closed before the restored file reaches the window: it waits for it, then keeps the tab the Undo brought back.
    await test.user.click(within(tab("Changes")).getByTestId("layout-tab-close"));
    expect(written(test.client).length).toBe(2);
    pushed(test, MANIFEST);
    await waitFor(() =>
      expect(written(test.client)).toEqual([
        set(withoutSide(0), MANIFEST),
        set(MANIFEST, withoutSide(0)),
        set(withoutSide(1), MANIFEST),
      ]),
    );
  });

  // spec: UI-375
  test("a tab another window closed leaves the selection on the same tab", async () => {
    const test = setup();
    await test.user.click(tab("Changes"));
    pushed(test, withoutSide(0));
    expect(tabs()).toEqual(["Chat", "Activity", "Changes", "Todos"]);
    expect(tab("Changes").getAttribute("aria-selected")).toBe("true");
  });

  // spec: UI-375, UI-376, DMN-364
  test("a write the daemon refuses because views.json changed meanwhile says so and writes nothing else", async () => {
    const test = setup({ manifest: MANIFEST }, { answer: false });
    await test.user.click(within(tab("Changes")).getByTestId("layout-tab-close"));
    await waitFor(() => expect(written(test.client)).toEqual([set(withoutSide(1), MANIFEST)]));
    const refused = new CallError({ kind: "daemon", error: { code: "conflict", message: "changed" } });
    await act(async () => test.client.callsTo("views.set")[0]?.reject(refused));
    await waitFor(() =>
      expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
        "Changes not closed: .omfp/views.json changed meanwhile, close it again",
      ]),
    );
    expect(test.host.window.get(SUCCESSES)).toEqual([]);
  });

  // spec: UI-375
  test("a tab selected while the close of the selected one is pending stays selected", async () => {
    const test = setup({ manifest: MANIFEST }, { answer: false });
    await test.user.click(tab("Workspace"));
    await test.user.click(within(tab("Workspace")).getByTestId("layout-tab-close"));
    await waitFor(() => expect(written(test.client).length).toBe(1));
    await test.user.click(tab("Todos"));
    act(() => test.client.viewsEvents("w1", { kind: "views_manifest", manifest: withoutSide(0), error: null }));
    await act(async () => test.client.callsTo("views.set")[0]?.resolve({}));
    test.flush();
    expect(tabs()).toEqual(["Chat", "Activity", "Changes", "Todos"]);
    expect(tab("Todos").getAttribute("aria-selected")).toBe("true");
  });
});
