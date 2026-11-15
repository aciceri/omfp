/** L4: opening files in the frame (UI-115, UI-116, DMN-060) and vim's mode on the mode line (UI-112), over a fake
 * client. */

import { afterEach, describe, expect, test } from "bun:test";
import type { ViewsState } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AgentFiles } from "../../src/frame/agent-files.tsx";
import { Frame } from "../../src/frame/frame.tsx";
import {
  EDITOR_MODE,
  FILE_PICKER,
  type KeyValueStorage,
  OPEN_FILES,
  openFile,
  SELECTED_THREAD,
  useCommand,
  type WidgetComponent,
} from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { fileTabId } from "../../src/vm/files.ts";
import { FileWidget } from "../../src/widgets/file/index.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

/** `omfp:file` as its props, so the tests see what a tab asks for. */
const StubFile: WidgetComponent = (props) => <p data-testid="stub-file">{JSON.stringify(props)}</p>;

/** The stub files shown: the other open files stay mounted, hidden (UI-400, UI-401). */
function shownFiles(): HTMLElement[] {
  return screen.queryAllByTestId("stub-file").filter((file) => file.closest("[hidden]") === null);
}

/** The props of the one file shown. */
function shownFile(): unknown {
  const shown = shownFiles();
  expect(shown.length).toBe(1);
  return JSON.parse(shown[0]?.textContent ?? "");
}

/** A text field `i` focuses, standing for the file editor. */
const StubChat: WidgetComponent = () => {
  useCommand({ id: "composer.focus", title: "Focus", run: () => document.getElementById("stub-input")?.focus() });
  return (
    <div>
      <textarea id="stub-input" data-testid="stub-input" />
      <button type="button" data-testid="card" data-omfp-path="fe/src/lib.rs" data-omfp-line="4">
        card
      </button>
      <button type="button" data-testid="changed" data-omfp-path="fe/src/main.rs" data-omfp-diff="">
        changed
      </button>
    </div>
  );
};

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", () => <p>header</p>],
  ["omfp:chat", StubChat],
  ["omfp:workspace", () => <p>workspace</p>],
  ["omfp:changes", () => <p>changes</p>],
  ["omfp:file", StubFile],
]);

class MemoryStorage implements KeyValueStorage {
  readonly items = new Map<string, string>();
  getItem(key: string): string | null {
    return this.items.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.items.set(key, value);
  }
}

function setup(views?: Partial<ViewsState>) {
  const storage = new MemoryStorage();
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: new ManualTimers(),
    storage,
  });
  base.client.answers.set("widgets.report", () => ({}));
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "one", "w1")] });
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  if (views !== undefined) {
    test.client.views("w1", views);
  }
  test.flush();
  return { ...test, storage, user: userEvent.setup() };
}

function press(key: string): void {
  act(() => {
    fireEvent.keyDown(document.activeElement ?? document.body, { key });
  });
}

/** The tabs of the `main` slot (the default view's side tabs are another container). */
function tabElements(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('[data-slot="main"] [role="tab"]')];
}

function tabs(): string[] {
  return tabElements().map((tab) => tab.textContent ?? "");
}

