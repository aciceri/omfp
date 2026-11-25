/**
 * L4: the specialized tool cards (UI-050..052) rendered from fixtures recorded from the pinned omp (UI-051), each in
 * its streaming, success and error states; code in them is highlighted (UI-090).
 */

import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { type JsonObject, replay, type ToolRun } from "@omfp/core";
import type { DiffLine } from "@omfp/kit";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { type ReactNode, useState } from "react";
import { findRule, parseCss } from "../../../../kit/test/support/css.ts";
import { type OmfpHost, OPEN_FILES, SELECTED_THREAD, strings } from "../../../src/sdk/index.ts";
import { type CardStatus, type ToolCard, toolCard } from "../../../src/vm/tool.ts";
import type { MessageView } from "../../../src/vm/transcript.ts";
import { MessageRow, ToolCardView } from "../../../src/widgets/chat/message.tsx";
import { type Clock, ClockContext } from "../../../src/widgets/chat/state.ts";
import { card, cards, composingCard, lspCard, streaming } from "../../support/cards.ts";
import { repo, thread, workspace } from "../../support/fixtures.ts";
import { renderWithHost, type TestHost, testHost } from "../../support/host.tsx";
import { fixtureEvents, isFrame } from "./fixtures.ts";
import { ManualClock } from "./harness.tsx";

const text = strings.chat.tool;

afterEach(cleanup);

/** The host of the last {@link show}. */
let shownHost: OmfpHost | undefined;

/** A test host showing thread `t1` of workspace `w1` (folder `/work/w1`, whose repository `src` holds the fixtures). */
function threadHost(): TestHost {
  const test = testHost();
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1", [{ ...repo("w1", "src", "main"), kind: "clone" }])],
    threads: [thread("t1", "cards", "w1")],
  });
  test.host.window.set(SELECTED_THREAD, "t1");
  return test;
}

/** Renders `toolCard` in thread `t1` of {@link threadHost}. */
function show(toolCard: ToolCard, clock?: Clock): HTMLElement {
  const test = threadHost();
  shownHost = test.host;
  const card = <ToolCardView threadId="t1" cardKey={toolCard.toolCallId} card={toolCard} />;
  renderWithHost(clock === undefined ? card : <ClockContext value={clock}>{card}</ClockContext>, test);
  test.flush();
  return screen.getByTestId("tool-card");
}

/** Renders the card and opens it. */
async function open(toolCard: ToolCard): Promise<HTMLElement> {
  const element = show(toolCard);
  if (element.querySelector("[aria-expanded=false]") !== null) {
    await userEvent.click(within(element).getByTestId("toggle"));
  }
  return element;
}

const byTestId = (element: HTMLElement, id: string) => within(element).getByTestId(id);
const texts = (element: HTMLElement, id: string) =>
  within(element)
    .queryAllByTestId(id)
    .map((node) => node.textContent);

/** A live edit settling in place, so the disclosure's explicit state can be checked across the transition. */
function EditLifecycleCard() {
  const [current, setCurrent] = useState(() => streaming("edit_tool_calls", "call_edit_1"));
  return (
    <>
      <button
        type="button"
        data-testid="settle-card"
        onClick={() => setCurrent(card("edit_tool_calls", "call_edit_1"))}
      >
        settle
      </button>
      <ToolCardView threadId="t1" cardKey={current.toolCallId} card={current} />
    </>
  );
}

/** The card of the async `bash` fixture's call after its first `count` events. */
function jobCard(count: number): ToolCard {
  const [found] = cards(replay(fixtureEvents("async_bash_tool_call").slice(0, count)));
  if (found === undefined) {
    throw new Error("no card");
  }
  return found;
}

describe("background bash card", () => {
  const events = fixtureEvents("async_bash_tool_call");
  const ended = events.findIndex(isFrame("tool_execution_end")) + 1;
  /** Up to the job's completed frame: its last running frame has output. */
  const output = events.findLastIndex(isFrame("tool_execution_update"));

  // spec: UI-270
  test("while the job runs: running, marked background, its last output line, no exit code", async () => {
    const started = show(jobCard(ended));
    expect(started.dataset.status).toBe("running");
    expect(byTestId(started, "tool-background").dataset.state).toBe("running");
    expect(byTestId(started, "tool-background").textContent).toBe("background");
    expect(within(started).queryByTestId("tool-last-line")).toBeNull();
    cleanup();
    const element = await open(jobCard(output));
    expect(element.dataset.status).toBe("running");
    expect(byTestId(element, "tool-last-line").textContent).toBe("bg-output-42");
    expect(byTestId(element, "tool-output").textContent).toBe("bg-output-42\n");
    expect(within(element).queryByTestId("tool-exit")).toBeNull();
  });

  // spec: UI-270
  test("once the job completed: its final output and exit 0", async () => {
    const element = await open(card("async_bash_tool_call", "call_bash_bg"));
    expect(element.dataset.status).toBe("succeeded");
    expect(byTestId(element, "tool-background").textContent).toBe("background · done");
    expect(byTestId(element, "tool-output").textContent).toContain("bg-output-42");
    expect(byTestId(element, "tool-exit").textContent).toBe("exit 0");
    expect(within(element).queryByTestId("tool-last-line")).toBeNull();
  });

  // spec: UI-270
  test("failed and cancelled jobs: failed cards with their state and exit code", () => {
    const base = jobCard(ended);
    const ofState = (state: "failed" | "cancelled"): ToolCard => ({
      ...base,
      status: "failed",
      view: {
        kind: "bash",
        command: "make",
        cwd: undefined,
        output: "boom",
        exitCode: 2,
        background: { jobId: "bg_1", state },
      },
    });
    const element = show(ofState("failed"));
    expect(byTestId(element, "tool-background").textContent).toBe("background · failed");
    expect(byTestId(element, "tool-exit").textContent).toBe("exit 2");
    cleanup();
    const cancelled = show(ofState("cancelled"));
    expect(byTestId(cancelled, "tool-background").textContent).toBe("background · cancelled");
  });
});

describe("bash card", () => {
  // spec: UI-050, UI-051, UI-695
  test("streaming: the command, live elapsed time and output so far, no exit code yet", async () => {
    const clock = new ManualClock(1_000);
    const element = show(streaming("bash_tool_call", "call_bash_1"), clock);
    expect([element.dataset.view, element.dataset.status]).toEqual(["bash", "running"]);
    expect(byTestId(element, "tool-subject").textContent).toBe("echo tool-output-123");
    expect(byTestId(element, "tool-duration").textContent).toBe("0 ms");
    act(() => clock.advance(1_250));
    expect(byTestId(element, "tool-duration").textContent).toBe("1.3 s");
    await userEvent.click(byTestId(element, "toggle"));
    expect(byTestId(element, "tool-output").textContent).toBe("tool-output-123\n");
    expect(within(element).queryByTestId("tool-exit")).toBeNull();
  });

  // spec: UI-050, UI-051, UI-137, UI-163
  test("success: collapsed, then the output and exit 0", async () => {
    const done = card("bash_tool_call", "call_bash_1");
    expect(within(show(done)).queryByTestId("tool-output")).toBeNull();
    cleanup();
    const element = await open(done);
    expect(byTestId(element, "tool-exit").textContent).toBe("exit 0");
    expect(byTestId(element, "tool-exit").dataset.ok).toBe("true");
  });

  // spec: UI-050, UI-051, UI-052, UI-137, UI-163
  test("error: expanded and distinct, with the exit code", () => {
    const element = show(card("bash_tool_error", "call_bash_1"));
    expect(element.dataset.status).toBe("failed");
    expect(element.querySelector(".omfp-tool")?.getAttribute("data-status")).toBe("failed");
    expect(byTestId(element, "tool-output").textContent).toStartWith("oops\n");
    expect(byTestId(element, "tool-exit").textContent).toBe("exit 3");
    expect(byTestId(element, "tool-exit").dataset.ok).toBe("false");
  });

  // spec: UI-050
  test("ANSI colors become theme classes", async () => {
    const done = card("bash_tool_call", "call_bash_1");
    const colored: ToolCard = {
      ...done,
      view: {
        kind: "bash",
        command: "ls",
        cwd: "/tmp",
        output: "\u001b[1;34mdir\u001b[0m file",
        exitCode: 0,
        background: undefined,
      },
    };
    const element = await open(colored);
    const output = byTestId(element, "tool-output");
    expect(output.textContent).toBe("dir file");
    expect(output.querySelector("span")?.className).toBe("omfp-ansi-fg-blue omfp-ansi-bold");
    expect(element.textContent).toContain("in /tmp");
  });

  // spec: UI-050, UI-163, UI-187
  test("open: the head on the border, the whole command, then the output under an `Output` rule", async () => {
    const element = await open(card("bash_tool_call", "call_bash_1"));
    expect(element.querySelector(".omfp-tool")?.classList.contains("omfp-bbox")).toBe(true);
    const body = element.querySelector(".omfp-tool__body");
    expect([...(body?.children ?? [])].map((part) => part.className)).toEqual([
      "omfp-tool-card__section",
      "omfp-bbox__rule",
      "omfp-tool-card__section",
    ]);
    expect(byTestId(element, "tool-command").textContent).toBe("echo tool-output-123");
    // The head is the card's h2, its rules h3 (UI-287).
    expect(within(within(element).getByRole("heading", { level: 2 })).getAllByRole("button").length).toBe(1);
    expect(within(element).getByRole("heading", { level: 3, name: "Output" })).toBeDefined();
    expect(body?.children[2]?.contains(byTestId(element, "tool-output"))).toBe(true);
  });
});

