/** L4: `omfp:transcript` over recorded fixtures. */

import { afterEach, beforeEach, describe, expect, setSystemTime, test } from "bun:test";
import type { JsonObject, ThreadEvent } from "@omfp/core";
import { act, cleanup, fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SELECTED_THREAD, scopesAt } from "../../../src/sdk/index.ts";
import { TranscriptWidget } from "../../../src/widgets/chat/index.ts";
import { renderWithHost } from "../../support/host.tsx";
import { fixtureEvents, isFrame } from "./fixtures.ts";
import { renderThread, THREAD } from "./harness.tsx";
import { fakeLayout, LIST_TOP, pagedLayout } from "./layout.ts";

let restoreLayout: () => void = () => {};
beforeEach(() => {
  restoreLayout = fakeLayout();
});
afterEach(() => {
  cleanup();
  restoreLayout();
});

const transcript = () => screen.getByTestId("transcript");

/** A history of `count` user messages. */
function longHistory(count: number): ThreadEvent[] {
  return [
    {
      kind: "history",
      session: { session_id: "s", resumed: true },
      messages: Array.from({ length: count }, (_, index) => ({ role: "user" as const, content: `message ${index}` })),
    },
  ];
}

/** Sets the scroll metrics happy-dom does not compute and fires `scroll`. */
function scrollTo(element: HTMLElement, scrollTop: number, scrollHeight: number, clientHeight = 600): void {
  Object.defineProperty(element, "scrollHeight", { configurable: true, value: scrollHeight });
  Object.defineProperty(element, "clientHeight", { configurable: true, value: clientHeight });
  element.scrollTop = scrollTop;
  fireEvent.scroll(element);
}

