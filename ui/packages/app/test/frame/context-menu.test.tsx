/**
 * L4: omfp's context menu (UI-314..UI-319) on messages, files and threads over a fake client: the right click, the
 * built-in items, the keyboard, a resting finger and the items a widget adds.
 */

import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import type { IndexSnapshot } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { CommandLine } from "../../src/frame/command-line.tsx";
import { GlobalCommands } from "../../src/frame/commands.tsx";
import { ContextMenuLayer, MENU_LONG_PRESS_MS } from "../../src/frame/context-menu.tsx";
import { runRequested } from "../../src/frame/window-requests.tsx";
import { DEFAULT_BINDINGS } from "../../src/keymap/keymap.ts";
import { ERRORS, OPEN_FILES, SELECTED_THREAD, SUCCESSES, useCommand, useContextMenu } from "../../src/sdk/index.ts";
import { TranscriptWidget } from "../../src/widgets/chat/index.ts";
import { ThreadsWidget } from "../../src/widgets/threads/index.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";
import { summary } from "../widgets/chat/fixtures.ts";
import { renderThread, THREAD } from "../widgets/chat/harness.tsx";
import { fakeLayout } from "../widgets/chat/layout.ts";

let restoreLayout: () => void = () => {};
beforeEach(() => {
  restoreLayout = fakeLayout();
});
afterEach(() => {
  cleanup();
  restoreLayout();
});

const HISTORY: ThreadEvent[] = [
  {
    kind: "history",
    session: { session_id: "s", resumed: true },
    messages: [
      { role: "user", content: "first question", timestamp: 1 },
      { role: "assistant", content: [{ type: "text", text: "first answer" }], timestamp: 2 },
      { role: "user", content: "second question", timestamp: 3 },
    ],
  } as ThreadEvent,
];

/** omp's tree for {@link HISTORY}: one branch. */
const TREE = {
  leafId: "u2",
  tree: [
    {
      entry: { id: "u1", parentId: null, type: "message", message: { role: "user", timestamp: 1 } },
      children: [
        {
          entry: { id: "a1", parentId: "u1", type: "message", message: { role: "assistant", timestamp: 2 } },
          children: [
            {
              entry: { id: "u2", parentId: "a1", type: "message", message: { role: "user", timestamp: 3 } },
              children: [],
            },
          ],
        },
      ],
    },
  ],
};

const GLOBAL = (
  <GlobalCommands viewIds={[]} currentView={undefined} widgets={[]} addWidget={() => Promise.resolve(false)} />
);

/** The transcript of {@link HISTORY} with the context menu, and `extra` beside it. */
function transcript(extra: ReactNode = null) {
  const rendered = renderThread(
    <>
      <ContextMenuLayer />
      <TranscriptWidget />
      {extra}
    </>,
    HISTORY,
  );
  rendered.client.answers.set("thread.tree", () => TREE);
  rendered.client.answers.set("threads.fork", () => ({ ...summary("01F"), title: "01T (fork)" }));
  return { ...rendered, user: userEvent.setup() };
}

/** Markup carrying targets, with the frame's commands and the context menu; `t1` of `w1` selected. */
function page(markup: ReactNode) {
  const base = testHost();
  base.host.window.set(SELECTED_THREAD, "t1");
  const rendered = renderWithHost(
    <>
      {GLOBAL}
      <ContextMenuLayer />
      {markup}
    </>,
    base,
  );
  rendered.client.connected();
  rendered.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "checkout", "w1")] });
  rendered.flush();
  return { ...rendered, user: userEvent.setup() };
}

const INDEX: Partial<IndexSnapshot> = {
  workspaces: [{ ...workspace("w1"), name: "checkout" }],
  threads: [thread("t1", "checkout-v2", "w1", { updatedAt: 5 }), thread("t2", "payments", "w1", { updatedAt: 4 })],
};