describe("read card", () => {
  // spec: UI-050, UI-051
  test("streaming: the path relative to its repository, the repository as a prefix", () => {
    const element = show(streaming("read_tool_calls", "call_read_1"));
    expect(element.dataset.status).toBe("running");
    expect(byTestId(element, "tool-repo").textContent).toBe("src/");
    // A muted prefix inside the card, not a chip: no box in a box.
    expect(byTestId(element, "tool-repo").classList.contains("omfp-chip")).toBe(false);
    expect(byTestId(element, "tool-path").textContent).toBe("src/lib.rs");
  });

  // spec: UI-050, UI-051, UI-090
  test("success: the result is collapsed, then the highlighted lines and their range", async () => {
    const done = card("read_tool_calls", "call_read_1");
    expect(within(show(done)).queryByTestId("tool-content")).toBeNull();
    cleanup();
    const element = await open(done);
    expect(byTestId(element, "tool-range").textContent).toBe("1–9 of 9");
    const content = byTestId(element, "tool-content");
    expect(content.textContent).toStartWith("//! Tool card fixture crate.");
    await waitFor(() => expect(content.querySelector(".hljs-keyword")?.textContent).toBe("pub"));
  });

  // spec: UI-050, UI-051, UI-052
  test("error: omp's message, expanded", () => {
    const element = show(card("read_tool_calls", "call_read_2"));
    expect(byTestId(element, "tool-error").textContent).toBe("Path 'src/missing.rs' not found");
    expect(within(element).queryByTestId("tool-content")).toBeNull();
  });
});

describe("fetch card", () => {
  // spec: UI-050, UI-051
  test("streaming and success: the URL, then the page", async () => {
    expect(byTestId(show(streaming("web_tool_calls", "call_read_1")), "tool-subject").textContent).toBe(
      "http://127.0.0.1:29417/page",
    );
    cleanup();
    const element = await open(card("web_tool_calls", "call_read_1"));
    expect(byTestId(element, "tool-link").getAttribute("href")).toBe("http://127.0.0.1:29417/page");
    expect(byTestId(element, "tool-content").textContent).toContain("# Tool cards");
  });

  // spec: UI-050, UI-051, UI-052
  test("error: a 404 is a failure with omp's note", () => {
    const element = show(card("web_tool_calls", "call_read_2"));
    expect(element.dataset.status).toBe("failed");
    expect(byTestId(element, "tool-error").textContent).toContain("Method: failed");
  });
});

describe("edit card", () => {
  // spec: UI-050, UI-051, UI-055, UI-695
  test("streaming: the file and omp's preview of the diff are open without another click", () => {
    const element = show(streaming("edit_tool_calls", "call_edit_1"));
    expect(byTestId(element, "toggle").getAttribute("aria-expanded")).toBe("true");
    expect(byTestId(element, "tool-path").textContent).toBe("src/lib.rs");
    const diff = byTestId(element, "tool-diff");
    expect([...diff.querySelectorAll("[data-kind=add]")].map((row) => row.textContent)).toEqual([
      "4+    left.saturating_add(right)",
    ]);
    expect(within(element).queryByTestId("tool-input")).toBeNull();
  });

  // spec: UI-054, UI-055, UI-056, UI-116
  test("while the model writes it, before omp's first preview: the edit as written, its path not opening yet", () => {
    const partialArgs = '{"input": "[src/lib.rs#72CA]\\nPUT 4.=4:\\n+    left.sat';
    const element = show(composingCard("edit", partialArgs, "c-edit"));
    expect([element.dataset.status, element.dataset.omfpPath]).toEqual(["composing", undefined]);
    expect(byTestId(element, "toggle").getAttribute("aria-expanded")).toBe("true");
    expect(byTestId(element, "tool-path").textContent).toBe("src/lib.rs");
    expect(within(element).queryByTestId("tool-path-open")).toBeNull();
    expect(byTestId(element, "tool-input").textContent).toBe("[src/lib.rs#72CA]\nPUT 4.=4:\n+    left.sat");
  });

  // spec: UI-055
  test("a preview's error shows as such; several files each under their path", () => {
    const single = show(streaming("edit_tool_calls", "call_edit_2"));
    expect(within(single).queryByTestId("tool-preview-error")).toBeNull();
    expect(within(single).queryByTestId("tool-error")).toBeNull();
    expect(within(single).queryByTestId("tool-preview-file")).toBeNull();
    cleanup();
    const run: ToolRun = {
      tool_call_id: "c-files",
      tool_name: "edit",
      args: { input: "[src/a.rs#1A2B]\nPUT 1.=1:\n+new\n" },
      intent: null,
      status: "running",
      partial_result: null,
      preview: {
        files: [
          { path: "src/a.rs", diff: "-1|old\n+1|new" },
          { path: "src/b.rs", rename: "src/c.rs", error: "No line 9 in src/b.rs" },
        ],
        generation: 2,
        streaming: false,
      },
      result: null,
      background: null,
    };
    const element = show(toolCard("c-files", undefined, run, undefined, undefined));
    expect(texts(element, "tool-preview-file")).toEqual(["src/a.rs", "src/b.rs → src/c.rs"]);
    expect(within(element).getAllByTestId("tool-diff").length).toBe(1);
    expect(texts(element, "tool-preview-error")).toEqual(["No line 9 in src/b.rs"]);
  });

  // spec: UI-695, UI-055
  test("an explicit collapse of a live edit wins when the card settles", async () => {
    renderWithHost(<EditLifecycleCard />, threadHost());
    expect(screen.getByTestId("tool-diff").textContent).toContain("left.saturating_add(right)");
    await userEvent.click(screen.getByTestId("toggle"));
    expect(screen.queryByTestId("tool-diff")).toBeNull();
    await userEvent.click(screen.getByTestId("settle-card"));
    expect(screen.getByTestId("toggle").getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByTestId("tool-diff")).toBeNull();
  });

  // spec: UI-050, UI-051, UI-090
  test("success: a highlighted diff view", async () => {
    const element = await open(card("edit_tool_calls", "call_edit_1"));
    const diff = byTestId(element, "tool-diff");
    expect([...diff.querySelectorAll("[data-kind=del]")].map((row) => row.textContent)).toEqual(["4−    left + right"]);
    expect([...diff.querySelectorAll("[data-kind=add]")].map((row) => row.textContent)).toEqual([
      "4+    left.saturating_add(right)",
    ]);
    await waitFor(() => expect(diff.querySelector(".hljs-keyword")?.textContent).toBe("pub"));
  });

  // spec: UI-050, UI-051, UI-052
  test("error: omp's reason and the refused edit", () => {
    const element = show(card("edit_tool_calls", "call_edit_2"));
    expect(byTestId(element, "tool-error").textContent).toContain("is not from this session");
    expect(byTestId(element, "tool-input").textContent).toContain("PUT 8.=8:");
  });
});