describe("transcript", () => {
  // spec: UI-095, UI-730
  test("search stays in this conversation and visits matching messages", () => {
    const test = renderThread(<TranscriptWidget />, longHistory(5));
    act(() => {
      test.host.commands.run("nav.search");
    });
    const input = screen.getByLabelText("Search conversation");
    fireEvent.change(input, { target: { value: "message" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(screen.getByTestId("conversation-search").textContent).toContain("2 / 5");
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(screen.getByTestId("conversation-search").textContent).toContain("1 / 5");
    fireEvent.change(input, { target: { value: "message 2" } });
    expect(screen.getByTestId("conversation-search").textContent).toContain("1 / 1");
    expect(transcript().dataset.pinned).toBe("false");
    fireEvent.keyDown(input, { key: "Escape" });
    expect(document.activeElement?.getAttribute("data-index")).toBe("2");
    act(() => {
      test.host.commands.run("nav.search");
    });
    fireEvent.change(screen.getByLabelText("Search conversation"), { target: { value: "not present" } });
    expect(screen.getByTestId("conversation-search").textContent).toContain("0 / 0");
    fireEvent.keyDown(screen.getByLabelText("Search conversation"), { key: "Escape" });
    expect(screen.queryByTestId("conversation-search")).toBeNull();
  });

  // spec: UI-040, UI-094
  test("a text reply renders the user's text and the assistant's Markdown", () => {
    renderThread(<TranscriptWidget />, fixtureEvents("text_reply"));
    const messages = screen.getAllByTestId("message");
    expect(messages.map((message) => message.dataset.role)).toEqual(["user", "assistant"]);
    const [user, assistant] = messages;
    expect(user && within(user).getByTestId("plain-text").textContent).not.toBe("");
    const markdown = assistant && within(assistant).getByTestId("markdown");
    expect(markdown?.classList.contains("omfp-prose")).toBe(true);
    expect(markdown?.textContent).not.toBe("");
  });

  // spec: UI-040, UI-042, UI-147
  test("streaming deltas update the reply in place while it is marked streaming", () => {
    const events = fixtureEvents("text_reply");
    const start = events.findIndex(isFrame("message_update", "text_delta"));
    const { client, flush } = renderThread(<TranscriptWidget />, events.slice(0, start + 1));
    const streaming = () => screen.getAllByTestId("message").find((message) => message.dataset.streaming === "true");
    const first = streaming()?.textContent ?? "";
    expect(streaming()?.getAttribute("aria-busy")).toBe("true");
    const end = events.findIndex(isFrame("message_update", "text_end"));
    client.threadEvents(THREAD, ...events.slice(start + 1, end));
    flush();
    expect((streaming()?.textContent ?? "").length).toBeGreaterThan(first.length);
    client.threadEvents(THREAD, ...events.slice(end));
    flush();
    expect(streaming()).toBeUndefined();
  });

  // spec: UI-147, UI-191
  test("a streaming reply's Markdown is rendered in chunks that stay in place as it grows; the finished one whole", () => {
    const update = (assistantMessageEvent: JsonObject): ThreadEvent => ({
      kind: "omp",
      frame: { type: "message_update", messageId: "live", message: { role: "assistant" }, assistantMessageEvent },
    });
    const { client, flush } = renderThread(<TranscriptWidget />, [
      { kind: "omp", frame: { type: "message_start", messageId: "live", message: { role: "assistant", content: [] } } },
      update({ type: "text_start", contentIndex: 0, block: { type: "text", text: "" } }),
      update({ type: "text_delta", contentIndex: 0, delta: "First paragraph.\n\nSecond" }),
    ]);
    const chunks = () => [...screen.getByTestId("markdown").querySelectorAll(":scope > .omfp-markdown")];
    const [first, second] = chunks();
    expect([first?.textContent, second?.textContent]).toEqual(["First paragraph.", "Second"]);
    // The streaming text is the reply's last block: the caret trails its last character.
    expect(screen.getByTestId("markdown").classList.contains("omfp-caret-trail")).toBe(true);
    client.threadEvents(THREAD, update({ type: "text_delta", contentIndex: 0, delta: " half." }));
    flush();
    expect(chunks()[0]).toBe(first);
    expect(chunks()[1]?.textContent).toBe("Second half.");
    const text = "First paragraph.\n\nSecond half.";
    client.threadEvents(THREAD, update({ type: "text_end", contentIndex: 0, content: text, block: { type: "text" } }), {
      kind: "omp",
      frame: {
        type: "message_end",
        messageId: "live",
        message: { role: "assistant", content: [{ type: "text", text }] },
      },
    });
    flush();
    const markdown = screen.getByTestId("markdown");
    expect([markdown.classList.contains("omfp-markdown"), markdown.querySelectorAll("p").length]).toEqual([true, 2]);
  });

  // spec: UI-041, UI-171, UI-694
  test("settled thinking is collapsed until expanded", async () => {
    const thinking: ThreadEvent = {
      kind: "history",
      session: { session_id: "s", resumed: true },
      messages: [{ role: "assistant", content: [{ type: "thinking", thinking: "secret plan" }] }],
    };
    renderThread(<TranscriptWidget />, [thinking]);
    const block = screen.getByTestId("thinking");
    expect(block.dataset.expanded).toBe("false");
    expect(within(block).queryByText("secret plan")).toBeNull();
    const head = within(block).getByTestId("thinking-head");
    expect(head.textContent).toBe("▸Thought");
    await userEvent.click(head);
    expect(block.dataset.expanded).toBe("true");
    expect(head.textContent).toBe("▾Thought");
    expect(within(block).getByText("secret plan").closest("[hidden]")).toBeNull();
  });

  // spec: UI-694
  test("live thinking has a distinct state and an explicit expansion survives settling", async () => {
    const update = (assistantMessageEvent: JsonObject): ThreadEvent => ({
      kind: "omp",
      frame: { type: "message_update", messageId: "live", message: { role: "assistant" }, assistantMessageEvent },
    });
    const { client, flush } = renderThread(<TranscriptWidget />, [
      { kind: "omp", frame: { type: "message_start", messageId: "live", message: { role: "assistant", content: [] } } },
      update({ type: "thinking_start", contentIndex: 0, block: { type: "thinking", thinking: "" } }),
      update({ type: "thinking_delta", contentIndex: 0, delta: "secret plan" }),
    ]);
    const live = screen.getByTestId("thinking");
    expect([live.dataset.live, live.dataset.expanded]).toEqual(["true", "false"]);
    const liveHead = within(live).getByTestId("thinking-head");
    expect(liveHead.textContent).toBe("▸Thinking…");
    await userEvent.click(liveHead);
    client.threadEvents(
      THREAD,
      update({
        type: "thinking_end",
        contentIndex: 0,
        content: "secret plan",
        block: { type: "thinking", thinking: "secret plan" },
      }),
      {
        kind: "omp",
        frame: {
          type: "message_end",
          messageId: "live",
          message: { role: "assistant", content: [{ type: "thinking", thinking: "secret plan" }] },
        },
      },
    );
    flush();
    const settled = screen.getByTestId("thinking");
    expect([settled.dataset.live, settled.dataset.expanded]).toEqual([undefined, "true"]);
    expect(within(settled).getByTestId("thinking-head").textContent).toBe("▾Thought");
  });

  // spec: UI-050
  test("a tool without a specialized card renders through the fallback card, arguments and result on demand", async () => {
    const call: ThreadEvent = {
      kind: "history",
      session: { session_id: "s", resumed: true },
      messages: [
        { role: "assistant", content: [{ type: "toolCall", id: "c1", name: "eval", arguments: { code: "1 + 1" } }] },
        { role: "toolResult", toolCallId: "c1", toolName: "eval", content: [{ type: "text", text: "2" }] },
      ],
    };
    renderThread(<TranscriptWidget />, [call]);
    const card = screen.getByTestId("tool-card");
    expect([card.dataset.tool, card.dataset.view, card.dataset.status]).toEqual(["eval", "generic", "succeeded"]);
    expect(within(card).getByTestId("tool-summary").textContent).toBe("code: 1 + 1");
    expect(within(card).queryByTestId("tool-result")).toBeNull();
    await userEvent.click(within(card).getByTestId("toggle"));
    expect(within(card).getByTestId("tool-args").textContent).toContain('"code"');
    expect(within(card).getByTestId("tool-result").textContent).toBe("2");
  });

  // spec: UI-050, UI-052
  test("a failed tool is distinct and expanded", () => {
    const failed: ThreadEvent = {
      kind: "history",
      session: { session_id: "s", resumed: true },
      messages: [
        { role: "assistant", content: [{ type: "toolCall", id: "c1", name: "read", arguments: { path: "x" } }] },
        { role: "toolResult", toolCallId: "c1", isError: true, content: [{ type: "text", text: "ENOENT" }] },
      ],
    };
    renderThread(<TranscriptWidget />, [failed]);
    const card = screen.getByTestId("tool-card");
    expect(card.dataset.status).toBe("failed");
    expect(card.querySelector(".omfp-tool")?.getAttribute("data-status")).toBe("failed");
    expect(within(card).getByTestId("tool-error").textContent).toBe("ENOENT");
  });

  // spec: UI-470, UI-471
  test("a turn of several tool calls shows one Assistant name line; a steer starts the next turn", () => {
    const call = (id: string) => ({
      role: "assistant" as const,
      content: [
        { type: "thinking" as const, thinking: "next step" },
        { type: "toolCall" as const, id, name: "eval", arguments: { code: id } },
      ],
    });
    const result = (id: string) => ({
      role: "toolResult" as const,
      toolCallId: id,
      toolName: "eval",
      content: [{ type: "text" as const, text: "ok" }],
    });
    renderThread(<TranscriptWidget />, [
      {
        kind: "history",
        session: { session_id: "s", resumed: true },
        messages: [
          { role: "user", content: "plot it" },
          ...["c1", "c2", "c3", "c4"].flatMap((id) => [call(id), result(id)]),
          { role: "assistant", content: [{ type: "text", text: "Plotted." }] },
          { role: "user", steering: true, content: "bigger" },
          { role: "assistant", content: [{ type: "text", text: "Bigger." }] },
        ],
      },
    ]);
    const assistant = screen.getAllByTestId("message").filter((message) => message.dataset.role === "assistant");
    expect(assistant.map((message) => message.dataset.continues)).toEqual([
      "false",
      "true",
      "true",
      "true",
      "true",
      "false",
    ]);
    // One name line, in one article, per turn: the turn's later messages follow under it.
    expect(assistant.map((message) => message.querySelectorAll(".omfp-msg__name").length)).toEqual([1, 0, 0, 0, 0, 1]);
    expect(assistant.map((message) => message.tagName)).toEqual(["ARTICLE", "DIV", "DIV", "DIV", "DIV", "ARTICLE"]);
    expect(screen.getAllByTestId("thinking").length).toBe(4);
    expect(screen.getAllByTestId("tool-card").length).toBe(4);
  });

  // spec: UI-043
  test("notices are inline rows; unknown kinds expand to their JSON", async () => {
    const events: ThreadEvent[] = [
      ...fixtureEvents("text_reply"),
      { kind: "unknown", raw: { type: "irc_card", from: "advisor" } },
    ];
    renderThread(<TranscriptWidget />, events);
    const notice = screen.getAllByTestId("notice").find((row) => row.dataset.kind === "unknown");
    expect(notice).toBeDefined();
    if (notice !== undefined) {
      const json = within(notice).getByTestId("notice-json");
      expect(json.closest("[hidden]")).not.toBeNull();
      await userEvent.click(within(notice).getByTestId("toggle"));
      expect(json.closest("[hidden]")).toBeNull();
      expect(json.textContent).toContain('"irc_card"');
    }
  });

  // spec: UI-345, UI-347
  test("a command's output is a monospace card; a URL omp opened has Open, Copy and the loopback note", async () => {
    const url = "https://auth.example.com/authorize?state=s1";
    const events: ThreadEvent[] = [
      ...fixtureEvents("text_reply"),
      { kind: "omp", frame: { type: "command_output", text: "Workspace roots:\n  /w/app" } },
      {
        kind: "omp",
        frame: {
          type: "command_output",
          text: `━━━ OAuth Authorization Required ━━━\nOpening your browser to authorize. If it did not open:\n${url}`,
        },
      },
    ];
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
      renderThread(<TranscriptWidget />, events);
      const cards = screen.getAllByTestId("command-output");
      expect(cards.map((card) => [card.dataset.kind, within(card).getByTestId("command-output-text").tagName])).toEqual(
        [
          ["command", "PRE"],
          ["command", "PRE"],
        ],
      );
      const [dirs, oauth] = cards;
      expect(dirs && within(dirs).getByTestId("command-output-text").textContent).toBe("Workspace roots:\n  /w/app");
      expect(dirs && within(dirs).queryByTestId("opened-url")).toBeNull();
      expect(dirs && within(dirs).queryByTestId("loopback-note")).toBeNull();
      if (oauth === undefined) {
        throw new Error("no OAuth card");
      }
      expect(
        within(oauth)
          .getAllByTestId("opened-url")
          .map((row) => row.dataset.url),
      ).toEqual([url]);
      expect(within(oauth).getByTestId("loopback-note").textContent).toContain("loopback address");
      await userEvent.click(within(oauth).getByRole("button", { name: `Open ${url} in this browser` }));
      expect(opened).toEqual([[url, "_blank", "noopener"]]);
      await userEvent.click(within(oauth).getByRole("button", { name: `Copy ${url}` }));
      expect(copied).toEqual([url]);
    } finally {
      window.open = open;
      Reflect.deleteProperty(navigator, "clipboard");
    }
  });

  // spec: UI-043
  test("omp's startup records are collapsed system rows; a record with nothing to summarize has no disclosure", async () => {
    const startup: ThreadEvent[] = [
      {
        kind: "history",
        session: { session_id: "s", resumed: true },
        messages: [
          {
            role: "custom",
            customType: "xdev-mount-notice",
            content:
              "<system-notice>\nxd:// device inventory changed.\n- xd://ast_edit — Structural edits\n</system-notice>",
            display: false,
            timestamp: 1,
          },
          { role: "custom", customType: "marker", content: "", display: true, timestamp: 2 },
        ],
      },
      { kind: "omp", frame: { type: "notice", level: "info", message: "xd://: mounted ast_edit", source: "xdev" } },
    ];
    const { client, flush } = renderThread(<TranscriptWidget />, startup);
    expect(screen.getAllByTestId("system-summary").map((row) => row.textContent)).toEqual([
      "xd:// device inventory changed.",
    ]);
    expect(screen.queryAllByTestId("notice").length).toBe(0);
    client.indexEvents({
      kind: "settings_changed",
      settings: { ui: { showXdevNotices: true }, keymap: [], error: null },
    });
    flush();
    const rows = screen.getAllByTestId("system-record");
    expect(rows.map((row) => within(row).getByTestId("system-summary").textContent)).toEqual([
      "xd:// device inventory changed.",
      "xd://: mounted ast_edit",
    ]);
    const [inventory, mounted] = rows;
    if (inventory === undefined || mounted === undefined) {
      throw new Error("two system rows expected");
    }
    expect(screen.queryAllByTestId("notice").length).toBe(0);
    const full = within(inventory).getByTestId("system-text");
    expect(full.closest("[hidden]")).not.toBeNull();
    await userEvent.click(within(inventory).getByTestId("toggle"));
    expect(full.closest("[hidden]")).toBeNull();
    expect(full.textContent).toBe("xd:// device inventory changed.\n- xd://ast_edit — Structural edits");
    expect(within(mounted).queryByTestId("toggle")).toBeNull();
    const [record, ...others] = screen.getAllByTestId("message");
    expect([record?.dataset.role, others.length]).toEqual(["record", 0]);
    expect(record && within(record).queryByTestId("toggle")).toBeNull();
    client.indexEvents({
      kind: "settings_changed",
      settings: { ui: { showXdevNotices: false }, keymap: [], error: null },
    });
    flush();
    expect(screen.getAllByTestId("system-summary").map((row) => row.textContent)).toEqual([
      "xd:// device inventory changed.",
    ]);
    expect(screen.queryAllByTestId("notice").length).toBe(0);
  });

  // spec: UI-042
  test("a long transcript keeps only the items around the viewport in the DOM", () => {
    renderThread(<TranscriptWidget />, longHistory(500));
    const rendered = screen.getAllByTestId("transcript-item").length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(30);
  });

  // spec: UI-042
  test("only the user unpins: wheel up, scroll-up keys and moving the view up, not layout scrolls; the bottom pins again", () => {
    renderThread(<TranscriptWidget />, longHistory(50));
    const element = transcript();
    expect(element.dataset.pinned).toBe("true");
    scrollTo(element, 1000, 5000);
    expect(element.dataset.pinned).toBe("true");
    fireEvent.wheel(element, { deltaY: -100 });
    expect(element.dataset.pinned).toBe("false");
    scrollTo(element, 4400, 5000);
    expect(element.dataset.pinned).toBe("true");
    fireEvent.keyDown(element, { key: "ArrowUp" });
    expect(element.dataset.pinned).toBe("false");
    scrollTo(element, 4400, 5000);
    expect(element.dataset.pinned).toBe("true");
    // A move up with no input event (WebKitGTK's scrollbar), however small: unpinned.
    scrollTo(element, 4390, 5000);
    expect(element.dataset.pinned).toBe("false");
  });

  // spec: UI-042
  test("a move up its scroll event has not reported yet unpins at the next render instead of being followed", () => {
    const { client, flush } = renderThread(<TranscriptWidget />, longHistory(50));
    const element = transcript();
    scrollTo(element, 4400, 5000);
    expect(element.dataset.pinned).toBe("true");
    // WebKitGTK moved the view 5 px up from the end (4400: a smooth scroll's first frame) and sends the scroll event
    // at the next frame; a streamed delta renders first.
    element.scrollTop = 4395;
    client.thread(THREAD, longHistory(51));
    flush();
    expect([element.scrollTop, element.dataset.pinned]).toEqual([4395, "false"]);
    fireEvent.scroll(element);
    expect([element.scrollTop, element.dataset.pinned]).toEqual([4395, "false"]);
  });

  // spec: UI-042
  test("switching threads and back returns to where the user had scrolled up to", () => {
    const { client, flush, host } = renderThread(<TranscriptWidget />, longHistory(50));
    fireEvent.wheel(transcript(), { deltaY: -100 });
    scrollTo(transcript(), 400, 5000);
    act(() => host.window.set(SELECTED_THREAD, "02T"));
    client.thread("02T", longHistory(3));
    flush();
    expect(transcript().dataset.threadId).toBe("02T");
    act(() => host.window.set(SELECTED_THREAD, THREAD));
    client.thread(THREAD, longHistory(50));
    flush();
    expect(transcript().scrollTop).toBe(400);
    expect(transcript().dataset.pinned).toBe("false");
  });

  // spec: UI-042, UI-095
  test("the transcript scrolls itself on PageUp/PageDown/Home/End", () => {
    renderThread(<TranscriptWidget />, longHistory(50));
    const element = transcript();
    scrollTo(element, 4400, 5000);
    // happy-dom does not clamp: pinned, the transcript set it to the full height.
    const bottom = element.scrollTop;
    fireEvent.keyDown(element, { key: "PageUp" });
    expect(element.scrollTop).toBe(bottom - 568);
    expect(element.dataset.pinned).toBe("false");
    fireEvent.keyDown(element, { key: "PageDown" });
    expect(element.scrollTop).toBe(4400);
    // The browser reports the scroll; back at the bottom, the view is pinned again.
    fireEvent.scroll(element);
    expect(element.dataset.pinned).toBe("true");
    fireEvent.keyDown(element, { key: "Home" });
    expect(element.scrollTop).toBe(0);
    expect(element.dataset.pinned).toBe("false");
    fireEvent.keyDown(element, { key: "End" });
    expect(element.scrollTop).toBeGreaterThanOrEqual(4400);
    expect(element.dataset.pinned).toBe("true");
  });

  // spec: UI-095
  test("transcript-scope nav commands move between messages", () => {
    const { host } = renderThread(<TranscriptWidget />, longHistory(5));
    const element = transcript();
    act(() => element.focus());
    const scopes = scopesAt(document.activeElement);
    expect(scopes).toEqual(["transcript"]);
    const run = (id: string) => act(() => void host.commands.run(id, [], scopesAt(document.activeElement)));
    const current = () => document.activeElement?.getAttribute("data-index");
    run("nav.prev");
    expect(current()).toBe("4");
    run("nav.prev");
    expect(current()).toBe("3");
    expect(element.dataset.pinned).toBe("false");
    run("nav.top");
    expect(current()).toBe("0");
    run("nav.next");
    expect(current()).toBe("1");
    run("nav.bottom");
    expect(current()).toBe("4");
    expect(element.dataset.pinned).toBe("true");
  });

  // spec: UI-750, UI-370
  test("day separators label the days of an old thread inside their messages' items", () => {
    // Monday 16 November 2026, 09:00 local time.
    setSystemTime(new Date(2026, 10, 16, 9));
    try {
      const at = (month: number, day: number, hour: number, year = 2026) =>
        new Date(year, month - 1, day, hour).getTime();
      const { host } = renderThread(<TranscriptWidget />, [
        {
          kind: "history",
          session: { session_id: "s", resumed: true },
          messages: [
            { role: "user", content: "last year", timestamp: at(11, 12, 10, 2025) },
            { role: "user", content: "no time" },
            { role: "user", content: "yesterday morning", timestamp: at(11, 15, 8) },
            { role: "user", content: "yesterday night", timestamp: at(11, 15, 23) },
            { role: "user", content: "today", timestamp: at(11, 16, 8) },
          ],
        },
      ]);
      const separators = screen.getAllByRole("separator");
      expect(separators.map((separator) => separator.textContent)).toEqual([
        new Intl.DateTimeFormat(undefined, {
          weekday: "short",
          day: "numeric",
          month: "short",
          year: "numeric",
        }).format(new Date(2025, 10, 12)),
        "Yesterday",
        "Today",
      ]);
      expect(separators.map((separator) => separator.getAttribute("aria-label"))).toEqual(
        separators.map((separator) => separator.textContent),
      );
      expect(separators.map((separator) => separator.querySelector("time")?.getAttribute("datetime"))).toEqual([
        "2025-11-12",
        "2026-11-15",
        "2026-11-16",
      ]);
      expect(separators.some((separator) => separator.hasAttribute("tabindex"))).toBe(false);
      // Each separator sits in its message's item: the items are the five messages, `j`/`k` step through them.
      const rows = screen.getAllByTestId("transcript-item");
      expect(rows.length).toBe(5);
      expect(rows.map((row) => row.querySelector('[role="separator"]') !== null)).toEqual([
        true,
        false,
        true,
        false,
        true,
      ]);
      act(() => transcript().focus());
      const run = (id: string) => act(() => void host.commands.run(id, [], scopesAt(document.activeElement)));
      run("nav.prev");
      expect(document.activeElement?.getAttribute("data-index")).toBe("4");
      run("nav.prev");
      expect(document.activeElement?.getAttribute("data-index")).toBe("3");
    } finally {
      setSystemTime();
    }
  });

  // spec: UI-750
  test("a thread written today shows no day separator", () => {
    const now = Date.now();
    renderThread(<TranscriptWidget />, [
      {
        kind: "history",
        session: { session_id: "s", resumed: true },
        messages: [
          { role: "user", content: "hello", timestamp: now },
          { role: "assistant", content: [{ type: "text", text: "hi" }], timestamp: now },
        ],
      },
    ]);
    expect(screen.queryAllByRole("separator").length).toBe(0);
  });

  /** The transcript with `count` messages laid out by {@link pagedLayout}, focused, and its commands and keys. */
  function paged(count: number, heights: (index: number) => number) {
    const restore = pagedLayout(heights);
    const { host } = renderThread(<TranscriptWidget />, longHistory(count));
    const element = transcript();
    act(() => element.focus());
    // Each command, then the scroll event the browser sends after it moved the view.
    const run = (id: string) => {
      act(() => void host.commands.run(id, [], scopesAt(document.activeElement)));
      act(() => void fireEvent.scroll(element));
    };
    const press = (key: string) => {
      fireEvent.keyDown(document.activeElement ?? element, { key });
      act(() => void fireEvent.scroll(element));
    };
    const state = () => [document.activeElement?.getAttribute("data-index"), element.scrollTop];
    return { element, run, press, state, restore };
  }

  // spec: UI-370
  test("j/k page through a message taller than the viewport before moving on; Up shows the previous one's end", () => {
    // A 600 px viewport; message 2 is 1500 px tall, the others 100.
    const { element, run, state, restore } = paged(5, (index) => (index === 2 ? 1500 : 100));
    try {
      run("nav.top");
      run("nav.next");
      expect(state()).toEqual(["1", 0]);
      // The long message's top is in view: focused without scrolling.
      run("nav.next");
      expect(state()).toEqual(["2", 0]);
      // Steps of 40 % of the viewport, the focus kept; the last step stops 16 px past its end.
      const steps = [];
      for (let press = 0; press < 5; press++) {
        run("nav.next");
        steps.push(state());
      }
      const end = LIST_TOP + 1700 - 600 + 16;
      expect(steps).toEqual([
        ["2", 240],
        ["2", 480],
        ["2", 720],
        ["2", 960],
        ["2", end],
      ]);
      // Its end shows: the next press moves on, as for any message that fits.
      run("nav.next");
      expect(state()).toEqual(["3", 1200]);
      expect(element.dataset.pinned).toBe("false");
      // Up: message 3 shows whole, so the focus moves up at once, to the long message whose end shows.
      run("nav.prev");
      expect(state()).toEqual(["2", 1200]);
      run("nav.prev");
      expect(state()).toEqual(["2", 960]);
      // Paging up until its top shows 16 px below the viewport's top, then on to message 1, scrolled into view.
      const up = [];
      for (let press = 0; press < 4; press++) {
        run("nav.prev");
        up.push(state()[1]);
      }
      expect(up).toEqual([720, 480, 240, LIST_TOP + 200 - 16]);
      run("nav.prev");
      expect(state()).toEqual(["1", 100]);
    } finally {
      restore();
    }
  });

  // spec: UI-370
  test("moving up from a message whose top is at the viewport's top shows the previous long message's end", () => {
    const { run, state, restore } = paged(10, (index) => (index === 1 ? 1500 : 100));
    try {
      run("nav.top");
      run("nav.next");
      // Page down through message 1 to its end and move on to message 2, then put its top at the viewport's top.
      for (let press = 0; press < 6; press++) {
        run("nav.next");
      }
      expect(state()[0]).toBe("2");
      const top = LIST_TOP + 1600;
      act(() => {
        transcript().scrollTop = top;
        fireEvent.scroll(transcript());
      });
      // Message 1 ends at the viewport's top: Up scrolls it up to show its last 600 px.
      run("nav.prev");
      expect(state()).toEqual(["1", top - 600]);
    } finally {
      restore();
    }
  });

  // spec: UI-370
  test("messages that fit move one by one as before", () => {
    const { run, state, restore } = paged(20, () => 100);
    try {
      run("nav.top");
      expect(state()).toEqual(["0", 0]);
      for (let press = 0; press < 6; press++) {
        run("nav.next");
      }
      // Message 6 ends below the viewport: scrolled into view at the bottom, as `scrollToIndex` aligns it.
      expect(state()).toEqual(["6", 100]);
      run("nav.prev");
      expect(state()).toEqual(["5", 100]);
    } finally {
      restore();
    }
  });

  // spec: UI-371
  test("Ctrl-d/Ctrl-u and PageDown/PageUp scroll and keep the focus until its message leaves the view", () => {
    const { element, run, press, state, restore } = paged(20, () => 100);
    try {
      run("nav.top");
      // Half a page down: message 0 left the view, so the focus goes to the message at its top.
      run("nav.halfDown");
      expect(state()).toEqual(["2", 300]);
      run("nav.halfDown");
      expect(state()).toEqual(["5", 600]);
      // Up again: message 5 still shows, the focus stays.
      run("nav.halfUp");
      expect(state()).toEqual(["5", 300]);
      expect(element.dataset.pinned).toBe("false");
      press("PageUp");
      expect(state()).toEqual(["5", 0]);
      press("PageDown");
      expect(state()).toEqual(["5", 568]);
      press("PageDown");
      expect(state()).toEqual(["11", 1136]);
    } finally {
      restore();
    }
  });

  // spec: UI-092
  test("the agent starting and finishing is announced", () => {
    const events = fixtureEvents("text_reply");
    const busyAt = events.findIndex(isFrame("agent_start"));
    const { client, flush, host } = renderThread(<TranscriptWidget />, events.slice(0, busyAt));
    client.threadEvents(THREAD, ...events.slice(busyAt, busyAt + 1));
    flush();
    expect(host.announcer.getSnapshot().message).toBe("Agent working");
    client.threadEvents(THREAD, ...events.slice(busyAt + 1));
    flush();
    expect(host.announcer.getSnapshot().message).toBe("Agent finished");
  });

  test("placeholders while no thread is selected or its snapshot is missing", () => {
    const { host } = renderWithHost(<TranscriptWidget />);
    expect(screen.getByTestId("no-thread")).toBeDefined();
    act(() => host.window.set(SELECTED_THREAD, THREAD));
    expect(screen.getByTestId("thread-loading").dataset.threadId).toBe(THREAD);
  });

  // spec: UI-335
  test("switching back to a recently shown thread renders its transcript at once, without a new subscribe", () => {
    const { client, flush, host } = renderThread(<TranscriptWidget />, longHistory(3));
    act(() => host.window.set(SELECTED_THREAD, "01U"));
    client.thread("01U", longHistory(1));
    flush();
    expect(screen.getAllByTestId("message").length).toBe(1);

    act(() => host.window.set(SELECTED_THREAD, THREAD));
    expect(transcript().dataset.threadId).toBe(THREAD);
    expect(screen.getAllByTestId("message").length).toBe(3);
    expect(client.subscribed.filter((topic) => topic === `thread/${THREAD}`).length).toBe(1);
  });
});
