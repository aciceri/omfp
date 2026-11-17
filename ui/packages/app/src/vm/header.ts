/**
 * The thread header (UI-030): title (placeholder until named, DMN-073), the workspace chip (UI-340), model and
 * thinking options, context usage, the cost with its subagents' (UI-690), and process state with its action; archive
 * state (UI-026); the saved model a failed start could not use (UI-751).
 */

import type { ThreadState } from "@omfp/core";
import { compactNumber } from "@omfp/kit";
import type { ThreadSession, ThreadSummary, Workspace } from "@omfp/protocol";
import { isObject } from "../sdk/json.ts";
import { strings } from "../strings/index.ts";
import { needsLogin } from "./login.ts";
import { type PendingRestart, pendingRestart } from "./mcp.ts";
import {
  displayTitle,
  isArchived,
  type ThreadStatus,
  threadStatus,
  type WorkspaceChip,
  workspaceChip,
} from "./threads.ts";

/** A model as `thread.models` and `thread.state` describe it (the fields the header uses). */
export interface ModelInfo {
  readonly id: string;
  readonly name: string;
  readonly provider: string;
}

/** `thread.state`'s fields the header uses. */
export interface SessionDetails {
  readonly model: ModelInfo | undefined;
  readonly thinkingLevel: string | undefined;
  readonly contextUsage: { readonly percent: number; readonly contextWindow: number } | undefined;
}

/** Thread data fetched by calls rather than kept by the reducer. */
export interface ThreadDetails {
  /** `thread.state`. */
  readonly session: SessionDetails | undefined;
  /** `thread.models`. */
  readonly models: readonly ModelInfo[];
  /** `thread.models` answered: an empty {@link ThreadDetails.models} is then omp's (UI-388). */
  readonly modelsLoaded: boolean;
  /** `thread.thinking_levels`. */
  readonly thinkingLevels: readonly string[];
  /** `provider/id` the thread resumes with while its start failed: `thread.session`'s `model`, else its
   * `sessionModel` (DMN-338, UI-751). */
  readonly saved: string | undefined;
}

export const NO_DETAILS: ThreadDetails = {
  session: undefined,
  models: [],
  modelsLoaded: false,
  thinkingLevels: [],
  saved: undefined,
};

/** One model choice: `value` is `provider/id`; `unavailable` marks the saved model omp cannot use (UI-751). */
export interface ModelOption {
  readonly value: string;
  readonly label: string;
  readonly unavailable?: true;
}

/** What the process can do next: stop a running or starting one, restart a stopped or failed one. */
export type ProcessAction = "stop" | "restart";

export interface HeaderView {
  readonly title: string;
  /** The title is the "New thread" placeholder (DMN-073). */
  readonly placeholder: boolean;
  /** The thread's workspace; `undefined` while it is unknown. */
  readonly workspace: WorkspaceChip | undefined;
  /** `provider/id` of the current model. */
  readonly model: string | undefined;
  /** The available models; the current one is included even when the list has not arrived. */
  readonly models: readonly ModelOption[];
  readonly thinking: string | undefined;
  readonly thinkingLevels: readonly string[];
  /** `42% of 200k`. */
  readonly context: string | undefined;
  /** Context tokens used and the window, for the token meter (UI-158). */
  readonly usage: { readonly used: number; readonly total: number } | undefined;
  readonly status: ThreadStatus;
  /** Why the process failed. */
  readonly failure: string | undefined;
  readonly action: ProcessAction | undefined;
  /** The header offers unarchive instead of archive (UI-026). */
  readonly archived: boolean;
  /** The daemon holds a restart while background work runs: offer it, named by its reasons (UI-349, PRT-134). */
  readonly restart: PendingRestart | undefined;
  /** The process failed because omp has no model it can use without a login or a key, or its saved model is
   * unavailable: offer Log in (UI-388, UI-751). */
  readonly login: boolean;
  /** omp offers no model at all: the model list offers Log in (UI-388). */
  readonly noModels: boolean;
  /** `provider/id` of the saved model the failed start could not use, absent from the usable models: the selector
   * shows it as unavailable, offers the usable ones and Log in; nothing changes until the user picks (UI-751). */
  readonly unavailable: string | undefined;
}

/** The header of a thread; the cost comes from {@link threadCost} and {@link costLabel}, so the header does not depend
 * on the reducer state, which changes with every streamed delta. */
