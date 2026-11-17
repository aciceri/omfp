/** The tool cards of recorded fixtures (UI-051): after the whole fixture, or just before a call's end (streaming). */

import type { ThreadState } from "@omfp/core";
import type { ToolCard } from "../../src/vm/tool.ts";
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
