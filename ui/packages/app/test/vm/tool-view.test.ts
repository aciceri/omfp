/**
 * L0: the specialized tool cards (UI-050..052) from fixtures recorded from the pinned omp (UI-051): each card in its
 * streaming (before `tool_execution_end`), success and error states.
 */

import { describe, expect, test } from "bun:test";
import { replay } from "@omfp/core";
import { expandedByDefault, type ToolCard } from "../../src/vm/tool.ts";
import { ompDiffLines, repoPath, type ToolView, toolView } from "../../src/vm/tool-view.ts";
import { card, cards, streaming } from "../support/cards.ts";
import { repo, workspace } from "../support/fixtures.ts";
import { fixtureEvents, isFrame } from "../widgets/chat/fixtures.ts";

function view<K extends ToolView["kind"]>(found: ToolCard, kind: K): Extract<ToolView, { kind: K }> {
  expect(found.view.kind).toBe(kind);
  return found.view as Extract<ToolView, { kind: K }>;
}

describe("bash card", () => {
  // spec: UI-050, UI-051
  test("streaming: the command and the output so far", () => {
    const running = streaming("bash_tool_call", "call_bash_1");
    expect(running.status).toBe("running");
    expect(view(running, "bash")).toEqual({
      kind: "bash",
      command: "echo tool-output-123",
      cwd: undefined,
      output: "tool-output-123\n",
      exitCode: undefined,
      background: undefined,
    });
  });

  // spec: UI-050, UI-051
  test("success: the output and exit code 0", () => {
    const done = card("bash_tool_call", "call_bash_1");
    expect([done.status, expandedByDefault(done)]).toEqual(["succeeded", false]);
    expect(view(done, "bash")).toMatchObject({ output: "tool-output-123\n", exitCode: 0 });
  });

  // spec: UI-050, UI-051, UI-052
  test("error: the exit code from omp's details, expanded by default", () => {
    const failed = card("bash_tool_error", "call_bash_1");
    expect([failed.status, expandedByDefault(failed)]).toEqual(["failed", true]);
    expect(view(failed, "bash")).toMatchObject({ command: "echo oops >&2; exit 3", exitCode: 3 });
    expect(view(failed, "bash").output).toStartWith("oops\n");
    expect(view(streaming("bash_tool_error", "call_bash_1"), "bash").exitCode).toBe(3);
  });

  // spec: UI-050, UI-052
  test("aborted: omp's error result has no exit code, so the card shows none", () => {
    const output = { text: "Command aborted", images: [], details: {} };
    const aborted = toolView("bash", { command: "sleep 3" }, output, false);
    expect(aborted).toMatchObject({ output: "Command aborted", exitCode: undefined });
  });
});

describe("read card", () => {
  // spec: UI-050, UI-051
  test("streaming: the path, no content yet", () => {
    const running = streaming("read_tool_calls", "call_read_1");
    expect(running.status).toBe("running");
    expect(view(running, "read")).toEqual({
      kind: "read",
      path: "src/lib.rs",
      selector: undefined,
      range: undefined,
      line: undefined,
      language: "rust",
      content: undefined,
      directory: false,
    });
  });

  // spec: UI-050, UI-051, UI-116
  test("success: the lines returned, without omp's line prefixes, and a directory listing", () => {
    const read = view(card("read_tool_calls", "call_read_1"), "read");
    expect(read.range).toBe("1–9 of 9");
    // The file opens at the first line read.
    expect(read.line).toBe(1);
    expect(read.content?.split("\n")[0]).toBe("//! Tool card fixture crate.");
    expect(read.content).not.toContain("#72CA");
    const listing = view(card("read_tool_calls", "call_read_3"), "read");
    expect([listing.path, listing.directory]).toEqual(["src", true]);
    expect(listing.content).toContain("lib.rs");
  });

  // spec: UI-050, UI-051, UI-052
  test("error: a missing file fails with omp's message", () => {
    const failed = card("read_tool_calls", "call_read_2");
    expect([failed.status, failed.result]).toEqual(["failed", "Path 'src/missing.rs' not found"]);
    expect(view(failed, "read").path).toBe("src/missing.rs");
  });
});

describe("fetch card (read of a URL)", () => {
  // spec: UI-050, UI-051
  test("streaming and success: the URL, then the page", () => {
    expect(view(streaming("web_tool_calls", "call_read_1"), "fetch")).toEqual({
      kind: "fetch",
      url: "http://127.0.0.1:29417/page",
      content: undefined,
      notes: [],
    });
    const done = card("web_tool_calls", "call_read_1");
    expect(done.status).toBe("succeeded");
    expect(view(done, "fetch").content).toContain("# Tool cards");
  });

  // spec: UI-050, UI-051, UI-052
  test("error: a failed fetch (omp says `isError: false`) and a bad selector both fail", () => {
    const missing = card("web_tool_calls", "call_read_2");
    expect(missing.status).toBe("failed");
    expect(view(missing, "fetch").notes).toEqual(["Failed to fetch URL (HTTP 404)"]);
    const selector = card("web_tool_calls", "call_read_3");
    expect(selector.status).toBe("failed");
    expect(selector.result).toContain("multiple range groups");
  });
});

