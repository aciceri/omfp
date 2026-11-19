/** L4: `useMcp`, `useMcpTool` and `mcpText` (AGT-095) over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import type { McpCallResult } from "@omfp/protocol";
import { act, cleanup, screen } from "@testing-library/react";
import { type Mcp, mcpText, useMcp, useMcpTool, type WidgetInfoScope, WidgetScope } from "../../src/sdk/index.ts";
import { renderWithHost } from "../support/host.tsx";

afterEach(cleanup);

const SCOPE: WidgetInfoScope = { instance: "issues", reference: "local:issues", workspaceId: "w1" };

let latest: Mcp | undefined;

function Probe({ server, workspaceId }: { server: string | null; workspaceId?: string }) {
  latest = useMcp(server, workspaceId === undefined ? undefined : { workspaceId });
  return <span data-testid="tools">{(latest.tools ?? []).map((tool) => tool.name).join(",")}</span>;
}

function ToolProbe({ args }: { args: Record<string, unknown> | null }) {
  const state = useMcpTool("linear", "list_issues", args);
  return <span data-testid="text">{mcpText(state.data)}</span>;
}

const RESULT: McpCallResult = { content: [{ type: "text", text: "LIN-1" }], isError: false };

describe("useMcp", () => {
  // spec: AGT-095
  test("lists the server's tools with mcp.tools in the widget's workspace", async () => {
    const test = renderWithHost(
      <WidgetScope value={SCOPE}>
        <Probe server="linear" />
      </WidgetScope>,
    );
    const [listing] = test.client.callsTo("mcp.tools");
    expect(listing?.params).toEqual({ workspaceId: "w1", server: "linear" });
    await act(async () => {
      listing?.resolve({
        workspaceId: "w1",
        server: "linear",
        tools: [{ name: "list_issues", inputSchema: {} }],
      });
    });
    expect(screen.getByTestId("tools").textContent).toBe("list_issues");
  });

  // spec: AGT-095
  test("the workspaceId option overrides the widget's; a null server lists nothing", () => {
    const test = renderWithHost(
      <WidgetScope value={SCOPE}>
        <Probe server="docs" workspaceId="w2" />
        <Probe server={null} />
      </WidgetScope>,
    );
    expect(test.client.callsTo("mcp.tools").map((call) => call.params)).toEqual([
      { workspaceId: "w2", server: "docs" },
    ]);
  });

  // spec: AGT-095
  test("call() sends mcp.call with the arguments", async () => {
    const test = renderWithHost(
      <WidgetScope value={SCOPE}>
        <Probe server="linear" />
      </WidgetScope>,
    );
    const pending = latest?.call("create_issue", { title: "Bug" });
    const [call] = test.client.callsTo("mcp.call");
    expect(call?.params).toEqual({
      workspaceId: "w1",
      server: "linear",
      tool: "create_issue",
      arguments: { title: "Bug" },
    });
    call?.resolve(RESULT);
    expect(await pending).toEqual(RESULT);
  });
});

describe("useMcpTool", () => {
  // spec: AGT-095
  test("calls the tool with its args and shows the text; args null calls nothing", async () => {
    const test = renderWithHost(
      <WidgetScope value={SCOPE}>
        <ToolProbe args={{ limit: 10 }} />
        <ToolProbe args={null} />
      </WidgetScope>,
    );
    const calls = test.client.callsTo("mcp.call");
    expect(calls.map((call) => call.params)).toEqual([
      { workspaceId: "w1", server: "linear", tool: "list_issues", arguments: { limit: 10 } },
    ]);
    await act(async () => {
      calls[0]?.resolve(RESULT);
    });
    expect(screen.getAllByTestId("text").map((span) => span.textContent)).toEqual(["LIN-1", ""]);
  });
});

describe("mcpText", () => {
  // spec: AGT-095
  test("joins the text blocks with newlines and skips the others", () => {
    expect(
      mcpText({
        content: [
          { type: "text", text: "a" },
          { type: "image", data: "xx", mimeType: "image/png" },
          { type: "text", text: "b" },
        ],
        isError: false,
      }),
    ).toBe("a\nb");
    expect(mcpText({ content: [], isError: false })).toBe("");
  });
});