describe("file tabs in the main slot", () => {
  // spec: UI-115, AGT-027
  test("a file opens as a selected tab after the chat; opening it again applies the line; closing goes back", async () => {
    const test = setup();
    // One tab needs no tab bar.
    expect(tabs()).toEqual([]);
    act(() => openFile(test.host, "fe/src/lib.rs", { line: 3 }));
    expect(tabs()).toEqual(["Chat", "lib.rs"]);
    const selected = tabElements().find((tab) => tab.getAttribute("aria-selected") === "true");
    expect(selected?.dataset.file).toBe("fe/src/lib.rs");
    expect(shownFile()).toEqual({
      path: "fe/src/lib.rs",
      line: 3,
      jump: 1,
    });
    act(() => openFile(test.host, "fe/README.md", { diff: true }));
    expect(tabs()).toEqual(["Chat", "lib.rs", "README.md"]);
    expect(shownFile()).toEqual({
      path: "fe/README.md",
      diff: true,
      jump: 1,
    });
    act(() => openFile(test.host, "fe/src/lib.rs", { line: 9 }));
    expect(shownFile()).toEqual({
      path: "fe/src/lib.rs",
      line: 9,
      jump: 2,
    });
    // Window-local, kept in localStorage, never written to views.json.
    expect(JSON.parse(test.storage.getItem("omfp.openFiles") ?? "{}").w1.active).toBe(fileTabId("fe/src/lib.rs"));
    expect(test.client.callsTo("views.set")).toEqual([]);
    // Each file tab has its own close mark: closing the selected one selects its neighbour.
    const closeOf = (file: string) =>
      within(tabElements().find((tab) => tab.dataset.file === file) as HTMLElement).getByTestId("file-tab-close");
    await test.user.click(closeOf("fe/src/lib.rs"));
    expect(tabs()).toEqual(["Chat", "README.md"]);
    await test.user.click(closeOf("fe/README.md"));
    expect(tabs()).toEqual([]);
    expect(screen.getByTestId("stub-input")).toBeDefined();
  });

  // spec: UI-365
  test("a middle click closes the file tab under the pointer, selected or not; other tabs ignore it", () => {
    const test = setup();
    act(() => openFile(test.host, "fe/src/lib.rs"));
    act(() => openFile(test.host, "fe/README.md"));
    expect(tabs()).toEqual(["Chat", "lib.rs", "README.md"]);
    const tab = (file: string) => tabElements().find((element) => element.dataset.file === file) as HTMLElement;
    const chat = tabElements().find((element) => element.textContent === "Chat") as HTMLElement;
    // The middle button's press must not start the browser's autoscroll.
    expect(fireEvent.mouseDown(tab("fe/src/lib.rs"), { button: 1 })).toBe(false);
    fireEvent(tab("fe/src/lib.rs"), new MouseEvent("auxclick", { button: 1, bubbles: true, cancelable: true }));
    expect(tabs()).toEqual(["Chat", "README.md"]);
    const selected = tabElements().find((element) => element.getAttribute("aria-selected") === "true");
    expect(selected?.dataset.file).toBe("fe/README.md");
    fireEvent(chat, new MouseEvent("auxclick", { button: 1, bubbles: true, cancelable: true }));
    fireEvent(tab("fe/README.md"), new MouseEvent("auxclick", { button: 2, bubbles: true, cancelable: true }));
    expect(tabs()).toEqual(["Chat", "README.md"]);
    fireEvent(tab("fe/README.md"), new MouseEvent("auxclick", { button: 1, bubbles: true, cancelable: true }));
    expect(tabs()).toEqual([]);
  });

  // spec: UI-366
  test("every file tab carries its close mark: an unselected one closes without being selected; Delete closes the focused tab", async () => {
    const test = setup();
    act(() => openFile(test.host, "fe/src/lib.rs"));
    act(() => openFile(test.host, "fe/README.md"));
    const tab = (file: string) => tabElements().find((element) => element.dataset.file === file) as HTMLElement;
    expect(screen.getAllByTestId("file-tab-close").length).toBe(2);
    await test.user.click(within(tab("fe/src/lib.rs")).getByTestId("file-tab-close"));
    expect(tabs()).toEqual(["Chat", "README.md"]);
    const selected = tabElements().find((element) => element.getAttribute("aria-selected") === "true");
    expect(selected?.dataset.file).toBe("fe/README.md");
    act(() => tab("fe/README.md").focus());
    await test.user.keyboard("{Delete}");
    expect(tabs()).toEqual([]);
  });

  // spec: UI-115
  test("selecting the chat tab keeps the files open; tabs are per workspace", async () => {
    const test = setup();
    act(() => openFile(test.host, "a.rs"));
    await test.user.click(tabElements()[0] as HTMLElement);
    expect(screen.getByTestId("stub-input")).toBeDefined();
    expect(test.host.window.get(OPEN_FILES).w1?.tabs.map((tab) => tab.path)).toEqual(["a.rs"]);
    expect(test.host.window.get(OPEN_FILES).w1?.active).toBe("0.0.1.0");
  });

  // spec: UI-115, UI-366
  test("a view without a main slot opens the files as tabs of a floating panel, closed by their own × or Esc", async () => {
    const test = setup({
      manifest: { default: "plain", views: { plain: { layout: ["row", {}, "omfp:chat", "omfp:workspace"] } } },
    });
    act(() => openFile(test.host, "fe/src/lib.rs"));
    act(() => openFile(test.host, "fe/src/main.rs", { line: 2 }));
    const panel = screen.getByTestId("floating-file");
    expect(panel.getAttribute("aria-label")).toBe("Open files (2)");
    expect(
      within(panel)
        .getAllByTestId("floating-file-tab")
        .map((tab) => tab.dataset.file),
    ).toEqual(["fe/src/lib.rs", "fe/src/main.rs"]);
    expect(shownFile()).toEqual({
      path: "fe/src/main.rs",
      line: 2,
      jump: 1,
    });
    // Selecting a tab shows its file; the selection is window-local like the main slot's.
    await test.user.click(within(panel).getAllByTestId("floating-file-tab")[0] as HTMLElement);
    expect(shownFile()).toMatchObject({ path: "fe/src/lib.rs" });
    expect(test.host.window.get(OPEN_FILES).w1?.active).toBe(fileTabId("fe/src/lib.rs"));
    // Closing the selected file by its own × selects its neighbour.
    const lib = within(panel).getAllByTestId("floating-file-tab")[0] as HTMLElement;
    await test.user.click(within(lib).getByTestId("file-tab-close"));
    expect(
      within(panel)
        .getAllByTestId("floating-file-tab")
        .map((tab) => tab.dataset.file),
    ).toEqual(["fe/src/main.rs"]);
    act(() => screen.getByTestId("floating-file-move").focus());
    press("Escape");
    expect(screen.queryByTestId("floating-file")).toBeNull();
  });

  // spec: UI-115
  test("the floating panel moves, resizes and minimises, and keeps that in this window", async () => {
    const test = setup({
      manifest: { default: "plain", views: { plain: { layout: ["row", {}, "omfp:chat"] } } },
    });
    act(() => openFile(test.host, "a.rs"));
    act(() => screen.getByTestId("floating-file-move").focus());
    press("ArrowRight");
    press("ArrowDown");
    act(() => screen.getByTestId("floating-file-resize").focus());
    press("ArrowRight");
    const panel = screen.getByTestId("floating-file");
    // happy-dom lays nothing out: the panel starts as a 0×0 box at 0,0, and a resize keeps the minimum size.
    expect([panel.style.left, panel.style.top, panel.style.width, panel.style.height]).toEqual([
      "16px",
      "16px",
      "240px",
      "120px",
    ]);
    await test.user.click(screen.getByTestId("floating-file-minimise"));
    expect(screen.getByTestId("floating-file").dataset.minimised).toBe("");
    expect(shownFiles().length).toBe(0);
    expect(JSON.parse(test.storage.getItem("omfp.floatingFiles") ?? "{}")).toEqual({
      box: { left: 16, top: 16, width: 240, height: 120 },
      minimised: true,
    });
    await test.user.click(screen.getByTestId("floating-file-minimise"));
    expect(shownFiles().length).toBe(1);
  });
});

