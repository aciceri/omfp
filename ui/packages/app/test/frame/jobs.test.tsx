/** L4: declared jobs in the window (UI-740..UI-748) over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError, type CallMethod } from "@omfp/client";
import { type JobCounts, jobsTopic } from "@omfp/protocol";
import { act, cleanup, screen, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { Frame } from "../../src/frame/frame.tsx";
import { initUrl } from "../../src/frame/init.tsx";
import { DEFAULT_BINDINGS } from "../../src/keymap/keymap.ts";
import {
  ERRORS,
  type InitApi,
  OPEN_FILES,
  OPEN_JOB,
  REVEAL_WIDGET,
  SELECTED_THREAD,
  SUCCESSES,
  type WidgetComponent,
} from "../../src/sdk/index.ts";
import { strings } from "../../src/strings/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { jobView, runInfo, thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

const stub =
  (name: string): WidgetComponent =>
  (props) => <p data-testid={`stub-${name}`}>{String(props.workspaceId ?? name)}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", stub("chat")],
  ["omfp:jobs", stub("jobs")],
]);

const COUNTS: JobCounts = { total: 3, manual: 1, schedule: 1, service: 1, running: 1, failing: 1, error: false };
const USER_URL = initUrl("/widgets/test", "user", "u1");

let heard: string[] = [];

const userInit = (omfp: InitApi) => {
  omfp.on("job_started", (run) => heard.push(`started ${run.id}`));
  omfp.on("job_ended", (run) => heard.push(`ended ${run.id}`));
  omfp.on("job_failed", (run) => heard.push(`failed ${run.id}`));
  omfp.command({
    id: "my.jobs",
    title: "Jobs",
    run: () => {
      heard.push(`${omfp.jobs.list().map((job) => job.name)} ${omfp.jobs.get("deploy")?.status} ${omfp.jobs.error}`);
      void omfp.jobs.run("deploy");
      void omfp.jobs.enable("deploy", false);
    },
  });
};

function setup(answers: readonly CallMethod[] = ["threads.archive", "threads.delete", "widgets.report"]) {
  heard = [];
  const base = testHost(undefined, {
    importer: (url) => (url === USER_URL ? Promise.resolve({ default: userInit }) : new Promise<unknown>(() => {})),
  });
  for (const method of answers) {
    base.client.answers.set(method, () => ({}));
  }
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [{ ...workspace("w1"), jobs: COUNTS }, workspace("w2"), { ...workspace("w3"), jobs: { ...COUNTS } }],
    threads: [
      thread("t1", "checkout", "w1"),
      thread("t2", "mail", "w1", { provenance: { createdBy: { kind: "job", job: "mail" }, client: "omfp-daemon" } }),
    ],
    init: "u1",
  });
  act(() => base.host.window.set(SELECTED_THREAD, "t1"));
  test.client.views("w1", {});
  test.flush();
  return { ...test, user: userEvent.setup() };
}

/** The element `testId` of workspace `workspaceId`'s section, if any. */
function inSection(workspaceId: string, testId: string): HTMLElement | null {
  const section = screen
    .getAllByTestId("workspace-section")
    .find((element) => element.dataset.workspaceId === workspaceId);
  return section === undefined ? null : within(section).queryByTestId(testId);
}

/** Presses the element, failing the test when there is none. */
function press(user: UserEvent, element: Element | null | undefined): Promise<void> {
  if (element === null || element === undefined) {
    throw new Error("nothing to press");
  }
  return user.click(element);
}

/** The job rows under `workspaceId`'s expanded `Jobs (N)`. */
function jobRows(workspaceId: string): HTMLElement[] {
  const list = inSection(workspaceId, "jobs-list");
  return list === null ? [] : within(list).queryAllByTestId("job-side-row");
}

