/**
 * L4: the specialized tool cards (UI-050..052) rendered from fixtures recorded from the pinned omp (UI-051), each in
 * its streaming, success and error states; code in them is highlighted (UI-090).
 */

import { afterEach, describe, expect, test } from "bun:test";
import { replay } from "@omfp/core";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { type ReactNode, useState } from "react";
import { type OmfpHost, OPEN_FILES, SELECTED_THREAD, strings } from "../../../src/sdk/index.ts";
import { type CardStatus, type ToolCard, toolCard } from "../../../src/vm/tool.ts";
import type { MessageView } from "../../../src/vm/transcript.ts";
import { MessageRow, ToolCardView } from "../../../src/widgets/chat/message.tsx";
import { type Clock, ClockContext } from "../../../src/widgets/chat/state.ts";
import { card, cards, lspCard, streaming } from "../../support/cards.ts";
import { repo, thread, workspace } from "../../support/fixtures.ts";
import { renderWithHost, testHost } from "../../support/host.tsx";
import { fixtureEvents, isFrame } from "./fixtures.ts";
import { ManualClock } from "./harness.tsx";

const text = strings.chat.tool;

afterEach(cleanup);

/** The host of the last {@link show}. */
let shownHost: OmfpHost | undefined;

/** Renders `toolCard` in workspace `w1` (folder `/work/w1`, whose repository `src` holds the fixtures' files). */
function show(toolCard: ToolCard, clock?: Clock): HTMLElement {
  const test = testHost();
  shownHost = test.host;
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1", [{ ...repo("w1", "src", "main"), kind: "clone" }])],
    threads: [thread("t1", "cards", "w1")],
  });
  test.host.window.set(SELECTED_THREAD, "t1");
  const card = <ToolCardView cardKey={toolCard.toolCallId} card={toolCard} />;
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
      <ToolCardView cardKey={current.toolCallId} card={current} />
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
  // spec: UI-050, UI-051, UI-695
  test("streaming: the file and the edit as written are open without another click", () => {
    const element = show(streaming("edit_tool_calls", "call_edit_1"));
    expect(byTestId(element, "toggle").getAttribute("aria-expanded")).toBe("true");
    expect(byTestId(element, "tool-path").textContent).toBe("src/lib.rs");
    expect(byTestId(element, "tool-input").textContent).toContain("left.saturating_add(right)");
  });

  // spec: UI-695
  test("an explicit collapse of a live edit wins when the card settles", async () => {
    const test = testHost();
    test.client.connected();
    test.client.index({
      workspaces: [workspace("w1", [{ ...repo("w1", "src", "main"), kind: "clone" }])],
      threads: [thread("t1", "cards", "w1")],
    });
    test.host.window.set(SELECTED_THREAD, "t1");
    renderWithHost(<EditLifecycleCard />, test);
    expect(screen.getByTestId("tool-input").textContent).toContain("left.saturating_add(right)");
    await userEvent.click(screen.getByTestId("toggle"));
    expect(screen.queryByTestId("tool-input")).toBeNull();
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

  // spec: UI-053
  test("long content: the first 6 lines, then the whole content on demand", async () => {
    const recorded = card("write_tool_calls", "call_write_1");
    const content = `${Array.from({ length: 9 }, (_, at) => `line ${at + 1}`).join("\n")}\n`;
    const element = await open({ ...recorded, view: { ...recorded.view, content } as ToolCard["view"] });
    expect(byTestId(element, "tool-content").textContent).toBe("line 1\nline 2\nline 3\nline 4\nline 5\nline 6");
    const more = byTestId(element, "tool-more");
    expect([more.textContent, more.getAttribute("aria-expanded")]).toEqual(["… 3 more lines", "false"]);
    await userEvent.click(more);
    expect(byTestId(element, "tool-content").textContent).toBe(content);
    await userEvent.click(byTestId(element, "tool-more"));
    expect(byTestId(element, "tool-content").textContent).toEndWith("line 6");
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
    expect(texts(element, "lsp-note")).toEqual(["notes.txt: No language server found"]);
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
    const [changed, deleted] = within(element).getAllByTestId("tool-file");
    expect(deleted !== undefined && within(deleted).queryByTestId("path-link")).toBeNull();
    await userEvent.click(within(changed as HTMLElement).getByTestId("path-link"));
    expect(opened()).toEqual([{ path: "src/lib.rs", line: undefined, diff: true }]);
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
    expect(byTestId(hover, "lsp-hover").querySelector("p")?.textContent).toBe("Adds.");
    cleanup();
    const actions = await open(
      lspCard({ action: "code_actions", file: "src/lib.rs", line: 3 }, "1 code action(s):\n  0: [quickfix] Use u32"),
    );
    expect(texts(actions, "lsp-action")).toEqual(["0quickfixUse u32"]);
  });
});

describe("todo card", () => {
  // spec: UI-050, UI-051
  test("streaming: no list yet; success: the checklist", async () => {
    expect(texts(await open(streaming("todo_tool_calls", "call_todo_1")), "todo-task")).toEqual([]);
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
    expect(texts(element, "ask-question")).toEqual(["Which database?SQLite", "Which features?Auth, Search"]);
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

  // spec: UI-474
  test("a click or Enter on a thumbnail opens the image at full size; Escape closes it", async () => {
    const element = show(imageCard(1));
    expect(screen.queryAllByTestId("tool-image-dialog").length).toBe(0);
    await userEvent.click(byTestId(element, "tool-image"));
    const dialog = screen.getByTestId("tool-image-dialog");
    expect(within(dialog).getByRole("heading").textContent).toBe("Image from read /w/plot.png");
    const full = within(dialog).getByTestId("tool-image-full");
    expect([full.getAttribute("src"), full.getAttribute("alt")]).toEqual([
      "data:image/png;base64,iVBORw0KGgo=",
      "Image from read /w/plot.png",
    ]);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryAllByTestId("tool-image-dialog").length).toBe(0);
    // The focus is back on the thumbnail: Enter opens it again.
    expect(document.activeElement?.getAttribute("data-testid")).toBe("tool-image");
    await userEvent.keyboard("{Enter}");
    expect(screen.queryAllByTestId("tool-image-dialog").length).toBe(1);
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
      <MessageRow cardKey="m3" streaming continues={false} view={readsReply(reads)} />
    </>
  );
}
