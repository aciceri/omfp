/** Recorded thread fixtures (TST-012) for the conversation view models and widgets. */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { OMP_VERSION, replay, type ThreadEvent, type ThreadState } from "@omfp/core";
import type { FileCandidates, ThreadSummary } from "@omfp/protocol";
import { isObject } from "../../../src/sdk/json.ts";

export type FixtureName =
  | "text_reply"
  | "bash_tool_call"
  | "async_bash_tool_call"
  | "bash_tool_error"
  | "approval_granted"
  | "ask_answered"
  | "ask_cancelled"
  | "follow_up_then_abort"
  | "restart_with_history"
  | "read_tool_calls"
  | "edit_tool_calls"
  | "write_tool_calls"
  | "search_tool_calls"
  | "todo_tool_calls"
  | "web_tool_calls"
  | "task_tool_call"
  | "live_no_credential";

/** The `ThreadEvent`s of `fixtures/threads/<omp>/<name>.jsonl`. */
export function fixtureEvents(name: FixtureName): ThreadEvent[] {
  const path = join(import.meta.dir, "../../../../../../fixtures/threads", OMP_VERSION, `${name}.jsonl`);
  return readFileSync(path, "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => JSON.parse(line) as ThreadEvent);
}

/** The events before the first one `stop` accepts (all of them when none does). */
export function eventsUntil(name: FixtureName, stop: (event: ThreadEvent) => boolean): ThreadEvent[] {
  const events = fixtureEvents(name);
  const end = events.findIndex(stop);
  return end === -1 ? events : events.slice(0, end);
}

/** The state after the whole fixture. */
export function fixture(name: FixtureName): ThreadState {
  return replay(fixtureEvents(name));
}

/** The state after the events before the first one `stop` accepts. */
export function fixtureUntil(name: FixtureName, stop: (event: ThreadEvent) => boolean): ThreadState {
  return replay(eventsUntil(name, stop));
}

/** The state after the fixture's first `count` events. */
export function fixturePrefix(name: FixtureName, count: number): ThreadState {
  return replay(fixtureEvents(name).slice(0, count));
}

/** An omp frame of type `type` (and, for `message_update`, assistant event `delta`). */
export function isFrame(type: string, delta?: string): (event: ThreadEvent) => boolean {
  return (event) => {
    if (event.kind !== "omp") {
      return false;
    }
    const frame = event.frame;
    if (!isObject(frame) || frame.type !== type) {
      return false;
    }
    const update = frame.assistantMessageEvent;
    return delta === undefined || (isObject(update) && update.type === delta);
  };
}

/** The `tool_execution_end` frame of call `toolCallId`. */
export function isToolEnd(toolCallId: string): (event: ThreadEvent) => boolean {
  return (event) =>
    isFrame("tool_execution_end")(event) &&
    event.kind === "omp" &&
    isObject(event.frame) &&
    event.frame.toolCallId === toolCallId;
}

/** A thread summary for the index. */
export function summary(id: string, settled = true): ThreadSummary {
  return {
    id,
    title: id,
    titleSource: "set",
    workspaceId: "01W",
    process: { state: "running" },
    settled,
    needsAttention: false,
    provenance: { createdBy: { kind: "user" }, client: "test" },
    createdAt: 1,
    updatedAt: 1,
    position: 0,
  };
}

/** A `workspaces.find_file { fuzzy: true }` answer naming `paths`, best first (UI-732). */
export function candidates(...paths: string[]): FileCandidates {
  return { candidates: paths.map((path) => ({ path, match: "fuzzy" })) };
}

/** What {@link priced} makes a fixture's model calls cost: the fake model has no price, so the recorded ones cost 0. */
export interface Prices {
  /** Each assistant message (`usage.cost.total`). */
  readonly assistant: number;
  /** Each `task` result (`details.usage.cost.total`). */
  readonly task: number;
  /** What each subagent progress reports spent so far (`progress.cost`). */
  readonly progress: number;
}

/** `events` with the model calls they record costing {@link Prices}. */
export function priced(events: readonly ThreadEvent[], prices: Prices): ThreadEvent[] {
  return events.map((event) => price(event, prices, false) as ThreadEvent);
}

function price(value: unknown, prices: Prices, inDetails: boolean): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => price(item, prices, inDetails));
  }
  if (!isObject(value)) {
    return value;
  }
  return Object.fromEntries(
    Object.entries(value).map(([key, item]) => {
      if (key === "usage" && isObject(item) && isObject(item.cost)) {
        return [key, { ...item, cost: { ...item.cost, total: inDetails ? prices.task : prices.assistant } }];
      }
      if (key === "cost" && typeof item === "number") {
        return [key, prices.progress];
      }
      return [key, price(item, prices, inDetails || key === "details")];
    }),
  );
}
