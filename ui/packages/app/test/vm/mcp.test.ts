/** L0: the rows of `omfp:mcp` (UI-290..UI-292). */

import { describe, expect, test } from "bun:test";
import type { McpServer, McpServers } from "@omfp/protocol";
import { mcpRows, mcpStatus, pendingRestart, reauthPrompt } from "../../src/vm/mcp.ts";

function server(name: string, extra: Partial<McpServer> = {}): McpServer {
  return {
    name,
    transport: "stdio",
    target: `${name}-server --stdio`,
    source: { path: "/work/w1/.omp/mcp.json", workspacePath: ".omp/mcp.json", tool: "omp" },
    state: "configured",
    checkable: true,
    ...extra,
  };
}

function listing(servers: McpServer[], extra: Partial<McpServers> = {}): McpServers {
  return {
    workspaceId: "w1",
    threadId: "t1",
    running: true,
    projectConfig: "/work/w1/.omp/mcp.json",
    servers,
    ...extra,
  };
}

const SERVERS = [
  server("docs", { transport: "http", target: "https://docs.example/mcp", state: "connected", tools: 1 }),
  server("fs", { state: "failed", error: "ENOENT: no such file" }),
  server("old", { state: "disabled", checkable: false }),
  server("cursor", {
    checkable: false,
    source: { path: "/work/w1/.cursor/mcp.json", workspacePath: ".cursor/mcp.json", tool: "cursor" },
  }),
];

describe("mcpRows", () => {
  // spec: UI-290, UI-291, PRT-114
  test("marks, state, tools, error, source and the note of servers omp cannot check", () => {
    const rows = mcpRows(listing(SERVERS));
    expect(rows.map((row) => [row.name, row.mark, row.stateLabel, row.tools ?? null])).toEqual([
      ["docs", "done", "connected", "1 tool"],
      ["fs", "failed", "failed", null],
      ["old", "idle", "disabled", null],
      ["cursor", "idle", "configured", null],
    ]);
    expect(rows[1]?.error).toBe("ENOENT: no such file");
    expect(rows.map((row) => [row.source, row.openPath ?? null, row.origin])).toEqual([
      [".omp/mcp.json", ".omp/mcp.json", "omp"],
      [".omp/mcp.json", ".omp/mcp.json", "omp"],
      [".omp/mcp.json", ".omp/mcp.json", "omp"],
      [".cursor/mcp.json", ".cursor/mcp.json", "cursor"],
    ]);
    expect(rows.map((row) => row.note !== undefined)).toEqual([false, false, false, true]);
    expect(rows.map((row) => row.disabled)).toEqual([false, false, true, false]);
    expect(rows[0]?.transport).toBe("http");
  });

  // spec: UI-291
  test("what is checked turns into the spinner: every checkable server, or the one reconnected", () => {
    const marks = (pending: Parameters<typeof mcpRows>[1], extra: Partial<McpServers> = {}) =>
      mcpRows(listing(SERVERS, extra), pending).map((row) => row.mark);
    expect(marks({ kind: "check" })).toEqual(["running", "running", "idle", "idle"]);
    expect(marks({ kind: "check" }, { running: false })).toEqual(["done", "failed", "idle", "idle"]);
    expect(marks({ kind: "reconnect", name: "fs" })).toEqual(["done", "running", "idle", "idle"]);
    expect(marks({ kind: "reconnect", name: undefined }, { running: false })).toEqual([
      "running",
      "running",
      "idle",
      "idle",
    ]);
    const checking = mcpRows(listing(SERVERS), { kind: "check" });
    expect([checking[0]?.stateLabel, checking[0]?.tools, checking[1]?.error]).toEqual([
      "checking…",
      undefined,
      undefined,
    ]);
  });

  // spec: UI-290
  test("the status line says whose servers these are", () => {
    expect(mcpStatus(listing([]))).toBe("Checked by the thread's oh-my-pi.");
    expect(mcpStatus(listing([], { running: false }))).toBe("The thread is not running: its servers start with it.");
    expect(mcpStatus(listing([], { threadId: null, running: false }))).toBe(
      "No thread selected: the servers configured for this workspace, not checked.",
    );
    expect(mcpRows(listing([server("x", { checkable: false })], { running: false }))[0]?.note).toBeUndefined();
  });

  // spec: UI-348
  test("enabled HTTP and SSE servers can be reauthorized with /mcp reauth <name>", () => {
    const servers = [
      ...SERVERS,
      server("events", { transport: "sse" }),
      server("off", { transport: "http", state: "disabled" }),
    ];
    expect(mcpRows(listing(servers)).map((row) => [row.name, row.reauthorizable])).toEqual([
      ["docs", true],
      ["fs", false],
      ["old", false],
      ["cursor", false],
      ["events", true],
      ["off", false],
    ]);
    expect(reauthPrompt("docs")).toBe("/mcp reauth docs");
  });
});

describe("pendingRestart", () => {
  // spec: UI-349, PRT-134
  test("a held-back restart is named by its reasons, and only an MCP reason shows it in omfp:mcp", () => {
    expect(pendingRestart(undefined)).toBeUndefined();
    expect(pendingRestart([])).toBeUndefined();
    const named = (reasons: Parameters<typeof pendingRestart>[0]) => {
      const restart = pendingRestart(reasons);
      return [restart?.label, restart?.mcp];
    };
    expect(named(["mcp_config"])).toEqual(["Reload to load MCP tools", true]);
    expect(named(["mcp_reauth", "mcp_config"])).toEqual(["Reload to load MCP tools", true]);
    expect(named(["login"])).toEqual(["Restart to load new models", false]);
    expect(named(["login", "mcp_reauth"])).toEqual(["Restart to load new models and MCP tools", true]);
  });
});