export function headerView(
  summary: ThreadSummary,
  details: ThreadDetails,
  workspaces: readonly Workspace[],
): HeaderView {
  const current = details.session?.model;
  const process = summary.process;
  const unavailable =
    process.state === "failed" &&
    details.modelsLoaded &&
    details.saved !== undefined &&
    !details.models.some((option) => modelValue(option) === details.saved)
      ? details.saved
      : undefined;
  const model = unavailable ?? (current === undefined ? undefined : modelValue(current));
  const models = details.models.map(modelOption);
  if (unavailable !== undefined) {
    models.unshift({
      value: unavailable,
      label: strings.shell.header.unavailableOption(unavailable),
      unavailable: true,
    });
  } else if (current !== undefined && !models.some((option) => option.value === model)) {
    models.unshift(modelOption(current));
  }
  const thinking = details.session?.thinkingLevel;
  const thinkingLevels = [...details.thinkingLevels];
  if (thinking !== undefined && !thinkingLevels.includes(thinking)) {
    thinkingLevels.unshift(thinking);
  }
  const usage = details.session?.contextUsage;
  return {
    title: displayTitle(summary),
    placeholder: summary.titleSource === "placeholder" || summary.title.trim() === "",
    workspace: workspaceChip(summary.workspaceId, workspaces),
    model,
    models,
    thinking,
    thinkingLevels,
    context:
      usage === undefined
        ? undefined
        : strings.shell.header.context(Math.round(usage.percent), compactNumber(usage.contextWindow)),
    usage:
      usage === undefined
        ? undefined
        : { used: Math.round((usage.percent / 100) * usage.contextWindow), total: usage.contextWindow },
    status: threadStatus(summary),
    failure: process.state === "failed" ? process.message : undefined,
    action: processAction(process.state),
    archived: isArchived(summary),
    restart: pendingRestart(summary.restartPending),
    login:
      unavailable !== undefined ||
      (process.state === "failed" && (needsLogin(process.message) || needsLogin(process.stderr_tail))),
    noModels: details.modelsLoaded && (unavailable === undefined ? models : details.models).length === 0,
    unavailable,
  };
}

function processAction(state: ThreadSummary["process"]["state"]): ProcessAction {
  switch (state) {
    case "running":
    case "starting":
      return "stop";
    case "stopped":
    case "failed":
      return "restart";
  }
}

function modelValue(model: ModelInfo): string {
  return `${model.provider}/${model.id}`;
}

export function modelOption(model: ModelInfo): ModelOption {
  return { value: modelValue(model), label: `${model.name} (${model.provider})` };
}

/** What a thread spent, split as omp's status line splits it (UI-690): the session's own model calls, its subagents'. */
export interface ThreadCost {
  /** The session's model calls (UI-691): its assistant messages, plus the calls omp makes outside the transcript. */
  readonly session: number;
  /** Its subagents' (UI-692). */
  readonly subagents: number;
}

/** The costs a transcript reports: its assistant messages', and its completed `task` results'. */
interface TranscriptCosts {
  readonly assistant: number;
  readonly tasks: number;
}

/** Totals computed per transcript array: the header renders on every event, the transcript changes per message. */
const COSTS = new WeakMap<ThreadState["transcript"], TranscriptCosts>();

/** `usage.cost.total` of a message's usage or a `task` result's details; 0 when it is not a number. */
function usageCost(usage: unknown): number {
  const total = record(record(usage)?.cost)?.total;
  return typeof total === "number" ? total : 0;
}

/** What the assistant messages and the `task` results of a transcript cost, as omp's `get_session_stats` counts them:
 * a `task` result carries its subagents' usage in `details.usage`. */
function transcriptCosts(transcript: ThreadState["transcript"]): TranscriptCosts {
  const known = COSTS.get(transcript);
  if (known !== undefined) {
    return known;
  }
  let assistant = 0;
  let tasks = 0;
  for (const { message } of transcript) {
    if (message.role === "assistant") {
      assistant += usageCost(message.usage);
    } else if (message.role === "toolResult" && message.toolName === "task") {
      tasks += usageCost(record(message.details)?.usage);
    }
  }
  const costs = { assistant, tasks };
  COSTS.set(transcript, costs);
  return costs;
}

/** What the subagents' latest progress reports spent so far (`progress.cost`), running ones included. */
function progressCost(subagents: ThreadState["subagents"]): number {
  let total = 0;
  for (const subagent of Object.values(subagents)) {
    const cost = subagent.progress?.progress.cost;
    total += typeof cost === "number" ? cost : 0;
  }
  return total;
}

/**
 * What thread `state` spent (UI-690..UI-692): the session's own cost is its assistant messages' plus `untracked`
 * ({@link untrackedCost}); its subagents' is what their `task` results report or, when more, what their progress
 * reports — running subagents included —, as omp's status line takes the larger of the two.
 */