describe("edit card", () => {
  // spec: UI-050, UI-051
  test("streaming: the target file from the edit's header and the edit itself", () => {
    const running = streaming("edit_tool_calls", "call_edit_1");
    expect(running.status).toBe("running");
    const edit = view(running, "edit");
    expect([edit.path, edit.diff, edit.language]).toEqual(["src/lib.rs", undefined, "rust"]);
    expect(edit.input).toContain("PUT 4.=4:");
  });

  // spec: UI-050, UI-051
  test("success: omp's diff as numbered lines", () => {
    expect(view(card("edit_tool_calls", "call_edit_1"), "edit").diff).toEqual([
      { kind: "context", text: "", newLine: 2 },
      { kind: "context", text: "pub fn add(left: u64, right: u64) -> u64 {", newLine: 3 },
      { kind: "del", text: "    left + right", oldLine: 4 },
      { kind: "add", text: "    left.saturating_add(right)", newLine: 4 },
      { kind: "context", text: "}", newLine: 5 },
      { kind: "context", text: "", newLine: 6 },
    ]);
  });

  // spec: UI-050, UI-051, UI-052
  test("error: a refused edit keeps its input and omp's reason", () => {
    const failed = card("edit_tool_calls", "call_edit_2");
    expect(failed.status).toBe("failed");
    expect(failed.result).toContain("is not from this session");
    const edit = view(failed, "edit");
    expect([edit.path, edit.diff]).toEqual(["src/lib.rs", undefined]);
    expect(ompDiffLines("...\n+3|x")).toEqual([
      { kind: "hunk", text: "..." },
      { kind: "add", text: "x", newLine: 3 },
    ]);
  });
});

describe("write card", () => {
  // spec: UI-050, UI-051
  test("streaming and success: the path and the written content", () => {
    const running = streaming("write_tool_calls", "call_write_1");
    expect(running.status).toBe("running");
    expect(running.result).toBe("Writing 44 bytes to notes.md...");
    expect(view(card("write_tool_calls", "call_write_1"), "write")).toEqual({
      kind: "write",
      path: "notes.md",
      content: "# Notes\n\n- add saturates\n- greet says hello\n",
      language: "markdown",
    });
  });

  // spec: UI-050, UI-051, UI-052
  test("error: writing over a directory fails", () => {
    const failed = card("write_tool_calls", "call_write_2");
    expect(failed.status).toBe("failed");
    expect(failed.result).toContain("EISDIR");
    expect(view(failed, "write").path).toBe("src");
  });
});

describe("search cards", () => {
  // spec: UI-050, UI-051
  test("streaming: the query and where it searches", () => {
    expect(view(streaming("search_tool_calls", "call_grep_1"), "search")).toEqual({
      kind: "search",
      query: "pub fn",
      scope: "src",
      files: undefined,
      total: undefined,
      truncated: false,
    });
    // glob streams the files it found.
    expect(view(streaming("search_tool_calls", "call_glob_1"), "search").files).toEqual([
      { path: "src/lib.rs", count: undefined },
    ]);
  });

  // spec: UI-050, UI-051
  test("success: matching files with their counts", () => {
    expect(view(card("search_tool_calls", "call_grep_1"), "search")).toMatchObject({
      files: [{ path: "src/lib.rs", count: 2 }],
      total: "2 matches in 1 file",
    });
    expect(view(card("search_tool_calls", "call_glob_1"), "search")).toMatchObject({
      query: "src/**/*.rs",
      scope: "src",
      files: [{ path: "src/lib.rs", count: undefined }],
      total: "1 file",
    });
  });

  // spec: UI-050, UI-051, UI-052
  test("error: a missing folder fails", () => {
    for (const id of ["call_grep_2", "call_glob_2"]) {
      const failed = card("search_tool_calls", id);
      expect([failed.status, failed.result]).toEqual(["failed", "Path not found: missing"]);
    }
  });
});

describe("todo card", () => {
  // spec: UI-050, UI-051
  test("streaming: no list until omp answers", () => {
    const running = streaming("todo_tool_calls", "call_todo_1");
    expect(running.status).toBe("running");
    expect(view(running, "todo")).toEqual({ kind: "todo", op: "init", phases: [] });
  });

  // spec: UI-050, UI-051
  test("success: the checklist after the call", () => {
    expect(view(card("todo_tool_calls", "call_todo_3"), "todo").phases).toEqual([
      {
        name: "Tasks",
        tasks: [
          { content: "Read the code", status: "completed" },
          { content: "Fix the bug", status: "in_progress" },
        ],
      },
    ]);
  });

  // spec: UI-050, UI-051, UI-052
  test("error: an unknown task fails and still shows the list", () => {
    const failed = card("todo_tool_calls", "call_todo_2");
    expect(failed.status).toBe("failed");
    expect(failed.result).toStartWith('Errors: Task "Write the docs" not found');
    expect(view(failed, "todo").phases[0]?.tasks.length).toBe(2);
  });
});

