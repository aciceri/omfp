/** L4: `omfp:browser` over a fake client (UI-870..UI-872). */

import { afterEach, describe, expect, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import { cleanup, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SELECTED_THREAD } from "../../src/sdk/index.ts";
import { BrowserWidget } from "../../src/widgets/browser/index.ts";
import { browserSession, evalCall, screenshot } from "../support/browser.ts";
import { renderWithHost, testHost } from "../support/host.tsx";
import { summary } from "./chat/fixtures.ts";

afterEach(cleanup);

const THREAD = "01T";

function setup(events: readonly ThreadEvent[], select = true) {
  const base = testHost();
  if (select) {
    base.host.window.set(SELECTED_THREAD, THREAD);
  }
  const test = renderWithHost(<BrowserWidget />, base);
  test.client.connected();
  test.client.index({ threads: [summary(THREAD)] });
  test.client.thread(THREAD, events);
  test.flush();
  return test;
}

describe("omfp:browser", () => {
  // spec: UI-870, UI-187
  test("a box per tab titled on its border, the most recently used first, with its address and last action", () => {
    setup(browserSession());
    expect(screen.getByTestId("browser").textContent).toContain("not the live page");
    const boxes = screen.getAllByTestId("browser-tab");
    expect(boxes.map((box) => [box.dataset.tab, box.dataset.state])).toEqual([
      ["docs", "open"],
      ["main", "closed"],
    ]);
    const [docs, main] = boxes as [HTMLElement, HTMLElement];
    expect(docs.classList.contains("omfp-bbox")).toBe(true);
    expect(within(docs).getByRole("heading", { level: 2, name: "docs" })).toBeDefined();
    expect(main.querySelector(".omfp-bbox__end")?.textContent).toBe("closed");
    const link = within(docs).getByTestId("browser-url");
    expect([link.tagName, link.getAttribute("href"), link.getAttribute("target")]).toEqual([
      "A",
      "https://docs.example.com/api",
      "_blank",
    ]);
    expect(within(docs).getByTestId("browser-action").textContent).toBe('docs.screenshot({"fullPage":true})');
    expect(within(main).getByTestId("browser-action").textContent).toBe("close main");
  });

  // spec: UI-872, UI-474
  test("the latest screenshot opens at full size in a dialog; Escape closes it", async () => {
    setup(browserSession());
    const [docs] = screen.getAllByTestId("browser-tab") as [HTMLElement];
    const thumb = within(docs).getByTestId("browser-screenshot");
    expect(thumb.getAttribute("aria-label")).toBe("Open the latest screenshot of tab docs at full size");
    expect(thumb.querySelector("img")?.getAttribute("src")).toBe(`data:image/png;base64,${btoa("docs-2")}`);
    expect(screen.queryAllByTestId("browser-screenshot-dialog").length).toBe(0);
    await userEvent.click(thumb);
    const dialog = screen.getByTestId("browser-screenshot-dialog");
    expect(within(dialog).getByRole("heading").textContent).toBe("Latest screenshot of tab docs");
    const full = within(dialog).getByTestId("browser-screenshot-full");
    expect([full.getAttribute("src"), full.getAttribute("alt")]).toEqual([
      `data:image/png;base64,${btoa("docs-2")}`,
      "Latest screenshot of tab docs",
    ]);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryAllByTestId("browser-screenshot-dialog").length).toBe(0);
  });

  // spec: UI-870, UI-872
  test("a new call updates the tab; an address that is not http(s) is text; no screenshot says so", () => {
    const test = setup(
      evalCall(
        "a",
        [
          { op: "browser", detail: "open main https://a.test/" },
          { op: "browser", detail: "main.screenshot()" },
        ],
        [screenshot("first")],
      ),
    );
    expect(screen.getByTestId("browser-screenshot").querySelector("img")?.getAttribute("src")).toBe(
      `data:image/png;base64,${btoa("first")}`,
    );
    test.client.threadEvents(THREAD, ...evalCall("b", [{ op: "browser", detail: 'pop.goto("javascript:alert(1)")' }]));
    test.flush();
    const [pop] = screen.getAllByTestId("browser-tab") as [HTMLElement];
    expect(pop.dataset.tab).toBe("pop");
    const address = within(pop).getByTestId("browser-url");
    expect([address.tagName, address.getAttribute("href"), address.textContent]).toEqual([
      "P",
      null,
      "javascript:alert(1)",
    ]);
    expect(within(pop).getByTestId("browser-no-screenshot").textContent).toBe("No screenshot yet");
  });

  // spec: UI-870
  test("without browser calls or a selected thread, an empty state", () => {
    setup([]);
    expect(screen.getByTestId("browser-none").textContent).toContain("No browser tabs");
    cleanup();
    setup([], false);
    expect(screen.getByTestId("browser-no-thread").textContent).toContain("No thread selected");
  });
});