describe("unsaved edits of an open file", () => {
  const TEXT = "fn one() {}";
  const withFile: WidgetRegistry = new Map<string, WidgetComponent>([...registry, ["omfp:file", FileWidget]]);

  /** The frame with the real `omfp:file`, `fe/src/lib.rs` open in it and edited: `XY` typed at its start. */
  async function edited(views?: Partial<ViewsState>) {
    const storage = new MemoryStorage();
    const base = testHost(undefined, {
      importer: () => new Promise<unknown>(() => {}),
      reportTimers: new ManualTimers(),
      storage,
      filePoll: () => () => {},
    });
    base.client.answers.set("widgets.report", () => ({}));
    base.client.answers.set("workspaces.read_file", () => ({
      path: "fe/src/lib.rs",
      kind: "text",
      size: TEXT.length,
      hash: "h1",
      unchanged: false,
      content: TEXT,
      truncated: false,
      repo: "fe",
      head: TEXT,
    }));
    const test = renderWithHost(<Frame registry={withFile} now={() => 0} />, base);
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "one", "w1")] });
    act(() => test.host.window.set(SELECTED_THREAD, "t1"));
    if (views !== undefined) {
      test.client.views("w1", views);
    }
    test.flush();
    act(() => openFile(test.host, "fe/src/lib.rs"));
    const user = userEvent.setup();
    await user.click(await screen.findByTestId("file-edit"));
    await waitFor(() => expect(screen.getByTestId("file-editor").querySelector(".cm-content")).not.toBeNull());
    // Editing starts in insert mode with the focus in the editor (UI-111).
    await user.keyboard("XY");
    expect(editorText()).toBe(`XY${TEXT}`);
    return { ...test, user };
  }

  function editorText(): string {
    const lines = screen.getByTestId("file-editor").querySelectorAll(".cm-line");
    return [...lines].map((line) => line.textContent).join("\n");
  }

  /** The file is still being edited, with the user's text, unsaved. */
  function stillEdited(): void {
    expect(editorText()).toBe(`XY${TEXT}`);
    const editor = screen.getByTestId("file-editor");
    expect([editor.dataset.mode, editor.dataset.dirty, editor.dataset.readOnly]).toEqual(["edit", "true", "false"]);
    expect(screen.queryAllByTestId("file-save").length).toBe(1);
  }

  // spec: UI-400
  test("survive showing another tab of the main slot: the file's tab stays mounted, hidden and inert, then shows as it was", async () => {
    const test = await edited();
    const [chat, file] = tabElements();
    await test.user.click(chat as HTMLElement);
    const panel = screen.getByTestId("file").closest(".omfp-view__panel") as HTMLElement;
    expect([panel.hidden, panel.hasAttribute("inert"), panel.getAttribute("role")]).toEqual([true, true, null]);
    expect(screen.getByTestId("stub-input").closest("[hidden]")).toBeNull();
    expect(document.querySelectorAll('[data-slot="main"] [role="tabpanel"]').length).toBe(1);
    await test.user.click(file as HTMLElement);
    expect(panel.hidden).toBe(false);
    stillEdited();
    // Leaving still warns about the unsaved change.
    await test.user.click(screen.getByTestId("file-done"));
    expect(screen.getByTestId("file-notice").textContent).toBe("Unsaved changes: save with :w, or drop them with :q!");
  });

  // spec: UI-401
  test("survive minimising the floating file panel and restoring it", async () => {
    const test = await edited({
      manifest: { default: "plain", views: { plain: { layout: ["row", {}, "omfp:chat"] } } },
    });
    await test.user.click(screen.getByTestId("floating-file-minimise"));
    expect(screen.getByTestId("floating-file").dataset.minimised).toBe("");
    expect(screen.getByTestId("file").closest("[hidden]")).not.toBeNull();
    expect(screen.queryAllByTestId("floating-file-tab").filter((tab) => tab.closest("[hidden]") === null).length).toBe(
      0,
    );
    await test.user.click(screen.getByTestId("floating-file-minimise"));
    expect(screen.getByTestId("file").closest("[hidden]")).toBeNull();
    stillEdited();
  });
});

