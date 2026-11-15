/** L4: the widget hooks of the SDK (`useWidget`, `useViews`, `useStored`, `useJob`, `useToolRun`) over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import type { JsonValue, ThreadEvent } from "@omfp/core";
import {
  type JobInfo,
  type JobLine,
  type JobState,
  jobTopic,
  type TerminalInfo,
  type TerminalState,
  terminalTopic,
  viewsTopic,
} from "@omfp/protocol";
import { act, cleanup, screen } from "@testing-library/react";
import { type ReactNode, useState } from "react";
import {
  SELECTED_THREAD,
  SlotReport,
  SlotReportScope,
  type ToolRunTarget,
  useJob,
  useStored,
  useTerminal,
  useToolRun,
  useViews,
  useWidget,
  type WidgetInfoScope,
  WidgetScope,
} from "../../src/sdk/index.ts";
import { type CallRecord, released, renderWithHost, type TestHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

const WS = "01W";
const SCOPE: WidgetInfoScope = { instance: "tests", reference: "./widgets/tests", workspaceId: WS };

function inScope(ui: ReactNode, scope: WidgetInfoScope = SCOPE): ReactNode {
  return <WidgetScope value={scope}>{ui}</WidgetScope>;
}

function text(testId: string): string | null {
  return screen.getByTestId(testId).textContent;
}

function only(calls: CallRecord[]): CallRecord {
  expect(calls.length).toBe(1);
  const [call] = calls;
  if (call === undefined) {
    throw new Error("no call");
  }
  return call;
}

function Scope() {
  const { instance, reference, workspaceId } = useWidget();
  return <span data-testid="scope">{[instance, reference, workspaceId].join(" ")}</span>;
}

function Counter() {
  const [count, setCount, loaded] = useStored("count", 0);
  return (
    <>
      <span data-testid="count">{`${count} ${loaded}`}</span>
      <button type="button" data-testid="inc" onClick={() => setCount((value) => (value ?? 0) + 1)} />
      <button type="button" data-testid="set" onClick={() => setCount(10)} />
      <button type="button" data-testid="clear" onClick={() => setCount(null)} />
    </>
  );
}

function ViewsProbe() {
  const views = useViews(WS);
  return <span data-testid="views">{views === undefined ? "none" : views.widgets.length}</span>;
}

describe("widget scope", () => {
  // spec: AGT-016
  test("useWidget gives the instance, reference and workspace of the slot", () => {
    renderWithHost(inScope(<Scope />));
    expect(text("scope")).toBe("tests ./widgets/tests 01W");
  });

  // spec: AGT-016
  test("useWidget outside a widget slot throws", () => {
    expect(() => renderWithHost(<Scope />)).toThrow("useWidget outside a widget slot");
  });

  // spec: PRT-040, AGT-016
  test("useViews follows views/<id> while mounted", async () => {
    const { client, flush, view } = renderWithHost(<ViewsProbe />);
    expect(client.subscribed).toEqual([viewsTopic(WS)]);
    expect(text("views")).toBe("none");
    client.views(WS, { widgets: [{ source: "user", name: "clock", version: "1", entry: "index.tsx" }] });
    flush();
    expect(text("views")).toBe("1");
    view.unmount();
    await released();
    expect(client.unsubscribed).toEqual([viewsTopic(WS)]);
  });
});

describe("useStored", () => {
  // spec: AGT-018
  test("reads the instance's value, initial until stored, and follows pushes", () => {
    const { client, flush } = renderWithHost(inScope(<Counter />));
    expect(text("count")).toBe("0 false");
    client.views(WS, { stored: { other: { count: 7 } } });
    flush();
    expect(text("count")).toBe("0 true");
    client.viewsEvents(WS, { kind: "widget_stored", instance: "tests", key: "count", value: 3 });
    flush();
    expect(text("count")).toBe("3 true");
    client.viewsEvents(WS, { kind: "widget_stored", instance: "tests", key: "count", value: null });
    flush();
    expect(text("count")).toBe("0 true");
  });

  // spec: AGT-018
  test("set stores a value, an updater's result or null with widgets.state_set", () => {
    const { client, flush } = renderWithHost(inScope(<Counter />));
    client.views(WS, { stored: { tests: { count: 4 } } });
    flush();

    act(() => screen.getByTestId("inc").click());
    act(() => screen.getByTestId("inc").click());
    act(() => screen.getByTestId("set").click());
    act(() => screen.getByTestId("clear").click());

    expect(client.callsTo("widgets.state_set").map((call) => call.params)).toEqual([
      { workspaceId: WS, instance: "tests", key: "count", value: 5 },
      // The second updater starts from the value set last, before its push arrived.
      { workspaceId: WS, instance: "tests", key: "count", value: 6 },
      { workspaceId: WS, instance: "tests", key: "count", value: 10 },
      { workspaceId: WS, instance: "tests", key: "count", value: null },
    ]);
  });
});

function info(id: string, status: JobInfo["status"] = { state: "running" }): JobInfo {
  return { id, workspaceId: WS, command: "make test", startedAt: 1, status };
}

function jobState(id: string, lines: JobLine[], status?: JobInfo["status"]): JobState {
  return { job: info(id, status), lines, dropped: 0 };
}

const out = (text: string): JobLine => ({ stream: "stdout", text });
const err = (text: string): JobLine => ({ stream: "stderr", text });

function JobProbe() {
  const job = useJob("make test");
  return (
    <>
      <span data-testid="job">
        {[job.status, job.exitCode, job.signal, job.runs, job.startedAt, job.endedAt]
          .map((part) => String(part))
          .join(" ")}
      </span>
      <span data-testid="output">
        {[job.lines, job.stdout, job.stderr].map((lines) => lines.join("|")).join(" / ")}
      </span>
      <button type="button" data-testid="restart" onClick={job.restart} />
      <button type="button" data-testid="kill" onClick={job.kill} />
    </>
  );
}

const STORED = "job:make test";

/** Renders the job probe with `stored` as the views state's values of the instance. */
function renderJob(stored: Record<string, unknown> | undefined, slot?: (test: TestHost) => SlotReport): TestHost {
  const test = testHost();
  const report = slot?.(test);
  renderWithHost(inScope(<SlotReportScope value={report}>{<JobProbe />}</SlotReportScope>), test);
  if (stored !== undefined) {
    test.client.views(WS, { stored: { tests: stored } });
    test.flush();
  }
  return test;
}

