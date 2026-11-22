/** L4: `omfp:mcp` over a fake client (UI-290..UI-294, UI-348, UI-349). */

import { afterEach, describe, expect, test } from "bun:test";
import type { McpServer, McpServers, ThreadSummary } from "@omfp/protocol";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DRAFT_THREAD, ERRORS, OPEN_FILES, SELECTED_THREAD } from "../../src/sdk/index.ts";
import { BUILTIN_WIDGETS } from "../../src/view/registry.ts";
import { DEFAULT_MANIFEST } from "../../src/view/views.ts";
import { McpWidget } from "../../src/widgets/mcp/index.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { FakeClient, renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

const PROJECT = {
  path: "/work/w1/.omp/mcp.json",
  workspacePath: ".omp/mcp.json",
  tool: "omp",
} as const;
const CURSOR = { path: "/work/w1/.cursor/mcp.json", workspacePath: ".cursor/mcp.json", tool: "cursor" } as const;

function server(name: string, extra: Partial<McpServer> = {}): McpServer {
  return {
    name,
    transport: "stdio",
    target: `${name}-server`,
    source: PROJECT,
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

const CHECKED = listing([
  server("docs", { transport: "http", target: "http://127.0.0.1:9/mcp", state: "connected", tools: 2 }),
  server("fs", { state: "failed", error: "ENOENT: no such file or directory, posix_spawn 'fs-server'" }),
  server("mine", { source: CURSOR, state: "connected", tools: 1 }),
]);

/** The widget on thread `t1` of workspace `w1`, whose omp checked {@link CHECKED}. */
function setup(summary: Partial<ThreadSummary> = {}) {
  const base = testHost(new FakeClient());
  base.client.answers.set("thread.mcp_servers", () => CHECKED);
  base.host.window.set(SELECTED_THREAD, "t1");
  const test = renderWithHost(<McpWidget />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "work", "w1", summary)] });
  test.flush();
  return { ...test, user: userEvent.setup() };
}

function boxes() {
  return screen.queryAllByTestId("mcp-server").map((box) => ({
    name: box.dataset.name,
    state: box.dataset.state,
    text: within(box).getByTestId("mcp-state").textContent,
  }));
}