describe("the file picker", () => {
  const candidates = [
    { path: "app/src/theme.ts", repo: "app", match: "basename", mentioned: true },
    { path: "lib/theme.ts", repo: "lib", match: "basename", mentioned: false },
    { path: "notes/theme.ts", match: "basename", mentioned: false },
  ] as const;

  function pick() {
    const test = setup();
    act(() => openFile(test.host, "theme.ts", { line: 3 }));
    act(() =>
      test.host.window.set(FILE_PICKER, { workspaceId: "w1", path: "theme.ts", options: { line: 3 }, candidates }),
    );
    return test;
  }

  function options(): string[] {
    return screen.getAllByTestId("file-picker-option").map((option) => option.dataset.path ?? "");
  }

  function highlighted(): string | undefined {
    return screen.getAllByTestId("file-picker-option").find((option) => option.getAttribute("aria-selected") === "true")
      ?.dataset.path;
  }

  // spec: UI-117
  test("arrows move through the candidates, Enter opens the highlighted one in the missing path's tab", async () => {
    const test = pick();
    expect(document.activeElement === screen.getByTestId("file-picker-input")).toBe(true);
    expect(options()).toEqual(["app/src/theme.ts", "lib/theme.ts", "notes/theme.ts"]);
    expect(screen.getAllByTestId("file-picker-option")[0]?.textContent).toContain("in the conversation");
    expect(highlighted()).toBe("app/src/theme.ts");
    await test.user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}{ArrowUp}");
    expect(highlighted()).toBe("notes/theme.ts");
    await test.user.keyboard("li");
    expect(options()).toEqual(["lib/theme.ts"]);
    await test.user.keyboard("{Enter}");
    expect(screen.queryByTestId("file-picker")).toBeNull();
    expect(test.host.window.get(OPEN_FILES).w1?.tabs.map((tab) => [tab.path, tab.line])).toEqual([["lib/theme.ts", 3]]);
    expect(shownFile()).toMatchObject({ path: "lib/theme.ts" });
  });

  // spec: UI-117
  test("Escape cancels the opening: the missing path's tab closes", async () => {
    const test = pick();
    await test.user.keyboard("{Escape}");
    expect(screen.queryByTestId("file-picker")).toBeNull();
    expect(test.host.window.get(OPEN_FILES).w1?.tabs).toEqual([]);
    expect(tabs()).toEqual([]);
  });
});