describe("jobs in the left column", () => {
  // spec: UI-740
  test("a workspace with jobs has a collapsed Jobs (N) row whose marks at its end spell everything out", () => {
    const test = setup();
    expect(inSection("w2", "jobs-toggle")).toBeNull();
    const toggle = inSection("w1", "jobs-toggle");
    expect([
      toggle?.querySelector(".omfp-toggle-row__glyph")?.textContent,
      toggle?.querySelector(".omfp-row__title")?.textContent,
      toggle?.getAttribute("aria-expanded"),
    ]).toEqual(["▸", "Jobs (3)", "false"]);
    expect(inSection("w1", "jobs-list")).toBeNull();
    const marks = inSection("w1", "job-counts");
    const label = "3 jobs: 1 service, 1 scheduled, 1 manual · 1 running · 1 failed";
    expect([marks?.getAttribute("aria-label"), marks?.getAttribute("title"), marks?.getAttribute("role")]).toEqual([
      label,
      label,
      "img",
    ]);
    expect(marks?.parentElement?.classList.contains("omfp-toggle-row__end")).toBe(true);
    expect(
      [...(marks?.querySelectorAll("[data-part]") ?? [])].map((part) => [
        part.getAttribute("data-part"),
        part.querySelector("[data-state]")?.getAttribute("data-state"),
      ]),
    ).toEqual([
      ["running", "running"],
      ["failing", "failed"],
    ]);
    // The window follows the jobs of every counted workspace: the refused file's reason shows.
    expect(test.client.subscribed).toEqual(expect.arrayContaining([jobsTopic("w1"), jobsTopic("w3")]));
    test.client.indexEvents({
      kind: "workspace_upserted",
      workspace: { ...workspace("w3"), jobs: { ...COUNTS, running: 0, failing: 0, error: true } },
    });
    test.client.jobs("w3", { error: "jobs.a: unknown key" });
    test.flush();
    expect(inSection("w3", "job-counts")?.getAttribute("aria-label")).toBe(
      "3 jobs: 1 service, 1 scheduled, 1 manual · jobs.json is invalid: jobs.a: unknown key",
    );
    expect(
      [...(inSection("w3", "job-counts")?.querySelectorAll("[data-part]") ?? [])].map((part) =>
        part.getAttribute("data-part"),
      ),
    ).toEqual(["error"]);
  });

  // spec: UI-740, UI-741, UI-742, UI-092
  test("expanded, a row per job (mark, name, state); pressing one reveals omfp:jobs on that job, a floating panel for another workspace", async () => {
    const test = setup();
    test.client.jobs("w1", {
      jobs: [
        jobView("deploy", { description: "Ship it" }),
        jobView("watch", { trigger: "service" }, { status: "running", run: runInfo("r4", "watch") }),
        jobView("lint", {}, { lastRun: runInfo("r3", "lint", { status: "failed", exitCode: 1 }) }),
      ],
    });
    test.client.jobs("w3", { jobs: [jobView("mail")] });
    test.flush();
    await press(test.user, inSection("w1", "jobs-toggle"));
    expect(inSection("w1", "jobs-toggle")?.getAttribute("aria-expanded")).toBe("true");
    expect(inSection("w3", "jobs-list")).toBeNull();
    const rows = jobRows("w1");
    expect(
      rows.map((row) => [
        row.dataset.job,
        row.dataset.state,
        row.querySelector(".omfp-row__title")?.textContent,
        row.querySelector(".omfp-row__meta")?.textContent,
      ]),
    ).toEqual([
      ["deploy", "idle", "deploy", "idle"],
      ["watch", "running", "watch", "running"],
      ["lint", "failed", "lint", "failed"],
    ]);
    // The state in words is the mark's name, so it stays when Run or Stop take the meta's place (focus, no hover).
    expect(
      rows.map((row) => {
        const mark = row.querySelector(".omfp-state-mark");
        const meta = row.querySelector(".omfp-row__meta");
        return [mark?.getAttribute("role"), mark?.getAttribute("aria-label"), meta?.getAttribute("aria-hidden")];
      }),
    ).toEqual([
      ["img", "idle", "true"],
      ["img", "running", "true"],
      ["img", "failed", "true"],
    ]);
    expect(
      within(rows[0] as HTMLElement)
        .getByTestId("job-side-open")
        .getAttribute("title"),
    ).toBe("Ship it");

    await press(test.user, within(rows[1] as HTMLElement).getByTestId("job-side-open"));
    test.flush();
    expect(test.host.window.get(OPEN_FILES).w1?.widgets).toEqual(["omfp:jobs"]);
    expect(test.host.window.get(OPEN_JOB)).toMatchObject({ workspaceId: "w1", job: "watch" });
    expect(screen.queryByTestId("floating-widget")).toBeNull();

    await press(test.user, inSection("w3", "jobs-toggle"));
    await press(test.user, within(jobRows("w3")[0] as HTMLElement).getByTestId("job-side-open"));
    test.flush();
    expect(test.host.window.get(OPEN_JOB)).toMatchObject({ workspaceId: "w3", job: "mail" });
    const panel = screen.getByTestId("floating-widget");
    expect(within(panel).getByTestId("floating-widget-title").textContent).toBe("Jobs · w3");
    expect(within(panel).getByTestId("stub-jobs").textContent).toBe("w3");
    await test.user.click(within(panel).getByTestId("floating-widget-close"));

    // `:jobs` reveals omfp:jobs for the current workspace.
    act(() => test.host.window.set(OPEN_FILES, {}));
    act(() => {
      test.host.commands.run("jobs");
    });
    test.flush();
    expect(test.host.window.get(OPEN_FILES).w1?.widgets).toEqual(["omfp:jobs"]);
    expect(test.host.window.get(REVEAL_WIDGET)).toBeUndefined();
  });

  // spec: UI-740, UI-743
  test("a job row's Run or Stop takes its state's place on hover and focus, through the calls of omfp:jobs; an error is a toast", async () => {
    const test = setup();
    test.client.jobs("w1", {
      jobs: [
        jobView("deploy"),
        jobView("watch", { trigger: "service" }, { status: "running", run: runInfo("r4", "watch") }),
        jobView("old", { enabled: false }, { status: "disabled" }),
      ],
    });
    test.flush();
    await press(test.user, inSection("w1", "jobs-toggle"));
    const [deploy, watch, old] = jobRows("w1");
    for (const row of [deploy, watch]) {
      const actions = row?.querySelector(".omfp-nested-row__actions");
      expect(actions?.previousElementSibling?.querySelector(".omfp-row__meta")).not.toBeNull();
    }
    expect(within(watch as HTMLElement).queryByTestId("job-side-run")).toBeNull();
    expect(
      within(old as HTMLElement)
        .getByTestId("job-side-run")
        .hasAttribute("disabled"),
    ).toBe(true);
    await press(test.user, within(deploy as HTMLElement).getByRole("button", { name: "Run deploy" }));
    await press(test.user, within(watch as HTMLElement).getByRole("button", { name: "Stop watch" }));
    expect(
      test.client.calls.filter((call) => call.method.startsWith("jobs.")).map((call) => [call.method, call.params]),
    ).toEqual([
      ["jobs.run", { workspaceId: "w1", job: "deploy" }],
      ["jobs.kill", { jobId: "r4" }],
    ]);
    expect(test.host.window.get(REVEAL_WIDGET)).toBeUndefined();
    const refused = new CallError({ kind: "daemon", error: { code: "conflict", message: "deploy is running" } });
    await act(async () => {
      test.client.callsTo("jobs.run")[0]?.reject(refused);
      await Promise.resolve();
    });
    expect(test.host.window.get(ERRORS)).toEqual(["deploy is running (conflict)"]);
  });

  // spec: UI-746
  test("a thread a job made says so in its row's tooltip", () => {
    setup();
    const rows = screen.getAllByTestId("thread-row");
    expect(rows.map((row) => row.getAttribute("title"))).toEqual([null, "mail · job mail"]);
  });
});

