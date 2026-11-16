/** L0: file and widget tabs (UI-115, UI-373) and the text a file had before a patch (UI-080). */

import { describe, expect, test } from "bun:test";
import {
  closeOpenedTab,
  closeTab,
  decodeFloatingPanel,
  decodeOpenFiles,
  type FileTabs,
  FLOATING_MIN,
  FLOATING_PANEL_DEFAULT,
  fileName,
  fileTabId,
  movePanel,
  NO_FILE_TABS,
  neighbourTab,
  openedTabIds,
  openTab,
  openWidgetTab,
  originalFromPatch,
  resizePanel,
  retargetTab,
  widgetTabId,
  workspaceFilePath,
} from "../../src/vm/files.ts";

describe("file tabs", () => {
  // spec: UI-115
  test("opening adds a tab at the end; opening it again gives it the new line and diff", () => {
    let tabs = openTab(NO_FILE_TABS, "fe/src/a.rs", { line: 3 });
    tabs = openTab(tabs, "fe/src/b.rs", { diff: true });
    expect(tabs.tabs.map((tab) => tab.path)).toEqual(["fe/src/a.rs", "fe/src/b.rs"]);
    tabs = openTab(tabs, "fe/src/a.rs", { line: 9 });
    expect(tabs.tabs.map((tab) => tab.path)).toEqual(["fe/src/a.rs", "fe/src/b.rs"]);
    expect(tabs.tabs[0]).toEqual({ path: "fe/src/a.rs", line: 9, diff: undefined, seq: 2 });
  });

  // spec: UI-115, UI-366
  test("closing the selected tab selects its neighbour, then the slot's own first tab", () => {
    let tabs: FileTabs = ["a", "b", "c"].reduce((open, path) => openTab(open, path), NO_FILE_TABS);
    expect(neighbourTab(tabs, fileTabId("b"))).toBe(fileTabId("a"));
    expect(neighbourTab(tabs, fileTabId("a"))).toBe(fileTabId("b"));
    tabs = closeTab(tabs, "b");
    expect(tabs.tabs.map((tab) => tab.path)).toEqual(["a", "c"]);
    tabs = closeTab(tabs, "c");
    expect(neighbourTab(tabs, fileTabId("a"))).toBeUndefined();
    tabs = closeTab(tabs, "a");
    expect(tabs).toEqual({ tabs: [], widgets: [] });
    expect(closeTab(tabs, "gone")).toBe(tabs);
    expect(neighbourTab(tabs, fileTabId("gone"))).toBeUndefined();
  });

  // spec: UI-373
  test("widget tabs open once after the files; closing one selects its neighbour among all opened tabs", () => {
    let tabs = openWidgetTab(openTab(NO_FILE_TABS, "a.rs"), "omfp:activity");
    tabs = openTab(tabs, "b.rs");
    tabs = openWidgetTab(tabs, "omfp:todos");
    expect(openedTabIds(tabs)).toEqual([
      fileTabId("a.rs"),
      fileTabId("b.rs"),
      widgetTabId("omfp:activity"),
      widgetTabId("omfp:todos"),
    ]);
    expect(openWidgetTab(tabs, "omfp:activity")).toBe(tabs);
    expect(neighbourTab(tabs, widgetTabId("omfp:activity"))).toBe(fileTabId("b.rs"));
    tabs = closeOpenedTab(tabs, widgetTabId("omfp:activity"));
    expect(tabs.widgets).toEqual(["omfp:todos"]);
    tabs = closeTab(closeTab(tabs, "b.rs"), "a.rs");
    expect(tabs).toEqual({ tabs: [], widgets: ["omfp:todos"] });
    // Files keep the widget tabs.
    expect(retargetTab(openTab(tabs, "x"), "x", "y").widgets).toEqual(["omfp:todos"]);
    expect(closeOpenedTab(tabs, widgetTabId("gone"))).toBe(tabs);
  });

  // spec: UI-115, UI-373
  test("the persisted tabs decode leniently", () => {
    expect(
      decodeOpenFiles({
        w1: { tabs: [{ path: "a.rs", line: 2, diff: true }, { nope: 1 }, "x"], active: "file:a.rs" },
        w2: { tabs: "bad" },
        w3: null,
        w4: { tabs: [], widgets: ["omfp:activity", 3, "omfp:activity"], active: "widget:omfp:activity" },
      }),
    ).toEqual({
      // The selection stored before it was per thread (UI-372) is not carried over.
      w1: { tabs: [{ path: "a.rs", line: 2, diff: true, seq: 0 }], widgets: [] },
      w4: { tabs: [], widgets: ["omfp:activity"] },
    });
    expect(decodeOpenFiles([])).toBeUndefined();
    expect(decodeOpenFiles("x")).toBeUndefined();
  });

  // spec: UI-115
  test("names and workspace paths", () => {
    expect(fileName("fe/src/lib.rs")).toBe("lib.rs");
    expect(fileName("Makefile")).toBe("Makefile");
    expect(workspaceFilePath({ name: "fe", isRoot: false }, "src/lib.rs")).toBe("fe/src/lib.rs");
    expect(workspaceFilePath({ name: "ws", isRoot: true }, "src/lib.rs")).toBe("src/lib.rs");
  });
});