describe("write card", () => {
  // spec: UI-050, UI-051, UI-090, UI-695
  test("streaming content opens automatically; settled content stays collapsed until opened", async () => {
    const live = show(streaming("write_tool_calls", "call_write_1"));
    expect(byTestId(live, "toggle").getAttribute("aria-expanded")).toBe("true");
    expect(byTestId(live, "tool-path").textContent).toBe("notes.md");
    expect(byTestId(live, "tool-content").textContent).toContain("# Notes");
    cleanup();
    const done = card("write_tool_calls", "call_write_1");
    expect(within(show(done)).queryByTestId("tool-content")).toBeNull();
    cleanup();
    const element = await open(done);
    const content = byTestId(element, "tool-content");
    expect(content.textContent).toBe("# Notes\n\n- add saturates\n- greet says hello\n");
    await waitFor(() => expect(content.querySelector(".hljs-section")?.textContent).toBe("# Notes"));
  });

  // spec: UI-053, UI-935
  test("long content: the first 12 lines, then the whole content on demand, scrolling inside the card", async () => {
    const recorded = card("write_tool_calls", "call_write_1");
    const content = numbered(20);
    const element = await open({ ...recorded, view: { ...recorded.view, content } as ToolCard["view"] });
    expect(byTestId(element, "tool-content").textContent).toBe(numbered(12).trimEnd());
    const more = byTestId(element, "tool-more");
    expect([more.textContent, more.getAttribute("aria-expanded")]).toEqual(["… 8 more lines", "false"]);
    await userEvent.click(more);
    expect(byTestId(element, "tool-content").textContent).toBe(content);
    expect(element.querySelector(".omfp-bounded")?.getAttribute("data-expanded")).toBe("true");
    // The expanded body takes the focus to scroll by keyboard.
    const body = within(element).getByRole("region", { name: text.bodyLabel });
    expect([body.tabIndex, body.contains(byTestId(element, "tool-content"))]).toEqual([0, true]);
    await userEvent.click(byTestId(element, "tool-more"));
    expect(byTestId(element, "tool-content").textContent).toEndWith("line 12");
    // happy-dom lays nothing out: the rules themselves are checked. Live, about 12 code lines; expanded, about a
    // viewport; both scroll inside the card.
    const chat = parseCss(readFileSync(join(import.meta.dir, "../../../src/widgets/chat/chat.css"), "utf8"));
    const rule = (selector: string) => findRule(chat, ["@layer omfp-widgets"], selector).declarations;
    const live = rule('.omfp-bounded[data-live="true"] > .omfp-bounded__body');
    const expanded = rule('.omfp-bounded[data-expanded="true"] > .omfp-bounded__body');
    expect([live.get("overflow-y"), expanded.get("overflow-y")]).toEqual(["auto", "auto"]);
    expect(live.get("max-block-size")).toStartWith("calc(12 * var(--omfp-lh-code) * var(--omfp-fs-code)");
    expect(expanded.get("max-block-size")).toContain("100dvh");
  });

  // spec: UI-053, UI-695, UI-935
  test("live: a window following the last lines written; scrolled up it stops following until back at its end", async () => {
    renderWithHost(<GrowingWrite />, threadHost());
    const box = screen.getByTestId("tool-window");
    expect(screen.getByTestId("tool-content").textContent).toBe(numbered(30));
    expect(box.dataset.following).toBe("true");
    expect(screen.getByTestId("tool-earlier").textContent).toBe("… 18 earlier lines");
    expect(screen.queryByTestId("tool-more")).toBeNull();
    // Laid out 1000px tall in a 100px window, then scrolled up by the user.
    Object.defineProperty(box, "scrollHeight", { configurable: true, value: 1000 });
    Object.defineProperty(box, "clientHeight", { configurable: true, value: 100 });
    box.scrollTop = 100;
    fireEvent.scroll(box);
    expect(box.dataset.following).toBe("false");
    expect(screen.getByTestId("tool-earlier").textContent).toBe("… 3 earlier lines");
    await userEvent.click(screen.getByTestId("grow"));
    expect(screen.getByTestId("tool-content").textContent).toBe(numbered(40));
    expect([box.dataset.following, box.scrollTop]).toEqual(["false", 100]);
    // Back at its end, it follows what is written next.
    box.scrollTop = 900;
    fireEvent.scroll(box);
    expect(box.dataset.following).toBe("true");
    Object.defineProperty(box, "scrollHeight", { configurable: true, value: 1500 });
    await userEvent.click(screen.getByTestId("grow"));
    expect([box.dataset.following, box.scrollTop]).toEqual(["true", 1500]);
  });

  // spec: UI-053
  test("short content: no expander", async () => {
    const element = await open(card("write_tool_calls", "call_write_1"));
    expect(within(element).queryByTestId("tool-more")).toBeNull();
  });

  // spec: UI-050, UI-051, UI-052
  test("error: writing over a directory", () => {
    expect(byTestId(show(card("write_tool_calls", "call_write_2")), "tool-error").textContent).toContain("EISDIR");
  });
});