describe("useJob", () => {
  // spec: AGT-019
  test("without a stored job it starts one once the views state is known and stores its id and runs", async () => {
    const { client, flush } = renderJob(undefined);
    expect(text("job")).toBe("pending null null 0 null null");
    expect(client.callsTo("jobs.start")).toEqual([]);
    client.views(WS);
    flush();

    const start = only(client.callsTo("jobs.start"));
    expect(start.params).toEqual({ workspaceId: WS, command: "make test" });
    flush();
    expect(client.callsTo("jobs.start").length).toBe(1);
    await act(async () => start.resolve(info("01JOB")));
    expect(only(client.callsTo("widgets.state_set")).params).toEqual({
      workspaceId: WS,
      instance: "tests",
      key: STORED,
      value: { id: "01JOB", runs: 1 },
    });
    expect(text("job")).toBe("pending null null 0 null null");

    client.viewsEvents(WS, { kind: "widget_stored", instance: "tests", key: STORED, value: { id: "01JOB", runs: 1 } });
    flush();
    expect(client.subscribed).toContain(jobTopic("01JOB"));
    expect(text("job")).toBe("pending null null 1 null null");
    client.job(jobState("01JOB", [out("one")]));
    client.jobEvents("01JOB", { kind: "job_output", stream: "stderr", text: "two" });
    flush();
    expect(text("job")).toBe("running null null 1 1 null");
    expect(text("output")).toBe("one|two / one / two");
    client.jobEvents("01JOB", { kind: "job_exited", code: 2, signal: null, endedAt: 5 });
    flush();
    expect(text("job")).toBe("failed 2 null 1 1 5");
    expect(client.callsTo("jobs.start").length).toBe(1);
  });

  // spec: AGT-019
  test("a stored job is followed, running or finished, without starting another", () => {
    const { client, flush } = renderJob({ [STORED]: { id: "01OLD", runs: 3 } });
    client.job(jobState("01OLD", [out("done")], { state: "exited", code: null, signal: 15, endedAt: 3 }));
    flush();
    expect(text("job")).toBe("failed null 15 3 1 3");
    client.job(jobState("01OLD", [out("done")], { state: "exited", code: 0, signal: null, endedAt: 4 }));
    flush();
    expect(text("job")).toBe("succeeded 0 null 3 1 4");
    expect(client.callsTo("jobs.start")).toEqual([]);
  });

  // spec: AGT-019
  test("a stored job the daemon no longer knows is replaced by a new one, counted as a run", async () => {
    const { client, flush } = renderJob({ [STORED]: { id: "01GONE", runs: 2 } });
    expect(client.callsTo("jobs.start")).toEqual([]);
    client.failTopic(jobTopic("01GONE"), { code: "not_found", message: "no job 01GONE" });
    flush();
    const start = only(client.callsTo("jobs.start"));
    expect(start.params).toEqual({ workspaceId: WS, command: "make test" });
    await act(async () => start.resolve(info("01NEW")));
    expect(only(client.callsTo("widgets.state_set")).params).toMatchObject({
      key: STORED,
      value: { id: "01NEW", runs: 3 },
    });
  });

  // spec: AGT-019
  test("restart kills the running job, then starts and stores a new one, pending meanwhile; kill stops it", async () => {
    const { client, flush } = renderJob({ [STORED]: { id: "01JOB", runs: 1 } });
    client.job(jobState("01JOB", []));
    flush();
    expect(text("job")).toBe("running null null 1 1 null");

    act(() => screen.getByTestId("kill").click());
    expect(only(client.callsTo("jobs.kill")).params).toEqual({ jobId: "01JOB" });

    act(() => screen.getByTestId("restart").click());
    const kills = client.callsTo("jobs.kill");
    expect(kills.length).toBe(2);
    expect(client.callsTo("jobs.start")).toEqual([]);
    expect(text("job")).toBe("pending null null 1 1 null");
    await act(async () => kills[1]?.resolve({}));
    const start = only(client.callsTo("jobs.start"));
    await act(async () => start.resolve(info("01NEW")));
    expect(only(client.callsTo("widgets.state_set")).params).toMatchObject({
      key: STORED,
      value: { id: "01NEW", runs: 2 },
    });
    client.viewsEvents(WS, { kind: "widget_stored", instance: "tests", key: STORED, value: { id: "01NEW", runs: 2 } });
    client.job(jobState("01NEW", [], undefined));
    flush();
    expect(text("job")).toBe("running null null 2 1 null");
  });

  // spec: AGT-019
  test("restart of a finished job starts a new one without killing", () => {
    const { client, flush } = renderJob({ [STORED]: { id: "01JOB", runs: 1 } });
    client.job(jobState("01JOB", [], { state: "exited", code: 0, signal: null, endedAt: 2 }));
    flush();
    act(() => screen.getByTestId("restart").click());
    expect(client.callsTo("jobs.kill")).toEqual([]);
    expect(client.callsTo("jobs.start").length).toBe(1);
    expect(text("job")).toBe("pending 0 null 1 1 2");
  });

  // spec: AGT-019, AGT-026
  test("the job is reported to the widget's slot with its last lines, and leaves it on unmount", () => {
    let slot: SlotReport | undefined;
    const test = renderJob({ [STORED]: { id: "01JOB", runs: 2 } }, ({ host }) => {
      slot = new SlotReport(host.reports, {
        reference: "./widgets/tests",
        instance: "tests",
        source: "workspace",
        name: "tests",
        modulePath: "/widgets/test/01W/tests/",
      });
      return slot;
    });
    const lines = Array.from({ length: 12 }, (_, index) => (index % 2 === 0 ? out : err)(`line ${index}`));
    test.client.job(jobState("01JOB", lines));
    test.flush();
    expect(slot?.report().jobs).toEqual([
      {
        command: "make test",
        status: "running",
        exitCode: null,
        signal: null,
        runs: 2,
        startedAt: 1,
        tail: lines.slice(-10).map((line) => line.text),
      },
    ]);
    cleanup();
    expect(slot?.report().jobs).toEqual([]);
  });
});

