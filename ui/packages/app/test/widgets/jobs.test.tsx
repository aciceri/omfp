/** L4: `omfp:jobs` (UI-742, UI-743) over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import { jobsTopic, jobTopic } from "@omfp/protocol";
import { act, cleanup, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Activity, type ReactNode, useState } from "react";
import {
  ERRORS,
  OPEN_FILES,
  OPEN_JOB,
  SELECTED_THREAD,
  type WidgetInfoScope,
  WidgetScope,
} from "../../src/sdk/index.ts";
import { JobsWidget } from "../../src/widgets/jobs/index.ts";
import { jobView, runInfo, thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

const SCOPE: WidgetInfoScope = { instance: "omfp:jobs", reference: "omfp:jobs", workspaceId: "w1" };

/** A tab's content as the kit's `TabPanel` keeps it: in an `Activity` that a button hides and shows again. */
function InTab({ children }: { children: ReactNode }): ReactNode {
  const [shown, setShown] = useState(true);
  return (
    <>
      <button type="button" data-testid="tab-switch" onClick={() => setShown((current) => !current)}>
        switch
      </button>
      <Activity mode={shown ? "visible" : "hidden"}>{children}</Activity>
    </>
  );
}

function setup(props: Record<string, unknown> = {}) {
  const base = testHost();
  base.host.window.set(SELECTED_THREAD, "t1");
  const test = renderWithHost(
    <WidgetScope value={SCOPE}>
      <InTab>
        <JobsWidget {...props} />
      </InTab>
    </WidgetScope>,
    base,
  );
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1"), workspace("w2")],
    threads: [thread("t1", "checkout", "w1"), thread("t9", "mail · 2026-11-16 07:00", "w1")],
  });
  test.flush();
  return { ...test, user: userEvent.setup() };
}

function row(name: string): HTMLElement {
  const found = screen.getAllByTestId("job-row").find((element) => element.dataset.job === name);
  if (found === undefined) {
    throw new Error(`no row ${name}`);
  }
  return found;
}

