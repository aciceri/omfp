/** L4: `thread.resume` (`:resume`), the Resume of a turn a daemon restart interrupted (UI-934). */

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup } from "@testing-library/react";
import { GlobalCommands } from "../../src/frame/commands.tsx";
import { SELECTED_THREAD } from "../../src/sdk/index.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

function setup(selected: string) {
  const base = testHost();
  base.host.window.set(SELECTED_THREAD, selected);
  const test = renderWithHost(
    <GlobalCommands viewIds={[]} currentView={undefined} widgets={[]} addWidget={() => Promise.resolve(false)} />,
    base,
  );
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1")],
    threads: [thread("t1", "cut", "w1", { interruptedAt: 1_732_000_000_000 }), thread("t2", "fine", "w1")],
  });
  test.flush();
  return test;
}

describe("thread.resume", () => {
  // spec: UI-934, PRT-230
  test("`:resume` continues the selected thread's interrupted turn, and is offered only while it has one", () => {
    const { client, host } = setup("t1");
    expect(host.commands.resolve("resume")?.id).toBe("thread.resume");
    expect(host.commands.list().find((command) => command.id === "thread.resume")?.description).toContain(
      "interrupted by a daemon restart",
    );
    act(() => {
      expect(host.commands.run("thread.resume")).toBe(true);
    });
    expect(client.callsTo("thread.resume").map((call) => call.params)).toEqual([{ threadId: "t1" }]);

    act(() => host.window.set(SELECTED_THREAD, "t2"));
    expect(host.commands.resolve("thread.resume")).toBeUndefined();
  });
});
