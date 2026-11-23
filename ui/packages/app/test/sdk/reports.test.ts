/** L4: the window's widget reports (`widgets.report`): batching, resending and console attribution. */

import { afterEach, describe, expect, test } from "bun:test";
import type { ReportWidgetsParams } from "@omfp/protocol";
import { type SlotIdentity, SlotReport } from "../../src/sdk/index.ts";
import { FakeClient, ManualTimers, type TestHost, testHost } from "../support/host.tsx";

const WS = "01W";
const IDENTITY: SlotIdentity = {
  reference: "./widgets/tests",
  instance: "tests",
  source: "workspace",
  name: "tests",
  modulePath: "/widgets/test/01W/tests/",
};

interface Reporting extends TestHost {
  readonly timers: ManualTimers;
  /** The `widgets.report` params sent so far. */
  sent(): ReportWidgetsParams[];
}

function reporting(): Reporting {
  const timers = new ManualTimers();
  const test = testHost(new FakeClient(), { reportTimers: timers });
  test.client.answers.set("widgets.report", () => ({}));
  return {
    ...test,
    timers,
    sent: () => test.client.callsTo("widgets.report").map((call) => call.params as ReportWidgetsParams),
  };
}

const stops: Array<() => void> = [];
afterEach(() => {
  for (const stop of stops.splice(0)) {
    stop();
  }
});

describe("widget reports", () => {
  // spec: AGT-026, PRT-043
  test("changes are batched into one report per delay and an unchanged report is not sent again", () => {
    const { host, timers, sent } = reporting();
    const slot = new SlotReport(host.reports, IDENTITY);
    slot.text = () => "Tests 1/3";
    host.reports.show(WS, { epoch: "e1", seq: 4 });
    host.reports.add(slot);
    slot.setLoad({ state: "rendered", version: "v1", errors: [] });
    slot.setJob("make test", {
      command: "make test",
      status: "running",
      exitCode: null,
      signal: null,
      runs: 1,
      startedAt: 7,
      tail: ["PASS 1"],
    });
    expect(timers.pending).toBe(1);
    expect(sent()).toEqual([]);

    timers.run();
    expect(sent()).toEqual([
      {
        workspaceId: WS,
        views: { epoch: "e1", seq: 4 },
        widgets: [
          {
            reference: "./widgets/tests",
            instance: "tests",
            source: "workspace",
            name: "tests",
            version: "v1",
            state: "rendered",
            errors: [],
            console: [],
            text: "Tests 1/3",
            jobs: [
              {
                command: "make test",
                status: "running",
                exitCode: null,
                signal: null,
                runs: 1,
                startedAt: 7,
                tail: ["PASS 1"],
              },
            ],
          },
        ],
        fileErrors: [],
      },
    ]);

    slot.changed();
    timers.run();
    expect(sent().length).toBe(1);

    slot.setBoundary("boom");
    timers.run();
    expect(sent()[1]?.widgets[0]).toMatchObject({ state: "render_error", errors: ["boom"] });
    // The next version replaces the boundary.
    slot.setBoundary(undefined);
    slot.setLoad({ state: "compile_error", version: "v2", errors: ["index.tsx:3:7: Unexpected token"] });
    timers.run();
    expect(sent()[2]?.widgets[0]).toMatchObject({
      version: "v2",
      state: "compile_error",
      errors: ["index.tsx:3:7: Unexpected token"],
    });

    host.reports.release(slot);
    timers.run();
    expect(sent()[3]?.widgets).toEqual([]);
  });

  // spec: AGT-026
  test("a window without a workspace reports none, and a new connection gets the report again", () => {
    const { host, timers, sent } = reporting();
    host.reports.add(new SlotReport(host.reports, IDENTITY));
    host.reports.show(undefined, undefined);
    timers.run();
    expect(sent()).toEqual([{ workspaceId: null, views: null, widgets: [], fileErrors: [] }]);

    host.reports.resend();
    timers.run();
    expect(sent().length).toBe(2);
    expect(sent()[1]).toEqual({ workspaceId: null, views: null, widgets: [], fileErrors: [] });
  });

  // spec: UI-304
  test("the window's init file errors ride along; an unchanged list sends nothing", () => {
    const { host, timers, sent } = reporting();
    host.reports.show(WS, undefined);
    host.reports.setFileErrors([{ file: ".omfp/init.ts", message: "boom" }]);
    timers.run();
    expect(sent().map((report) => report.fileErrors)).toEqual([[{ file: ".omfp/init.ts", message: "boom" }]]);
    host.reports.setFileErrors([{ file: ".omfp/init.ts", message: "boom" }]);
    timers.run();
    expect(sent().length).toBe(1);
    host.reports.setFileErrors([]);
    timers.run();
    expect(sent().map((report) => report.fileErrors)).toEqual([[{ file: ".omfp/init.ts", message: "boom" }], []]);
  });

  // spec: UI-350
  test("the window state rides along once set; an unchanged state sends nothing", () => {
    const { host, timers, sent } = reporting();
    host.reports.show(WS, undefined);
    timers.run();
    expect(sent()[0]?.window).toBeUndefined();
    const window = { threadId: "t1", focused: true, activeView: "work", defaultView: "work", views: [], commands: [] };
    host.reports.setWindow(window);
    timers.run();
    expect(sent()[1]?.window).toEqual(window);
    host.reports.setWindow({ ...window });
    timers.run();
    expect(sent().length).toBe(2);
    host.reports.setWindow({ ...window, focused: false });
    timers.run();
    expect(sent()[2]?.window?.focused).toBe(false);
  });

  // spec: AGT-026
  test("a report the daemon did not take is sent again with the next change", async () => {
    const { client, host, timers, sent } = reporting();
    client.answers.delete("widgets.report");
    host.reports.show(WS, undefined);
    timers.run();
    await client.callsTo("widgets.report")[0]?.reject(new Error("closed"));
    await Promise.resolve();
    host.reports.show(WS, undefined);
    host.reports.schedule();
    timers.run();
    expect(sent().length).toBe(2);
  });

  // spec: AGT-026
  test("console errors and uncaught errors whose stack names a widget's module are that widget's", () => {
    const { host, timers, sent } = reporting();
    const quiet = window.console.error;
    window.console.error = () => {};
    stops.push(() => {
      window.console.error = quiet;
    });
    const slot = new SlotReport(host.reports, IDENTITY);
    const other = new SlotReport(host.reports, {
      ...IDENTITY,
      reference: "./widgets/clock",
      instance: "clock",
      name: "clock",
      modulePath: "/widgets/test/01W/clock/",
    });
    host.reports.show(WS, undefined);
    host.reports.add(slot);
    host.reports.add(other);
    stops.push(host.reports.watchConsole(window));

    const failure = new TypeError("x is undefined");
    failure.stack =
      "TypeError: x is undefined\n    at Tests (http://localhost/widgets/test/01W/tests/v1/index.tsx:4:2)";
    window.console.error("render failed:", failure);
    window.console.error("not a widget's");
    window.dispatchEvent(
      new ErrorEvent("error", {
        error: new Error("tick"),
        message: "tick",
        filename: "http://localhost/widgets/test/01W/clock/c1/index.js",
      }),
    );
    timers.run();

    const [report] = sent();
    expect(report?.widgets.map((widget) => [widget.name, widget.console])).toEqual([
      ["tests", ["render failed: TypeError: x is undefined"]],
      ["clock", ["Error: tick"]],
    ]);
  });
});