describe("omfp:mcp", () => {
  // spec: UI-290, UI-291, PRT-111
  test("the selected thread's servers: a box each with its mark, state, tools, error and command", async () => {
    const { client } = setup();
    await waitFor(() => expect(boxes().length).toBe(3));
    expect(boxes()).toEqual([
      { name: "docs", state: "done", text: "connected · 2 tools" },
      { name: "fs", state: "failed", text: "failed" },
      { name: "mine", state: "done", text: "connected · 1 tool" },
    ]);
    expect(screen.getAllByTestId("mcp-target")[0]?.textContent).toBe("httphttp://127.0.0.1:9/mcp");
    expect(screen.getAllByTestId("mcp-server-error").map((line) => line.textContent)).toEqual([
      "ENOENT: no such file or directory, posix_spawn 'fs-server'",
    ]);
    expect(screen.getByTestId("mcp-status").textContent).toBe("Checked by the thread's omp.");
    expect(client.callsTo("thread.mcp_servers").map((call) => call.params)).toEqual([{ threadId: "t1" }]);
    expect(screen.getAllByRole("heading", { level: 2 }).map((heading) => heading.textContent)).toEqual([
      "docs",
      "fs",
      "mine",
    ]);
  });

  // spec: UI-292
  test("a source is its workspace path, a button opening it in a file tab, followed by its tool", async () => {
    const { host, user } = setup();
    await waitFor(() => expect(boxes().length).toBe(3));
    const sources = screen.getAllByTestId("mcp-source");
    expect(sources.map((source) => source.textContent)).toEqual([".omp/mcp.json", ".omp/mcp.json", ".cursor/mcp.json"]);
    expect(
      screen.getAllByTestId("mcp-server").map((box) => box.querySelector(".omfp-mcp__origin")?.textContent),
    ).toEqual(["omp", "omp", "cursor"]);
    await user.click(sources[0] ?? document.body);
    await user.click(sources[2] ?? document.body);
    expect(host.window.get(OPEN_FILES).w1?.tabs.map((tab) => tab.path)).toEqual([".omp/mcp.json", ".cursor/mcp.json"]);
  });

  // spec: UI-294
  test("without servers it names the workspace's .omp/mcp.json and the JSON shapes, no user config", async () => {
    const base = testHost(new FakeClient());
    base.client.answers.set("thread.mcp_servers", () => listing([]));
    base.host.window.set(SELECTED_THREAD, "t1");
    const test = renderWithHost(<McpWidget />, base);
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "work", "w1")] });
    test.flush();
    const none = await screen.findByTestId("mcp-none");
    const hint = none.textContent ?? "";
    expect(hint).toContain("No MCP servers");
    expect(hint).toContain("Add them to this workspace's /work/w1/.omp/mcp.json");
    expect(hint).toContain('"command": "<program>"');
    expect(hint).toContain('"type": "http"');
    expect(hint).toContain("Cursor (.cursor/)");
    expect(hint).not.toContain("every workspace");
  });

  // spec: UI-293, PRT-112
  test("Reconnect restarts the thread for one server, which spins until the daemon answers", async () => {
    const { client, user } = setup();
    await waitFor(() => expect(boxes().length).toBe(3));
    await user.click(screen.getByRole("button", { name: "Reconnect fs (restarts the thread's omp)" }));

    const [reconnect] = client.callsTo("thread.mcp_reconnect");
    expect(reconnect?.params).toEqual({ threadId: "t1", name: "fs" });
    await waitFor(() => expect(boxes().map((box) => box.state)).toEqual(["done", "running", "done"]));
    expect(boxes()[1]?.text).toBe("checking…");
    expect(screen.getByTestId("mcp-reload-all").hasAttribute("disabled")).toBe(true);

    reconnect?.resolve(
      listing([server("docs", { state: "connected", tools: 2 }), server("fs", { state: "connected", tools: 4 })]),
    );
    await waitFor(() => expect(boxes().map((box) => box.text)).toEqual(["connected · 2 tools", "connected · 4 tools"]));
    expect(screen.getByTestId("mcp-reload-all").hasAttribute("disabled")).toBe(false);
  });

  // spec: UI-293, PRT-112
  test("Reload all restarts the thread for every server; a refusal shows the daemon's reason", async () => {
    const { client, user } = setup();
    await waitFor(() => expect(boxes().length).toBe(3));
    await user.click(screen.getByTestId("mcp-reload-all"));
    const [reload] = client.callsTo("thread.mcp_reconnect");
    expect(reload?.params).toEqual({ threadId: "t1" });
    await waitFor(() => expect(boxes().map((box) => box.state)).toEqual(["running", "running", "running"]));

    reload?.reject(new Error("the thread is working: a restart would abort it"));
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toBe("the thread is working: a restart would abort it"),
    );
    expect(boxes().map((box) => box.state)).toEqual(["done", "failed", "done"]);

    client.answers.set("thread.mcp_servers", () => listing([server("docs", { state: "connected", tools: 3 })]));
    await user.click(screen.getByTestId("mcp-check"));
    await waitFor(() => expect(boxes().map((box) => box.text)).toEqual(["connected · 3 tools"]));
    expect(screen.queryAllByRole("alert").length).toBe(0);
  });

  // spec: UI-290, PRT-110
  test("without a selected thread, the current workspace's configured servers, with no reconnect", async () => {
    const base = testHost(new FakeClient());
    base.client.answers.set("workspaces.mcp_servers", () =>
      listing([server("docs"), server("old", { state: "disabled", checkable: false })], {
        threadId: null,
        running: false,
      }),
    );
    base.host.window.set(DRAFT_THREAD, { workspaceId: "w1" });
    const test = renderWithHost(<McpWidget />, base);
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [] });
    test.flush();

    await waitFor(() => expect(boxes().map((box) => box.text)).toEqual(["configured", "disabled"]));
    expect(test.client.callsTo("workspaces.mcp_servers").map((call) => call.params)).toEqual([{ workspaceId: "w1" }]);
    expect(screen.getByTestId("mcp-status").textContent).toBe(
      "No thread selected: the servers configured for this workspace, not checked.",
    );
    expect(screen.queryAllByTestId("mcp-reconnect").length).toBe(0);
    expect(screen.queryAllByTestId("mcp-reload-all").length).toBe(0);
  });

  // spec: UI-290, UI-300
  test("a built-in `:widget add` offers, in the default view as the side tab after Background", () => {
    expect(BUILTIN_WIDGETS.has("omfp:mcp")).toBe(true);
    expect(JSON.stringify(DEFAULT_MANIFEST)).toContain('"omfp:background","omfp:mcp"]');
  });

  // spec: UI-348
  test("Reauthorize, on HTTP and SSE servers only, sends /mcp reauth <name> to the thread; a refusal is an error toast", async () => {
    const { client, host, user } = setup();
    await waitFor(() => expect(boxes().length).toBe(3));
    const buttons = screen.getAllByTestId("mcp-reauthorize");
    expect(buttons.map((button) => button.closest<HTMLElement>("[data-testid=mcp-server]")?.dataset.name)).toEqual([
      "docs",
    ]);
    await user.click(screen.getByRole("button", { name: "Reauthorize docs (sends /mcp reauth docs to the thread)" }));
    const [prompt] = client.callsTo("thread.prompt");
    expect(prompt?.params).toEqual({ threadId: "t1", text: "/mcp reauth docs" });
    prompt?.reject(new Error("the thread is busy"));
    await waitFor(() => expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["the thread is busy"]));
  });

  // spec: UI-349
  test("while the daemon holds a restart for new MCP tools, one click reloads the thread", async () => {
    const { client, user } = setup({ restartPending: ["mcp_reauth", "login"] });
    await waitFor(() => expect(boxes().length).toBe(3));
    const reload = screen.getByTestId("mcp-reload-tools");
    expect(reload.textContent).toBe("Restart to load new models and MCP tools");
    await user.click(reload);
    expect(client.callsTo("thread.mcp_reconnect").map((call) => call.params)).toEqual([{ threadId: "t1" }]);
  });
});
