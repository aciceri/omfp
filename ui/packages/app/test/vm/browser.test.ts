/** L0: the browser tabs of a thread from its `eval` calls (UI-870..UI-872). */

import { describe, expect, test } from "bun:test";
import { replay, type ThreadEvent } from "@omfp/core";
import { browserView } from "../../src/vm/browser.ts";
import { browserSession, evalCall, runningEvalCall, screenshot } from "../support/browser.ts";

/** The tabs as plain values; `movedBy` only when a call may have moved the tab (UI-871). */
function tabs(events: readonly ThreadEvent[]) {
  return browserView(replay(events)).tabs.map(({ name, url, movedBy, action, closed, screenshot: shot }) => ({
    name,
    url,
    ...(movedBy === undefined ? {} : { movedBy }),
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
      {
        name: "main",
        url: "https://example.com/",
        movedBy: 'main.click("#more")',
        action: "close main",
        closed: true,
        screenshot: "main-1",
      },
    ]);
  });

  // spec: UI-871, UI-872
  test("before the running call, the shared screenshot call left both tabs without a new screenshot", () => {
    const events = browserSession();
    const running = events.findIndex((event) => JSON.stringify(event).includes("call_eval_5"));
    expect(tabs(events.slice(0, running))).toEqual([
      {
        name: "main",
        url: "https://example.com/",
        movedBy: 'main.click("#more")',
        action: "close main",
        closed: true,
        screenshot: "main-1",
      },
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
  test("an address stays the one open or goto reported; the first later call that may have moved the tab is noted", () => {
    const moved = (events: readonly ThreadEvent[]) =>
      tabs(events).map(({ name, url, movedBy }) => [name, url, movedBy]);
    const opened = evalCall("a", [
      { op: "browser", detail: "open main file:///tmp/page.html" },
      { op: "browser", detail: "main.id(3).text()" },
      { op: "browser", detail: 'main.frame("a.b(").attr("#x", "href")' },
      { op: "browser", detail: "main.screenshot()" },
      { op: "browser", detail: 'side.click("a")' },
    ]);
    expect(moved(opened)).toEqual([
      ["side", undefined, undefined],
      ["main", "file:///tmp/page.html", undefined],
    ]);
    // A click that navigates: omp's lines say what was called, never what `url()` or `waitForUrl` answered.
    const clicked = [
      ...opened,
      ...evalCall("b", [
        { op: "browser", detail: "main.id(5).click()" },
        { op: "browser", detail: 'main.waitForUrl("example.com")' },
        { op: "browser", detail: "main.url()" },
      ]),
    ];
    expect(moved(clicked)[0]).toEqual(["main", "file:///tmp/page.html", "main.id(5).click()"]);
    // Opening the tab again reports where it is; goto sends it to its address; back, reload and a run may move it.
    const reported = [...clicked, ...evalCall("c", [{ op: "browser", detail: "open main https://example.com/" }])];
    expect(moved(reported)[0]).toEqual(["main", "https://example.com/", undefined]);
    const moving = ["main.back()", "main.reload()", 'main.waitForUrl("**/done")', 'main.run(location.assign("/x"))'];
    for (const detail of [...moving, 'main.press("Enter")']) {
      const after = [...reported, ...evalCall("d", [{ op: "browser", detail }])];
      expect(moved(after)[0]).toEqual(["main", "https://example.com/", detail]);
      const sent = [...after, ...evalCall("e", [{ op: "browser", detail: 'main.goto("https://b.test/")' }])];
      expect(moved(sent)[0]).toEqual(["main", "https://b.test/", undefined]);
    }
  });

  // spec: UI-871
  test("after a branch navigation, only the restored branch's calls count, not the runs the state still holds", () => {
    const events = browserSession();
    const before = events.slice(
      0,
      events.findIndex((event) => JSON.stringify(event).includes("call_eval_5")),
    );
    const branch = events
      .slice(
        0,
        events.findIndex((event) => JSON.stringify(event).includes("call_eval_2")),
      )
      .flatMap((event) => {
        const frame = event.kind === "omp" ? (event.frame as { type?: unknown; message?: unknown } | null) : null;
        return frame?.type === "message_end" ? [frame.message] : [];
      });
    const history = {
      kind: "history",
      messages: branch,
      session: { resumed: true, session_file: "/s/branch.jsonl", session_id: "s" },
    } as ThreadEvent;
    expect(tabs([...before, history])).toEqual([
      { name: "main", url: "https://example.com/", action: "main.screenshot()", closed: false, screenshot: "main-1" },
    ]);
    // A call in flight that the transcript does not hold yet still counts.
    const [, ...inFlight] = runningEvalCall("call_eval_6", [{ op: "browser", detail: "open side https://c.test/" }]);
    expect(tabs([...before, history, ...inFlight]).map(({ name, url }) => [name, url])).toEqual([
      ["side", "https://c.test/"],
      ["main", "https://example.com/"],
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