describe("closing a thread with its runs", () => {
  // spec: UI-744
  test("a conflict listing runs asks once, then repeats the call with the choice; Cancel changes nothing", async () => {
    const test = setup(["widgets.report"]);
    const conflict = new CallError({
      kind: "daemon",
      error: {
        code: "conflict",
        message: "runs",
        details: { runs: [runInfo("r1", null, { command: "npm run dev", startedAt: Date.now() })] },
      },
    });
    act(() => {
      test.host.commands.run("archive");
    });
    await act(async () => {
      test.client.callsTo("threads.archive")[0]?.reject(conflict);
      await Promise.resolve();
    });
    const dialog = await screen.findByTestId("close-runs-dialog");
    test.flush();
    expect(within(dialog).getByTestId("close-run").textContent).toContain("npm run dev");
    await test.user.click(within(dialog).getByTestId("close-runs-stop"));
    const again = test.client.callsTo("threads.archive")[1];
    expect(again?.params).toEqual({ threadId: "t1", jobs: "kill" });
    expect(test.host.window.get(SELECTED_THREAD)).toBe("t1");
    await act(async () => {
      again?.resolve({});
      await Promise.resolve();
    });
    expect(test.host.window.get(SELECTED_THREAD)).toBeUndefined();

    // The row's ×: Keep them running.
    await test.user.click(screen.getAllByTestId("row-archive-thread")[1] ?? document.body);
    await act(async () => {
      test.client.callsTo("threads.archive")[2]?.reject(conflict);
      await Promise.resolve();
    });
    await test.user.click(await screen.findByTestId("close-runs-keep"));
    expect(test.client.callsTo("threads.archive")[3]?.params).toEqual({ threadId: "t2", jobs: "keep" });

    // Cancel: nothing more is called; a conflict without runs is an error as before.
    act(() => test.host.window.set(SELECTED_THREAD, "t1"));
    test.flush();
    act(() => {
      test.host.commands.run("delete");
    });
    await act(async () => {
      test.client.callsTo("threads.delete")[0]?.reject(conflict);
      await Promise.resolve();
    });
    await test.user.click(await screen.findByTestId("close-runs-cancel"));
    expect(test.client.callsTo("threads.delete")).toHaveLength(1);
    expect(test.host.window.get(SELECTED_THREAD)).toBe("t1");
    act(() => {
      test.host.commands.run("delete");
    });
    const plain = new CallError({ kind: "daemon", error: { code: "conflict", message: "busy" } });
    await act(async () => {
      test.client.callsTo("threads.delete")[1]?.reject(plain);
      await Promise.resolve();
    });
    expect(screen.queryByTestId("close-runs-dialog")).toBeNull();
    expect(test.host.window.get(ERRORS)).toEqual(["busy (conflict)"]);
  });
});

