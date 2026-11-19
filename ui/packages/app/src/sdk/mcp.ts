/**
 * A workspace's MCP tools for widgets (AGT-095): {@link useMcp} lists a server's tools (`mcp.tools`) and calls them
 * (`mcp.call`), {@link useMcpTool} calls one tool like `useCall`, {@link mcpText} reads a result's text.
 */

import type { McpCallResult, McpTool } from "@omfp/protocol";
import { useCallback, useContext } from "react";
import { type CallState, useCall } from "./hooks.ts";
import { WidgetScope } from "./widget.ts";

/** The workspace an MCP hook talks to: the option's, else the calling widget's. */
export interface McpOptions {
  readonly workspaceId?: string;
}

/** What {@link useMcp} gives (AGT-095). */
export interface Mcp {
  /** The server's tools; `undefined` before the first listing. */
  readonly tools: McpTool[] | undefined;
  /** Why the latest listing failed (a `CallError`), `undefined` once one succeeds. */
  readonly error: unknown;
  readonly refreshing: boolean;
  /** Lists the tools again. */
  refresh(): void;
  /** Calls `tool` with `args` (`mcp.call`); rejects with the daemon's error. */
  call(tool: string, args?: Record<string, unknown>): Promise<McpCallResult>;
}

function useWorkspace(options: McpOptions | undefined): string | undefined {
  const scope = useContext(WidgetScope);
  return options?.workspaceId ?? scope?.workspaceId ?? undefined;
}

/** The tools of the workspace's MCP server `server` (none listed for `null`/`undefined`) and a way to call them. */
export function useMcp(server: string | null | undefined, options?: McpOptions): Mcp {
  const workspaceId = useWorkspace(options);
  const client = useCall();
  const listing = useCall(
    "mcp.tools",
    workspaceId === undefined || server === null || server === undefined ? null : { workspaceId, server },
  );
  const call = useCallback(
    (tool: string, args?: Record<string, unknown>): Promise<McpCallResult> => {
      if (workspaceId === undefined || server === null || server === undefined) {
        return Promise.reject(new Error("useMcp: no workspace or server"));
      }
      return client("mcp.call", { workspaceId, server, tool, ...(args === undefined ? {} : { arguments: args }) });
    },
    [client, workspaceId, server],
  );
  return {
    tools: listing.data?.tools,
    error: listing.error,
    refreshing: listing.refreshing,
    refresh: listing.refresh,
    call,
  };
}

/** `useCall("mcp.call", …)` for the tool `tool` of `server` with `args` (`null` calls nothing). */
export function useMcpTool(
  server: string,
  tool: string,
  args: Record<string, unknown> | null,
  options?: McpOptions,
): CallState<McpCallResult> {
  const workspaceId = useWorkspace(options);
  return useCall(
    "mcp.call",
    workspaceId === undefined || args === null ? null : { workspaceId, server, tool, arguments: args },
  );
}

/** The text of `result`'s `text` content blocks joined with newlines; `""` when it has none. */
export function mcpText(result: McpCallResult | undefined): string {
  if (result === undefined) {
    return "";
  }
  return result.content
    .flatMap((block) => {
      if (typeof block !== "object" || block === null) {
        return [];
      }
      const { type, text } = block as { type?: unknown; text?: unknown };
      return type === "text" && typeof text === "string" ? [text] : [];
    })
    .join("\n");
}
