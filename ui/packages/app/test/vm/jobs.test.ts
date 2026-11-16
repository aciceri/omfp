/** L0: the view models of declared jobs (UI-740..UI-748): counts, rows, runs, provenance and run events. */

import { describe, expect, test } from "bun:test";
import type { JobCounts } from "@omfp/protocol";
import {
  formatWhen,
  jobCountsView,
  jobRow,
  runEventKinds,
  runMark,
  runOutcome,
  runRow,
  startedByText,
  threadJob,
} from "../../src/vm/jobs.ts";
import { jobView, provenance, runInfo, thread } from "../support/fixtures.ts";

const NONE: JobCounts = { total: 0, manual: 0, schedule: 0, service: 0, running: 0, failing: 0, error: false };

/** Local noon of 2026-11-16, and an hour past it. */
const NOON = new Date(2026, 10, 16, 12, 0).getTime();
const ONE = new Date(2026, 10, 16, 13, 5).getTime();

describe("job counts", () => {
  // spec: UI-740
  test("running and failed counts show with their marks, else the total; the label spells everything out", () => {
    const counts = { ...NONE, total: 3, manual: 1, schedule: 1, service: 1 };
    expect(jobCountsView(counts, null)).toEqual({
      running: 0,
      failing: 0,
      error: false,
      total: 3,
      label: "3 jobs: 1 service, 1 scheduled, 1 manual",
    });
    expect(jobCountsView({ ...counts, running: 1, failing: 1 }, null)).toEqual({
      running: 1,
      failing: 1,
      error: false,
      total: undefined,
      label: "3 jobs: 1 service, 1 scheduled, 1 manual · 1 running · 1 failed",
    });
    expect(jobCountsView({ ...NONE, total: 1, service: 2, error: true }, "jobs.a: unknown key")).toEqual({
      running: 0,
      failing: 0,
      error: true,
      total: undefined,
      label: "1 job: 2 services · jobs.json is invalid: jobs.a: unknown key",
    });
    expect(jobCountsView({ ...NONE, error: true }, undefined).label).toBe("jobs.json is invalid");
  });
});

describe("job rows", () => {
  // spec: UI-742
  test("a row says its trigger, action, state with its time, and last run", () => {
    const manual = jobRow(jobView("deploy", { description: "Ship it" }), NOON);
    expect(manual).toMatchObject({
      name: "deploy",
      description: "Ship it",
      trigger: "manual",
      action: "make deploy",
      status: "idle",
      mark: "idle",
      statusText: "idle",
      lastRun: undefined,
      start: "run",
      enabled: true,
    });

    const scheduled = jobRow(
      jobView(
        "mail",
        { trigger: "schedule", schedule: "0 7 * * *", command: null, prompt: { text: "read", thread: "reuse" } },
        {
          nextAt: ONE,
          lastRun: runInfo("r1", "mail", { status: "failed", exitCode: 2, endedAt: NOON - 86_400_000 }),
        },
      ),
      NOON,
    );
    expect(scheduled.trigger).toBe("0 7 * * * · next 13:05");
    expect(scheduled.action).toBe("prompt · reuse thread");
    expect(scheduled.lastRun).toBe("last 2026-11-15 12:00: failed · exit 2");
    expect(scheduled.lastRunStatus).toBe("failed");

    const run = runInfo("r2", "watch", { startedAt: NOON - 90_000 });
    const service = jobRow(
      jobView("watch", { trigger: "service", restart: "on-failure" }, { status: "running", run }),
      NOON,
    );
    expect([service.trigger, service.statusText, service.mark, service.run?.id]).toEqual([
      "service · restart on failure",
      "running 1 min 30 s",
      "running",
      "r2",
    ]);
    const waiting = jobRow(jobView("watch", { trigger: "service" }, { status: "waiting", nextAt: ONE }), NOON);
    expect([waiting.statusText, waiting.mark, waiting.start, waiting.trigger]).toEqual([
      "waiting · restart 13:05",
      "awaiting",
      "start",
      "service · restart always",
    ]);
    expect(jobRow(jobView("watch", { trigger: "service" }, { status: "stopped" }), NOON).start).toBe("start");
    expect(jobRow(jobView("old", { enabled: false }, { status: "disabled" }), NOON)).toMatchObject({
      statusText: "disabled",
      enabled: false,
    });
    expect(jobRow(jobView("new", { command: null, prompt: { text: "x", thread: "new" } }), NOON).action).toBe(
      "prompt · new thread",
    );
  });

  test("times are the hour on the same day, else the date too", () => {
    expect(formatWhen(ONE, NOON)).toBe("13:05");
    expect(formatWhen(new Date(2026, 0, 2, 3, 4).getTime(), NOON)).toBe("2026-01-02 03:04");
  });
});

describe("runs", () => {
  // spec: UI-743
  test("a run row says what started it, who asked, its times, how it ended and its thread", () => {
    const threads = new Map([["t1", thread("t1", "checkout", "w1")]]);
    const asked = runRow(
      runInfo("r1", "deploy", {
        startedBy: { kind: "thread", threadId: "t1" },
        startedAt: NOON,
        endedAt: ONE,
        status: "killed",
        signal: 15,
      }),
      threads,
      NOON,
    );
    expect(asked).toEqual({
      id: "r1",
      trigger: "by hand",
      startedBy: "by checkout",
      times: "12:00 – 13:05",
      status: "killed",
      mark: "idle",
      outcome: "killed · signal 15",
      reason: undefined,
      threadId: undefined,
      output: true,
    });
    const prompt = runRow(
      runInfo("r2", "mail", { trigger: "schedule", threadId: "t9", status: "skipped", reason: "running" }),
      threads,
      NOON,
    );
    expect(prompt.output).toBe(false);
    expect(runRow(runInfo("r4", "mail", { command: null, status: "succeeded" }), threads, NOON).output).toBe(false);
    expect([prompt.trigger, prompt.startedBy, prompt.reason, prompt.threadId, prompt.outcome]).toEqual([
      "scheduled",
      undefined,
      "running",
      "t9",
      "skipped",
    ]);
    expect(startedByText({ kind: "user" }, threads)).toBe("by you");
    expect(startedByText({ kind: "mcp" }, threads)).toBe("by an MCP client");
    expect(startedByText({ kind: "job", job: "nightly" }, threads)).toBe("by job nightly");
    expect(startedByText({ kind: "thread", threadId: "gone" }, threads)).toBe("by gone");
    expect(runOutcome(runInfo("r3", "x", { status: "succeeded", exitCode: 0 }))).toBe("succeeded");
    expect(
      ["running", "succeeded", "failed", "interrupted", "skipped"].map((status) => runMark(status as never)),
    ).toEqual(["running", "done", "failed", "failed", "idle"]);
  });
});

describe("provenance and run events", () => {
  // spec: UI-746
  test("a thread a job made names the job", () => {
    expect(threadJob(thread("t1", "mail", "w1"))).toBeUndefined();
    const made = thread("t2", "mail", "w1", { provenance: { ...provenance, createdBy: { kind: "job", job: "mail" } } });
    expect(threadJob(made)).toBe("mail");
  });

  // spec: UI-748
  test("run_started starts, run_ended ends, and a failed end also fails", () => {
    expect(runEventKinds("run_started", runInfo("r1", "a"))).toEqual(["job_started"]);
    expect(runEventKinds("run_ended", runInfo("r1", "a", { status: "succeeded" }))).toEqual(["job_ended"]);
    expect(runEventKinds("run_ended", runInfo("r1", "a", { status: "failed" }))).toEqual(["job_ended", "job_failed"]);
  });
});