describe("job commands", () => {
  // spec: UI-745
  test("job.run, job.kill, job.enable and job.disable act on the current workspace's job, completing its names", async () => {
    const test = setup();
    test.client.jobs("w1", {
      jobs: [jobView("deploy"), jobView("watch", {}, { status: "running", run: runInfo("r4", "watch") })],
    });
    test.flush();
    const run = test.host.commands.resolve("job-run");
    expect(run?.id).toBe("job.run");
    expect(run?.args?.([])).toEqual(["deploy", "watch"]);
    expect(run?.args?.(["deploy"])).toEqual([]);
    for (const [name, args] of [
      ["job-run", ["deploy"]],
      ["job-kill", ["watch"]],
      ["job-enable", ["deploy"]],
      ["job-disable", ["deploy"]],
    ] as const) {
      act(() => {
        test.host.commands.run(name, args);
      });
    }
    expect(
      test.client.calls.filter((call) => call.method.startsWith("jobs.")).map((call) => [call.method, call.params]),
    ).toEqual([
      ["jobs.run", { workspaceId: "w1", job: "deploy" }],
      ["jobs.kill", { jobId: "r4" }],
      ["jobs.enable", { workspaceId: "w1", name: "deploy", enabled: true }],
      ["jobs.enable", { workspaceId: "w1", name: "deploy", enabled: false }],
    ]);
    await act(async () => {
      test.client.callsTo("jobs.run")[0]?.resolve(runInfo("r5", "deploy"));
      await Promise.resolve();
    });
    expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual(["deploy started"]);
    act(() => {
      test.host.commands.run("job-run", []);
    });
    expect(test.host.window.get(ERRORS)).toEqual(["Name a job of the current workspace"]);
    expect(DEFAULT_BINDINGS.filter((binding) => binding.command.startsWith("job."))).toEqual([]);
  });
});

describe("jobs for init files", () => {
  // spec: UI-748
  test("omfp.jobs reads and acts on the current workspace's jobs; run events reach listeners once, never a snapshot's", async () => {
    const test = setup();
    await act(async () => {
      await test.host.init.settled();
    });
    test.client.jobs("w1", { jobs: [jobView("deploy")] });
    test.flush();
    act(() => {
      test.host.commands.run("my.jobs");
    });
    expect(heard).toEqual(["deploy idle null"]);
    expect(test.client.calls.filter((call) => call.method.startsWith("jobs.")).map((call) => call.method)).toEqual([
      "jobs.run",
      "jobs.enable",
    ]);
    heard = [];
    test.client.jobsEvents(
      "w1",
      { kind: "run_started", run: runInfo("r1", "deploy") },
      { kind: "run_ended", run: runInfo("r1", "deploy", { status: "failed" }) },
    );
    test.flush();
    test.client.jobs("w3");
    test.flush();
    test.client.jobsEvents("w3", { kind: "run_ended", run: runInfo("r2", "nightly", { status: "succeeded" }) });
    test.flush();
    // A new snapshot announces nothing.
    test.client.jobs("w1", { jobs: [jobView("deploy")] });
    test.flush();
    expect(heard).toEqual(["started r1", "ended r1", "failed r1", "ended r2"]);
  });

  // spec: UI-748
  test("run events reach the listeners once each without a frame in between: a background tab misses none", async () => {
    const test = setup();
    await act(async () => {
      await test.host.init.settled();
    });
    test.client.jobs("w1", { jobs: [jobView("deploy")] });
    test.flush();
    heard = [];
    const ids = Array.from({ length: 100 }, (_, at) => `r${at}`);
    // No `flush`: the frame store commits nothing while these arrive.
    for (const id of ids) {
      test.client.jobsEvents("w1", { kind: "run_ended", run: runInfo(id, "deploy", { status: "succeeded" }) });
    }
    expect(heard).toEqual(ids.map((id) => `ended ${id}`));
    test.flush();
    expect(heard).toHaveLength(100);
  });
});

describe("one meaning of job", () => {
  // spec: UI-747
  test("omp's background runs are background commands wherever the window names them", () => {
    const { shell, chat } = strings;
    const named = [
      shell.background.noneHint,
      shell.background.showJob("bg_1"),
      chat.tool.backgroundCommand("bg_1"),
      shell.header.inBackgroundTitle(2),
      shell.commands.stopDescription,
    ];
    expect(named.filter((text) => !/background command/i.test(text) || /\bjobs?\b/.test(text))).toEqual([]);
  });
});
