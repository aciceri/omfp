/** L1: the thread header's view-model (port of the Dioxus `vm/header.rs` tests). */

import { describe, expect, test } from "bun:test";
import { emptyThreadState, replay, type ThreadEvent, type ThreadState } from "@omfp/core";
import {
  compactionsEnded,
  costLabel,
  decodeModels,
  decodeSession,
  decodeThinkingLevels,
  formatCost,
  headerView,
  NO_DETAILS,
  savedModel,
  splitModel,
  type ThreadDetails,
  threadCost,
  untrackedCost,
} from "../../src/vm/header.ts";
import { repo, thread, workspace } from "../support/fixtures.ts";
import { fixture, fixtureEvents, isFrame, priced } from "../widgets/chat/fixtures.ts";

const opus = {
  id: "opus",
  name: "Opus",
  api: "anthropic",
  provider: "anthropic",
  baseUrl: "",
  reasoning: true,
  input: ["text"],
  contextWindow: 200_000,
  maxTokens: 32000,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
};

function details(): ThreadDetails {
  return {
    session: decodeSession({
      sessionId: "s",
      model: opus,
      thinkingLevel: "high",
      contextUsage: { tokens: 84000, contextWindow: 200_000, percent: 42.0 },
    }),
    models: decodeModels({ models: [{ ...opus, id: "haiku", name: "Haiku" }, { broken: true }] }),
    modelsLoaded: true,
    thinkingLevels: decodeThinkingLevels({ levels: ["off", "low", 3] }),
    saved: undefined,
  };
}

function withCosts(...costs: number[]): ThreadState {
  const state = emptyThreadState();
  state.transcript = [
    { message_id: null, message: { role: "user", content: "hi" } },
    ...costs.map((total) => ({
      message_id: null,
      message: { role: "assistant" as const, content: [], usage: { cost: { total } } },
    })),
  ];
  return state;
}