/** A search widget: one job slot, `search`, whose command follows the query. */
function SearchProbe() {
  const [query, setQuery] = useState("a");
  const job = useJob(`grep -r ${query}`, { key: "search" });
  return (
    <>
      <span data-testid="job">{[job.status, job.runs, job.lines.join("|")].join(" ")}</span>
      <button type="button" data-testid="query" onClick={() => setQuery((current) => `${current}b`)} />
      <button type="button" data-testid="restart" onClick={job.restart} />
    </>
  );
}

const SEARCH = "jobkey:search";

function renderSearch(stored: Record<string, unknown>, slot?: (test: TestHost) => SlotReport): TestHost {
  const test = testHost();
  const report = slot?.(test);
  renderWithHost(inScope(<SlotReportScope value={report}>{<SearchProbe />}</SlotReportScope>), test);
  test.client.views(WS, { stored: { tests: stored } });
  test.flush();
  return test;
}

function searchState(id: string, command: string, lines: JobLine[]): JobState {
  return { job: { ...info(id), command }, lines, dropped: 0 };
}

describe("useJob with a key", () => {
  // spec: AGT-086, UI-355
  test("the first run is stored under the key with its command, without replacing anything", async () => {
    const { client, flush } = renderSearch({});
    const start = only(client.callsTo("jobs.start"));
    expect(start.params).toEqual({ workspaceId: WS, command: "grep -r a" });
    await act(async () => start.resolve(info("01A")));
    expect(only(client.callsTo("widgets.state_set")).params).toEqual({
      workspaceId: WS,
      instance: "tests",
      key: SEARCH,
      value: { id: "01A", runs: 1, command: "grep -r a" },
    });
    flush();
  });

  // spec: AGT-086, UI-355
  test("a new command under the key replaces the stored job: pending without the old lines, one stored slot", async () => {
    const { client, flush } = renderSearch({ [SEARCH]: { id: "01A", runs: 1, command: "grep -r a" } });
    client.job(searchState("01A", "grep -r a", [out("hit a")]));
    flush();
    expect(text("job")).toBe("running 1 hit a");
    expect(client.callsTo("jobs.start")).toEqual([]);

    act(() => screen.getByTestId("query").click());
    const start = only(client.callsTo("jobs.start"));
    expect(start.params).toEqual({ workspaceId: WS, command: "grep -r ab", replaces: "01A" });
    expect(text("job")).toBe("pending 1 ");
    expect(client.callsTo("jobs.kill")).toEqual([]);
    await act(async () => start.resolve(info("01B")));
    const stored = { id: "01B", runs: 2, command: "grep -r ab" };
    expect(only(client.callsTo("widgets.state_set")).params).toEqual({
      workspaceId: WS,
      instance: "tests",
      key: SEARCH,
      value: stored,
    });
    client.viewsEvents(WS, { kind: "widget_stored", instance: "tests", key: SEARCH, value: stored });
    flush();
    expect(client.subscribed).toContain(jobTopic("01B"));
    expect(client.unsubscribed).toContain(jobTopic("01A"));
    client.job(searchState("01B", "grep -r ab", [out("hit ab")]));
    flush();
    expect(text("job")).toBe("running 2 hit ab");
    expect(client.callsTo("jobs.start").length).toBe(1);
  });

  // spec: UI-355
  test("a command changing again while its replacement starts runs the newest once the first is stored", async () => {
    const { client, flush } = renderSearch({ [SEARCH]: { id: "01A", runs: 1, command: "grep -r a" } });
    act(() => screen.getByTestId("query").click());
    act(() => screen.getByTestId("query").click());
    const first = only(client.callsTo("jobs.start"));
    expect(first.params).toEqual({ workspaceId: WS, command: "grep -r ab", replaces: "01A" });
    await act(async () => first.resolve(info("01B")));
    client.viewsEvents(WS, {
      kind: "widget_stored",
      instance: "tests",
      key: SEARCH,
      value: { id: "01B", runs: 2, command: "grep -r ab" },
    });
    flush();
    const starts = client.callsTo("jobs.start");
    expect(starts.length).toBe(2);
    expect(starts[1]?.params).toEqual({ workspaceId: WS, command: "grep -r abb", replaces: "01B" });
  });

  // spec: AGT-086, UI-355
  test("restart under a key replaces the running job in one jobs.start, without jobs.kill", () => {
    const { client, flush } = renderSearch({ [SEARCH]: { id: "01A", runs: 1, command: "grep -r a" } });
    client.job(searchState("01A", "grep -r a", []));
    flush();
    act(() => screen.getByTestId("restart").click());
    expect(client.callsTo("jobs.kill")).toEqual([]);
    expect(only(client.callsTo("jobs.start")).params).toEqual({
      workspaceId: WS,
      command: "grep -r a",
      replaces: "01A",
    });
    expect(text("job")).toBe("pending 1 ");
  });

  // spec: UI-356, AGT-026
  test("the slot report holds one job per key, with the current command", () => {
    let slot: SlotReport | undefined;
    const test = renderSearch({ [SEARCH]: { id: "01A", runs: 1, command: "grep -r a" } }, ({ host }) => {
      slot = new SlotReport(host.reports, {
        reference: "./widgets/tests",
        instance: "tests",
        source: "workspace",
        name: "tests",
        modulePath: "/widgets/test/01W/tests/",
      });
      return slot;
    });
    test.client.job(searchState("01A", "grep -r a", [out("hit a")]));
    test.flush();
    expect(slot?.report().jobs.map((job) => [job.command, job.status, job.tail.join("|")])).toEqual([
      ["grep -r a", "running", "hit a"],
    ]);
    act(() => screen.getByTestId("query").click());
    expect(slot?.report().jobs.map((job) => [job.command, job.status, job.tail.join("|")])).toEqual([
      ["grep -r ab", "pending", ""],
    ]);
  });
});

