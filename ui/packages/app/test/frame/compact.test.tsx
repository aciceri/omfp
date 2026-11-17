/** L4: `:compact` on the selected thread (UI-910, UI-911) over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import type { ThreadSummary } from "@omfp/protocol";
import { act, cleanup, waitFor } from "@testing-library/react";
import { GlobalCommands } from "../../src/frame/commands.tsx";
import { DEFAULT_BINDINGS } from "../../src/keymap/keymap.ts";
import { ERRORS, type OmfpHost, SELECTED_THREAD, SUCCESSES } from "../../src/sdk/index.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

function setup(summary: ThreadSummary = thread("t1", "checkout", "w1")) {
  const base = testHost();
  base.host.window.set(SELECTED_THREAD, summary.id);
  const test = renderWithHost(
    <GlobalCommands viewIds={[]} currentView={undefined} widgets={[]} addWidget={() => Promise.resolve(false)} />,
    base,
  );
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [summary] });
  test.flush();
  return test;
}

const successes = (host: OmfpHost) => host.window.get(SUCCESSES).map((success) => success.text);

describe(":compact", () => {
  // spec: UI-910, UI-911
  test("compacts the selected thread with the words after it as the summary's focus, then says how big it was", async () => {
    const test = setup();
    test.client.answers.set("thread.compact", () => ({
      summary: "We fixed the cart",
      firstKeptEntryId: "e9",
      tokensBefore: 123_456,
    }));
    act(() => {
      expect(test.host.commands.run("compact", ["keep", "the", "API", "decisions"])).toBe(true);
    });
    await waitFor(() => expect(successes(test.host)).toEqual(["Compacted the context (it held 123k tokens)"]));
    expect(test.client.callsTo("thread.compact").map((call) => call.params)).toEqual([
      { threadId: "t1", instructions: "keep the API decisions" },
    ]);
  });

  // spec: UI-910, UI-911
  test("without words it sends no focus; a result without a size still says it compacted", async () => {
    const test = setup();
    test.client.answers.set("thread.compact", () => ({ summary: "", firstKeptEntryId: "", tokensBefore: 0 }));
    act(() => {
      test.host.commands.run("thread.compact", ["  "]);
    });
    await waitFor(() => expect(successes(test.host)).toEqual(["Compacted the context"]));
    expect(test.client.callsTo("thread.compact").map((call) => call.params)).toEqual([{ threadId: "t1" }]);
  });

  // spec: UI-913
  test("a first word naming a mode sends omp's /compact with it; snapcompact refuses a focus", async () => {
    const test = setup();
    test.client.answers.set("thread.prompt", () => ({ promptId: "req_1", agentInvoked: false }));
    act(() => {
      test.host.commands.run("compact", ["Soft", "keep", "the", "plan"]);
    });
    await waitFor(() => expect(successes(test.host)).toEqual(["Compacting the context (soft)"]));
    expect(test.client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: "t1", text: "/compact soft keep the plan" },
    ]);
    act(() => {
      test.host.commands.run("compact", ["snapcompact", "the", "plan"]);
    });
    expect(test.host.window.get(ERRORS)).toEqual([
      "snapcompact takes no focus: it keeps the history as images, without a summary",
    ]);
    expect(test.client.callsTo("thread.prompt").length).toBe(1);
    expect(test.client.callsTo("thread.compact")).toEqual([]);
    expect(test.host.commands.resolve("compact")?.args?.([])).toEqual(["soft", "remote", "snapcompact"]);
  });

  // spec: UI-910
  test("is refused while the thread works, without calling omp", () => {
    const test = setup(thread("t1", "checkout", "w1", { settled: false }));
    act(() => {
      test.host.commands.run("compact", []);
    });
    expect(test.host.window.get(ERRORS)).toEqual([
      "The thread is working: compact it once its turn ends (Abort ends it now)",
    ]);
    expect(test.client.callsTo("thread.compact")).toEqual([]);
    expect(successes(test.host)).toEqual([]);
  });

  // spec: UI-911
  test("omp's refusal is an error toast", async () => {
    const test = setup();
    act(() => {
      test.host.commands.run("compact", []);
    });
    const [call] = test.client.callsTo("thread.compact");
    call?.reject(
      new CallError({
        kind: "daemon",
        error: { code: "omp_error", message: "Nothing to compact (session too small)" },
      }),
    );
    await waitFor(() =>
      expect(test.host.window.get(ERRORS)).toEqual(["Nothing to compact (session too small) (omp_error)"]),
    );
    expect(successes(test.host)).toEqual([]);
  });

  // spec: UI-910
  test("is a command of a selected unarchived thread without a default key", () => {
    const test = setup();
    const command = test.host.commands.list().find((entry) => entry.id === "thread.compact");
    expect(command?.aliases).toEqual(["compact"]);
    expect(DEFAULT_BINDINGS.filter((binding) => binding.command === "thread.compact")).toEqual([]);
    act(() =>
      test.client.index({
        workspaces: [workspace("w1")],
        threads: [thread("t1", "checkout", "w1", { archivedAt: 5 })],
      }),
    );
    test.flush();
    expect(test.host.commands.resolve("compact")).toBeUndefined();
    act(() => test.host.window.set(SELECTED_THREAD, undefined));
    expect(test.host.commands.resolve("compact")).toBeUndefined();
  });
});
