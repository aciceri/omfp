/**
 * The rows of `omfp:mcp` (UI-290..UI-292): the MCP servers of a workspace or a thread (`workspaces.mcp_servers`,
 * `thread.mcp_servers`, PRT-110..PRT-114) as text, marks and actions.
 */

import type { McpServer, McpServers, RestartReason } from "@omfp/protocol";
import { strings } from "../strings/index.ts";

const text = strings.shell.mcp;

/** The mark of a row: `done` (●) connected, `failed` (●) failed, `running` (the spinner) being checked, `idle` else. */
export type McpMark = "done" | "failed" | "running" | "idle";

export interface McpRow {
  readonly name: string;
  readonly mark: McpMark;
  /** `connected`, `failed`, `checking…`, … */
  readonly stateLabel: string;
  /** `N tools` once connected. */
  readonly tools: string | undefined;
  /** `stdio`, `http` or `sse`, then the command or URL. */
  readonly transport: string;
  readonly target: string;
  /** omp's error when the check failed. */
  readonly error: string | undefined;
  /** Where it is defined, as shown: the workspace-relative path when the file is in the workspace, else absolute. */
  readonly source: string;
  /** The tool whose config it is (`omp`, `cursor`, …). */
  readonly origin: string;
  /** The workspace path the file tab opens; `undefined` for a file outside the workspace. */
  readonly openPath: string | undefined;
  /** Why the row has no live state although a thread runs (omp checks only its own files). */
  readonly note: string | undefined;
  readonly disabled: boolean;
  /** An enabled `http` or `sse` server: omp can authorize it again with OAuth (`/mcp reauth <name>`, UI-348). */
  readonly reauthorizable: boolean;
}

/** What is in flight: nothing, a check of every server, or a reconnect of one server (`name`) or all (`all`). */
export type McpPending =
  | { readonly kind: "none" }
  | { readonly kind: "check" }
  | { readonly kind: "reconnect"; readonly name: string | undefined };

export const MCP_IDLE: McpPending = { kind: "none" };

/** A restart the daemon holds back while background work runs (UI-349): its button's label and hint. */
export interface PendingRestart {
  readonly label: string;
  readonly hint: string;
  /** It loads new MCP tools: `omfp:mcp` offers it too. */
  readonly mcp: boolean;
}

/** The restart a thread summary's `restartPending` holds (PRT-134), named by its reasons; `undefined` when none. */
export function pendingRestart(reasons: readonly RestartReason[] | undefined): PendingRestart | undefined {
  if (reasons === undefined || reasons.length === 0) {
    return undefined;
  }
  const login = reasons.includes("login");
  const mcp = reasons.some((reason) => reason !== "login");
  const text = strings.shell.mcp.restartPending;
  return { ...(login ? (mcp ? text.both : text.login) : text.mcp), mcp };
}

/** The prompt that makes the thread's omp authorize `name` again (UI-348): a local slash command, answered by
 * `command_output` notices. */
export function reauthPrompt(name: string): string {
  return `/mcp reauth ${name}`;
}

/** Whether `server` is being checked by `pending`. */
function inFlight(server: McpServer, pending: McpPending, running: boolean): boolean {
  if (server.state === "disabled" || !server.checkable) {
    return false;
  }
  switch (pending.kind) {
    case "none":
      return false;
    case "check":
      return running;
    case "reconnect":
      return pending.name === undefined || pending.name === server.name;
  }
}

function mark(server: McpServer): McpMark {
  switch (server.state) {
    case "connected":
      return "done";
    case "failed":
      return "failed";
    default:
      return "idle";
  }
}

/** One row per server of `listing`, in omp's precedence order; `pending` turns the servers it checks into spinners. */
export function mcpRows(listing: McpServers, pending: McpPending = MCP_IDLE): McpRow[] {
  const threadRuns = listing.threadId !== undefined && listing.threadId !== null && listing.running;
  return listing.servers.map((server) => {
    const checking = inFlight(server, pending, listing.running);
    const tools = server.tools ?? undefined;
    return {
      name: server.name,
      mark: checking ? "running" : mark(server),
      stateLabel: checking ? text.state.checking : text.state[server.state],
      tools: !checking && tools !== undefined ? text.tools(tools) : undefined,
      transport: server.transport,
      target: server.target,
      error: checking ? undefined : (server.error ?? undefined),
      source: server.source.workspacePath ?? server.source.path,
      origin: server.source.tool,
      openPath: server.source.workspacePath ?? undefined,
      note: threadRuns && !server.checkable && server.state !== "disabled" ? text.notCheckable : undefined,
      disabled: server.state === "disabled",
      reauthorizable: server.state !== "disabled" && (server.transport === "http" || server.transport === "sse"),
    };
  });
}

/** The line over the boxes: whose servers these are and whether they were checked. */
export function mcpStatus(listing: McpServers): string {
  if (listing.threadId === undefined || listing.threadId === null) {
    return text.workspaceOnly;
  }
  return listing.running ? text.checked : text.notRunning;
}