/** The left column with the frame's commands, the `:` line and the context menu; `t2` selected. */
function column() {
  const rendered = renderWithHost(
    <>
      <ThreadsWidget />
      {GLOBAL}
      <CommandLine bindings={DEFAULT_BINDINGS} />
      <ContextMenuLayer />
    </>,
  );
  for (const method of ["threads.archive", "threads.unarchive", "threads.delete"] as const) {
    rendered.client.answers.set(method, () => ({}));
  }
  rendered.client.answers.set("threads.search", () => ({ results: [] }));
  rendered.client.connected();
  rendered.client.index(INDEX);
  act(() => rendered.host.window.set(SELECTED_THREAD, "t2"));
  rendered.flush();
  return { ...rendered, user: userEvent.setup() };
}

function row(id: string): HTMLElement {
  const found = screen.getAllByTestId("thread-row").find((each) => each.dataset.threadId === id);
  if (found === undefined) {
    throw new Error(`no row ${id}`);
  }
  return found;
}

const messages = () => screen.getAllByTestId("message");
const items = () => within(screen.getByTestId("context-menu")).getAllByTestId("context-menu-item");
const titles = () => items().map((item) => item.textContent);

function item(title: string): HTMLElement {
  const found = items().find((each) => each.textContent === title);
  if (found === undefined) {
    throw new Error(`no item ${title}`);
  }
  return found;
}

/** A widget adding a message item `Pin` (and items the menu leaves out); `ran` gets each run's arguments. */
function Pins({ ran }: { ran: string[][] }) {
  useCommand({ id: "my.pin", title: "Pin the message", run: (args) => void ran.push([...args]) });
  useCommand({ id: "my.never", title: "Never", enabled: () => false, run: () => undefined });
  useContextMenu({ on: "message", items: [{ command: "my.pin", title: "Pin" }, "fork", "my.never", "my.missing"] });
  return null;
}

function touch(y: number) {
  return { pointerId: 1, isPrimary: true, button: 0, pointerType: "touch", clientX: 10, clientY: y };
}

