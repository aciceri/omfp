/** L4: the view and the selected tabs remembered per thread (UI-367, UI-372), over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, screen, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { Frame } from "../../src/frame/frame.tsx";
import {
  chooseFile,
  DRAFT_MESSAGE,
  DRAFT_THREAD,
  type KeyValueStorage,
  type OmfpHost,
  openFile,
  REVEAL_WIDGET,
  SELECTED_THREAD,
  threadActions,
  type WidgetComponent,
} from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

const STUBS: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:chat", () => <p>chat</p>],
  ["omfp:workspace", () => <p>workspace</p>],
  ["omfp:changes", () => <p>changes</p>],
  ["omfp:background", () => <p>background</p>],
  ["omfp:todos", () => <p>todos</p>],
  ["omfp:activity", () => <p>activity</p>],
  ["omfp:file", () => <p>file</p>],
]);

const SIDE = ["tabs", {}, "omfp:workspace", "omfp:changes", "omfp:background"];

/** `work`: the `main` slot (path `0.0`) beside side tabs (path `0.1`); `lean`: the `main` slot alone. */
const MANIFEST = {
  default: "work",
  views: {
    work: { title: "Work", layout: ["row", {}, ["tabs", { id: "main" }, "omfp:chat"], SIDE] },
    lean: { title: "Lean", layout: ["tabs", { id: "main" }, "omfp:chat", "omfp:todos"] },
  },
};

class MemoryStorage implements KeyValueStorage {
  readonly items = new Map<string, string>();
  getItem(key: string): string | null {
    return this.items.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.items.set(key, value);
  }
}

function setup(storage = new MemoryStorage()) {
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: new ManualTimers(),
    storage,
  });
  base.client.answers.set("widgets.report", () => ({}));
  const test = renderWithHost(<Frame registry={STUBS} now={() => 0} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1")],
    threads: [thread("t1", "one", "w1"), thread("t2", "two", "w1"), thread("t3", "three", "w1")],
  });
  test.client.views("w1", { manifest: MANIFEST });
  test.flush();
  return { ...test, storage, user: userEvent.setup() };
}

function select(test: { host: OmfpHost; flush: () => void }, threadId: string | undefined): void {
  act(() => test.host.window.set(SELECTED_THREAD, threadId));
  test.flush();
}

function switchView(test: { host: OmfpHost; flush: () => void }, viewId: string): void {
  act(() => test.host.commands.run("view.switch", [viewId]));
  test.flush();
}

function shownView(): string | undefined {
  return screen.getByTestId("view").dataset.view;
}

/** The tabs of the shown view's container at `selector` (the views kept mounted, hidden, have theirs too). */
function tabsOf(selector: string): HTMLElement[] {
  const container = screen.getByTestId("view").querySelector<HTMLElement>(selector);
  return container === null ? [] : [...container.querySelectorAll<HTMLElement>('[role="tab"]')];
}

/** The label of the selected tab of the shown view's container at `selector`. */
function selectedOf(selector: string): string | undefined {
  return tabsOf(selector).find((tab) => tab.getAttribute("aria-selected") === "true")?.textContent ?? undefined;
}

const MAIN = '[data-slot="main"]';
const SIDE_TABS = '[data-path="0.1"]';

async function click(user: UserEvent, selector: string, label: string): Promise<void> {
  const tab = tabsOf(selector).find((element) => element.textContent === label);
  expect(tab?.textContent).toBe(label);
  await user.click(tab as HTMLElement);
}

