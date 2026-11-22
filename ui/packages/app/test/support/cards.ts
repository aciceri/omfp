/** The tool cards of recorded fixtures (UI-051): after the whole fixture, or just before a call's end (streaming). */

import type { JsonObject, ThreadState, ToolRun } from "@omfp/core";
import { type ToolCard, toolCard } from "../../src/vm/tool.ts";
import { items, rowContent } from "../../src/vm/transcript.ts";
import { type FixtureName, fixture, fixtureUntil, isToolEnd } from "../widgets/chat/fixtures.ts";

/** Every tool card of the transcript's rows of `state`, in order, those of read groups (UI-765) included. */
export function cards(state: ThreadState): ToolCard[] {
  return items(state).flatMap((item) => {
    const content = rowContent(state, item);
    return content?.type === "message"
      ? content.view.blocks.flatMap((block) =>
          block.type === "tool" ? [block.card] : block.type === "readGroup" ? block.cards : [],
        )
      : [];
  });
}

function find(state: ThreadState, id: string, where: string): ToolCard {
  const found = cards(state).find((candidate) => candidate.toolCallId === id);
  if (found === undefined) {
    throw new Error(`no card ${id} ${where}`);
  }
  return found;
}

/** The card of call `id` after the whole fixture. */
export function card(name: FixtureName, id: string): ToolCard {
  return find(fixture(name), id, `in ${name}`);
}

/** The card of call `id` just before its `tool_execution_end`. */
export function streaming(name: FixtureName, id: string): ToolCard {
  return find(fixtureUntil(name, isToolEnd(id)), id, `before its end in ${name}`);
}

/**
 * The card of an `lsp` call built by hand in omp 19.1.0's shape (`lsp/tool.ts`, `tools/tool-commands.ts`): a `bash`
 * call running the `lsp` tool command whose result's text is the answer and whose single `details.toolCommands`
 * dispatch echoes the arguments and carries the tool's own details (`{ action, serverName, success }`). Recorded fixtures need a language server; these do not.
 */
export function lspCard(args: JsonObject, answer: string, success = true): ToolCard {
  const run: ToolRun = {
    tool_call_id: "call_lsp",
    tool_name: "bash",
    args: { command: `lsp ${Object.values(args).join(" ")}` },
    intent: null,
    status: "succeeded",
    partial_result: null,
    preview: null,
    result: {
      content: [{ type: "text", text: answer }],
      details: {
        toolCommands: [
          {
            tool: "lsp",
            mode: "execute",
            exitCode: success ? 0 : 1,
            args,
            inner: { action: args.action ?? null, serverName: "rust-analyzer", success, request: args },
          },
        ],
      },
    },
    background: null,
  };
  return toolCard("call_lsp", undefined, run, undefined, undefined);
}

/**
 * The card of an `omfp_card` call (AGT-096) built by hand: its arguments, its run's status and, once it ended, the text
 * of its result (`{"status":"shown"}`, omp's error).
 */
export function omfpCard(args: JsonObject, status: ToolRun["status"], answer?: string, id = "call_card"): ToolCard {
  const run: ToolRun = {
    tool_call_id: id,
    tool_name: "omfp_card",
    args,
    intent: null,
    status,
    partial_result: null,
    preview: null,
    result: answer === undefined ? null : { content: [{ type: "text", text: answer }] },
    background: null,
  };
  return toolCard(id, undefined, run, undefined, undefined);
}