describe("bounded bodies", () => {
  // spec: UI-935
  test("a live window takes the focus and scrolls on the paging keys, which never reach the transcript", async () => {
    const outside: string[] = [];
    const listen = (event: KeyboardEvent) => outside.push(event.key);
    document.addEventListener("keydown", listen);
    try {
      renderWithHost(<GrowingWrite />, threadHost());
      const box = screen.getByTestId("tool-window");
      const region = within(screen.getByTestId("tool-card")).getByRole("region", { name: text.bodyLabel });
      expect([region.dataset.testid, box.tabIndex]).toEqual(["tool-window", 0]);
      Object.defineProperty(box, "scrollHeight", { configurable: true, value: 1000 });
      Object.defineProperty(box, "clientHeight", { configurable: true, value: 100 });
      // Laid out, at its end: it scrolls itself there.
      box.scrollTop = 900;
      fireEvent.scroll(box);
      box.focus();
      await userEvent.keyboard("{Home}");
      expect(box.scrollTop).toBe(0);
      fireEvent.scroll(box);
      expect(box.dataset.following).toBe("false");
      await userEvent.keyboard("{PageDown}");
      expect(box.scrollTop).toBeGreaterThan(0);
      await userEvent.keyboard("{End}");
      expect(box.scrollTop).toBe(900);
      fireEvent.scroll(box);
      expect(box.dataset.following).toBe("true");
      await userEvent.keyboard("{ArrowUp}");
      expect(outside).toEqual([]);
    } finally {
      document.removeEventListener("keydown", listen);
    }
  });

  // spec: UI-935
  test("a wheel turn a window scrolls itself never reaches the transcript; at its edge it goes on", () => {
    const outside: number[] = [];
    const listen = (event: WheelEvent) => outside.push(event.deltaY);
    document.addEventListener("wheel", listen);
    try {
      renderWithHost(<GrowingWrite />, threadHost());
      const box = screen.getByTestId("tool-window");
      Object.defineProperty(box, "scrollHeight", { configurable: true, value: 1000 });
      Object.defineProperty(box, "clientHeight", { configurable: true, value: 100 });
      box.scrollTop = 900;
      // Up from its end, the window scrolls: the transcript neither sees it nor unpins.
      fireEvent.wheel(box, { deltaY: -40 });
      // Down at its end, and up at its top, the turn goes on to the transcript.
      fireEvent.wheel(box, { deltaY: 40 });
      box.scrollTop = 0;
      fireEvent.wheel(box, { deltaY: -40 });
      expect(outside).toEqual([40, -40]);
    } finally {
      document.removeEventListener("wheel", listen);
    }
  });

  // spec: UI-935
  test("a long output: its last 12 lines once ended, the earlier ones on demand; live, a following window", async () => {
    const output = numbered(30);
    const view: ToolCard["view"] = {
      kind: "bash",
      command: "seq 30",
      cwd: undefined,
      output,
      exitCode: 0,
      background: undefined,
    };
    const element = await open({ ...card("bash_tool_call", "call_bash_1"), view });
    const earlier = byTestId(element, "tool-more");
    expect([earlier.textContent, earlier.getAttribute("aria-expanded")]).toEqual(["… 18 earlier lines", "false"]);
    // Its control is above the output it extends.
    const after = earlier.compareDocumentPosition(byTestId(element, "tool-output"));
    expect(after & Node.DOCUMENT_POSITION_FOLLOWING).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(byTestId(element, "tool-output").textContent).toBe(output.split("\n").slice(18).join("\n"));
    await userEvent.click(earlier);
    expect(byTestId(element, "tool-output").textContent).toBe(output);
    cleanup();
    const live = await open({ ...streaming("bash_tool_call", "call_bash_1"), view });
    expect(byTestId(live, "tool-output").closest("[data-testid=tool-window]")).not.toBeNull();
    expect(byTestId(live, "tool-output").textContent).toBe(output);
    expect(byTestId(live, "tool-earlier").textContent).toBe("… 18 earlier lines");
    expect(within(live).queryByTestId("tool-more")).toBeNull();
  });

  // spec: UI-935, UI-055
  test("an edit: live, its preview in a following window; once ended, its first 40 lines, the rest on demand", async () => {
    const lines: DiffLine[] = [
      { kind: "hunk", text: "@@ -1,0 +1,60 @@" },
      ...Array.from({ length: 60 }, (_, at) => ({
        kind: "add" as const,
        text: `let x${at} = ${at};`,
        newLine: at + 1,
      })),
    ];
    const live = streaming("edit_tool_calls", "call_edit_1");
    const preview = [{ path: "src/lib.rs", rename: undefined, diff: lines, error: undefined, language: "rust" }];
    const element = show({ ...live, view: { ...live.view, preview } as ToolCard["view"] });
    expect(byTestId(element, "tool-diff").closest("[data-testid=tool-window]")).not.toBeNull();
    expect(byTestId(element, "tool-diff").querySelectorAll("tr").length).toBe(61);
    expect(byTestId(element, "tool-earlier").textContent).toBe("… 49 earlier lines");
    cleanup();
    const done = card("edit_tool_calls", "call_edit_1");
    const settled = await open({ ...done, view: { ...done.view, diff: lines } as ToolCard["view"] });
    expect(byTestId(settled, "tool-diff").querySelectorAll("tr").length).toBe(40);
    const more = byTestId(settled, "tool-more");
    expect(more.textContent).toBe("… 21 more lines");
    await userEvent.click(more);
    expect(byTestId(settled, "tool-diff").querySelectorAll("tr").length).toBe(61);
  });

  // spec: UI-935
  test("a fetched page, a fallback card's arguments and result, a mount's props: their first 12 lines once ended", async () => {
    const page = card("web_tool_calls", "call_read_1");
    const fetched = await open({ ...page, view: { ...page.view, content: numbered(20) } as ToolCard["view"] });
    expect(byTestId(fetched, "tool-content").textContent).toBe(numbered(12).trimEnd());
    expect(byTestId(fetched, "tool-more").textContent).toBe("… 8 more lines");
    cleanup();
    const fields = Object.fromEntries(Array.from({ length: 20 }, (_, at) => [`field${at}`, at]));
    const ended = (name: string, args: JsonObject, answer: string): ToolCard => {
      const run: ToolRun = {
        tool_call_id: `c-${name}`,
        tool_name: name,
        args,
        intent: null,
        status: "succeeded",
        partial_result: null,
        preview: null,
        result: { content: [{ type: "text", text: answer }] },
        background: null,
      };
      return toolCard(`c-${name}`, undefined, run, undefined, undefined);
    };
    const generic = ended("custom_tool", fields, numbered(20));
    const fallback = await open(generic);
    expect(texts(fallback, "tool-more")).toEqual([
      text.more(generic.args.split("\n").length - 12, text.nouns.line),
      text.more(8, text.nouns.line),
    ]);
    expect(byTestId(fallback, "tool-result").textContent).toBe(numbered(12).trimEnd());
    cleanup();
    const update = await open(ended("omfp_ui_update", { id: "board", props: fields }, 'Updated "board".'));
    // 20 props and the braces: 22 lines.
    expect(byTestId(update, "tool-more").textContent).toBe("… 10 more lines");
  });

  // spec: UI-935
  test("a read: its first 10 lines once ended, the rest on demand", async () => {
    const read = card("read_tool_calls", "call_read_1");
    const element = await open({ ...read, view: { ...read.view, content: numbered(15) } as ToolCard["view"] });
    expect(byTestId(element, "tool-content").textContent).toBe(numbered(10).trimEnd());
    const more = byTestId(element, "tool-more");
    expect(more.textContent).toBe("… 5 more lines");
    await userEvent.click(more);
    expect(byTestId(element, "tool-content").textContent).toBe(numbered(15));
  });

  // spec: UI-935, UI-916
  test("a search: its first 8 files and each file's first 6 matches, the rest on demand", async () => {
    const grep = card("search_tool_calls", "call_grep_1");
    const matches = Array.from({ length: 8 }, (_, at) => ({
      line: at + 1,
      code: `fn f${at}() {}`,
      captures: undefined,
    }));
    const files = Array.from({ length: 10 }, (_, at) => ({
      path: `src/m${at}.rs`,
      count: undefined,
      matches: at === 0 ? matches : [],
    }));
    const element = await open({ ...grep, view: { ...grep.view, files } as ToolCard["view"] });
    expect([texts(element, "tool-file").length, texts(element, "tool-match").length]).toEqual([8, 6]);
    expect(texts(element, "tool-more")).toEqual([text.more(2, text.nouns.match), text.more(2, text.nouns.file)]);
    // The file's matches expanded first stay expanded when the files are.
    const [moreMatches, moreFiles] = within(element).getAllByTestId("tool-more");
    await userEvent.click(moreMatches as HTMLElement);
    await userEvent.click(moreFiles as HTMLElement);
    expect([texts(element, "tool-file").length, texts(element, "tool-match").length]).toEqual([10, 8]);
  });

  // spec: UI-935, UI-919
  test("an lsp answer: its first 2 locations, the rest on demand", async () => {
    const found = [2, 5, 9].map((line) => `  src/main.rs:${line}:9\n    ${line}:     add(1, 2)`);
    const element = await open(
      lspCard(
        { action: "references", file: "src/lib.rs", line: 3, symbol: "add" },
        `Found 3 reference(s):\n${found.join("\n")}`,
      ),
    );
    expect(texts(element, "lsp-location-item").length).toBe(2);
    await userEvent.click(byTestId(element, "tool-more"));
    expect(texts(element, "lsp-location-item").length).toBe(3);
  });

  // spec: UI-935
  test("web results: the first 8 links, each excerpt at most 3 lines", async () => {
    const search = card("web_tool_calls", "call_web_search_1");
    const links = Array.from({ length: 10 }, (_, at) => ({
      title: `Result ${at}`,
      url: `http://127.0.0.1/${at}`,
      snippet: "An excerpt.",
    }));
    const element = await open({ ...search, view: { ...search.view, links } as ToolCard["view"] });
    expect(texts(element, "tool-link").length).toBe(8);
    expect(byTestId(element, "tool-more").textContent).toBe(text.more(2, text.nouns.link));
    const chat = parseCss(readFileSync(join(import.meta.dir, "../../../src/widgets/chat/chat.css"), "utf8"));
    const snippet = findRule(chat, ["@layer omfp-widgets"], ".omfp-tool-card__snippet").declarations;
    expect([snippet.get("-webkit-line-clamp"), snippet.get("line-clamp"), snippet.get("overflow")]).toEqual([
      "3",
      "3",
      "hidden",
    ]);
  });

  // spec: UI-935
  test("a task: its first 4 subagents; a running one's output in a window following its last lines", async () => {
    const task = card("task_tool_call", "call_task_1");
    const agent = (name: string, status: string, output: string) => ({
      name,
      agent: "task",
      task: `Do ${name}`,
      status,
      activity: undefined,
      tools: 0,
      output,
    });
    const agents = [
      agent("A", "running", numbered(20)),
      ...["B", "C", "D", "E"].map((name) => agent(name, "completed", "done")),
    ];
    const element = await open({ ...task, view: { ...task.view, agents } as ToolCard["view"] });
    expect(texts(element, "subagent").length).toBe(4);
    expect(byTestId(element, "tool-more").textContent).toBe(text.more(1, text.nouns.agent));
    const [running] = within(element).getAllByTestId("subagent-output");
    expect(running?.closest("[data-testid=tool-window]")).not.toBeNull();
    expect(byTestId(element, "tool-earlier").textContent).toBe("… 8 earlier lines");
  });

  // spec: UI-935, UI-163
  test("a long todo phase: its task in progress, the open ones after it and the last done, the rest on demand", async () => {
    const todo = card("todo_tool_calls", "call_todo_3");
    const tasks = Array.from({ length: 16 }, (_, at) => ({
      content: `Task ${at}`,
      status: at < 5 ? "completed" : at === 5 ? "in_progress" : "pending",
    }));
    const element = await open({
      ...todo,
      view: { ...todo.view, phases: [{ name: "Plan", tasks }] } as ToolCard["view"],
    });
    expect(
      within(element)
        .getAllByTestId("todo-task")
        .map((task) => task.dataset.status),
    ).toEqual(["completed", "in_progress", ...Array.from({ length: 7 }, () => "pending")]);
    expect(byTestId(element, "tool-more").textContent).toBe(text.more(7, text.nouns.task));
    await userEvent.click(byTestId(element, "tool-more"));
    expect(texts(element, "todo-task").length).toBe(16);
  });
});

/** `count` numbered lines (`line 1`, …), each ending with a newline. */
function numbered(count: number): string {
  return Array.from({ length: count }, (_, at) => `line ${at + 1}\n`).join("");
}

/** A live `write` of 30 lines written on, 10 more on each click of `grow`, so its window is checked across updates. */
function GrowingWrite() {
  const [lines, setLines] = useState(30);
  const live = streaming("write_tool_calls", "call_write_1");
  const current: ToolCard = { ...live, view: { ...live.view, content: numbered(lines) } as ToolCard["view"] };
  return (
    <>
      <button type="button" data-testid="grow" onClick={() => setLines((count) => count + 10)}>
        grow
      </button>
      <ToolCardView threadId="t1" cardKey={current.toolCallId} card={current} />
    </>
  );
}

describe("search cards", () => {
  // spec: UI-050, UI-051
  test("streaming: the query; success: the files with their match counts", async () => {
    expect(byTestId(show(streaming("search_tool_calls", "call_grep_1")), "tool-subject").textContent).toBe("pub fn");
    cleanup();
    const grep = await open(card("search_tool_calls", "call_grep_1"));
    expect(byTestId(grep, "tool-total").textContent).toBe("2 matches in 1 file");
    expect(texts(grep, "tool-file")).toEqual(["src/lib.rs2"]);
    cleanup();
    const glob = await open(card("search_tool_calls", "call_glob_1"));
    expect([byTestId(glob, "tool-subject").textContent, byTestId(glob, "tool-total").textContent]).toEqual([
      "src/**/*.rs",
      "1 file",
    ]);
  });

  // spec: UI-050, UI-051, UI-052
  test("error: a missing folder", () => {
    expect(byTestId(show(card("search_tool_calls", "call_glob_2")), "tool-error").textContent).toBe(
      "Path not found: missing",
    );
  });
});