const THREAD = "01T";

function TerminalProbe() {
  const terminal = useTerminal("read secret", { cwd: "repo", cols: 90, rows: 30 });
  return (
    <>
      <span data-testid="terminal">
        {[terminal.status, terminal.runs, terminal.state?.scrollback.length ?? 0].join(" ")}
      </span>
      <button type="button" data-testid="terminal-input" onClick={() => terminal.input("secret\n")} />
      <button type="button" data-testid="terminal-resize" onClick={() => terminal.resize(100, 40)} />
      <button type="button" data-testid="terminal-kill" onClick={terminal.kill} />
    </>
  );
}

function terminalInfo(id: string): TerminalInfo {
  return {
    id,
    workspaceId: WS,
    command: "read secret",
    cwd: "/workspace/repo",
    cols: 90,
    rows: 30,
    startedAt: 1,
    status: { state: "running" },
  };
}

describe("useTerminal", () => {
  // spec: TRM-101, TRM-102, TRM-105
  test("starts once, follows the stored PTY and forwards input, resize and kill", async () => {
    const test = testHost();
    renderWithHost(inScope(<TerminalProbe />), test);
    test.client.views(WS);
    test.flush();

    const start = only(test.client.callsTo("terminals.start"));
    expect(start.params).toEqual({ workspaceId: WS, command: "read secret", cwd: "repo", cols: 90, rows: 30 });
    await act(async () => start.resolve(terminalInfo("01TERM")));
    expect(only(test.client.callsTo("widgets.state_set")).params).toMatchObject({
      key: "terminal:read secret",
      value: { id: "01TERM", runs: 1, command: "read secret" },
    });
    test.client.viewsEvents(WS, {
      kind: "widget_stored",
      instance: "tests",
      key: "terminal:read secret",
      value: { id: "01TERM", runs: 1, command: "read secret" },
    });
    test.flush();
    expect(test.client.subscribed).toContain(terminalTopic("01TERM"));

    const state: TerminalState = { terminal: terminalInfo("01TERM"), scrollback: ["cmVhZHkK"], dropped: 0 };
    test.client.terminal(state);
    test.flush();
    expect(text("terminal")).toBe("running 1 1");
    act(() => screen.getByTestId("terminal-input").click());
    act(() => screen.getByTestId("terminal-resize").click());
    act(() => screen.getByTestId("terminal-kill").click());
    expect(only(test.client.callsTo("terminals.input")).params).toEqual({
      terminalId: "01TERM",
      data: "c2VjcmV0Cg==",
    });
    expect(only(test.client.callsTo("terminals.resize")).params).toEqual({
      terminalId: "01TERM",
      cols: 100,
      rows: 40,
    });
    expect(only(test.client.callsTo("terminals.kill")).params).toEqual({ terminalId: "01TERM" });
    expect(test.client.callsTo("terminals.start").length).toBe(1);
  });
});