describe("header", () => {
  // spec: UI-030, UI-011, UI-340
  test("the header shows model, thinking, context and the workspace chip", () => {
    const workspaces = [workspace("w1", [repo("w1", "fe", "main")])];
    const view = headerView(thread("t", "checkout", "w1"), details(), workspaces);
    expect(view.model).toBe("anthropic/opus");
    expect(view.models).toEqual([
      { value: "anthropic/opus", label: "Opus (anthropic)" },
      { value: "anthropic/haiku", label: "Haiku (anthropic)" },
    ]);
    expect(view.thinking).toBe("high");
    expect(view.thinkingLevels).toEqual(["high", "off", "low"]);
    expect(view.context).toBe("42% of 200k");
    expect(view.workspace).toEqual({
      workspaceId: "w1",
      label: "w1 · 1 repo",
      tooltip: "fe:main (worktree)",
      repoCount: 1,
    });
    expect(view.action).toBe("stop");
  });

  // spec: UI-030
  test("an unnamed thread shows the placeholder title; process state decides stop or restart", () => {
    const unnamed = headerView(thread("t", "", "w"), NO_DETAILS, []);
    expect([unnamed.title, unnamed.placeholder]).toEqual(["New thread", true]);
    const failed = headerView(thread("t", "x", "w", { process: { state: "failed", message: "boom" } }), NO_DETAILS, []);
    expect([failed.status, failed.failure, failed.action, failed.placeholder]).toEqual([
      "failed",
      "boom",
      "restart",
      false,
    ]);
  });

  // spec: UI-026
  test("an archived thread's header offers unarchive", () => {
    expect(headerView(thread("t", "x", "w", { archivedAt: 4 }), NO_DETAILS, []).archived).toBe(true);
    expect(headerView(thread("t", "x", "w"), NO_DETAILS, []).archived).toBe(false);
  });

  // spec: UI-349
  test("a thread whose daemon holds a restart offers it, named by its reasons", () => {
    const restart = (summary: ReturnType<typeof thread>) => headerView(summary, NO_DETAILS, []).restart?.label;
    expect(restart(thread("t", "x", "w", { restartPending: ["mcp_config"] }))).toBe("Reload to load MCP tools");
    expect(restart(thread("t", "x", "w", { restartPending: ["login"] }))).toBe("Restart to load new models");
    expect(restart(thread("t", "x", "w"))).toBeUndefined();
  });

  // spec: UI-388
  test("a start that failed for want of a login, or no model at all, offers Log in", () => {
    const failed = (message: string, stderr_tail?: string) =>
      headerView(
        thread("t", "x", "w", {
          process: { state: "failed", message, ...(stderr_tail === undefined ? {} : { stderr_tail }) },
        }),
        NO_DETAILS,
        [],
      ).login;
    const noDefault =
      "No default model selected. Use /login, set an API key environment variable, or select a local model with /model or --model.";
    expect([
      failed("omp failed to start: server closed", noDefault),
      failed(noDefault),
      failed("omp failed to start: server closed", "error: unknown option --foo"),
      failed("boom"),
    ]).toEqual([true, true, false, false]);
    expect(headerView(thread("t", "x", "w"), NO_DETAILS, []).login).toBe(false);

    const noModels = { ...NO_DETAILS, modelsLoaded: true };
    expect(headerView(thread("t", "x", "w"), noModels, []).noModels).toBe(true);
    // Not before `thread.models` answered, nor while omp lists models.
    expect(headerView(thread("t", "x", "w"), NO_DETAILS, []).noModels).toBe(false);
    expect(headerView(thread("t", "x", "w"), details(), []).noModels).toBe(false);
  });

  // spec: UI-751
  test("a failed start whose saved model is not usable shows it unavailable, the usable models and Log in", () => {
    const failed = thread("t", "x", "w", {
      process: { state: "failed", message: "omp failed to start: server closed" },
    });
    const saved = { ...details(), session: undefined, saved: "anthropic/claude-opus-5-5" };
    const view = headerView(failed, saved, []);
    expect([view.unavailable, view.model, view.login, view.noModels]).toEqual([
      "anthropic/claude-opus-5-5",
      "anthropic/claude-opus-5-5",
      true,
      false,
    ]);
    expect(view.models).toEqual([
      { value: "anthropic/claude-opus-5-5", label: "anthropic/claude-opus-5-5 (unavailable)", unavailable: true },
      { value: "anthropic/haiku", label: "Haiku (anthropic)" },
    ]);
    // No usable model at all: `No models` and Log in.
    expect(headerView(failed, { ...saved, models: [] }, []).noModels).toBe(true);

    // A usable saved model, a running thread, or models not read yet: nothing is unavailable.
    const usable = { ...saved, saved: "anthropic/haiku" };
    expect(headerView(failed, usable, []).unavailable).toBeUndefined();
    expect(headerView(thread("t", "x", "w"), saved, []).unavailable).toBeUndefined();
    expect(headerView(failed, { ...saved, modelsLoaded: false }, []).unavailable).toBeUndefined();

    // The model set on the thread wins over the one its session file saved.
    expect(savedModel({ model: "fake/scripted", sessionModel: "anthropic/claude-opus-5-5" })).toBe("fake/scripted");
    expect(savedModel({ sessionModel: "anthropic/claude-opus-5-5" })).toBe("anthropic/claude-opus-5-5");
    expect(savedModel({ model: null, sessionModel: null })).toBeUndefined();
  });

  // spec: UI-030, UI-691
  test("the session's own cost sums the assistant usage of the transcript", () => {
    expect(threadCost(withCosts())).toEqual({ session: 0, subagents: 0 });
    expect(formatCost(threadCost(withCosts(0.0125, 0.0125)).session)).toBe("$0.0250");
    expect(formatCost(12.5)).toBe("$12.50");
    // Only `task` results carry a subagent's spend.
    const bash = withCosts(0.5);
    bash.transcript.push({
      message_id: null,
      message: { role: "toolResult", toolName: "bash", details: { usage: { cost: { total: 3 } } } },
    });
    expect(threadCost(bash)).toEqual({ session: 0.5, subagents: 0 });
  });

  // spec: UI-692
  test("subagents cost what their task results report, or more while their progress says so", () => {
    const events = fixtureEvents("task_tool_call");
    const started = events.findIndex(isFrame("subagent_lifecycle"));
    const ended = events.findIndex((event, index) => index > started && isFrame("subagent_lifecycle")(event));
    const prices = { assistant: 0.125, task: 1.25, progress: 0.5 };
    // While the subagent runs, its progress is all there is.
    expect(threadCost(replay(priced(events.slice(0, ended), prices)))).toEqual({ session: 0.125, subagents: 0.5 });
    // Three assistant messages; one of the two task results ran a subagent.
    expect(threadCost(replay(priced(events, prices)))).toEqual({ session: 0.375, subagents: 1.25 });
    expect(threadCost(replay(priced(events, { ...prices, progress: 2 })))).toEqual({ session: 0.375, subagents: 2 });
    expect(threadCost(fixture("task_tool_call"))).toEqual({ session: 0, subagents: 0 });
  });

  // spec: UI-691
  test("the session's own cost adds what omp spent outside the transcript, never below zero", () => {
    const state = replay(priced(fixtureEvents("task_tool_call"), { assistant: 0.125, task: 1.25, progress: 0 }));
    // `thread.session` counts the transcript's assistant messages and task results, plus calls like titles.
    expect(untrackedCost(2, state)).toBe(0.375);
    expect(untrackedCost(1, state)).toBe(0);
    expect(threadCost(state, untrackedCost(2, state))).toEqual({ session: 0.75, subagents: 1.25 });
  });

  // spec: UI-690
  test("the cost label: the session's own cost, then the subagents' in parentheses, both in its description", () => {
    expect(costLabel({ session: 0.5, subagents: 0 })).toEqual({
      text: "$0.5000",
      description: "Cost: session $0.5000",
    });
    expect(costLabel({ session: 0.38, subagents: 1.27 })).toEqual({
      text: "$0.3800 (+$1.27)",
      description: "Cost: session $0.3800, subagents $1.27",
    });
    expect(costLabel({ session: 0, subagents: 1.27 })?.text).toBe("$0.0000 (+$1.27)");
    expect([costLabel({ session: 0, subagents: 0 }), costLabel(undefined)]).toEqual([undefined, undefined]);
  });

  // spec: UI-030
  test("model values are parsed from the selector", () => {
    expect(splitModel("openrouter/anthropic/opus")).toEqual({ provider: "openrouter", modelId: "anthropic/opus" });
    expect(splitModel("opus")).toBeUndefined();
    expect(splitModel("/opus")).toBeUndefined();
  });

  // spec: UI-030
  test("malformed call results decode to nothing", () => {
    expect(decodeModels(null)).toEqual([]);
    expect(decodeThinkingLevels({ levels: "high" })).toEqual([]);
    expect(decodeSession("x")).toEqual({ model: undefined, thinkingLevel: undefined, contextUsage: undefined });
  });

  // spec: UI-030
  test("the compactions that ended, automatic or manual and whatever their outcome, count; their starts do not", () => {
    const events: ThreadEvent[] = [
      { kind: "omp", frame: { type: "manual_compaction_start" } },
      { kind: "omp", frame: { type: "manual_compaction_end", aborted: false, skipped: true, errorMessage: "x" } },
      { kind: "omp", frame: { type: "auto_compaction_start", reason: "threshold", action: "context-full" } },
      {
        kind: "omp",
        frame: { type: "auto_compaction_end", action: "context-full", aborted: false, willRetry: false },
      },
    ];
    expect(compactionsEnded(emptyThreadState().notices)).toBe(0);
    const { notices } = replay(events);
    expect(compactionsEnded(notices)).toBe(2);
    expect(compactionsEnded(notices)).toBe(2);
  });
});