describe("code-intelligence cards", () => {
  /** The files the last shown card opened. */
  const opened = () => shownHost?.window.get(OPEN_FILES).w1?.tabs.map(({ path, line, diff }) => ({ path, line, diff }));

  // spec: UI-915, UI-916, UI-051
  test("ast_grep: named after the device's tool; each match's line opens the file there, its code highlighted", async () => {
    const element = await open(card("code_intel_tool_calls", "call_ast_grep_1"));
    expect([element.dataset.tool, byTestId(element, "tool-subject").textContent]).toEqual([
      "ast_grep",
      "pub fn $NAME($$$ARGS) -> $RET { $$$BODY }",
    ]);
    expect(byTestId(element, "tool-total").textContent).toBe("2 matches in 1 file");
    expect(texts(element, "tool-match-line")).toEqual(["3", "7"]);
    const code = within(element).getAllByTestId("tool-match-code");
    expect(code.map((block) => [block.dataset.lang, block.textContent])).toEqual([
      ["rust", "pub fn add(left: u64, right: u64) -> u64 {\n    left + right\n}"],
      ["rust", 'pub fn greet(name: &str) -> String {\n    format!("Hello, {name}!")\n}'],
    ]);
    expect(texts(element, "tool-match-captures")[0]).toBe(
      "ARGS=[left: u64, ,, right: u64], BODY=[left + right], NAME=add, RET=u64",
    );
    await userEvent.click(within(element).getAllByTestId("tool-match-line")[1] as HTMLElement);
    expect(opened()).toEqual([{ path: "src/lib.rs", line: 7, diff: undefined }]);
  });

  // spec: UI-916, UI-052
  test("ast_grep: a missing folder fails with omp's message", () => {
    expect(byTestId(show(card("code_intel_tool_calls", "call_ast_grep_2")), "tool-error").textContent).toBe(
      "Path not found: missing",
    );
  });

  // spec: UI-917, UI-051
  test("ast_edit: the preview's diff per file; resolving it lists the files, which open against HEAD", async () => {
    const preview = await open(card("code_intel_tool_calls", "call_ast_edit_1"));
    expect(byTestId(preview, "tool-preview").textContent).toBe(text.preview);
    expect(byTestId(preview, "tool-total").textContent).toBe("1 replacement in 1 file");
    const diff = byTestId(preview, "tool-diff");
    expect(diff.textContent).toContain("left + right");
    expect(diff.textContent).toContain("left.saturating_add(right)");
    cleanup();
    const resolved = await open(card("code_intel_tool_calls", "call_resolve_1"));
    expect(byTestId(resolved, "tool-subject").textContent).toBe("AST Edit: 1 replacement in 1 file");
    expect(texts(resolved, "tool-file")).toEqual(["src/lib.rs1 replacement"]);
    await userEvent.click(within(byTestId(resolved, "tool-file")).getByTestId("path-link"));
    expect(opened()).toEqual([{ path: "src/lib.rs", line: undefined, diff: true }]);
  });

  // spec: UI-918, UI-051
  test("lsp without a server: omp's note and where omp looks for language servers", async () => {
    const element = await open(card("code_intel_tool_calls", "call_lsp_1"));
    expect(element.dataset.tool).toBe("lsp");
    expect(texts(element, "lsp-note")).toEqual(["notes.txt: No language server handles .txt files"]);
    expect(byTestId(element, "lsp-environment").textContent).toBe(text.serverEnvironment);
  });

  // spec: UI-918, UI-116
  test("lsp diagnostics: severity, location opening the file at its line, message and source", async () => {
    const answer = "1 error(s):\n# src/\n## lib.rs\n  4:5 [error] [rustc] mismatched types (E0308)";
    const element = await open(lspCard({ action: "diagnostics", file: "src/lib.rs" }, answer));
    const [row] = within(element).getAllByTestId("lsp-diagnostic");
    expect(row?.dataset.severity).toBe("error");
    expect(row?.textContent).toBe("errorsrc/lib.rs:4:5mismatched typesrustc · E0308");
    await userEvent.click(byTestId(element, "lsp-location"));
    expect(opened()).toEqual([{ path: "src/lib.rs", line: 4, diff: undefined }]);
    cleanup();
    expect(
      byTestId(await open(lspCard({ action: "diagnostics", file: "src/lib.rs" }, "OK")), "lsp-clean").textContent,
    ).toBe(text.noIssues);
  });

  // spec: UI-918
  test("lsp diagnostics: no 'No issues' while a file was not checked; the unchecked file is named", async () => {
    const answer = "✓ src/a.ts: no issues\n⚠ src/b.ts: no diagnostics report yet from tsserver; retry once it finishes";
    const element = await open(lspCard({ action: "diagnostics", file: "src/*.ts" }, answer));
    expect(within(element).queryByTestId("lsp-clean")).toBeNull();
    expect(texts(element, "lsp-note")).toEqual([
      "src/b.ts: no diagnostics report yet from tsserver; retry once it finishes",
    ]);
  });

  // spec: UI-918, UI-052
  test("lsp: a failed call shows omp's error, not an answer", () => {
    const element = show(
      lspCard({ action: "hover", file: "src/lib.rs", line: 4 }, "LSP error: request timed out", false),
    );
    expect(element.dataset.status).toBe("failed");
    expect(byTestId(element, "tool-error").textContent).toBe("LSP error: request timed out");
    expect(within(element).queryByTestId("tool-result")).toBeNull();
  });

  // spec: UI-919, UI-116
  test("lsp references: the file and line in the head, each location with its source, opening at its line", async () => {
    const answer = "Found 1 reference(s):\n  src/main.rs:2:9\n    2:     add(1, 2)";
    const element = await open(lspCard({ action: "references", file: "src/lib.rs", line: 3, symbol: "add" }, answer));
    expect([element.dataset.omfpPath, element.dataset.omfpLine]).toEqual(["src/lib.rs", "3"]);
    expect(byTestId(element, "lsp-context").textContent).toBe("    add(1, 2)");
    await userEvent.click(byTestId(element, "lsp-location"));
    expect(opened()).toEqual([{ path: "src/main.rs", line: 2, diff: undefined }]);
  });

  // spec: UI-919, UI-116
  test("lsp rename: the files it changed open against HEAD; a deleted file does not", async () => {
    const answer = "Applied rename:\n  Applied 2 edit(s) to src/lib.rs\n  Deleted src/old.rs";
    const element = await open(lspCard({ action: "rename", file: "src/lib.rs", line: 3, new_name: "sum" }, answer));
    expect(texts(element, "tool-file")).toEqual(["src/lib.rs2 edits", "src/old.rsdeleted"]);
    expect(byTestId(element, "lsp-rename").textContent).toBe("renamed to sum");
    expect(byTestId(element, "lsp-edit-total").textContent).toBe("2 edits in 1 file");
    const [changed, deleted] = within(element).getAllByTestId("tool-file");
    expect(deleted !== undefined && within(deleted).queryByTestId("path-link")).toBeNull();
    await userEvent.click(within(changed as HTMLElement).getByTestId("path-link"));
    expect(opened()).toEqual([{ path: "src/lib.rs", line: undefined, diff: true }]);
  });

  // spec: UI-919, UI-116
  test("lsp rename preview: old → new name, the total; files open at the call's line, a long list collapses", async () => {
    const paths = Array.from({ length: 10 }, (_, index) => `src/m${index}.rs`);
    const rows = paths.map((path, index) => `  ${path}: ${index + 1} ${index === 0 ? "edit" : "edits"}`);
    const element = await open(
      lspCard(
        { action: "rename", file: "src/m1.rs", line: 3, symbol: "add", new_name: "sum", apply: false },
        `Rename preview:\n${rows.join("\n")}`,
      ),
    );
    expect(byTestId(element, "lsp-rename").textContent).toBe("add → sum");
    expect(byTestId(element, "lsp-edit-total").textContent).toBe("55 edits in 10 files");
    expect(texts(element, "tool-file")).toEqual(
      paths.slice(0, 8).map((path, index) => `${path}${index + 1} ${index === 0 ? "edit" : "edits"}`),
    );
    const more = byTestId(element, "tool-more");
    expect([more.textContent, more.getAttribute("aria-expanded")]).toEqual(["… 2 more files", "false"]);
    await userEvent.click(more);
    expect(texts(element, "tool-file").length).toBe(10);
    expect(byTestId(element, "tool-more").textContent).toBe(text.fewer(text.nouns.file));
    const links = within(element).getAllByTestId("path-link");
    await userEvent.click(links[1] as HTMLElement);
    await userEvent.click(links[9] as HTMLElement);
    expect(opened()).toEqual([
      { path: "src/m1.rs", line: 3, diff: undefined },
      { path: "src/m9.rs", line: undefined, diff: undefined },
    ]);
  });

  // spec: UI-919
  test("lsp symbols open at their lines; hover is Markdown; code actions are listed", async () => {
    const symbols = await open(
      lspCard({ action: "symbols", file: "src/lib.rs" }, "Symbols in src/lib.rs:\nƒ add @ line 3"),
    );
    await userEvent.click(byTestId(symbols, "lsp-symbol-line"));
    expect(opened()).toEqual([{ path: "src/lib.rs", line: 3, diff: undefined }]);
    cleanup();
    const workspace = await open(
      lspCard(
        { action: "symbols", file: "*", query: "add" },
        'Found 1 symbol(s) matching "add":\n  ƒ add (Calc) @ src/main.rs:3:8',
      ),
    );
    expect(texts(workspace, "lsp-symbol")).toEqual(["add (Calc)src/main.rs:3"]);
    await userEvent.click(byTestId(workspace, "lsp-symbol-line"));
    expect(opened()).toEqual([{ path: "src/main.rs", line: 3, diff: undefined }]);
    cleanup();
    const hover = await open(
      lspCard({ action: "hover", file: "src/lib.rs", line: 3 }, "```rust\nfn add()\n```\n\nAdds."),
    );
    // Five lines: the first 4, the rest on demand.
    expect(byTestId(hover, "tool-more").textContent).toBe(text.more(1, text.nouns.line));
    await userEvent.click(byTestId(hover, "tool-more"));
    expect(byTestId(hover, "lsp-hover").querySelector("p")?.textContent).toBe("Adds.");
    cleanup();
    const actions = await open(
      lspCard({ action: "code_actions", file: "src/lib.rs", line: 3 }, "1 code action(s):\n  0: [quickfix] Use u32"),
    );
    expect(texts(actions, "lsp-action")).toEqual(["0quickfixUse u32"]);
  });
});

