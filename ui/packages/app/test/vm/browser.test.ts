/** L0: the browser tabs of a thread from its `eval` calls (UI-870..UI-872). */

import { describe, expect, test } from "bun:test";
import { replay, type ThreadEvent } from "@omfp/core";
import { browserView } from "../../src/vm/browser.ts";
import { browserSession, evalCall, runningEvalCall, screenshot } from "../support/browser.ts";

function tabs(events: readonly ThreadEvent[]) {
  return browserView(replay(events)).tabs.map(({ name, url, action, closed, screenshot: shot }) => ({
    name,
    url,
    action,
    closed,
    screenshot: shot === undefined ? undefined : atob(shot.data),
  }));
}

describe("browserView", () => {
  // spec: UI-870, UI-871, UI-872
  test("each tab with its address, last action and latest screenshot, the most recently used first", () => {
    expect(tabs(browserSession())).toEqual([
      {
        name: "docs",
        url: "https://docs.example.com/api",
        action: 'docs.screenshot({"fullPage":true})',
        closed: false,
        screenshot: "docs-2",
      },
      { name: "main", url: "https://example.com/", action: "close main", closed: true, screenshot: "main-1" },
    ]);
  });

  // spec: UI-871, UI-872
  test("before the running call, the shared screenshot call left both tabs without a new screenshot", () => {
    const events = browserSession();
    const running = events.findIndex((event) => JSON.stringify(event).includes("call_eval_5"));
    expect(tabs(events.slice(0, running))).toEqual([
      { name: "main", url: "https://example.com/", action: "close main", closed: true, screenshot: "main-1" },
      {
        name: "docs",
        url: "https://docs.example.com/api",
        action: "docs.screenshot()",
        closed: false,
        screenshot: undefined,
      },
    ]);
  });

  // spec: UI-871
  test("close all closes every tab; a tab used again opens; an unknown close, tabs and errors list nothing", () => {
    expect(
      tabs([
        ...evalCall("a", [
          { op: "browser", detail: "close ghost" },
          { op: "browser", detail: "targets" },
          { op: "browser", error: "No tab named main" },
          { op: "browser", detail: "open a.b" },
          { op: "browser", detail: "open c about:blank" },
        ]),
        ...evalCall("b", [{ op: "browser", detail: "close all" }]),
        ...evalCall("c", [{ op: "browser", detail: 'a.b.frame("#f").click("a")' }]),
      ]),
    ).toEqual([
      { name: "a.b", url: undefined, action: 'a.b.frame("#f").click("a")', closed: false, screenshot: undefined },
      { name: "c", url: "about:blank", action: "close all", closed: true, screenshot: undefined },
    ]);
  });

  // spec: UI-872
  test("a screenshot goes to the one tab that could take it: a frame's, a run's, never with the computer or a plain image", () => {
    const plain = { type: "image", data: btoa("display"), mimeType: "image/png" };
    expect(
      tabs([
        ...evalCall(
          "a",
          [
            { op: "browser", detail: "open main https://a.test/" },
            { op: "browser", detail: "open side https://b.test/" },
            { op: "browser", detail: 'main.frame("#f").screenshot()' },
            { op: "browser", detail: "side.title()" },
          ],
          [screenshot("frame"), plain],
        ),
        ...evalCall("b", [{ op: "browser", detail: "side.run(await page.screenshot())" }], [screenshot("run")]),
        ...evalCall(
          "c",
          [
            { op: "browser", detail: "side.screenshot()" },
            { op: "computer", detail: "screenshot" },
          ],
          [screenshot("computer")],
        ),
        ...evalCall("d", [{ op: "browser", detail: "main.screenshot()" }], [plain]),
      ]).map(({ name, screenshot: shot }) => [name, shot]),
    ).toEqual([
      ["main", "frame"],
      ["side", "run"],
    ]);
  });

  // spec: UI-870
  test("a thread without browser calls has no tabs", () => {
    expect(tabs(runningEvalCall("a", []))).toEqual([]);
    expect(tabs([])).toEqual([]);
  });
});