describe("workspace-wide file picker", () => {
  const candidates = [
    { path: "app/src/config.ts", repo: "app", match: "fuzzy", mentioned: false },
    { path: "lib/ui/config.ts", repo: "lib", match: "fuzzy", mentioned: false },
  ] as const;

  // spec: UI-731
  test("Space f disambiguates matching basenames by repository and folder, and opens the arrow-selected file", async () => {
    const test = setup();
    press(" ");
    press("f");
    await act(async () => {
      test.client.callsTo("workspaces.find_file").at(-1)?.resolve({ candidates, truncated: false });
    });
    const rows = screen.getAllByTestId("workspace-file-picker-option");
    expect(rows.map((row) => row.dataset.id)).toEqual(["app/src/config.ts", "lib/ui/config.ts"]);
    expect(rows[0]?.textContent).toContain("app/src");
    expect(rows[1]?.textContent).toContain("lib/ui");
    await test.user.keyboard("{ArrowDown}{Enter}");
    expect(screen.queryByTestId("workspace-file-picker")).toBeNull();
    expect(shownFile()).toMatchObject({ path: "lib/ui/config.ts" });
  });

  // spec: UI-731
  test("late query results cannot replace the current matches; Escape preserves existing file tabs", async () => {
    const test = setup();
    act(() => openFile(test.host, "notes/keep.md"));
    press(" ");
    press("f");
    const initial = test.client.callsTo("workspaces.find_file").at(-1);
    const input = screen.getByTestId("workspace-file-picker-input");
    await test.user.type(input, "lbut");
    await act(async () => {
      test.client
        .callsTo("workspaces.find_file")
        .at(-1)
        ?.resolve({ candidates: [candidates[1]], truncated: false });
    });
    expect(screen.getAllByTestId("workspace-file-picker-option").map((row) => row.dataset.id)).toEqual([
      "lib/ui/config.ts",
    ]);
    await act(async () => {
      initial?.resolve({ candidates: [candidates[0]], truncated: false });
    });
    expect(screen.getAllByTestId("workspace-file-picker-option").map((row) => row.dataset.id)).toEqual([
      "lib/ui/config.ts",
    ]);
    await test.user.keyboard("{Escape}");
    expect(shownFile()).toMatchObject({ path: "notes/keep.md" });
    expect(screen.queryByTestId("workspace-file-picker")).toBeNull();
  });
});