function omp(frame: { [key: string]: JsonValue }): ThreadEvent {
  return { kind: "omp", frame };
}

const START = omp({
  type: "tool_execution_start",
  toolCallId: "c1",
  toolName: "bash",
  args: { command: "cargo nextest run" },
});
const UPDATE = omp({
  type: "tool_execution_update",
  toolCallId: "c1",
  toolName: "bash",
  args: { command: "cargo nextest run" },
  partialResult: { content: [{ type: "text", text: "Starting 3 tests\nPASS a\n" }] },
});
const END = omp({
  type: "tool_execution_end",
  toolCallId: "c1",
  toolName: "bash",
  result: { content: [{ type: "text", text: "Starting 3 tests\nPASS a\nFAIL b" }] },
  isError: true,
});
const OTHER = omp({ type: "tool_execution_start", toolCallId: "c2", toolName: "read", args: { path: "nextest.toml" } });

function ToolProbe({ target }: { target: ToolRunTarget }) {
  const run = useToolRun(target);
  return (
    <span data-testid="run">
      {run === undefined
        ? "none"
        : [run.toolCallId, run.toolName, run.status, run.isError, run.lines.join("|")].map(String).join(" ")}
    </span>
  );
}

describe("useToolRun", () => {
  // spec: AGT-020
  test("follows a run of the selected thread by id: partial output while running, the result once ended", () => {
    const { client, flush, host } = renderWithHost(<ToolProbe target="c1" />);
    act(() => host.window.set(SELECTED_THREAD, THREAD));
    client.thread(THREAD, [START, UPDATE]);
    flush();
    expect(text("run")).toBe("c1 bash running false Starting 3 tests|PASS a");
    client.threadEvents(THREAD, END);
    flush();
    expect(text("run")).toBe("c1 bash failed true Starting 3 tests|PASS a|FAIL b");
  });

  // spec: AGT-020
  test("matches the latest run whose command matches, in a given thread", () => {
    const { client, flush } = renderWithHost(<ToolProbe target={{ match: /nextest/, threadId: THREAD }} />);
    client.thread(THREAD, [START, OTHER]);
    flush();
    // The `read` run matches through its JSON arguments and is the latest.
    expect(text("run")).toBe("c2 read running false ");
    cleanup();

    const second = renderWithHost(<ToolProbe target={{ match: "cargo", threadId: THREAD }} />);
    second.client.thread(THREAD, [START, UPDATE, OTHER]);
    second.flush();
    expect(text("run")).toBe("c1 bash running false Starting 3 tests|PASS a");
  });

  // spec: AGT-020
  test("is undefined without a matching run", () => {
    const { client, flush } = renderWithHost(<ToolProbe target={{ match: /nope/, threadId: THREAD }} />);
    client.thread(THREAD, [START]);
    flush();
    expect(text("run")).toBe("none");
  });
});
