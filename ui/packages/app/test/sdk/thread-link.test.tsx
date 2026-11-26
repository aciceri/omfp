/** L4: links to a thread in a reply (`omfp://thread/<id>`): selecting it, a new window on it, its title and state. */

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, fireEvent, screen } from "@testing-library/react";
import { SELECTED_THREAD } from "../../src/sdk/index.ts";
import { threadWindowUrl } from "../../src/sdk/thread-link.tsx";
import { MessageRow } from "../../src/widgets/chat/message.tsx";
import { thread } from "../support/fixtures.ts";
import { renderWithHost, type TestHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

/** An assistant reply saying `markdown` in thread `t1`, with `t1` selected and `t2` (`Fix the parser`) in the index. */
function reply(markdown: string, streaming = false): TestHost {
  const test = testHost();
  test.client.connected();
  test.client.index({
    threads: [thread("t1", "Current", "w1"), thread("t2", "Fix the parser", "w1", { settled: false })],
  });
  test.host.window.set(SELECTED_THREAD, "t1");
  renderWithHost(
    <MessageRow
      threadId="t1"
      cardKey="m1"
      streaming={streaming}
      continues={false}
      view={{
        role: "assistant",
        label: "Assistant",
        blocks: [{ type: "text", text: markdown }],
        error: undefined,
        steering: false,
      }}
    />,
    test,
  );
  test.flush();
  return test;
}

/** Runs `body` with `window.open` recording its calls. */
function withOpen(body: (opened: unknown[][]) => void): void {
  const open = window.open;
  const opened: unknown[][] = [];
  window.open = (...args: unknown[]) => {
    opened.push(args);
    return null;
  };
  try {
    body(opened);
  } finally {
    window.open = open;
  }
}

describe("thread links", () => {
  // spec: UI-045
  test.each([false, true])("a click on a link to a thread selects it in this window (streaming: %p)", (streaming) => {
    const test = reply("Fixed in [the parser thread](omfp://thread/t2).", streaming);
    const link = screen.getByTestId("thread-link");
    expect(link.textContent).toBe("the parser thread");
    expect(link.getAttribute("href")).toBe("#thread=t2");
    act(() => {
      fireEvent.click(link);
    });
    expect(test.host.window.get(SELECTED_THREAD)).toBe("t2");
  });

  // spec: UI-045
  test("a middle click or a click with Ctrl, Cmd or Shift opens a new window on the thread, keeping the selection", () => {
    const test = reply("See <omfp://thread/t2>.");
    const link = screen.getByTestId("thread-link");
    withOpen((opened) => {
      act(() => {
        fireEvent.click(link, { ctrlKey: true });
        fireEvent.click(link, { metaKey: true });
        fireEvent.click(link, { shiftKey: true });
        fireEvent(link, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
      });
      const url = threadWindowUrl(window.location.href, "t2");
      expect(opened).toEqual([
        [url, "_blank", "noopener"],
        [url, "_blank", "noopener"],
        [url, "_blank", "noopener"],
        [url, "_blank", "noopener"],
      ]);
    });
    expect(test.host.window.get(SELECTED_THREAD)).toBe("t1");
  });

  // spec: UI-045
  test("a new window's address is this page's with the thread as its only fragment", () => {
    expect(threadWindowUrl("https://host:7420/app/?x=1#local=1", "01 A&B")).toBe(
      "https://host:7420/app/?x=1#thread=01%20A%26B",
    );
  });

  // spec: UI-046
  test("a bare id or link shows the thread's current title and state mark; written text stays", () => {
    const test = reply("[t2](omfp://thread/t2), <omfp://thread/t2> and [the fix](omfp://thread/t2).");
    const links = screen.getAllByTestId("thread-link");
    expect(links.map((link) => link.textContent)).toEqual(["Fix the parser", "Fix the parser", "the fix"]);
    expect(links.map((link) => link.querySelector(".omfp-state-mark")?.getAttribute("data-state"))).toEqual([
      "running",
      "running",
      undefined,
    ]);
    act(() => {
      test.client.indexEvents({
        kind: "thread_upserted",
        thread: thread("t2", "Parser fixed", "w1", { needsAttention: true }),
      });
    });
    test.flush();
    const renamed = screen.getAllByTestId("thread-link");
    expect(renamed.map((link) => link.textContent)).toEqual(["Parser fixed", "Parser fixed", "the fix"]);
    expect(renamed[0]?.querySelector(".omfp-state-mark")?.getAttribute("data-state")).toBe("awaiting");
    expect(renamed[0]?.dataset.status).toBe("awaiting");
  });

  // spec: UI-046
  test("a link to a thread the index does not have, or no longer has, is plain text", () => {
    const test = reply("[gone](omfp://thread/t9) and [the fix](omfp://thread/t2).");
    expect(screen.getAllByTestId("thread-link").map((link) => link.textContent)).toEqual(["the fix"]);
    expect(screen.getAllByTestId("thread-link-text").map((text) => text.textContent)).toEqual(["gone"]);
    act(() => {
      test.client.indexEvents({ kind: "thread_removed", threadId: "t2" });
    });
    test.flush();
    expect(screen.queryByTestId("thread-link")).toBeNull();
    expect(screen.getAllByTestId("thread-link-text").map((text) => text.textContent)).toEqual(["gone", "the fix"]);
    expect(screen.getByTestId("markdown").querySelector("a")).toBeNull();
  });
});