export function threadCost(state: ThreadState, untracked = 0): ThreadCost {
  const { assistant, tasks } = transcriptCosts(state.transcript);
  return { session: assistant + untracked, subagents: Math.max(tasks, progressCost(state.subagents)) };
}

/**
 * What omp spent outside the transcript (its `model_usage` entries: titles, judgments…, UI-691): `sessionCost`, the
 * cost `thread.session` reports, less what the transcript of `state` accounts for; never below 0.
 */
export function untrackedCost(sessionCost: number, state: ThreadState): number {
  const { assistant, tasks } = transcriptCosts(state.transcript);
  return Math.max(0, sessionCost - assistant - tasks);
}

/** The cost to show (UI-690): `$0.3800 (+$1.27)` and its description; `undefined` while nothing was spent. */
export function costLabel(cost: ThreadCost | undefined): { text: string; description: string } | undefined {
  if (cost === undefined || (cost.session <= 0 && cost.subagents <= 0)) {
    return undefined;
  }
  const session = formatCost(cost.session);
  const subagents = cost.subagents > 0 ? formatCost(cost.subagents) : undefined;
  return {
    text: strings.shell.header.cost(session, subagents),
    description: strings.shell.header.costDescription(session, subagents),
  };
}

/** `$0.0123` below a dollar, `$1.23` above. */
export function formatCost(cost: number): string {
  return cost < 1 ? `$${cost.toFixed(4)}` : `$${cost.toFixed(2)}`;
}

/** Counts per notices array: the header asks on every event, the notices change per notice. */
const COMPACTIONS = new WeakMap<ThreadState["notices"], number>();

/**
 * How many compactions, automatic or manual, the thread's notices saw end: the header reads the context usage again
 * when it changes (UI-030), whatever their outcome.
 */
export function compactionsEnded(notices: ThreadState["notices"]): number {
  const known = COMPACTIONS.get(notices);
  if (known !== undefined) {
    return known;
  }
  const count = notices.filter(
    ({ frame }) => frame.type === "auto_compaction_end" || frame.type === "manual_compaction_end",
  ).length;
  COMPACTIONS.set(notices, count);
  return count;
}

/** The model a thread whose start failed resumes with (UI-751): `thread.session`'s `model`, the one set on the thread
 * (omp then starts with it), else its `sessionModel`, the one its session file saved (DMN-338). */
export function savedModel(session: Pick<ThreadSession, "model" | "sessionModel">): string | undefined {
  return session.model ?? session.sessionModel ?? undefined;
}

/** The model choice `provider/id` split in two (the id may contain slashes). */
export function splitModel(value: string): { provider: string; modelId: string } | undefined {
  const slash = value.indexOf("/");
  if (slash <= 0 || slash === value.length - 1) {
    return undefined;
  }
  return { provider: value.slice(0, slash), modelId: value.slice(slash + 1) };
}

/** `thread.models`' result (`{ models: ModelInfo[] }`); malformed entries are skipped. */
export function decodeModels(result: unknown): ModelInfo[] {
  const models = record(result)?.models;
  return Array.isArray(models) ? models.flatMap((model) => decodeModel(model) ?? []) : [];
}

/** `thread.thinking_levels`' result (`{ levels: string[] }`). */
export function decodeThinkingLevels(result: unknown): string[] {
  const levels = record(result)?.levels;
  return Array.isArray(levels) ? levels.filter((level): level is string => typeof level === "string") : [];
}

/** `thread.state`'s result (omp's `SessionState`). */
export function decodeSession(result: unknown): SessionDetails {
  const session = record(result);
  const usage = record(session?.contextUsage);
  const percent = usage?.percent;
  const contextWindow = usage?.contextWindow;
  const thinkingLevel = session?.thinkingLevel;
  return {
    model: decodeModel(session?.model),
    thinkingLevel: typeof thinkingLevel === "string" ? thinkingLevel : undefined,
    contextUsage:
      typeof percent === "number" && typeof contextWindow === "number" ? { percent, contextWindow } : undefined,
  };
}

function decodeModel(value: unknown): ModelInfo | undefined {
  const model = record(value);
  const { id, name, provider } = model ?? {};
  return typeof id === "string" && typeof name === "string" && typeof provider === "string"
    ? { id, name, provider }
    : undefined;
}

/** `value` when it is an object, so optional chains reach into untyped results. */
function record(value: unknown): Readonly<Record<string, unknown>> | undefined {
  return isObject(value) ? value : undefined;
}