describe("tabs per thread", () => {
  // spec: UI-372, UI-367
  test("switching A → B → A → B restores each thread's view and selected chat, file and widget tabs", async () => {
    const test = setup();
    const { host, user } = test;
    select(test, "t1");
    // Thread A: the work view, a file tab in `main`, the side's second tab.
    switchView(test, "work");
    act(() => openFile(host, "a.rs"));
    await click(user, SIDE_TABS, "Changes");
    expect([shownView(), selectedOf(MAIN), selectedOf(SIDE_TABS)]).toEqual(["work", "a.rs", "Changes"]);

    // Thread B, never shown: the default view with the first tabs; the file stays open there.
    select(test, "t2");
    expect([shownView(), selectedOf(MAIN), selectedOf(SIDE_TABS)]).toEqual(["work", "Chat", "Workspace"]);
    expect(tabsOf(MAIN).map((tab) => tab.textContent)).toEqual(["Chat", "a.rs"]);
    // A revealed widget's tab opens for the workspace and is selected for B alone; then B moves to the lean view.
    act(() => host.window.set(REVEAL_WIDGET, { reference: "omfp:activity", nonce: 1 }));
    test.flush();
    expect(selectedOf(MAIN)).toBe("Activity");
    switchView(test, "lean");
    expect(selectedOf(MAIN)).toBe("Chat");
    await click(user, MAIN, "Todos");

    // Back to A: its view and its tabs.
    select(test, "t1");
    expect([shownView(), selectedOf(MAIN), selectedOf(SIDE_TABS)]).toEqual(["work", "a.rs", "Changes"]);
    // Back to B: the lean view with its own tab; its work view kept the widget tab.
    select(test, "t2");
    expect([shownView(), selectedOf(MAIN)]).toEqual(["lean", "Todos"]);
    switchView(test, "work");
    expect([selectedOf(MAIN), selectedOf(SIDE_TABS)]).toEqual(["Activity", "Workspace"]);
    switchView(test, "lean");

    // A again, then B again: nothing leaked between them.
    select(test, "t1");
    expect([shownView(), selectedOf(MAIN), selectedOf(SIDE_TABS)]).toEqual(["work", "a.rs", "Changes"]);
    select(test, "t2");
    expect([shownView(), selectedOf(MAIN)]).toEqual(["lean", "Todos"]);

    // Kept in localStorage: a reload shows each thread with its tabs.
    const { storage } = test;
    cleanup();
    const reloaded = setup(storage);
    select(reloaded, "t1");
    expect([shownView(), selectedOf(MAIN), selectedOf(SIDE_TABS)]).toEqual(["work", "a.rs", "Changes"]);
    select(reloaded, "t2");
    expect([shownView(), selectedOf(MAIN)]).toEqual(["lean", "Todos"]);
  });

  // spec: UI-372
  test("a file opened right after a thread switch is selected in the view that thread shows", () => {
    const test = setup();
    const { host } = test;
    select(test, "t1");
    switchView(test, "work");
    select(test, "t2");
    switchView(test, "lean");
    // As the agent's `omfp_open_file` does: select the thread, then open the file, before the frame renders it.
    act(() => {
      host.window.set(SELECTED_THREAD, "t1");
      openFile(host, "b.rs");
    });
    test.flush();
    expect([shownView(), selectedOf(MAIN)]).toEqual(["work", "b.rs"]);
    select(test, "t2");
    expect([shownView(), selectedOf(MAIN)]).toEqual(["lean", "Chat"]);
  });

  // spec: UI-372
  test("a file opened while the shown thread's workspace views still load is selected in its view once they arrive", () => {
    const test = setup();
    const { client, host } = test;
    client.indexEvents(
      { kind: "workspace_upserted", workspace: workspace("w2") },
      { kind: "thread_upserted", thread: thread("t5", "five", "w2") },
    );
    select(test, "t5");
    expect(shownView()).toBe("default");
    act(() => openFile(host, "b.rs"));
    client.views("w2", { manifest: MANIFEST });
    test.flush();
    expect([shownView(), selectedOf(MAIN)]).toEqual(["work", "b.rs"]);
  });

  // spec: UI-372
  test("a file opened in a draft while its workspace's views load waits for that workspace's draft, not another's", async () => {
    const test = setup();
    const { client, host, user } = test;
    client.indexEvents({ kind: "workspace_upserted", workspace: workspace("w2") });
    test.flush();
    const draft = (workspaceId: string) => {
      act(() => threadActions(host).openDraft(workspaceId));
      test.flush();
    };
    // w1's draft has b.rs open and its chat selected.
    draft("w1");
    act(() => openFile(host, "b.rs"));
    await click(user, MAIN, "Chat");
    // w2's draft opens b.rs while w2's views load: it waits for w2's view.
    draft("w2");
    expect(shownView()).toBe("default");
    act(() => openFile(host, "b.rs"));
    draft("w1");
    expect(selectedOf(MAIN)).toBe("Chat");
    draft("w2");
    client.views("w2", { manifest: MANIFEST });
    test.flush();
    expect([shownView(), selectedOf(MAIN)]).toEqual(["work", "b.rs"]);
  });

  // spec: UI-372, UI-366
  test("closing a floating panel's selected file selects its neighbour in that panel", async () => {
    const test = setup();
    const { client, flush, host, user } = test;
    client.viewsEvents("w1", {
      kind: "views_manifest",
      manifest: { ...MANIFEST, panels: { files: ["tabs", { id: "main" }, "omfp:workspace"] } },
      error: null,
    });
    flush();
    select(test, "t1");
    act(() => {
      openFile(host, "a.rs");
      openFile(host, "b.rs");
    });
    const panelTabs = () => [
      ...document.querySelectorAll<HTMLElement>('[data-testid="floating-widget"] [data-slot="main"] [role="tab"]'),
    ];
    const panelSelected = () => panelTabs().find((tab) => tab.getAttribute("aria-selected") === "true")?.textContent;
    expect(panelSelected()).toBe("Workspace");
    await user.click(panelTabs().find((tab) => tab.textContent?.includes("b.rs")) as HTMLElement);
    expect(panelSelected()).toBe("b.rs");
    const b = within(panelTabs().find((tab) => tab.textContent?.includes("b.rs")) as HTMLElement);
    await user.click(b.getByTestId("file-tab-close"));
    expect(panelSelected()).toBe("a.rs");
  });

  // spec: UI-372
  test("after a switch to a workspace whose views load, the file is selected in its view once they arrive", () => {
    const test = setup();
    const { client, host } = test;
    client.indexEvents(
      { kind: "workspace_upserted", workspace: workspace("w2") },
      { kind: "thread_upserted", thread: thread("t5", "five", "w2") },
    );
    test.flush();
    select(test, "t1");
    act(() => {
      host.window.set(SELECTED_THREAD, "t5");
      openFile(host, "b.rs");
    });
    test.flush();
    // The built-in view stands in meanwhile: the file waits for the workspace's own view.
    expect(shownView()).toBe("default");
    client.views("w2", { manifest: MANIFEST });
    test.flush();
    expect([shownView(), selectedOf(MAIN)]).toEqual(["work", "b.rs"]);
  });

  // spec: UI-372, UI-117
  test("a missing path's file found after a thread switch is selected by the thread that opened it", () => {
    const test = setup();
    const { host } = test;
    select(test, "t1");
    act(() => openFile(host, "theme.ts"));
    select(test, "t2");
    expect(selectedOf(MAIN)).toBe("Chat");
    // The lookup answers while t2 shows: the found file takes the bare name's tab, and its selection.
    act(() => chooseFile(host, "w1", "theme.ts", "ui/theme.ts"));
    expect(selectedOf(MAIN)).toBe("Chat");
    select(test, "t1");
    const selected = tabsOf(MAIN).find((tab) => tab.getAttribute("aria-selected") === "true");
    expect(selected?.dataset.file).toBe("ui/theme.ts");
  });

  // spec: UI-372
  test("a remembered tab that is gone gives way to the container's first tab", async () => {
    const test = setup();
    const { client, flush, host, user } = test;
    select(test, "t1");
    act(() => openFile(host, "a.rs"));
    await click(user, SIDE_TABS, "Background");
    // Another thread closes the file A had selected: A falls back to the chat, and does not select the file again
    // when it opens later in another thread.
    select(test, "t2");
    act(() => openFile(host, "a.rs"));
    const close = within(tabsOf(MAIN).find((tab) => tab.textContent?.includes("a.rs")) as HTMLElement);
    await user.click(close.getByTestId("file-tab-close"));
    act(() => openFile(host, "a.rs"));
    expect(selectedOf(MAIN)).toBe("a.rs");
    select(test, "t1");
    expect(selectedOf(MAIN)).toBe("Chat");
    expect(selectedOf(SIDE_TABS)).toBe("Background");
    // The side tab removed from the view: the first tab; back in the view, it is selected again.
    const side = (...children: string[]) => {
      const views = {
        ...MANIFEST.views,
        work: { ...MANIFEST.views.work, layout: ["row", {}, MANIFEST.views.work.layout[2], ["tabs", {}, ...children]] },
      };
      client.viewsEvents("w1", { kind: "views_manifest", manifest: { ...MANIFEST, views }, error: null });
      flush();
    };
    side("omfp:workspace", "omfp:changes");
    expect(selectedOf(SIDE_TABS)).toBe("Workspace");
    side("omfp:workspace", "omfp:changes", "omfp:background");
    expect(selectedOf(SIDE_TABS)).toBe("Background");
  });

  // spec: UI-372
  test("the draft thread selects its own tabs, which the thread it becomes keeps", async () => {
    const test = setup();
    const { client, flush, host, user } = test;
    select(test, "t1");
    await click(user, SIDE_TABS, "Changes");
    act(() => {
      host.window.set(SELECTED_THREAD, undefined);
      host.window.set(DRAFT_THREAD, { workspaceId: "w1" });
      host.window.set(DRAFT_MESSAGE, { text: "start", images: [], pastes: [] });
    });
    flush();
    expect(selectedOf(SIDE_TABS)).toBe("Workspace");
    await click(user, SIDE_TABS, "Background");
    let sent = Promise.resolve(false);
    act(() => {
      sent = threadActions(host).sendDraft();
    });
    // The daemon lists the new thread, then answers its creation.
    client.indexEvents({ kind: "thread_upserted", thread: thread("t9", "nine", "w1") });
    flush();
    await act(async () => {
      client.callsTo("threads.create")[0]?.resolve(thread("t9", "nine", "w1"));
      expect(await sent).toBe(true);
    });
    expect(host.window.get(SELECTED_THREAD)).toBe("t9");
    expect(selectedOf(SIDE_TABS)).toBe("Background");
    // The next draft starts from the first tabs.
    act(() => threadActions(host).openDraft("w1"));
    flush();
    expect(selectedOf(SIDE_TABS)).toBe("Workspace");
    // Its thread answered before the index lists it: an index change meanwhile keeps the new thread's tabs.
    await click(user, SIDE_TABS, "Background");
    act(() => {
      host.window.set(DRAFT_MESSAGE, { text: "again", images: [], pastes: [] });
      sent = threadActions(host).sendDraft();
    });
    await act(async () => {
      client.callsTo("threads.create")[1]?.resolve(thread("t8", "eight", "w1"));
      expect(await sent).toBe(true);
    });
    client.indexEvents({ kind: "thread_upserted", thread: thread("t1", "renamed", "w1") });
    flush();
    client.indexEvents({ kind: "thread_upserted", thread: thread("t8", "eight", "w1") });
    client.views("w1", { manifest: MANIFEST });
    flush();
    expect([host.window.get(SELECTED_THREAD), selectedOf(SIDE_TABS)]).toEqual(["t8", "Background"]);
    // t1 kept its own.
    select(test, "t1");
    expect(selectedOf(SIDE_TABS)).toBe("Changes");
  });
});