describe("web_search card", () => {
  // spec: UI-050, UI-051
  test("streaming and success: the query, then the links", () => {
    expect(view(streaming("web_tool_calls", "call_web_search_1"), "web_search")).toEqual({
      kind: "web_search",
      query: "omfp tool cards",
      links: [],
    });
    expect(view(card("web_tool_calls", "call_web_search_1"), "web_search").links).toEqual([
      { title: "Tool cards", url: "http://127.0.0.1:29417/page", snippet: "How the transcript shows tool calls." },
      { title: "omp tools", url: "http://127.0.0.1/tools", snippet: "Reference of the agent's tools." },
    ]);
  });

  // spec: UI-050, UI-051, UI-052
  test("error: an engine failure (omp says `isError: false`) fails the card", () => {
    const failed = card("web_tool_calls", "call_web_search_2");
    expect([failed.status, expandedByDefault(failed)]).toEqual(["failed", true]);
    expect(failed.result).toContain("SearXNG API error (500)");
  });
});

describe("ask card", () => {
  // spec: UI-050, UI-051
  test("streaming: the questions of the form, unanswered", () => {
    expect(view(streaming("ask_answered", "call_ask_1"), "ask").questions).toEqual([
      { question: "Which database?", answer: undefined },
      { question: "Which features?", answer: undefined },
    ]);
  });

  // spec: UI-050, UI-051
  test("success: each question with the given answer", () => {
    expect(view(card("ask_answered", "call_ask_1"), "ask").questions).toEqual([
      { question: "Which database?", answer: "SQLite" },
      { question: "Which features?", answer: "" },
    ]);
  });

  // spec: UI-050, UI-051, UI-052
  test("error: a cancelled form keeps its questions", () => {
    const failed = card("ask_cancelled", "call_ask_1");
    expect([failed.status, failed.result]).toEqual(["failed", "Ask tool was cancelled by the user"]);
    expect(view(failed, "ask").questions).toEqual([{ question: "Which database?", answer: undefined }]);
  });
});

describe("task card", () => {
  // spec: UI-050, UI-051
  test("streaming: each subagent's name, task and live progress", () => {
    const events = fixtureEvents("task_tool_call");
    const updates = events.flatMap((event, index) => (isFrame("tool_execution_update")(event) ? [index] : []));
    const at = (count: number) => {
      const found = cards(replay(events.slice(0, count))).find((candidate) => candidate.toolCallId === "call_task_1");
      return view(found ?? card("task_tool_call", "missing"), "task");
    };
    // Started, before omp's first progress frame: the subagent is pending.
    const started = at(updates[0] ?? 0);
    expect(started.context).toBe("The workspace holds a small Rust library.");
    expect(started.agents).toEqual([
      {
        name: "Counter",
        agent: "task",
        task: "Count the functions in src/lib.rs",
        status: "pending",
        activity: undefined,
        tools: 0,
        output: undefined,
      },
    ]);
    // While its first tool runs, then once it wrote its output.
    expect(at((updates[1] ?? 0) + 1).agents[0]).toMatchObject({ status: "running", activity: "Reading the library" });
    const running = streaming("task_tool_call", "call_task_1");
    expect(running.status).toBe("running");
    expect(view(running, "task").agents[0]).toMatchObject({
      tools: 1,
      output: "src/lib.rs defines 2 functions: add and greet.",
    });
  });

  // spec: UI-050, UI-051
  test("success: the subagent's result", () => {
    const [agent] = view(card("task_tool_call", "call_task_1"), "task").agents;
    expect([agent?.status, agent?.output]).toEqual(["completed", "src/lib.rs defines 2 functions: add and greet."]);
  });

  // spec: UI-050, UI-051, UI-052, UI-406
  test("error: an unknown agent fails before it starts, and is not left pending", () => {
    const failed = card("task_tool_call", "call_task_2");
    expect(failed.status).toBe("failed");
    expect(failed.result).toContain('Unknown agent "no-such-agent"');
    expect(view(failed, "task").agents).toMatchObject([{ name: "Ghost", agent: "no-such-agent", status: "failed" }]);
  });
});

describe("paths in repositories", () => {
  // spec: UI-050
  test("a path in one of the workspace's repositories is shown relative to it with its name", () => {
    const ws = workspace("w1", [repo("w1", "fe", "main"), repo("w1", "be", "main")]);
    expect(repoPath("fe/src/app.ts", ws)).toEqual({ repo: "fe", path: "src/app.ts" });
    expect(repoPath("/work/w1/be/lib.rs", ws)).toEqual({ repo: "be", path: "lib.rs" });
    expect(repoPath("./notes.md", ws)).toEqual({ repo: undefined, path: "notes.md" });
    expect(repoPath("/etc/hosts", ws)).toEqual({ repo: undefined, path: "/etc/hosts" });
    expect(repoPath("fe/x", undefined)).toEqual({ repo: undefined, path: "fe/x" });
    const rooted = workspace("w2", [{ ...repo("w2", "w2", "main"), isRoot: true }]);
    expect(repoPath("src/a.rs", rooted)).toEqual({ repo: "w2", path: "src/a.rs" });
  });
});