describe("todo card", () => {
  // spec: UI-050, UI-051, UI-054
  test("streaming: the tasks the call gives, pending; success: the checklist", async () => {
    const live = await open(streaming("todo_tool_calls", "call_todo_1"));
    expect(
      within(live)
        .getAllByTestId("todo-task")
        .map((task) => [task.dataset.status, task.textContent]),
    ).toEqual([
      ["pending", "●Read the codepending"],
      ["pending", "●Fix the bugpending"],
    ]);
    cleanup();
    const element = await open(card("todo_tool_calls", "call_todo_3"));
    expect(
      within(element)
        .getAllByTestId("todo-task")
        .map((task) => [task.dataset.status, task.textContent]),
    ).toEqual([
      ["completed", "●Read the codedone"],
      ["in_progress", "●Fix the bugin progress"],
    ]);
  });

  // spec: UI-050, UI-051, UI-052
  test("error: an unknown task, with the list kept", () => {
    const element = show(card("todo_tool_calls", "call_todo_2"));
    expect(byTestId(element, "tool-error").textContent).toStartWith('Errors: Task "Write the docs" not found');
    expect(texts(element, "todo-task").length).toBe(2);
  });
});

describe("web_search card", () => {
  // spec: UI-050, UI-051
  test("streaming: the query; success: the links", async () => {
    expect(byTestId(show(streaming("web_tool_calls", "call_web_search_1")), "tool-subject").textContent).toBe(
      "omfp tool cards",
    );
    cleanup();
    const element = await open(card("web_tool_calls", "call_web_search_1"));
    const links = within(element).getAllByTestId("tool-link");
    expect(links.map((link) => link.querySelector("a")?.getAttribute("href"))).toEqual([
      "http://127.0.0.1:29417/page",
      "http://127.0.0.1/tools",
    ]);
    expect(links[0]?.textContent).toContain("Tool cards");
  });

  // spec: UI-050, UI-051, UI-052
  test("error: the engine's failure, expanded", () => {
    const element = show(card("web_tool_calls", "call_web_search_2"));
    expect(element.querySelector(".omfp-tool")?.getAttribute("data-status")).toBe("failed");
    expect(byTestId(element, "tool-error").textContent).toContain("SearXNG API error (500)");
  });
});

describe("ask card", () => {
  // spec: UI-050, UI-051
  test("streaming: the questions; success: with the given answers", async () => {
    expect(texts(await open(streaming("ask_answered", "call_ask_1")), "ask-answer")).toEqual([]);
    cleanup();
    const element = await open(card("ask_answered", "call_ask_1"));
    // An ask's first 3 questions, the rest on demand (UI-935).
    expect(texts(element, "ask-question")).toEqual([
      "Which database?Postgres",
      "Which features?Auth, Other: Elasticsearch",
      "Service nameorders",
    ]);
    await userEvent.click(byTestId(element, "tool-more"));
    expect(texts(element, "ask-question").at(-1)).toBe("Replicas3");
  });

  // spec: UI-050, UI-051, UI-052
  test("error: cancelled by the user", () => {
    const element = show(card("ask_cancelled", "call_ask_1"));
    expect(byTestId(element, "tool-error").textContent).toBe("Ask tool was cancelled by the user");
    expect(texts(element, "ask-question")).toEqual(["Which database?"]);
  });
});

describe("task card", () => {
  // spec: UI-050, UI-051
  test("streaming: each subagent's name, task and live progress; success: its output", async () => {
    const running = await open(streaming("task_tool_call", "call_task_1"));
    expect(byTestId(running, "tool-subject").textContent).toBe("Counter");
    expect(byTestId(running, "subagent-activity").textContent).toBe("Reading the library");
    cleanup();
    const element = await open(card("task_tool_call", "call_task_1"));
    expect(byTestId(element, "subagent").dataset.status).toBe("completed");
    expect(byTestId(element, "subagent-output").textContent).toBe("src/lib.rs defines 2 functions: add and greet.");
  });

  // spec: UI-050, UI-051, UI-052, UI-406
  test("error: an unknown agent", () => {
    const element = show(card("task_tool_call", "call_task_2"));
    expect(byTestId(element, "tool-error").textContent).toContain('Unknown agent "no-such-agent"');
    expect(byTestId(element, "subagent").dataset.status).toBe("failed");
  });
});

describe("widget mount card", () => {
  // spec: UI-054, UI-695
  test("while the model writes a mount, its id and code show as written, open; once mounted, the daemon's answer", async () => {
    const partialArgs = '{"id": "chart", "code": "export default function Chart() {';
    const call = { type: "toolCall" as const, name: "omfp_ui_mount", arguments: {}, partialArgs };
    const live = show(toolCard("c-mount", call, undefined, undefined, undefined));
    expect([live.dataset.status, live.dataset.view]).toEqual(["composing", "mount"]);
    expect(byTestId(live, "toggle").getAttribute("aria-expanded")).toBe("true");
    expect(byTestId(live, "tool-subject").textContent).toBe("chart");
    expect(byTestId(live, "tool-content").textContent).toBe("export default function Chart() {");
    cleanup();
    const answer = 'Mounted ./widgets/chart as "chart" in the side tabs; open windows show it now.';
    const run: ToolRun = {
      tool_call_id: "c-mounted",
      tool_name: "omfp_ui_mount",
      args: { id: "chart", widget: "./widgets/chart", place: "side_tab" },
      intent: null,
      status: "succeeded",
      partial_result: null,
      preview: null,
      result: { content: [{ type: "text", text: answer }] },
      background: null,
    };
    const mounted = toolCard("c-mounted", undefined, run, undefined, undefined);
    expect(mounted.summary).toBe("place: side_tab");
    const element = await open(mounted);
    expect(byTestId(element, "tool-subject").textContent).toBe("chart");
    expect(byTestId(element, "tool-result").textContent).toBe(answer);
  });

  // spec: UI-050
  test("an update shows the props it sets", async () => {
    const run: ToolRun = {
      tool_call_id: "c-update",
      tool_name: "omfp_ui_update",
      args: { id: "board", props: { rows: 2, title: null } },
      intent: null,
      status: "succeeded",
      partial_result: null,
      preview: null,
      result: { content: [{ type: "text", text: 'Updated "board".' }] },
      background: null,
    };
    const element = await open(toolCard("c-update", undefined, run, undefined, undefined));
    expect(byTestId(element, "tool-subject").textContent).toBe("board");
    expect(byTestId(element, "tool-props").textContent).toBe('{\n  "rows": 2,\n  "title": null\n}');
  });
});