describe("opening paths", () => {
  // spec: UI-116
  test("`:open` opens the focused element's path, or a typed path and line", async () => {
    const test = setup();
    act(() => screen.getByTestId("card").focus());
    press(":");
    await test.user.keyboard("open{Enter}");
    expect(shownFile()).toEqual({
      path: "fe/src/lib.rs",
      line: 4,
      jump: 1,
    });
    await test.user.click(tabElements()[0] as HTMLElement);
    act(() => screen.getByTestId("changed").focus());
    press(":");
    await test.user.keyboard("open{Enter}");
    expect(shownFile()).toEqual({
      path: "fe/src/main.rs",
      diff: true,
      jump: 1,
    });
    act(() => (document.activeElement as HTMLElement | null)?.blur());
    press(":");
    await test.user.keyboard("open docs/a.md:7{Enter}");
    expect(shownFile()).toEqual({
      path: "docs/a.md",
      line: 7,
      jump: 1,
    });
  });

  // spec: UI-116, UI-117
  test.each(["tone.wav", "media/tone.wav", "missing.unknown", "a file without extension"])(
    "an explicit filename opens a tab independently of inline prose recognition: %s",
    async (path) => {
      const test = setup();
      // A focused reference must not replace an explicit argument that prose would reject.
      act(() => screen.getByTestId("card").focus());
      press(":");
      await test.user.keyboard(`open ${path}:7:2{Enter}`);
      expect(shownFile()).toEqual({ path, line: 7, jump: 1 });
    },
  );

  // spec: UI-701
  test("`:open` with an absolute path outside the workspace folder opens it as typed", async () => {
    const test = setup();
    press(":");
    await test.user.keyboard("open /tmp/elsewhere/README.md:3{Enter}");
    expect(shownFile()).toEqual({
      path: "/tmp/elsewhere/README.md",
      line: 3,
      jump: 1,
    });
  });
});

describe("files the agent opens", () => {
  // spec: UI-116, DMN-060
  test("the window an `open_file` request names opens the file at its line, once", async () => {
    const test = setup();
    test.client.thread("t1", []);
    test.client.threadEvents("t1", { kind: "open_file", id: "o1", connection: 2, path: "fe/other.rs", line: 1 });
    test.flush();
    expect(tabs()).toEqual([]);

    // This window is connection 1 (the fake `system.hello`).
    const request = { kind: "open_file", id: "o2", connection: 1, path: "fe/src/lib.rs", line: 12 } as const;
    test.client.threadEvents("t1", request);
    test.flush();
    expect(tabs()).toEqual(["Chat", "lib.rs"]);
    expect(shownFile()).toEqual({
      path: "fe/src/lib.rs",
      line: 12,
      jump: 1,
    });

    // The thread's snapshot still carries the request (followed again, a resync): it is not opened twice.
    await test.user.click(screen.getByTestId("file-tab-close"));
    test.client.thread("t1", [request]);
    test.flush();
    expect(tabs()).toEqual([]);

    test.client.threadEvents("t1", { ...request, id: "o3", path: "fe/README.md", line: null });
    test.flush();
    expect(tabs()).toEqual(["Chat", "README.md"]);
    expect(shownFile()).toEqual({ path: "fe/README.md", jump: 1 });
  });

  // spec: UI-116, DMN-060
  test("an agent request switches back to its thread before opening the file", () => {
    const test = renderWithHost(<AgentFiles />);
    test.client.connected();
    test.client.index({
      workspaces: [workspace("w1"), workspace("w2")],
      threads: [thread("t1", "one", "w1"), thread("t2", "two", "w2")],
    });
    act(() => test.host.window.set(SELECTED_THREAD, "t2"));
    test.client.thread("t1", []);
    test.client.threadEvents("t1", {
      kind: "open_file",
      id: "o1",
      connection: 1,
      path: "fe/src/lib.rs",
      line: 12,
    });
    test.flush();

    expect(test.host.window.get(SELECTED_THREAD)).toBe("t1");
    expect(test.host.window.get(OPEN_FILES).w1?.tabs.map((tab) => tab.path)).toEqual(["fe/src/lib.rs"]);
    expect(test.host.window.get(OPEN_FILES).w2).toBeUndefined();
  });
});

describe("mode line", () => {
  // spec: UI-112
  test("while the file editor has the focus the mode line shows vim's mode", () => {
    const test = setup();
    // The mode pill is rendered again on each mode change (its entrance motion).
    const line = () => screen.getByTestId("mode-line");
    press("i");
    expect(line().textContent).toBe("INSERT");
    act(() => test.host.window.set(EDITOR_MODE, "visual"));
    expect([line().textContent, line().dataset.vim]).toEqual(["VISUAL", "visual"]);
    act(() => test.host.window.set(EDITOR_MODE, "normal"));
    expect(line().textContent).toBe("NORMAL");
    act(() => test.host.window.set(EDITOR_MODE, undefined));
    expect(line().textContent).toBe("INSERT");
  });
});
