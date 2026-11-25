import { describe, expect, test } from "bun:test";
import type { ThreadState, ToolRun } from "@omfp/core";
import { expandedByDefault, formatDuration, oneLine, type ToolCard, toolCard } from "../../src/vm/tool.ts";
import { items, messageView } from "../../src/vm/transcript.ts";
import { fixture, fixtureUntil, isFrame } from "../widgets/chat/fixtures.ts";

function cards(state: ThreadState): ToolCard[] {
  return items(state).flatMap((item) =>
    item.kind.type === "message"
      ? messageView(state, item.kind.index).blocks.flatMap((block) => (block.type === "tool" ? [block.card] : []))
      : [],
  );
}

describe("tool cards", () => {
  // spec: UI-050
  test("every card has the name, status, argument summary, arguments and result text", () => {
    const [card, ...rest] = cards(fixture("bash_tool_call"));
    expect(rest).toEqual([]);
    expect([card?.name, card?.status]).toEqual(["bash", "succeeded"]);
    expect(card?.summary).toBe("");
    expect(card?.args).toContain('"command"');
    expect(card?.result).toBe("tool-output-123\n");
    expect(card !== undefined && expandedByDefault(card)).toBe(false);
  });

  // spec: UI-050
  test("a running call shows its status", () => {
    const state = fixtureUntil("bash_tool_call", isFrame("tool_execution_end"));
    const run = state.tools[0];
    expect(run).toBeDefined();
    if (run !== undefined) {
      expect(toolCard(run.tool_call_id, undefined, run, undefined, undefined).status).toBe("running");
    }
  });

  // spec: UI-050, UI-054
  test("while streaming the fallback card shows the arguments parsed so far, in its body and its header", () => {
    const card = toolCard(
      "c1",
      { type: "toolCall", name: "eval", arguments: {}, partialArgs: '{"code": "1 +' },
      undefined,
      undefined,
      undefined,
    );
    expect(card.status).toBe("composing");
    expect(card.args).toBe('{\n  "code": "1 +"\n}');
    expect(card.summary).toBe("code: 1 +");
  });

  // spec: UI-050, UI-054
  test("while streaming a specialized card shows its subject and body as written, its intent as the summary", () => {
    const partialArgs = '{"i": "Writing the notes", "path": "a.md", "content": "Marker';
    const card = toolCard(
      "c1",
      { type: "toolCall", name: "write", arguments: {}, partialArgs },
      undefined,
      undefined,
      undefined,
    );
    expect([card.status, card.summary]).toEqual(["composing", "Writing the notes"]);
    expect(card.view).toEqual({ kind: "write", path: "a.md", content: "Marker", language: "markdown" });
    expect(JSON.parse(card.args)).toEqual({ path: "a.md", content: "Marker" });
    // Once the arguments ended (`toolcall_end`), before the run starts: the complete ones, the same intent.
    const ended = toolCard(
      "c1",
      {
        type: "toolCall",
        name: "write",
        arguments: { i: "Writing the notes", path: "a.md", content: "Marker\n" },
        partialArgs: '{"i": "Writing the notes", "path": "a.md", "content": "Marker\\n"}',
      },
      undefined,
      undefined,
      undefined,
    );
    expect([ended.status, ended.summary]).toEqual(["pending", "Writing the notes"]);
    expect(ended.view).toEqual({ kind: "write", path: "a.md", content: "Marker\n", language: "markdown" });
  });

  // spec: UI-054
  test("each specialized card grows from the arguments written so far", () => {
    const composing = (name: string, partialArgs: string) => {
      const card = toolCard(
        "c1",
        { type: "toolCall", name, arguments: {}, partialArgs },
        undefined,
        undefined,
        undefined,
      );
      expect(card.status).toBe("composing");
      return card.view;
    };
    expect(composing("bash", '{"command": "git sta')).toMatchObject({ kind: "bash", command: "git sta" });
    expect(composing("read", '{"path": "src/li')).toMatchObject({ kind: "read", path: "src/li" });
    expect(composing("grep", '{"pattern": "fn ma')).toMatchObject({ kind: "search", query: "fn ma" });
    expect(composing("omfp_card", '{"title": "Load", "code": "export default () =>')).toMatchObject({
      kind: "card",
      title: "Load",
      code: "export default () =>",
    });
    expect(composing("omfp_ui_mount", '{"id": "chart", "code": "export default')).toEqual({
      kind: "mount",
      id: "chart",
      reference: undefined,
      code: "export default",
      props: undefined,
    });
    const form = '{"html": "<form><fieldset><legend>Which database?</legend></fieldset><fieldset><legend>Whi';
    expect(composing("ask", form)).toEqual({
      kind: "ask",
      questions: [{ question: "Which database?", answer: undefined }],
    });
    expect(composing("todo", '{"op": "init", "items": ["Read the code", "Fix the b')).toEqual({
      kind: "todo",
      op: "init",
      phases: [
        {
          name: "Tasks",
          tasks: [
            { content: "Read the code", status: "pending" },
            { content: "Fix the b", status: "pending" },
          ],
        },
      ],
    });
    expect(composing("task", '{"tasks": [{"name": "Scout", "agent": "scout", "task": "Find the par')).toMatchObject({
      kind: "task",
      agents: [{ name: "Scout", agent: "scout", task: "Find the par", status: "pending" }],
    });
  });

  // spec: UI-054
  test("a long argument text is parsed again only once it grew by a 32nd; the complete arguments replace it at once", () => {
    const head = '{"path": "big.md", "content": "';
    const body = "x".repeat(4096);
    const content = (partialArgs: string, args: Record<string, string> = {}) => {
      const card = toolCard(
        "c-long",
        { type: "toolCall", name: "write", arguments: args, partialArgs },
        undefined,
        undefined,
        undefined,
      );
      return card.view.kind === "write" ? card.view.content?.length : undefined;
    };
    expect(content(head + body)).toBe(4096);
    // 4227 characters, 100 more than the last parse: under a 32nd (132), that parse stands.
    expect(content(head + body + "y".repeat(100))).toBe(4096);
    expect(content(head + body + "y".repeat(200))).toBe(4296);
    expect(content(`${head}${body}y"}`, { path: "big.md", content: `${body}y` })).toBe(4097);
  });

  // spec: UI-050
  test("a specialized card's header does not repeat its subject nor what its body shows", () => {
    const summary = (name: string, args: Record<string, string | number>) =>
      toolCard("c1", { type: "toolCall", name, arguments: args }, undefined, undefined, undefined).summary;
    expect(summary("write", { path: "repo/AGENTS.md", content: "Marker: one\nEdited.\n" })).toBe("");
    expect(summary("bash", { command: "git -C repo status --short", timeout: 30 })).toBe("timeout: 30");
    expect(summary("read", { path: "repo/AGENTS.md", limit: 5 })).toBe("limit: 5");
    expect(summary("edit", { path: "repo/a.rs", input: "[repo/a.rs#1A2B]\nPUT 1.=1:\n+x" })).toBe("");
    expect(summary("grep", { pattern: "Edited", path: "repo" })).toBe("path: repo");
    expect(summary("glob", { path: "src/**/*.ts" })).toBe("");
    expect(summary("web_search", { query: "omp rpc", limit: 3 })).toBe("limit: 3");
    // The ask card lists the form's questions.
    expect(summary("ask", { html: "<form><fieldset><legend>Which database?</legend></fieldset></form>" })).toBe("");
    // Tools without a subject keep every scalar argument.
    expect(summary("todo", { op: "done", task: "Edit AGENTS.md" })).toBe("op: done, task: Edit AGENTS.md");
  });

  // spec: UI-050, UI-052
  test("failed tools are expanded by default and take the error text", () => {
    const run: ToolRun = {
      tool_call_id: "c1",
      tool_name: "read",
      args: { path: "missing.txt", limit: 5, nested: { x: 1 } },
      intent: null,
      status: "failed",
      partial_result: null,
      preview: null,
      result: { content: [{ type: "text", text: "ENOENT" }] },
      background: null,
    };
    const card = toolCard("c1", undefined, run, undefined, undefined);
    expect(expandedByDefault(card)).toBe(true);
    expect(card.summary).toBe("limit: 5");
    expect(card.result).toBe("ENOENT");
  });

  // spec: UI-050
  test("duration runs from the calling message to the result", () => {
    const card = toolCard("c1", undefined, undefined, { role: "toolResult", timestamp: 3500 }, 1000);
    expect([card.status, card.durationMs]).toEqual(["succeeded", 2500]);
    expect(formatDuration(2500)).toBe("2.5 s");
    expect(formatDuration(125_000)).toBe("2 min 5 s");
    expect(formatDuration(40)).toBe("40 ms");
  });

  // spec: UI-680
  test("a negative duration shows as no time, never with a minus", () => {
    expect(formatDuration(-181)).toBe("0 ms");
    expect(formatDuration(-90_000)).toBe("0 ms");
    expect(formatDuration(0)).toBe("0 ms");
  });

  // spec: UI-472
  test("an omp run's result images are kept for the card; its text says [image] instead of the base64 JSON", () => {
    const run: ToolRun = {
      tool_call_id: "c1",
      tool_name: "eval",
      args: { language: "py", code: "plt.plot([1, 2])\nplt.show()" },
      intent: "Plotting the curve",
      status: "succeeded",
      partial_result: null,
      preview: null,
      result: { content: [{ type: "image", data: "R0lGOD", mimeType: "image/gif" }], details: {} },
      background: null,
    };
    const card = toolCard("c1", undefined, run, undefined, undefined);
    expect(card.images).toEqual([
      { mime: "image/gif", data: "R0lGOD", alt: "Image from eval plt.plot([1, 2]) plt.show()" },
    ]);
    expect(card.result).toBe("[image]");
    // Without a subject argument the alt text names the call by its summary (here its intent).
    const mcp = toolCard(
      "c1",
      undefined,
      { ...run, tool_name: "mcp_chart", args: { series: 3 } },
      undefined,
      undefined,
    );
    expect(mcp.images.map((image) => image.alt)).toEqual(["Image from mcp_chart Plotting the curve"]);
  });

  test("oneLine flattens whitespace and cuts with an ellipsis", () => {
    expect(oneLine("a\n  b\tc", 10)).toBe("a b c");
    expect(oneLine("abcdefghij", 5)).toBe("abcd…");
  });
});