describe("the context menu", () => {
  // spec: UI-314, UI-315, UI-316
  test("a right click on a message shows omfp's menu at the pointer; Fork from here forks from that message", async () => {
    const test = transcript();
    // The browser's menu does not open: the event is taken.
    expect(fireEvent.contextMenu(messages()[2] as HTMLElement, { clientX: 40, clientY: 50 })).toBe(false);
    expect(screen.getByTestId("context-menu").dataset.target).toBe("message");
    expect(screen.getByTestId("context-menu").getAttribute("aria-label")).toBe("Message menu");
    expect(titles()).toEqual(["Fork from here", "Copy"]);

    await test.user.click(item("Fork from here"));

    await waitFor(() =>
      expect(test.client.callsTo("threads.fork").map((call) => call.params)).toEqual([
        { threadId: THREAD, entryId: "u2" },
      ]),
    );
    expect(screen.queryByTestId("context-menu")).toBeNull();
  });

  // spec: UI-315, UI-316
  test("Copy copies the message clicked, not the conversation", async () => {
    const test = transcript();
    fireEvent.contextMenu(messages()[1] as HTMLElement, { clientX: 40, clientY: 50 });
    expect(titles()).toEqual(["Fork from here", "Copy"]);
    await test.user.click(item("Copy"));
    await waitFor(() =>
      expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual(["Copied the message"]),
    );
    expect(await navigator.clipboard.readText()).toBe("first answer\n");
    // A thread id is the global `:copy`'s (not rendered here), never this transcript's.
    expect(test.host.commands.resolve("copy", ["transcript"], ["1"])?.scope).toBe("transcript");
    expect(test.host.commands.resolve("copy", ["transcript"], ["01OTHER"])).toBeUndefined();
  });

  // spec: UI-314
  test("Shift, a text field, a widget's own menu and a place outside every target keep the browser's menu", () => {
    page(
      <div data-omfp-thread="t1">
        <span data-testid="plain">checkout</span>
        <input aria-label="field" data-testid="field" />
        <button type="button" data-testid="own" onContextMenu={(event) => event.preventDefault()}>
          own
        </button>
      </div>,
    );
    const outside = document.createElement("p");
    document.body.append(outside);
    try {
      expect(fireEvent.contextMenu(screen.getByTestId("plain"), { shiftKey: true })).toBe(true);
      expect(fireEvent.contextMenu(screen.getByTestId("field"))).toBe(true);
      expect(fireEvent.contextMenu(outside)).toBe(true);
      // The widget took it: neither menu opens.
      expect(fireEvent.contextMenu(screen.getByTestId("own"))).toBe(false);
      expect(screen.queryByTestId("context-menu")).toBeNull();
    } finally {
      outside.remove();
    }
  });

  // spec: UI-314, UI-316
  test("a file's menu wins inside a message: Open and Copy path, the path without its line", async () => {
    const test = page(
      <div data-omfp-message="0">
        <button type="button" data-omfp-path="src/lib.rs" data-omfp-line="4" data-testid="path">
          lib.rs
        </button>
      </div>,
    );
    expect(fireEvent.contextMenu(screen.getByTestId("path"), { clientX: 5, clientY: 5 })).toBe(false);
    expect(screen.getByTestId("context-menu").dataset.target).toBe("file");
    expect(titles()).toEqual(["Open", "Copy path"]);
    await test.user.click(item("Copy path"));
    await waitFor(() =>
      expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual(["Copied the path src/lib.rs"]),
    );
    expect(await navigator.clipboard.readText()).toBe("src/lib.rs");
  });

  // spec: UI-316
  test("an agent running `:copy-path` hears of a copy the clipboard refused", async () => {
    const test = page(null);
    const writeText = spyOn(navigator.clipboard, "writeText").mockImplementation(() =>
      Promise.reject(new Error("denied")),
    );
    try {
      expect(await runRequested(test.host, "file.copyPath", ["src/lib.rs:4"])).toEqual({
        kind: "not_run",
        reason: "failed",
        message: "denied",
      });
    } finally {
      writeText.mockRestore();
    }
  });

  // spec: UI-116, UI-316
  test("Open on a diff's reference opens its diff, as clicking it does", async () => {
    const test = page(
      <span data-omfp-path="fe/src/lib.rs" data-omfp-diff="" data-testid="path">
        lib.rs
      </span>,
    );
    fireEvent.contextMenu(screen.getByTestId("path"), { clientX: 5, clientY: 5 });
    await test.user.click(item("Open"));
    expect(test.host.window.get(OPEN_FILES).w1?.tabs.map(({ path, diff }) => ({ path, diff }))).toEqual([
      { path: "fe/src/lib.rs", diff: true },
    ]);
  });

  // spec: UI-315, UI-316
  test("a thread's menu acts on the thread clicked, not the selected one", async () => {
    const test = column();
    expect(fireEvent.contextMenu(row("t1"), { clientX: 5, clientY: 5 })).toBe(false);
    expect(screen.getByTestId("context-menu").dataset.target).toBe("thread");
    // Unarchive is offered on archived threads only.
    expect(titles()).toEqual(["Archive", "Copy conversation", "Export"]);
    await test.user.click(item("Archive"));
    await waitFor(() =>
      expect(test.client.callsTo("threads.archive").map((call) => call.params)).toEqual([{ threadId: "t1" }]),
    );
    expect(test.host.window.get(SELECTED_THREAD)).toBe("t2");
    const archived = thread("t3", "old", "w1", { archivedAt: 3 });
    test.client.indexEvents({ kind: "thread_upserted", thread: archived });
    test.flush();
    expect(test.host.commands.resolve("thread.unarchive", [], ["t3"])?.id).toBe("thread.unarchive");
    expect(test.host.commands.resolve("thread.archive", [], ["t3"])).toBeUndefined();
    // A thread the window does not know is no thread, never the selected one.
    expect(test.host.commands.resolve("thread.delete", [], ["gone"])).toBeUndefined();
    // The `:` line and agents pass the id along: the selected thread is live, the one named archived.
    expect(await runRequested(test.host, "thread.unarchive", ["t3"])).toEqual({
      kind: "ran",
      command: "thread.unarchive",
    });
    act(() => {
      test.host.commands.run("cmdline.open");
    });
    await test.user.type(screen.getByTestId("command-line-input"), "unarchive t3{Enter}");
    await waitFor(() =>
      expect(test.client.callsTo("threads.unarchive").map((call) => call.params)).toEqual([
        { threadId: "t3" },
        { threadId: "t3" },
      ]),
    );
    act(() => {
      test.host.commands.run("delete", ["t3"]);
    });
    await waitFor(() =>
      expect(test.client.callsTo("threads.delete").map((call) => call.params)).toEqual([{ threadId: "t3" }]),
    );
  });

  // spec: UI-317
  test("Shift-F10 and the menu key open the focused item's menu; Enter runs its first item, Escape closes it", async () => {
    expect(
      DEFAULT_BINDINGS.filter((binding) => binding.command === "menu.open").map(({ mode, keys }) => [mode, keys]),
    ).toEqual([
      ["normal", "Shift-F10"],
      ["normal", "ContextMenu"],
    ]);
    const test = column();
    act(() => {
      test.host.commands.run("menu.open");
    });
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
      "The focus is on nothing with a menu: focus a message (j and k), a file or a thread",
    ]);
    expect(screen.queryByTestId("context-menu")).toBeNull();

    act(() => row("t1").focus());
    act(() => {
      test.host.commands.run("menu", [], ["threads"]);
    });
    await waitFor(() => expect(document.activeElement?.getAttribute("data-command")).toBe("thread.archive"));
    await test.user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByTestId("context-menu")).toBeNull());
    await waitFor(() => expect(document.activeElement?.getAttribute("data-thread-id")).toBe("t1"));
    expect(test.client.callsTo("threads.archive")).toEqual([]);

    act(() => {
      test.host.commands.run("menu.open", [], ["threads"]);
    });
    await waitFor(() => expect(document.activeElement?.getAttribute("data-command")).toBe("thread.archive"));
    await test.user.keyboard("{Enter}");
    await waitFor(() =>
      expect(test.client.callsTo("threads.archive").map((call) => call.params)).toEqual([{ threadId: "t1" }]),
    );
  });

  // spec: UI-318
  test("a finger resting on a target opens its menu there, a link that keeps its presses too; the release is taken", async () => {
    page(
      <button
        type="button"
        data-omfp-path="src/lib.rs"
        data-testid="path"
        onPointerDown={(event) => event.stopPropagation()}
        onPointerUp={(event) => event.stopPropagation()}
      >
        lib.rs
      </button>,
    );
    const path = screen.getByTestId("path");
    fireEvent.pointerDown(path, touch(20));
    await waitFor(() => expect(screen.getByTestId("context-menu").dataset.target).toBe("file"), {
      timeout: MENU_LONG_PRESS_MS * 3,
    });
    // Android's own long-press menu during the same press.
    expect(fireEvent.contextMenu(path)).toBe(false);
    fireEvent.pointerUp(path, touch(20));
    expect(fireEvent.click(path)).toBe(false);
    expect(titles()).toEqual(["Open", "Copy path"]);
  });

  // spec: UI-318
  test("a finger that moves before the menu opens scrolls; once it opened, moving closes it", async () => {
    page(
      <span data-omfp-path="src/lib.rs" data-testid="path">
        lib.rs
      </span>,
    );
    const path = screen.getByTestId("path");
    fireEvent.pointerDown(path, touch(20));
    fireEvent.pointerMove(window, touch(40));
    fireEvent.pointerUp(window, touch(40));
    expect(fireEvent.click(path)).toBe(true);
    expect(screen.queryByTestId("context-menu")).toBeNull();

    fireEvent.pointerDown(path, touch(20));
    await waitFor(() => expect(screen.getByTestId("context-menu").dataset.target).toBe("file"), {
      timeout: MENU_LONG_PRESS_MS * 3,
    });
    fireEvent.pointerMove(window, touch(50));
    await waitFor(() => expect(screen.queryByTestId("context-menu")).toBeNull());
  });

  // spec: UI-315, UI-319
  test("a widget adds items while shown: each runs its command with the message's index, a repeated one shows once", async () => {
    const ran: string[][] = [];
    const test = transcript(<Pins ran={ran} />);
    fireEvent.contextMenu(messages()[2] as HTMLElement, { clientX: 40, clientY: 50 });
    expect(titles()).toEqual(["Fork from here", "Copy", "Pin"]);
    await test.user.click(item("Pin"));
    expect(ran).toEqual([["2"]]);
    expect(test.client.callsTo("threads.fork")).toEqual([]);
  });
});