describe("omfp:jobs", () => {
  // spec: UI-742
  test("one row per job: name, description as tooltip, trigger, action, state and last run", () => {
    const test = setup();
    expect(test.client.subscribed).toContain(jobsTopic("w1"));
    expect(screen.getByTestId("jobs-loading").textContent).toContain("Reading the jobs");
    const now = Date.now();
    test.client.jobs("w1", {
      jobs: [
        jobView(
          "deploy",
          { description: "Ship it" },
          {
            lastRun: runInfo("r1", "deploy", { status: "failed", exitCode: 1, endedAt: now }),
          },
        ),
        jobView("mail", {
          trigger: "schedule",
          schedule: "0 7 * * *",
          command: null,
          prompt: { text: "x", thread: "new" },
        }),
        jobView(
          "watch",
          { trigger: "service" },
          { status: "running", run: runInfo("r2", "watch", { startedAt: now }) },
        ),
      ],
    });
    test.flush();
    expect(screen.getAllByTestId("job-row").map((element) => [element.dataset.job, element.dataset.status])).toEqual([
      ["deploy", "idle"],
      ["mail", "idle"],
      ["watch", "running"],
    ]);
    const deploy = row("deploy");
    expect(within(deploy).getByTestId("job-toggle").getAttribute("title")).toBe("Ship it");
    expect(within(deploy).getByTestId("job-trigger").textContent).toBe("manual");
    expect(within(deploy).getByTestId("job-action").textContent).toBe("make deploy");
    expect(within(deploy).getByTestId("job-status").textContent).toBe("idle");
    expect(within(deploy).getByTestId("job-last-run").textContent).toMatch(/^last \d\d:\d\d: failed · exit 1$/);
    expect(within(row("mail")).getByTestId("job-action").textContent).toBe("prompt · new thread");
    expect(within(row("mail")).getByTestId("job-last-run").textContent).toBe("never ran");
    expect(within(row("watch")).getByTestId("job-status").textContent).toMatch(/^running /);
    expect(within(row("watch")).getByTestId("job-trigger").textContent).toBe("service · restart always");
    expect(within(row("watch")).getByTestId("job-last-run").textContent).toBe("");
  });

  // spec: UI-742
  test("a refused file shows its error above the rows; without jobs an empty state says how; both open jobs.json", async () => {
    const test = setup();
    test.client.jobs("w1", { jobs: [jobView("deploy")], error: "jobs.deploy.schedule: hour: 25 is out of range" });
    test.flush();
    expect(screen.getByTestId("jobs-error").textContent).toContain(
      "jobs.json is invalid: jobs.deploy.schedule: hour: 25 is out of range",
    );
    expect(screen.getAllByTestId("job-row")).toHaveLength(1);
    await test.user.click(within(screen.getByTestId("jobs-error")).getByTestId("jobs-open-file"));
    expect(test.host.window.get(OPEN_FILES).w1?.tabs.map((tab) => tab.path)).toEqual([".omfp/jobs.json"]);

    test.client.jobsEvents("w1", { kind: "jobs_set", jobs: [], error: null });
    test.flush();
    const empty = screen.getByTestId("jobs-none");
    expect(empty.textContent).toContain(".omfp/jobs.json");
    expect(within(empty).getByTestId("jobs-open-file").textContent).toBe("Open jobs.json");
  });

  // spec: UI-742
  test("props.workspaceId shows another workspace's jobs", () => {
    const test = setup({ workspaceId: "w2" });
    expect(test.client.subscribed).toContain(jobsTopic("w2"));
    expect(test.client.subscribed).not.toContain(jobsTopic("w1"));
  });

  // spec: UI-741, UI-742, UI-743
  test("an OPEN_JOB request for its workspace expands that job's row; one for another workspace changes nothing", () => {
    const test = setup();
    // Sent before the rows show (the reveal mounts the widget): the row opens once they do.
    act(() => test.host.window.set(OPEN_JOB, { workspaceId: "w1", job: "watch", nonce: 1 }));
    test.client.jobs("w1", { jobs: [jobView("deploy"), jobView("watch")] });
    test.flush();
    const expanded = () => screen.getAllByTestId("job-toggle").map((toggle) => toggle.getAttribute("aria-expanded"));
    expect(expanded()).toEqual(["false", "true"]);
    act(() => test.host.window.set(OPEN_JOB, { workspaceId: "w2", job: "deploy", nonce: 2 }));
    expect(expanded()).toEqual(["false", "true"]);
    act(() => test.host.window.set(OPEN_JOB, { workspaceId: "w1", job: "deploy", nonce: 3 }));
    expect(expanded()).toEqual(["true", "false"]);
  });

  // spec: UI-742
  test("a handled OPEN_JOB request is cleared: back on the tab, the row the user expanded since stays expanded", async () => {
    const test = setup();
    test.client.jobs("w1", { jobs: [jobView("deploy"), jobView("watch")] });
    test.flush();
    const expanded = () => screen.getAllByTestId("job-toggle").map((toggle) => toggle.getAttribute("aria-expanded"));
    act(() => test.host.window.set(OPEN_JOB, { workspaceId: "w1", job: "watch", nonce: 1 }));
    expect(expanded()).toEqual(["false", "true"]);
    expect(test.host.window.get(OPEN_JOB)).toBeUndefined();
    await test.user.click(within(row("deploy")).getByTestId("job-toggle"));
    expect(expanded()).toEqual(["true", "false"]);
    // Another tab, then back: the tab's effects run again, the old request does not.
    await test.user.click(screen.getByTestId("tab-switch"));
    await test.user.click(screen.getByTestId("tab-switch"));
    expect(expanded()).toEqual(["true", "false"]);
  });

  // spec: UI-743
  test("Run, Start, Stop and Enable/Disable call the methods; an error is a toast", async () => {
    const test = setup();
    test.client.jobs("w1", {
      jobs: [
        jobView("deploy"),
        jobView("old", { enabled: false }, { status: "disabled" }),
        jobView("watch", { trigger: "service" }, { status: "running", run: runInfo("r2", "watch") }),
        jobView("web", { trigger: "service" }, { status: "stopped" }),
      ],
    });
    test.flush();
    await test.user.click(within(row("deploy")).getByTestId("job-run"));
    expect(within(row("web")).getByTestId("job-run").textContent).toBe("Start");
    await test.user.click(within(row("web")).getByTestId("job-run"));
    expect(within(row("old")).getByTestId("job-run").hasAttribute("disabled")).toBe(true);
    await test.user.click(within(row("old")).getByTestId("job-enable"));
    await test.user.click(within(row("watch")).getByTestId("job-stop"));
    await test.user.click(within(row("deploy")).getByTestId("job-enable"));
    expect(test.client.calls.map((call) => [call.method, call.params])).toEqual([
      ["jobs.run", { workspaceId: "w1", job: "deploy" }],
      ["jobs.run", { workspaceId: "w1", job: "web" }],
      ["jobs.enable", { workspaceId: "w1", name: "old", enabled: true }],
      ["jobs.kill", { jobId: "r2" }],
      ["jobs.enable", { workspaceId: "w1", name: "deploy", enabled: false }],
    ]);
    const refused = new CallError({ kind: "daemon", error: { code: "conflict", message: "deploy is running" } });
    await act(async () => {
      test.client.callsTo("jobs.run")[0]?.reject(refused);
      await Promise.resolve();
    });
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["deploy is running (conflict)"]);
  });

  // spec: UI-743
  test("one expanded row shows its runs newest first with Load older, a command run's output and a prompt's thread", async () => {
    const test = setup();
    test.client.jobs("w1", {
      jobs: [jobView("deploy"), jobView("mail", { command: null, prompt: { text: "x", thread: "new" } })],
    });
    test.flush();
    const toggle = within(row("deploy")).getByTestId("job-toggle");
    await test.user.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    const [first] = test.client.callsTo("jobs.runs");
    expect(first?.params).toEqual({ workspaceId: "w1", job: "deploy" });
    await act(async () => {
      first?.resolve({
        runs: [
          runInfo("r2", "deploy", { status: "succeeded", exitCode: 0, endedAt: 2000, startedBy: { kind: "user" } }),
          runInfo("r1", "deploy", { status: "failed", exitCode: 3, endedAt: 1500, trigger: "schedule" }),
        ],
        more: true,
      });
      await Promise.resolve();
    });
    const runs = () => screen.getAllByTestId("job-run-row").map((element) => element.dataset.run);
    expect(runs()).toEqual(["r2", "r1"]);
    const [newest] = screen.getAllByTestId("job-run-row");
    if (newest === undefined) {
      throw new Error("no run");
    }
    expect(within(newest).getByTestId("job-run-trigger").textContent).toBe("by hand by you");
    expect(within(newest).getByTestId("job-run-outcome").textContent).toBe("succeeded");

    await test.user.click(screen.getByTestId("job-runs-older"));
    const older = test.client.callsTo("jobs.runs")[1];
    expect(older?.params).toEqual({ workspaceId: "w1", job: "deploy", before: "r1" });
    await act(async () => {
      older?.resolve({ runs: [runInfo("r0", "deploy", { status: "killed", signal: 15 })], more: false });
      await Promise.resolve();
    });
    expect(runs()).toEqual(["r2", "r1", "r0"]);
    expect(screen.queryByTestId("job-runs-older")).toBeNull();

    await test.user.click(within(newest).getByTestId("job-run-output-toggle"));
    expect(test.client.subscribed).toContain(jobTopic("r2"));
    test.client.job({
      job: { id: "r2", workspaceId: "w1", command: "make deploy", startedAt: 1000, status: { state: "running" } },
      lines: [{ stream: "stdout", text: "deployed" }],
      dropped: 0,
    });
    test.flush();
    expect(screen.getByTestId("job-run-output").textContent).toContain("deployed");

    // One row at a time: expanding another collapses this one.
    await test.user.click(within(row("mail")).getByTestId("job-toggle"));
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    const prompt = test.client.callsTo("jobs.runs")[2];
    await act(async () => {
      // The job prompts now; its older run was a command run, with output (PRT-201).
      prompt?.resolve({
        runs: [
          runInfo("r5", "mail", { command: null, threadId: "t9", status: "succeeded" }),
          runInfo("r6", "mail", { status: "succeeded" }),
        ],
        more: false,
      });
      await Promise.resolve();
    });
    expect(screen.getAllByTestId("job-run-output-toggle").map((toggle) => toggle.closest("li")?.dataset.run)).toEqual([
      "r6",
    ]);
    expect(screen.getAllByTestId("job-run-thread")).toHaveLength(1);
    const open = screen.getByTestId("job-run-thread");
    expect(open.textContent).toBe("Open mail · 2026-11-16 07:00");
    await test.user.click(open);
    expect(test.host.window.get(SELECTED_THREAD)).toBe("t9");
  });

  // spec: UI-742
  test("another workspace's widget opens that workspace's jobs.json, not the current one's", async () => {
    const test = setup({ workspaceId: "w2" });
    test.client.jobs("w2", { jobs: [], error: "jobs: expected an object" });
    test.flush();
    await test.user.click(within(screen.getByTestId("jobs-error")).getByTestId("jobs-open-file"));
    const files = test.host.window.get(OPEN_FILES);
    expect([files.w1, files.w2?.tabs.map((tab) => tab.path)]).toEqual([undefined, [".omfp/jobs.json"]]);
  });

  // spec: UI-743
  test("an expanded row asks for its runs again on its run events, skipped ones included", async () => {
    const test = setup();
    test.client.jobs("w1", { jobs: [jobView("deploy"), jobView("other")] });
    test.flush();
    await test.user.click(within(row("deploy")).getByTestId("job-toggle"));
    await act(async () => {
      test.client.callsTo("jobs.runs")[0]?.resolve({ runs: [], more: false });
      await Promise.resolve();
    });
    expect(screen.getByTestId("job-no-runs")).not.toBeNull();
    // Another job's run is not this one's.
    test.client.jobsEvents("w1", { kind: "run_ended", run: runInfo("r0", "other", { status: "succeeded" }) });
    test.flush();
    expect(test.client.callsTo("jobs.runs")).toHaveLength(1);

    const skipped = runInfo("r1", "deploy", { status: "skipped", reason: "running", endedAt: 1000 });
    test.client.jobsEvents("w1", { kind: "run_ended", run: skipped });
    test.flush();
    const again = test.client.callsTo("jobs.runs")[1];
    expect(again?.params).toEqual({ workspaceId: "w1", job: "deploy" });
    await act(async () => {
      again?.resolve({ runs: [skipped], more: false });
      await Promise.resolve();
    });
    expect(screen.getAllByTestId("job-run-row").map((element) => element.dataset.status)).toEqual(["skipped"]);
  });

  // spec: UI-743
  test("an older page answered after a run event refreshed the first page is dropped: the list stays contiguous", async () => {
    const test = setup();
    test.client.jobs("w1", { jobs: [jobView("deploy")] });
    test.flush();
    await test.user.click(within(row("deploy")).getByTestId("job-toggle"));
    const page = (from: number, to: number) =>
      Array.from({ length: from - to + 1 }, (_, at) => runInfo(`r${from - at}`, "deploy", { status: "succeeded" }));
    await act(async () => {
      test.client.callsTo("jobs.runs")[0]?.resolve({ runs: page(20, 1), more: true });
      await Promise.resolve();
    });
    await test.user.click(screen.getByTestId("job-runs-older"));
    const stale = test.client.callsTo("jobs.runs")[1];
    expect(stale?.params).toEqual({ workspaceId: "w1", job: "deploy", before: "r1" });

    // Run 21 starts: the first page again, then the old cursor's answer arrives.
    test.client.jobsEvents("w1", { kind: "run_started", run: runInfo("r21", "deploy") });
    test.flush();
    const fresh = test.client.callsTo("jobs.runs")[2];
    expect(fresh?.params).toEqual({ workspaceId: "w1", job: "deploy" });
    await act(async () => {
      fresh?.resolve({ runs: [runInfo("r21", "deploy"), ...page(20, 2)], more: true });
      await Promise.resolve();
    });
    await act(async () => {
      stale?.resolve({ runs: [runInfo("r0", "deploy", { status: "succeeded" })], more: false });
      await Promise.resolve();
    });
    const runs = () => screen.getAllByTestId("job-run-row").map((element) => element.dataset.run);
    expect(runs()).toEqual(["r21", ...page(20, 2).map((run) => run.id)]);

    // Load older again continues from the new first page.
    await test.user.click(screen.getByTestId("job-runs-older"));
    const next = test.client.callsTo("jobs.runs")[3];
    expect(next?.params).toEqual({ workspaceId: "w1", job: "deploy", before: "r2" });
    await act(async () => {
      next?.resolve({ runs: page(1, 0), more: false });
      await Promise.resolve();
    });
    expect(runs()).toEqual(["r21", ...page(20, 0).map((run) => run.id)]);
  });
});