describe("result images", () => {
  /** A `read` of `/w/plot.png` whose result holds `count` PNG images. */
  function imageCard(count: number): ToolCard {
    return toolCard(
      "c1",
      { type: "toolCall", id: "c1", name: "read", arguments: { path: "/w/plot.png" } },
      undefined,
      {
        role: "toolResult",
        toolCallId: "c1",
        toolName: "read",
        content: [
          { type: "text", text: "Read image file [image/png]" },
          ...Array.from({ length: count }, () => ({
            type: "image" as const,
            data: "iVBORw0KGgo=",
            mimeType: "image/png",
          })),
        ],
      },
      undefined,
    );
  }

  // spec: UI-473
  test("the thumbnails show while the card is collapsed, in a row, each with its alt text", () => {
    const element = show(imageCard(2));
    expect(byTestId(element, "toggle").getAttribute("aria-expanded")).toBe("false");
    const thumbnails = within(element).getAllByTestId("tool-image");
    expect(thumbnails.map((thumbnail) => thumbnail.getAttribute("aria-label"))).toEqual([
      "Open image 1 of 2 from read /w/plot.png",
      "Open image 2 of 2 from read /w/plot.png",
    ]);
    expect(thumbnails.map((thumbnail) => thumbnail.querySelector("img")?.getAttribute("src"))).toEqual([
      "data:image/png;base64,iVBORw0KGgo=",
      "data:image/png;base64,iVBORw0KGgo=",
    ]);
    expect(byTestId(element, "tool-images").getAttribute("aria-label")).toBe("Images from read");
  });

  /** The full view's image and whether its zoom is fitted; a press on Zoom in zooms it (UI-549). */
  async function zoomable(dialog: HTMLElement): Promise<string[]> {
    const image = within(dialog).getByTestId("image-after");
    const fit = within(dialog).getByTestId("image-fit");
    const fitted = fit.getAttribute("aria-pressed") ?? "";
    await userEvent.click(within(dialog).getByRole("button", { name: "Zoom in" }));
    return [
      image.getAttribute("src") ?? "",
      image.getAttribute("alt") ?? "",
      fitted,
      fit.getAttribute("aria-pressed") ?? "",
    ];
  }

  // spec: UI-474
  test("a click or Enter on a thumbnail opens the image zoomable as large as the window; Escape closes it", async () => {
    const element = show(imageCard(1));
    expect(screen.queryAllByTestId("tool-image-dialog").length).toBe(0);
    await userEvent.click(byTestId(element, "tool-image"));
    const dialog = screen.getByTestId("tool-image-dialog");
    expect(within(dialog).getByRole("heading").textContent).toBe("Image from read /w/plot.png");
    // The kit's image viewer: fitted first, then zoomed by its buttons like a file's image.
    expect(await zoomable(dialog)).toEqual([
      "data:image/png;base64,iVBORw0KGgo=",
      "Image from read /w/plot.png",
      "true",
      "false",
    ]);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryAllByTestId("tool-image-dialog").length).toBe(0);
    // The focus is back on the thumbnail: Enter opens it again.
    expect(document.activeElement?.getAttribute("data-testid")).toBe("tool-image");
    await userEvent.keyboard("{Enter}");
    expect(screen.queryAllByTestId("tool-image-dialog").length).toBe(1);
  });

  // spec: UI-476, UI-474
  test("a message's image is a button opening it zoomable; Escape closes it, back to the image", async () => {
    renderWithHost(
      <MessageRow
        threadId="t1"
        cardKey="m1"
        streaming={false}
        continues={false}
        view={{
          role: "user",
          label: "You",
          blocks: [{ type: "image", mime: "image/png", data: "iVBORw0KGgo=" }],
          error: undefined,
          steering: false,
        }}
      />,
      testHost(),
    );
    const open = screen.getByTestId("message-image-open");
    expect([open.getAttribute("aria-label"), open.querySelector("img")?.getAttribute("alt")]).toEqual([
      "Open the attached image",
      "Attached image",
    ]);
    expect(screen.queryAllByTestId("message-image-dialog").length).toBe(0);
    await userEvent.click(open);
    const dialog = screen.getByTestId("message-image-dialog");
    expect(within(dialog).getByRole("heading").textContent).toBe("Attached image");
    expect(await zoomable(dialog)).toEqual(["data:image/png;base64,iVBORw0KGgo=", "Attached image", "true", "false"]);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryAllByTestId("message-image-dialog").length).toBe(0);
    expect(document.activeElement?.getAttribute("data-testid")).toBe("message-image-open");
    await userEvent.keyboard(" ");
    expect(screen.queryAllByTestId("message-image-dialog").length).toBe(1);
  });

  // spec: UI-475, UI-476
  test("a message's image and a result's thumbnail show at most at their natural size: never stretched, never upscaled", () => {
    renderWithHost(
      <MessageRow
        threadId="t1"
        cardKey="m1"
        streaming={false}
        continues={false}
        view={{
          role: "user",
          label: "You",
          blocks: [{ type: "image", mime: "image/png", data: "iVBORw0KGgo=" }],
          error: undefined,
          steering: false,
        }}
      />,
      testHost(),
    );
    const image = screen.getByTestId("message-image");
    const thumbnail = show(imageCard(1)).querySelector(".omfp-tool-images__img");
    for (const element of [image, thumbnail]) {
      expect([element?.getAttribute("width"), element?.getAttribute("height"), element?.getAttribute("style")]).toEqual(
        [null, null, null],
      );
    }
    // happy-dom lays nothing out: the rules themselves are checked, in both skins.
    const read = (path: string) => parseCss(readFileSync(join(import.meta.dir, path), "utf8"));
    const chat = read("../../../src/widgets/chat/chat.css");
    const rule = (selector: string) => findRule(chat, ["@layer omfp-widgets"], selector).declarations;
    const sizes = ["width", "height", "inline-size", "block-size", "min-inline-size", "min-block-size", "flex"];
    // The message's body is a flex column: the image's button must not stretch across it, nor the image in it.
    const button = rule(".omfp-button.omfp-message__open");
    expect([button.get("align-self"), button.get("max-inline-size"), button.get("block-size")]).toEqual([
      "flex-start",
      "min(100%, 32rem)",
      "auto",
    ]);
    expect(sizes.filter((size) => button.has(size) && size !== "block-size")).toEqual([]);
    const message = rule(".omfp-message__image");
    expect(message.get("max-inline-size")).toBe("100%");
    expect(sizes.filter((size) => message.has(size))).toEqual([]);
    const inThumbnail = rule(".omfp-tool-images__img");
    expect([inThumbnail.get("max-inline-size"), inThumbnail.get("max-block-size")]).toEqual(["100%", "100%"]);
    expect(sizes.filter((size) => inThumbnail.has(size))).toEqual([]);
    const boring = [...read("../../../src/boring.css"), ...read("../../../../kit/src/boring.css")];
    expect(
      boring
        .filter((css) => /omfp-message__(image|open)|omfp-tool-images/.test(css.selector))
        .map((css) => css.selector),
    ).toEqual([]);
  });
});

describe("opening files from cards", () => {
  /** The files the last shown card opened. */
  const opened = () => shownHost?.window.get(OPEN_FILES).w1?.tabs.map(({ path, line, diff }) => ({ path, line, diff }));

  // spec: UI-116
  test("a read's path opens the file at its first line, without toggling the card; the card carries it for :open", async () => {
    const element = show(card("read_tool_calls", "call_read_1"));
    expect([element.dataset.omfpPath, element.dataset.omfpLine]).toEqual(["src/lib.rs", "1"]);
    await userEvent.click(byTestId(element, "tool-path-open"));
    expect(opened()).toEqual([{ path: "src/lib.rs", line: 1, diff: undefined }]);
    expect(within(element).getByTestId("toggle").getAttribute("aria-expanded")).toBe("false");
  });

  // spec: UI-116
  test("an edit's and a write's path open the file against HEAD", async () => {
    await userEvent.click(byTestId(show(card("edit_tool_calls", "call_edit_1")), "tool-path-open"));
    expect(opened()).toEqual([{ path: "src/lib.rs", line: undefined, diff: true }]);
    cleanup();
    const write = show(streaming("write_tool_calls", "call_write_1"));
    expect(write.dataset.omfpDiff).toBe("");
    await userEvent.click(byTestId(write, "tool-path-open"));
    expect(opened()).toEqual([{ path: "notes.md", line: undefined, diff: true }]);
  });

  // spec: UI-116
  test("paths in the assistant's Markdown open the file; the user's text stays plain", async () => {
    const test = testHost();
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "cards", "w1")] });
    test.host.window.set(SELECTED_THREAD, "t1");
    shownHost = test.host;
    renderWithHost(
      <>
        <MessageRow
          threadId="t1"
          cardKey="m1"
          streaming={false}
          continues={false}
          view={{
            role: "assistant",
            label: "Assistant",
            blocks: [{ type: "text", text: "Changed `src/lib.rs:4`." }],
            error: undefined,
            steering: false,
          }}
        />
        <MessageRow
          threadId="t1"
          cardKey="m2"
          streaming={false}
          continues={false}
          view={{
            role: "user",
            label: "You",
            blocks: [{ type: "text", text: "`src/lib.rs`" }],
            error: undefined,
            steering: false,
          }}
        />
      </>,
      test,
    );
    test.flush();
    const links = screen.getAllByTestId("path-link");
    expect(links.map((link) => link.textContent)).toEqual(["src/lib.rs:4"]);
    await userEvent.click(links[0] as HTMLElement);
    expect(opened()).toEqual([{ path: "src/lib.rs", line: 4, diff: undefined }]);
  });

  // spec: UI-116
  test("each file of a grep result opens", async () => {
    const grep = await open(card("search_tool_calls", "call_grep_1"));
    await userEvent.click(within(byTestId(grep, "tool-file")).getByTestId("path-link"));
    expect(opened()).toEqual([{ path: "src/lib.rs", line: undefined, diff: undefined }]);
  });

  // spec: UI-702
  test("an absolute path outside the workspace folder (an agent's worktree elsewhere) opens as given", async () => {
    const path = "/tmp/elsewhere/README.md";
    const read = toolCard(
      "c1",
      { type: "toolCall", id: "c1", name: "read", arguments: { path } },
      undefined,
      { role: "toolResult", toolCallId: "c1", toolName: "read", content: [{ type: "text", text: "# Elsewhere" }] },
      undefined,
    );
    const element = show(read);
    expect(element.dataset.omfpPath).toBe(path);
    expect(byTestId(element, "tool-path-open").textContent).toContain(path);
    await userEvent.click(byTestId(element, "tool-path-open"));
    expect(opened()?.map((tab) => tab.path)).toEqual([path]);
  });
});