describe("a missing path's tab", () => {
  // spec: UI-117
  test("shows the chosen file in its place with its line, or gives way to the tab already showing it", () => {
    let tabs = openTab(NO_FILE_TABS, "a.rs");
    tabs = openTab(tabs, "theme.ts", { line: 7 });
    tabs = openTab(tabs, "z.rs");
    const moved = retargetTab(tabs, "theme.ts", "ui/theme.ts");
    expect(moved.tabs.map((tab) => [tab.path, tab.line])).toEqual([
      ["a.rs", undefined],
      ["ui/theme.ts", 7],
      ["z.rs", undefined],
    ]);

    const merged = retargetTab(tabs, "theme.ts", "z.rs");
    expect(merged.tabs.map((tab) => [tab.path, tab.line])).toEqual([
      ["a.rs", undefined],
      ["z.rs", 7],
    ]);

    // Without a tab of the missing path (an `omfp:file` placed in the view), the file opens.
    expect(retargetTab(NO_FILE_TABS, "theme.ts", "ui/theme.ts", { diff: true }).tabs).toEqual([
      { path: "ui/theme.ts", line: undefined, diff: true, seq: 1 },
    ]);
  });
});

describe("the floating panel", () => {
  const box = { left: 10, top: 20, width: 400, height: 300 };

  // spec: UI-115
  test("moves within the view and resizes down to its minimum", () => {
    expect(movePanel(FLOATING_PANEL_DEFAULT, box, 15.4, -50).box).toEqual({ ...box, left: 25, top: 0 });
    expect(resizePanel(FLOATING_PANEL_DEFAULT, box, 16, 16).box).toEqual({ ...box, width: 416, height: 316 });
    expect(resizePanel({ box, minimised: true }, box, -1000, -1000)).toEqual({
      box: { ...box, ...FLOATING_MIN },
      minimised: true,
    });
  });

  // spec: UI-115
  test("its persisted state decodes leniently", () => {
    expect(decodeFloatingPanel({ box, minimised: true })).toEqual({ box, minimised: true });
    expect(decodeFloatingPanel({ box: { left: "x" } })).toEqual(FLOATING_PANEL_DEFAULT);
    expect(decodeFloatingPanel("nope")).toBeUndefined();
  });
});

const HEADER = "diff --git a/f b/f\n--- a/f\n+++ b/f\n";

describe("originalFromPatch", () => {
  // spec: UI-080
  test("a change in the middle, an addition at the end and a deletion at the start", () => {
    const current = "one\nTWO\nthree\nfour\nfive\n";
    const patch = `${HEADER}@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n@@ -4,1 +4,2 @@\n four\n+five\n`;
    expect(originalFromPatch(current, patch)).toBe("one\ntwo\nthree\nfour\n");
    const deleted = `${HEADER}@@ -1,2 +0,0 @@\n-zero\n-half\n`;
    expect(originalFromPatch("one\n", deleted)).toBe("zero\nhalf\none\n");
  });

  // spec: UI-080
  test("a new file came from nothing, a deleted one from its lines; no patch is no change", () => {
    expect(originalFromPatch("a\nb\n", `${HEADER}@@ -0,0 +1,2 @@\n+a\n+b\n`)).toBe("");
    expect(originalFromPatch("", `${HEADER}@@ -1,2 +0,0 @@\n-a\n-b\n`)).toBe("a\nb\n");
    expect(originalFromPatch("same\n", "")).toBe("same\n");
  });

  // spec: UI-080
  test("the end-of-file markers say which side lacked a final line break", () => {
    const added = `${HEADER}@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+a\n`;
    expect(originalFromPatch("a\n", added)).toBe("a");
    const removed = `${HEADER}@@ -1 +1 @@\n-a\n+a\n\\ No newline at end of file\n`;
    expect(originalFromPatch("a", removed)).toBe("a\n");
  });

  // spec: UI-080
  test("a patch that does not fit the file, or without a hunk (binary), gives nothing", () => {
    const patch = `${HEADER}@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n`;
    expect(originalFromPatch("one\nchanged since\n", patch)).toBeUndefined();
    expect(originalFromPatch("x", `${HEADER}Binary files a/f and b/f differ\n`)).toBeUndefined();
    expect(originalFromPatch("one\n", `${HEADER}@@ -1,3 +5,3 @@\n a\n`)).toBeUndefined();
  });
});
