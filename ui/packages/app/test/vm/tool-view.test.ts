/**
 * L0: the specialized tool cards (UI-050..052) from fixtures recorded from the pinned omp (UI-051): each card in its
 * streaming (before `tool_execution_end`), success and error states.
 */

import { describe, expect, test } from "bun:test";
import { type JsonObject, replay, type ThreadEvent, type ToolRun } from "@omfp/core";
import { isObject } from "../../src/sdk/json.ts";
import { astMatches } from "../../src/vm/code-intel.ts";
import { expandedByDefault, type ToolCard, toolCard } from "../../src/vm/tool.ts";
import { deviceCall, ompDiffLines, quietFailure, repoPath, type ToolView, toolView } from "../../src/vm/tool-view.ts";
import { card, cards, deviceCard, lspCard, omfpCard, streaming } from "../support/cards.ts";
import { repo, workspace } from "../support/fixtures.ts";
import { fixtureEvents, isFrame } from "../widgets/chat/fixtures.ts";

function view<K extends ToolView["kind"]>(found: ToolCard, kind: K): Extract<ToolView, { kind: K }> {
  expect(found.view.kind).toBe(kind);
  return found.view as Extract<ToolView, { kind: K }>;
}

/** The `toolcall_end` of call `id`: the model finished writing its arguments. */
function argumentsEnd(id: string): (event: ThreadEvent) => boolean {
  return (event) => {
    if (event.kind !== "omp" || !isFrame("message_update", "toolcall_end")(event) || !isObject(event.frame)) {
      return false;
    }
    const update = event.frame.assistantMessageEvent;
    return isObject(update) && isObject(update.toolCall) && update.toolCall.id === id;
  };
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
  // spec: UI-050, UI-051, UI-055
  test("streaming: the target file from the edit's header, the edit itself and omp's preview of its diff", () => {
    const running = streaming("edit_tool_calls", "call_edit_1");
    expect(running.status).toBe("running");
    const edit = view(running, "edit");
    expect([edit.path, edit.diff, edit.language]).toEqual(["src/lib.rs", undefined, "rust"]);
    expect(edit.input).toContain("PUT 4.=4:");
    expect(edit.preview).toEqual([
      {
        path: "src/lib.rs",
        rename: undefined,
        diff: [
          { kind: "del", text: "    left + right", oldLine: 4 },
          { kind: "add", text: "    left.saturating_add(right)", newLine: 4 },
        ],
        error: undefined,
        language: "rust",
      },
    ]);
  });

  // spec: UI-054, UI-055, UI-051
  test("recorded: while the model writes the edit, the file and the edit so far; the arguments' end keeps them", () => {
    const events = fixtureEvents("edit_tool_calls");
    const end = events.findIndex(argumentsEnd("call_edit_1"));
    const at = (count: number) => {
      const found = cards(replay(events.slice(0, count))).find((candidate) => candidate.toolCallId === "call_edit_1");
      if (found === undefined) {
        throw new Error(`no edit card after ${count} events`);
      }
      return found;
    };
    // Three deltas before the end: the text stops inside the edit's body; no preview from omp yet.
    const composing = at(end - 3);
    expect([composing.status, composing.summary]).toEqual(["composing", "Saturating the addition"]);
    expect(view(composing, "edit")).toMatchObject({
      path: "src/lib.rs",
      preview: [],
      input: "[src/lib.rs#72CA]\nPUT 4.=4:\n+    left.saturati",
    });
    const written = at(end + 1);
    expect([written.status, written.summary]).toEqual(["pending", "Saturating the addition"]);
    expect(view(written, "edit")).toMatchObject({
      path: "src/lib.rs",
      input: "[src/lib.rs#72CA]\nPUT 4.=4:\n+    left.saturating_add(right)\n",
    });
  });

  // spec: UI-055, UI-051
  test("recorded: a preview's error shows as such while the call runs; once it ended, its result replaces the preview", () => {
    const edit = view(streaming("edit_tool_calls", "call_edit_2"), "edit");
    expect(edit.preview.map((file) => [file.path, file.diff])).toEqual([["src/lib.rs", undefined]]);
    expect(edit.preview[0]?.error).toContain("is not from this session");
    expect(view(card("edit_tool_calls", "call_edit_2"), "edit").preview).toEqual([]);
    expect(view(card("edit_tool_calls", "call_edit_1"), "edit").preview).toEqual([]);
  });

  // spec: UI-055
  test("a preview of several files shows each; its first path heads the card until the edit names one", () => {
    const run: ToolRun = {
      tool_call_id: "c-preview",
      tool_name: "edit",
      args: null,
      intent: null,
      status: "pending",
      partial_result: null,
      preview: {
        files: [
          { path: "src/a.rs", diff: "-1|old\n+1|new", firstChangedLine: 1, op: "update" },
          { path: "src/b.rs", rename: "src/c.rs", error: "No line 9 in src/b.rs" },
        ],
        generation: 3,
        streaming: true,
      },
      result: null,
      background: null,
    };
    const call = { type: "toolCall" as const, name: "edit", arguments: {}, partialArgs: '{"input": "[src/a' };
    const composing = toolCard("c-preview", call, run, undefined, undefined);
    expect(composing.status).toBe("composing");
    expect(view(composing, "edit")).toEqual({
      kind: "edit",
      path: "src/a.rs",
      diff: undefined,
      preview: [
        {
          path: "src/a.rs",
          rename: undefined,
          diff: [
            { kind: "del", text: "old", oldLine: 1 },
            { kind: "add", text: "new", newLine: 1 },
          ],
          error: undefined,
          language: "rust",
        },
        { path: "src/b.rs", rename: "src/c.rs", diff: undefined, error: "No line 9 in src/b.rs", language: "rust" },
      ],
      input: "[src/a",
      language: "rust",
    });
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
      { path: "src/lib.rs", count: undefined, matches: [] },
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

describe("tool commands", () => {
  const bash = (toolCommands: unknown[]) => ({
    text: "out",
    images: [],
    details: { toolCommands },
  });

  // spec: UI-915
  test("a bash call running one tool command is a call of that tool, with its own arguments and result details", () => {
    const args = { action: "diagnostics", file: "src/a.ts" };
    const inner = { action: "diagnostics", serverName: "tsserver", success: true };
    const output = bash([{ tool: "lsp", mode: "execute", exitCode: 0, args, inner }]);
    expect(deviceCall("bash", { command: "lsp diagnostics src/a.ts" }, output)).toEqual({
      name: "lsp",
      args,
      output: { text: "out", images: [], details: inner },
      exitCode: 0,
    });
    // The exit line `bash` appends to a failed command is the bash call's, not the tool's.
    const failed = { ...output, text: "Path not found: missing\n\n\nCommand exited with code 1" };
    expect(deviceCall("bash", { command: "lsp diagnostics missing" }, failed)?.output?.text).toBe(
      "Path not found: missing",
    );
  });

  // spec: UI-915
  test("a tool command that failed fails its card, also when the bash script went on (`|| true`)", () => {
    const run = (exitCode: number): ToolRun => ({
      tool_call_id: "call_grep",
      tool_name: "bash",
      args: { command: "ast_grep -j '{}' || true" },
      intent: null,
      status: "succeeded",
      partial_result: null,
      preview: null,
      result: {
        content: [{ type: "text", text: "no such path: src" }],
        details: { toolCommands: [{ tool: "ast_grep", mode: "execute", exitCode, args: {}, inner: {} }] },
      },
      background: null,
    });
    expect(toolCard("call_grep", undefined, run(1), undefined, undefined).status).toBe("failed");
    expect(toolCard("call_grep", undefined, run(0), undefined, undefined).status).toBe("succeeded");
  });

  // spec: UI-915, UI-051, UI-054
  test("recorded: while the model writes the command it is a bash call; the dispatch in its result makes it the tool's", () => {
    const events = fixtureEvents("code_intel_tool_calls");
    const end = events.findIndex(isFrame("message_update", "toolcall_end"));
    const [composing] = cards(replay(events.slice(0, end)));
    expect([composing?.toolCallId, composing?.name, composing?.summary]).toEqual([
      "call_ast_grep_1",
      "bash",
      "Finding the functions",
    ]);
    expect(composing === undefined ? undefined : view(composing, "bash").command).toBe(
      `ast_grep -j '{"pat":"pub fn $NAME($$$ARGS) -> $RET { $$$BODY }","path":"src"}'`,
    );
    const running = streaming("code_intel_tool_calls", "call_ast_grep_1");
    expect([running.name, running.status, running.summary]).toEqual(["ast_grep", "running", "Finding the functions"]);
    expect(JSON.parse(running.args)).toEqual({ pat: "pub fn $NAME($$$ARGS) -> $RET { $$$BODY }", path: "src" });
    expect(view(running, "search")).toMatchObject({ query: "pub fn $NAME($$$ARGS) -> $RET { $$$BODY }", scope: "src" });
    const lsp = card("code_intel_tool_calls", "call_lsp_1");
    expect([lsp.name, view(lsp, "lsp").file]).toEqual(["lsp", "notes.txt"]);
  });

  // spec: UI-915
  test("several dispatches, help only or no details yet stay a bash call", () => {
    const run = { command: "lsp diagnostics a.ts; ast_grep x" };
    const lsp = { tool: "lsp", mode: "execute", exitCode: 0, args: {}, inner: {} };
    const grep = { tool: "ast_grep", mode: "execute", exitCode: 0, args: {}, inner: {} };
    expect(deviceCall("bash", run, bash([lsp, grep]))).toBeUndefined();
    expect(deviceCall("bash", run, bash([{ tool: "lsp", mode: "help", exitCode: 0 }]))).toBeUndefined();
    expect(deviceCall("bash", run, bash([]))).toBeUndefined();
    expect(deviceCall("bash", run, { text: "out", images: [], details: {} })).toBeUndefined();
    expect(deviceCall("bash", run, undefined)).toBeUndefined();
  });
});

describe("xd:// device calls (sessions recorded by omp 18.x)", () => {
  // spec: UI-915, UI-051
  test("a write to xd://<tool> is a call of that tool, with its own arguments and result details", () => {
    const args = { action: "diagnostics", file: "src/lib.rs" };
    const device = deviceCard("lsp", args, "OK", { action: "diagnostics", serverName: "rust-analyzer", success: true });
    expect([device.name, device.status, device.result, JSON.parse(device.args)]).toEqual([
      "lsp",
      "succeeded",
      "OK",
      args,
    ]);
    expect(view(device, "lsp").file).toBe("src/lib.rs");
  });

  // spec: UI-915, UI-054
  test("while the model writes the arguments the device is known from the path parsed so far; other writes stay writes", () => {
    const partialArgs = '{"path":"xd://lsp","content":"{\\"action\\":\\"dia';
    const partial = toolCard(
      "call_device_partial",
      { type: "toolCall", name: "write", arguments: {}, partialArgs },
      undefined,
      undefined,
      undefined,
    );
    expect([partial.name, partial.status, partial.view.kind]).toEqual(["lsp", "composing", "lsp"]);
    expect(deviceCall("write", { path: "xd://lsp", content: '{"action":"dia' }, undefined)).toEqual({
      name: "lsp",
      args: undefined,
      output: undefined,
    });
    expect(deviceCall("write", { path: "notes.md", content: "{}" }, undefined)).toBeUndefined();
    expect(deviceCall("write", { path: "xd://lsp/actions", content: "{}" }, undefined)).toBeUndefined();
    expect(deviceCall("read", { path: "xd://lsp" }, undefined)).toBeUndefined();
    // A device without a specialized card (an MCP tool) keeps the fallback card, with its own arguments.
    const mcp = deviceCall("write", { path: "xd://mcp__docs_search", content: '{"q":"tool cards"}' }, undefined);
    expect(mcp).toEqual({ name: "mcp__docs_search", args: { q: "tool cards" }, output: undefined });
    expect(toolView("mcp__docs_search", mcp?.args, undefined, false).kind).toBe("generic");
  });
});

describe("ast_grep card", () => {
  // spec: UI-916, UI-051
  test("success: each file's matches with their first line, code and captures", () => {
    expect(view(card("code_intel_tool_calls", "call_ast_grep_1"), "search")).toEqual({
      kind: "search",
      query: "pub fn $NAME($$$ARGS) -> $RET { $$$BODY }",
      scope: "src",
      files: [
        {
          path: "src/lib.rs",
          count: 2,
          matches: [
            {
              line: 3,
              code: "pub fn add(left: u64, right: u64) -> u64 {\n    left + right\n}",
              captures: "ARGS=[left: u64, ,, right: u64], BODY=[left + right], NAME=add, RET=u64",
            },
            {
              line: 7,
              code: 'pub fn greet(name: &str) -> String {\n    format!("Hello, {name}!")\n}',
              captures: 'ARGS=[name: &str], BODY=[format!("Hello, {name}!")], NAME=greet, RET=String',
            },
          ],
        },
      ],
      total: "2 matches in 1 file",
      truncated: false,
    });
  });

  // spec: UI-916
  test("grouped output: folders nest, tags and notes leave the path, a lone file has no header", () => {
    const display = "# src/\n## a.ts#1A2B\n*3│x()\n\n## lib/\n### b.ts\n*9│y(\n 10│)\n  meta: A=1";
    expect(Object.fromEntries(astMatches(display, undefined))).toEqual({
      "src/a.ts": [{ line: 3, code: "x()", captures: undefined }],
      "src/lib/b.ts": [{ line: 9, code: "y(\n)", captures: "A=1" }],
    });
    expect(Object.fromEntries(astMatches("*4:z", "src/c.ts"))).toEqual({
      "src/c.ts": [{ line: 4, code: "z", captures: undefined }],
    });
  });

  // spec: UI-916, UI-052
  test("error: a missing folder fails with omp's message", () => {
    const failed = card("code_intel_tool_calls", "call_ast_grep_2");
    expect([failed.name, failed.status, failed.result]).toEqual(["ast_grep", "failed", "Path not found: missing"]);
  });
});

describe("ast_edit and resolve cards", () => {
  // spec: UI-917, UI-051
  test("the preview: each file's replacements as a diff, not applied yet", () => {
    expect(view(card("code_intel_tool_calls", "call_ast_edit_1"), "ast_edit")).toEqual({
      kind: "ast_edit",
      pattern: "left + right",
      files: [
        {
          path: "src/lib.rs",
          count: 1,
          diff: [
            { kind: "del", text: "left + right", oldLine: 4 },
            { kind: "add", text: "left.saturating_add(right)", newLine: 4 },
          ],
        },
      ],
      total: "1 replacement in 1 file",
      applied: false,
      truncated: false,
    });
  });

  // spec: UI-917, UI-051
  test("resolving it: what was applied and the files it changed", () => {
    const resolved = card("code_intel_tool_calls", "call_resolve_1");
    expect(resolved.name).toBe("resolve");
    expect(view(resolved, "resolve")).toEqual({
      kind: "resolve",
      action: "apply",
      label: "AST Edit: 1 replacement in 1 file",
      reason: "Saturating is safer.",
      files: [{ path: "src/lib.rs", note: "1 replacement", edits: undefined, removed: false }],
    });
  });
});

describe("lsp card", () => {
  // spec: UI-918, UI-051
  test("recorded: no language server for the file, with the hint about the daemon's environment", () => {
    const found = card("code_intel_tool_calls", "call_lsp_1");
    expect(found.status).toBe("succeeded");
    expect(view(found, "lsp")).toEqual({
      kind: "lsp",
      action: "diagnostics",
      file: "notes.txt",
      line: undefined,
      symbol: undefined,
      result: {
        type: "diagnostics",
        diagnostics: [],
        notes: ["notes.txt: No language server handles .txt files"],
        clean: false,
      },
      noServer: true,
    });
  });

  const lib = { path: "src/lib.rs", line: 4, column: 5 };

  // spec: UI-918
  test("diagnostics of one file: each with its severity, location, source and code", () => {
    const answer = [
      "1 error(s), 1 warning(s):",
      "# src/",
      "## lib.rs",
      "  4:5 [error] [rustc] mismatched types (E0308)",
      "  9:9 [warning] unused variable: `x`",
    ].join("\n");
    expect(view(lspCard({ action: "diagnostics", file: "src/lib.rs" }, answer), "lsp").result).toEqual({
      type: "diagnostics",
      diagnostics: [
        { ...lib, severity: "error", source: "rustc", code: "E0308", message: "mismatched types" },
        {
          path: "src/lib.rs",
          line: 9,
          column: 9,
          severity: "warning",
          source: undefined,
          code: undefined,
          message: "unused variable: `x`",
        },
      ],
      notes: [],
      clean: false,
    });
    expect(view(lspCard({ action: "diagnostics", file: "src/lib.rs" }, "OK"), "lsp").result).toEqual({
      type: "diagnostics",
      diagnostics: [],
      notes: [],
      clean: true,
    });
  });

  // spec: UI-918
  test("diagnostics of several files: their status rows become notes, their counts are dropped", () => {
    const answer = [
      "✘ src/a.ts: 1 error(s)",
      "# src/a.ts",
      "  3:7 [error] [ts] Cannot find name 'x'. (2304)",
      "✓ src/b.ts: no issues",
      "⚠ src/c.ts: no diagnostics report yet from tsserver; query the file alone for a longer wait",
    ].join("\n");
    expect(view(lspCard({ action: "diagnostics", file: "src/*.ts" }, answer), "lsp").result).toEqual({
      type: "diagnostics",
      diagnostics: [
        {
          path: "src/a.ts",
          line: 3,
          column: 7,
          severity: "error",
          source: "ts",
          code: "2304",
          message: "Cannot find name 'x'.",
        },
      ],
      notes: ["src/c.ts: no diagnostics report yet from tsserver; query the file alone for a longer wait"],
      clean: false,
    });
  });

  // spec: UI-918
  test("no issues only when every file was checked: a pending, failed or serverless file keeps it incomplete", () => {
    const diagnosticsOf = (answer: string) =>
      view(lspCard({ action: "diagnostics", file: "src/*.ts" }, answer), "lsp").result;
    expect(diagnosticsOf("✓ src/a.ts: no issues\n✓ src/b.ts: no issues")).toEqual({
      type: "diagnostics",
      diagnostics: [],
      notes: [],
      clean: true,
    });
    for (const unchecked of [
      "⚠ src/b.ts: no diagnostics report yet from tsserver; query the file alone for a longer wait",
      "✘ src/b.ts: all language servers failed (tsserver)",
      "✘ src/b.ts: No language server found",
    ]) {
      expect(diagnosticsOf(`✓ src/a.ts: no issues\n${unchecked}`)).toEqual({
        type: "diagnostics",
        diagnostics: [],
        notes: [unchecked.slice(2)],
        clean: false,
      });
    }
  });

  // spec: UI-918, UI-052
  test("a failed call (details.success false) is a failed card with omp's text", () => {
    const failed = lspCard({ action: "hover", file: "src/lib.rs", line: 4 }, "LSP error: request timed out", false);
    expect([failed.status, failed.result]).toEqual(["failed", "LSP error: request timed out"]);
    expect(quietFailure("lsp", { text: "fine", images: [], details: { success: true } })).toBeUndefined();
  });

  // spec: UI-919
  test("definitions and references: locations with their source; the ones past the context limit too", () => {
    const answer = [
      "Found 2 reference(s):",
      "  src/lib.rs:4:5",
      "    3: pub fn add(left: u64, right: u64) -> u64 {",
      "    4:     left + right",
      "  ... 1 additional reference(s) shown without context",
      "  src/main.rs:2:9",
    ].join("\n");
    const found = view(lspCard({ action: "references", file: "src/lib.rs", line: 4, symbol: "left" }, answer), "lsp");
    expect([found.file, found.line, found.symbol]).toEqual(["src/lib.rs", 4, "left"]);
    expect(found.result).toEqual({
      type: "locations",
      title: "Found 2 reference(s)",
      locations: [
        {
          ...lib,
          context: [
            { line: 3, text: "pub fn add(left: u64, right: u64) -> u64 {" },
            { line: 4, text: "    left + right" },
          ],
        },
        { path: "src/main.rs", line: 2, column: 9, context: [] },
      ],
      notes: ["... 1 additional reference(s) shown without context"],
    });
    expect(
      view(lspCard({ action: "definition", file: "src/lib.rs", line: 4 }, "No definition found"), "lsp").result,
    ).toEqual({
      type: "text",
      text: "No definition found",
    });
  });

  // spec: UI-919
  test("symbols of a file and of the workspace, at their lines", () => {
    const file = view(
      lspCard(
        { action: "symbols", file: "src/lib.rs" },
        "Symbols in src/lib.rs:\n◆ Calc @ line 1\n  ƒ add(left, right) @ line 3",
      ),
      "lsp",
    );
    expect(file.result).toEqual({
      type: "symbols",
      title: "Symbols in src/lib.rs",
      workspace: false,
      symbols: [
        { depth: 0, name: "Calc", path: "src/lib.rs", line: 1 },
        { depth: 1, name: "add(left, right)", path: "src/lib.rs", line: 3 },
      ],
    });
    const workspace = view(
      lspCard(
        { action: "symbols", file: "*", query: "add" },
        'Found 1 symbol(s) matching "add":\n  ƒ add (Calc) @ src/lib.rs:3:8',
      ),
      "lsp",
    );
    expect(workspace.result).toEqual({
      type: "symbols",
      title: 'Found 1 symbol(s) matching "add"',
      workspace: true,
      symbols: [{ depth: 0, name: "add (Calc)", path: "src/lib.rs", line: 3 }],
    });
  });

  // spec: UI-919
  test("renames and applied code actions: the files they changed and the commands they ran", () => {
    const applied = "Applied rename:\n  Applied 2 edit(s) to src/lib.rs\n  Applied 1 edit(s) to src/main.rs";
    expect(
      view(lspCard({ action: "rename", file: "src/lib.rs", line: 3, new_name: "sum" }, applied), "lsp").result,
    ).toEqual({
      type: "edits",
      title: "Applied rename",
      applied: true,
      rename: { from: undefined, to: "sum" },
      total: "3 edits in 2 files",
      files: [
        { path: "src/lib.rs", note: "2 edits", edits: 2, removed: false },
        { path: "src/main.rs", note: "1 edit", edits: 1, removed: false },
      ],
      commands: [],
      notes: [],
    });
    const preview = "Rename preview:\n  src/lib.rs: 2 edits\n  RENAME: src/old.rs ❯ src/new.rs\n  DELETE: src/gone.rs";
    expect(
      view(lspCard({ action: "rename", file: "src/lib.rs", line: 3, new_name: "sum", apply: false }, preview), "lsp")
        .result,
    ).toEqual({
      type: "edits",
      title: "Rename preview",
      applied: false,
      rename: { from: undefined, to: "sum" },
      total: "2 edits in 1 file",
      files: [
        { path: "src/lib.rs", note: "2 edits", edits: 2, removed: false },
        { path: "src/new.rs", note: "renamed from src/old.rs", edits: undefined, removed: false },
        { path: "src/gone.rs", note: "deleted", edits: undefined, removed: true },
      ],
      commands: [],
      notes: [],
    });
    const action =
      'Applied "Add import":\n  Workspace edit:\n    Applied 1 edit(s) to src/a.ts\n  Executed command(s):\n    _typescript.applyFix';
    expect(
      view(lspCard({ action: "code_actions", file: "src/a.ts", line: 1, query: "0", apply: true }, action), "lsp")
        .result,
    ).toEqual({
      type: "edits",
      title: 'Applied "Add import"',
      applied: true,
      rename: undefined,
      total: "1 edit in 1 file",
      files: [{ path: "src/a.ts", note: "1 edit", edits: 1, removed: false }],
      commands: ["_typescript.applyFix"],
      notes: [],
    });
  });

  // spec: UI-919
  test("a symbol rename: its old and new names, each file's edits and the total, as omp 19.1.0 words them", () => {
    // `textDocument/rename` answered with a `changes` map, previewed (`formatWorkspaceEdit`: `N edit`/`N edits`) and
    // applied (`applyWorkspaceEdit`: `Applied N edit(s) to`); `symbol` names the old name, `#N` its occurrence.
    const args = { action: "rename", file: "src/lib.rs", line: 3, symbol: "add#2", new_name: "sum" };
    const files = [
      { path: "src/lib.rs", note: "3 edits", edits: 3, removed: false },
      { path: "src/main.rs", note: "1 edit", edits: 1, removed: false },
      { path: "tests/calc test.rs", note: "2 edits", edits: 2, removed: false },
    ];
    const preview = "Rename preview:\n  src/lib.rs: 3 edits\n  src/main.rs: 1 edit\n  tests/calc test.rs: 2 edits";
    expect(view(lspCard({ ...args, apply: false }, preview), "lsp").result).toEqual({
      type: "edits",
      title: "Rename preview",
      applied: false,
      rename: { from: "add", to: "sum" },
      total: "6 edits in 3 files",
      files,
      commands: [],
      notes: [],
    });
    const applied =
      "Applied rename:\n  Applied 3 edit(s) to src/lib.rs\n  Applied 1 edit(s) to src/main.rs\n  Applied 2 edit(s) to tests/calc test.rs";
    expect(view(lspCard(args, applied), "lsp").result).toMatchObject({
      applied: true,
      rename: { from: "add", to: "sum" },
      total: "6 edits in 3 files",
      files,
    });
    expect(view(lspCard(args, "Rename returned no edits"), "lsp").result).toEqual({
      type: "text",
      text: "Rename returned no edits",
    });
  });

  // spec: UI-919
  test("renamed files keep paths with spaces, split at omp's separator (→ applied, the theme's cursor in a preview)", () => {
    const filesOf = (answer: string) => {
      const result = view(lspCard({ action: "rename_file", file: "a b.ts", new_name: "c d.ts" }, answer), "lsp").result;
      return result?.type === "edits" ? result.files : [];
    };
    const renamed = [{ path: "src/c d.ts", note: "renamed from src/a b.ts", edits: undefined, removed: false }];
    expect(filesOf("Renamed 1 file → src/c d.ts\n  Renamed src/a b.ts → src/c d.ts")).toEqual(renamed);
    expect(filesOf("Applied rename:\n  Renamed src/a b.ts → src/c d.ts")).toEqual(renamed);
    for (const cursor of ["❯", "\uf054", ">"]) {
      expect(filesOf(`Rename preview:\n  RENAME: src/a b.ts ${cursor} src/c d.ts`)).toEqual(renamed);
    }
  });

  // spec: UI-919
  test("code actions offered, hover as Markdown, anything else as text", () => {
    const offered =
      "2 code action(s):\n  0: [quickfix] Add import (preferred)\n  1: [source.organizeImports] Organize imports";
    expect(view(lspCard({ action: "code_actions", file: "src/a.ts", line: 1 }, offered), "lsp").result).toEqual({
      type: "actions",
      title: "2 code action(s)",
      actions: [
        { index: 0, kind: "quickfix", title: "Add import (preferred)" },
        { index: 1, kind: "source.organizeImports", title: "Organize imports" },
      ],
    });
    const hover = "```rust\nfn add(left: u64, right: u64) -> u64\n```\n\nAdds.";
    expect(view(lspCard({ action: "hover", file: "src/lib.rs", line: 3 }, hover), "lsp").result).toEqual({
      type: "hover",
      markdown: hover,
    });
    expect(view(lspCard({ action: "status" }, "rust-analyzer: ready"), "lsp").result).toEqual({
      type: "text",
      text: "rust-analyzer: ready",
    });
  });
});

describe("todo card", () => {
  // spec: UI-050, UI-051, UI-054
  test("streaming: until omp answers, the tasks the call gives, pending", () => {
    const running = streaming("todo_tool_calls", "call_todo_1");
    expect(running.status).toBe("running");
    expect(view(running, "todo")).toEqual({
      kind: "todo",
      op: "init",
      phases: [
        {
          name: "Tasks",
          tasks: [
            { content: "Read the code", status: "pending" },
            { content: "Fix the bug", status: "pending" },
          ],
        },
      ],
    });
  });

  // spec: UI-054
  test("an init's phases (`list`) or an append's items, as omp reads them; other calls, or ended ones, list none", () => {
    const phases = (args: JsonObject, ended = false) => {
      const shown = toolView("todo", args, undefined, false, null, ended);
      return shown.kind === "todo" ? shown.phases : undefined;
    };
    const list = [
      { phase: "Build", items: ["Compile"] },
      { phase: "", items: ["Ship"] },
    ];
    expect(phases({ op: "init", list, items: [] })).toEqual([
      { name: "Build", tasks: [{ content: "Compile", status: "pending" }] },
      { name: "Tasks", tasks: [{ content: "Ship", status: "pending" }] },
    ]);
    expect(phases({ op: "append", phase: "Build", items: ["Test"] })).toEqual([
      { name: "Build", tasks: [{ content: "Test", status: "pending" }] },
    ]);
    expect(phases({ op: "done", task: "Compile" })).toEqual([]);
    expect(phases({ op: "init", items: ["Compile"] }, true)).toEqual([]);
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
  test("streaming: the questions of the form (fieldset legends, labels outside fieldsets), unanswered", () => {
    expect(view(streaming("ask_answered", "call_ask_1"), "ask").questions).toEqual([
      { question: "Which database?", answer: undefined },
      { question: "Which features?", answer: undefined },
      { question: "Service name", answer: undefined },
      { question: "Replicas", answer: undefined },
    ]);
  });

  // spec: UI-050, UI-051
  test("success: each question with the given answer", () => {
    expect(view(card("ask_answered", "call_ask_1"), "ask").questions).toEqual([
      { question: "Which database?", answer: "Postgres" },
      { question: "Which features?", answer: "Auth, Other: Elasticsearch" },
      { question: "Service name", answer: "orders" },
      { question: "Replicas", answer: "3" },
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

describe("card of omfp_card", () => {
  // spec: UI-920, AGT-096
  test("a bare widget name is the workspace's widget; other references, props and the title stay as given", () => {
    const shown = '{"status":"shown"}';
    const bare = omfpCard({ widget: "chart", props: { points: [1, 2] }, title: "Load" }, "succeeded", shown);
    expect(view(bare, "card")).toEqual({
      kind: "card",
      reference: "./widgets/chart",
      code: undefined,
      props: { points: [1, 2] },
      title: "Load",
      wait: false,
      answer: { state: "none", value: undefined },
    });
    for (const reference of ["./widgets/clock", "omfp:file", "./widgets/tests"]) {
      expect(view(omfpCard({ widget: reference }, "succeeded", shown), "card").reference).toBe(reference);
    }
    const code = view(omfpCard({ code: "export default () => null;" }, "succeeded", shown), "card");
    expect([code.reference, code.code, code.props, code.title]).toEqual([
      undefined,
      "export default () => null;",
      {},
      undefined,
    ]);
  });

  // spec: UI-920
  test("the summary leaves out what the card shows elsewhere: its widget, code, props and title", () => {
    const called = omfpCard(
      { widget: "chart", title: "Load", props: { a: 1 }, wait: true, timeoutSeconds: 30 },
      "running",
    );
    expect(called.summary).toBe("timeoutSeconds: 30, wait: true");
  });

  // spec: UI-921, AGT-096
  test("the answer: waiting while a wait call runs, then the JSON status of its result, aborted when it failed", () => {
    const answer = (args: JsonObject, status: ToolRun["status"], text?: string) =>
      view(omfpCard(args, status, text), "card").answer;
    const waits = { widget: "ask", wait: true };
    expect(answer(waits, "running")).toEqual({ state: "waiting", value: undefined });
    expect(answer({ widget: "ask" }, "running")).toEqual({ state: "none", value: undefined });
    expect(answer({ widget: "ask" }, "succeeded", '{"status":"shown"}')).toEqual({ state: "none", value: undefined });
    expect(answer(waits, "succeeded", '{"status":"submitted","value":{"pick":2}}')).toEqual({
      state: "submitted",
      value: { pick: 2 },
    });
    expect(answer(waits, "succeeded", '{"status":"cancelled"}')).toEqual({ state: "cancelled", value: undefined });
    expect(answer(waits, "succeeded", '{"status":"timeout"}')).toEqual({ state: "timeout", value: undefined });
    expect(answer(waits, "failed", "cancelled")).toEqual({ state: "aborted", value: undefined });
    expect(answer(waits, "succeeded", "not json")).toEqual({ state: "none", value: undefined });
  });

  // spec: UI-922
  test("open by default while it runs or its turn is open; collapsed once a user message follows, even failed", () => {
    const running = omfpCard({ widget: "ask", wait: true }, "running");
    const done = omfpCard({ widget: "ask" }, "succeeded", '{"status":"shown"}');
    const failed = omfpCard({ widget: "ask", wait: true }, "failed", "cancelled");
    expect([running, done, failed].map((found) => expandedByDefault(found))).toEqual([true, false, false]);
    expect([running, done, failed].map((found) => expandedByDefault(found, "open"))).toEqual([true, true, true]);
    expect([running, done, failed].map((found) => expandedByDefault(found, "past"))).toEqual([true, false, false]);
  });

  // spec: UI-922
  test("once the thread settles a card without wait stays open in the latest exchange; a waiting one makes way", () => {
    const shown = omfpCard({ widget: "chart" }, "succeeded", '{"status":"shown"}');
    const answered = omfpCard({ widget: "ask", wait: true }, "succeeded", '{"status":"submitted","value":1}');
    expect([shown, answered].map((found) => expandedByDefault(found, "latest"))).toEqual([true, false]);
  });
});