describe("read group card", () => {
  /** Renders a reply whose only block is the read group of `reads` in workspace `w1`, as {@link show} does. */
  function showGroup(reads: ToolCard[], status: CardStatus): HTMLElement {
    const test = testHost();
    shownHost = test.host;
    test.client.connected();
    test.client.index({
      workspaces: [workspace("w1", [{ ...repo("w1", "src", "main"), kind: "clone" }])],
      threads: [thread("t1", "cards", "w1")],
    });
    test.host.window.set(SELECTED_THREAD, "t1");
    renderWithHost(
      <MessageRow
        threadId="t1"
        cardKey="m1"
        streaming={false}
        continues={false}
        view={{
          role: "assistant",
          label: "Assistant",
          blocks: [{ type: "readGroup", cards: reads, status }],
          error: undefined,
          steering: false,
        }}
      />,
      test,
    );
    test.flush();
    return screen.getByTestId("read-group");
  }

  /** The three reads of the fixture: a file, a missing file (failed) and a folder. */
  const reads = () => ["call_read_1", "call_read_2", "call_read_3"].map((id) => card("read_tool_calls", id));
  const calls = (element: HTMLElement) =>
    within(element)
      .queryAllByTestId("tool-card")
      .map((node) => node.dataset.call);

  // spec: UI-765, UI-052
  test("collapsed: one head with the count and the run's failed mark; the failed read stays shown with its error", () => {
    const group = showGroup(reads(), "failed");
    expect(group.dataset.status).toBe("failed");
    const head = byTestId(group, "read-group-toggle");
    expect(head.getAttribute("aria-expanded")).toBe("false");
    expect(byTestId(group, "read-group-count").textContent).toBe("3 files");
    expect(within(group).getByRole("region", { name: "Read 3 files" }).dataset.status).toBe("failed");
    expect(byTestId(group, "read-group-status").textContent).toBe("failed");
    expect(within(group).queryByTestId("read-group-reads")).toBeNull();
    expect(calls(group)).toEqual(["call_read_2"]);
    expect(byTestId(group, "tool-error").textContent).toBe("Path 'src/missing.rs' not found");
  });

  // spec: UI-765, UI-050
  test("open: every read in call order with its mark and path, each opening to its own read card", async () => {
    const group = showGroup(reads(), "failed");
    await userEvent.click(byTestId(group, "read-group-toggle"));
    expect(within(group).queryByTestId("read-group-shown")).toBeNull();
    const list = byTestId(group, "read-group-reads");
    expect(calls(list)).toEqual(["call_read_1", "call_read_2", "call_read_3"]);
    expect(
      within(list)
        .getAllByTestId("tool-card")
        .map((node) => node.dataset.status),
    ).toEqual(["succeeded", "failed", "succeeded"]);
    expect(texts(list, "tool-path")).toEqual(["src/lib.rs", "src/missing.rs", "src"]);
    const [first] = within(list).getAllByTestId("tool-card");
    if (first === undefined) {
      throw new Error("no read");
    }
    expect(within(first).queryByTestId("tool-content")).toBeNull();
    await userEvent.click(byTestId(first, "toggle"));
    expect(byTestId(first, "tool-content").textContent).toStartWith("//! Tool card fixture crate.");
  });

  // spec: UI-765, UI-116
  test("a read's path with its selector opens the file at its line without toggling it", async () => {
    const ranged = toolCard(
      "c4",
      { type: "toolCall", id: "c4", name: "read", arguments: { path: "src/lib.rs:4-6" } },
      undefined,
      {
        role: "toolResult",
        toolCallId: "c4",
        toolName: "read",
        content: [{ type: "text", text: "4:    left + right" }],
      },
      undefined,
    );
    const group = showGroup([card("read_tool_calls", "call_read_1"), ranged], "succeeded");
    await userEvent.click(byTestId(group, "read-group-toggle"));
    const read = within(group)
      .getAllByTestId("tool-card")
      .find((node) => node.dataset.call === "c4");
    if (read === undefined) {
      throw new Error("no ranged read");
    }
    expect(byTestId(read, "tool-selector").textContent).toBe(":4-6");
    expect([read.dataset.omfpPath, read.dataset.omfpLine]).toEqual(["src/lib.rs", "4"]);
    await userEvent.click(byTestId(read, "tool-path-open"));
    const opened = shownHost?.window.get(OPEN_FILES).w1?.tabs.map(({ path, line }) => ({ path, line }));
    expect(opened).toEqual([{ path: "src/lib.rs", line: 4 }]);
    expect(byTestId(read, "toggle").getAttribute("aria-expanded")).toBe("false");
  });

  // spec: UI-765, UI-473
  test("collapsed: a read's images stay shown under the head; open, under that read's card only", async () => {
    const image = toolCard(
      "c5",
      { type: "toolCall", id: "c5", name: "read", arguments: { path: "/w/plot.png" } },
      undefined,
      {
        role: "toolResult",
        toolCallId: "c5",
        toolName: "read",
        content: [{ type: "image", data: "iVBORw0KGgo=", mimeType: "image/png" }],
      },
      undefined,
    );
    const group = showGroup([card("read_tool_calls", "call_read_1"), image], "succeeded");
    expect(calls(group)).toEqual([]);
    expect(within(byTestId(group, "read-group-shown")).getAllByTestId("tool-image").length).toBe(1);
    await userEvent.click(byTestId(group, "read-group-toggle"));
    expect(within(group).queryByTestId("read-group-shown")).toBeNull();
    const [read] = within(group).getAllByTestId("tool-images");
    expect(read?.closest("[data-testid=tool-card]")?.getAttribute("data-call")).toBe("c5");
  });

  // spec: UI-765, UI-695
  test("running: the group spins and counts its elapsed time while a read runs", () => {
    const clock = new ManualClock(1_000);
    const test = testHost();
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "cards", "w1")] });
    test.host.window.set(SELECTED_THREAD, "t1");
    renderWithHost(
      <ClockContext value={clock}>
        <MessageRow
          threadId="t1"
          cardKey="m2"
          streaming
          continues={false}
          view={{
            role: "assistant",
            label: "Assistant",
            blocks: [
              {
                type: "readGroup",
                cards: [card("read_tool_calls", "call_read_1"), streaming("read_tool_calls", "call_read_3")],
                status: "running",
              },
            ],
            error: undefined,
            steering: false,
          }}
        />
      </ClockContext>,
      test,
    );
    test.flush();
    const group = screen.getByTestId("read-group");
    expect(group.dataset.status).toBe("running");
    expect(byTestId(group, "read-group-status").textContent).toBe("running");
    expect(byTestId(group, "read-group-duration").textContent).toBe("0 ms");
    act(() => clock.advance(1_500));
    expect(byTestId(group, "read-group-duration").textContent).toBe("1.5 s");
  });

  /** Renders `ui` in workspace `w1` as {@link show} does. */
  function render(ui: ReactNode, clock?: Clock): void {
    const test = testHost();
    shownHost = test.host;
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "cards", "w1")] });
    test.host.window.set(SELECTED_THREAD, "t1");
    renderWithHost(clock === undefined ? ui : <ClockContext value={clock}>{ui}</ClockContext>, test);
    test.flush();
  }

  // spec: UI-765, UI-052
  test("a read opened while it streams alone stays open once a second read turns it into a group", async () => {
    const first = streaming("read_tool_calls", "call_read_1");
    render(<GrowingReply first={first} second={streaming("read_tool_calls", "call_read_3")} />);
    const lone = screen.getByTestId("tool-card");
    expect(byTestId(lone, "toggle").getAttribute("aria-expanded")).toBe("false");
    await userEvent.click(byTestId(lone, "toggle"));
    await userEvent.click(screen.getByTestId("second-read"));
    const group = screen.getByTestId("read-group");
    await userEvent.click(byTestId(group, "read-group-toggle"));
    const read = within(group)
      .getAllByTestId("tool-card")
      .find((node) => node.dataset.call === first.toolCallId);
    expect(read && byTestId(read, "toggle").getAttribute("aria-expanded")).toBe("true");
  });

  // spec: UI-765, UI-695
  test("a read running in a collapsed group is observed from then on: opened later, its elapsed time counts from then", async () => {
    const clock = new ManualClock(1_000);
    const running = streaming("read_tool_calls", "call_read_3");
    render(
      <MessageRow
        threadId="t1"
        cardKey="m4"
        streaming
        continues={false}
        view={readsReply([card("read_tool_calls", "call_read_1"), running])}
      />,
      clock,
    );
    act(() => clock.advance(1_500));
    await userEvent.click(screen.getByTestId("read-group-toggle"));
    const read = within(screen.getByTestId("read-group-reads"))
      .getAllByTestId("tool-card")
      .find((node) => node.dataset.call === running.toolCallId);
    expect(read && byTestId(read, "tool-duration").textContent).toBe("1.5 s");
  });
});

/** A streaming reply of `reads`, its blocks as the view model groups them: a lone read is its own card. */
function readsReply(reads: ToolCard[]): MessageView {
  const [only] = reads;
  return {
    role: "assistant",
    label: "Assistant",
    blocks:
      reads.length === 1 && only !== undefined
        ? [{ type: "tool", card: only }]
        : [{ type: "readGroup", cards: reads, status: "running" }],
    error: undefined,
    steering: false,
  };
}

/** A streaming reply whose second read arrives on demand, turning its first read's card into a group. */
function GrowingReply({ first, second }: { first: ToolCard; second: ToolCard }) {
  const [reads, setReads] = useState([first]);
  return (
    <>
      <button type="button" data-testid="second-read" onClick={() => setReads([first, second])}>
        second
      </button>
      <MessageRow threadId="t1" cardKey="m3" streaming continues={false} view={readsReply(reads)} />
    </>
  );
}
