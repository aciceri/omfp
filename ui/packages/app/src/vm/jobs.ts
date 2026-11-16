/**
 * Declared jobs in the window (UI-740..UI-748): the left column's job counts, the rows and runs of `omfp:jobs`, the
 * job a thread was made by, and the init files' run events, as data the components render.
 */

import type { ThreadState as MarkState } from "@omfp/kit";
import type { CreatedBy, JobCounts, JobRunStatus, JobView, RunInfo, ThreadSummary } from "@omfp/protocol";
import { strings } from "../strings/index.ts";
import { displayTitle } from "./threads.ts";
import { formatDuration } from "./tool.ts";

const text = strings.shell.jobs;

/** The job that made `thread` (PRT-209), else `undefined`. */
export function threadJob(thread: Pick<ThreadSummary, "provenance">): string | undefined {
  const { createdBy } = thread.provenance;
  return createdBy.kind === "job" ? createdBy.job : undefined;
}

/** What the `job-counts` button of a workspace section shows (UI-740). */
export interface JobCountsView {
  /** Runs running now; shown with the running mark when some run. */
  readonly running: number;
  /** Enabled jobs whose last run failed; shown with the failed mark when some failed. */
  readonly failing: number;
  /** The file is refused: the warn mark. */
  readonly error: boolean;
  /** The total, shown muted with the jobs icon only when nothing runs, failed or is refused. */
  readonly total: number | undefined;
  /** The tooltip and accessible name, spelling everything out. */
  readonly label: string;
}

/** The button of `counts` (PRT-210); `error` is why the file is refused when the workspace's `jobs/<id>` says so. */
export function jobCountsView(counts: JobCounts, error: string | null | undefined): JobCountsView {
  const kinds = [
    counts.service > 0 ? text.countService(counts.service) : undefined,
    counts.schedule > 0 ? text.countSchedule(counts.schedule) : undefined,
    counts.manual > 0 ? text.countManual(counts.manual) : undefined,
  ].filter((part) => part !== undefined);
  const parts = [
    counts.total > 0 ? text.counts(counts.total, kinds.join(", ")) : undefined,
    counts.running > 0 ? text.countRunning(counts.running) : undefined,
    counts.failing > 0 ? text.countFailed(counts.failing) : undefined,
    counts.error ? text.invalid(error ?? undefined) : undefined,
  ].filter((part) => part !== undefined);
  const quiet = counts.running === 0 && counts.failing === 0 && !counts.error;
  return {
    running: counts.running,
    failing: counts.failing,
    error: counts.error,
    total: quiet ? counts.total : undefined,
    label: parts.join(" · "),
  };
}

/** `at` as the local `HH:MM` when it is on the day of `now`, else `YYYY-MM-DD HH:MM`. */
export function formatWhen(at: number, now: number): string {
  const date = new Date(at);
  const pad = (value: number) => String(value).padStart(2, "0");
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  const day = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  return date.toDateString() === new Date(now).toDateString() ? time : `${day} ${time}`;
}

/** How a run ended in words: its status, then its exit code or signal when it has one. */
export function runOutcome(run: RunInfo): string {
  const status = text.runStatus[run.status];
  if (run.signal !== undefined && run.signal !== null) {
    return `${status} · ${text.signal(run.signal)}`;
  }
  if (run.exitCode !== undefined && run.exitCode !== null && run.status !== "succeeded") {
    return `${status} · ${text.exit(run.exitCode)}`;
  }
  return status;
}

/** The mark of a job's state badge (UI-483's states). */
const STATUS_MARKS: Readonly<Record<JobView["status"], MarkState>> = {
  running: "running",
  waiting: "awaiting",
  stopped: "idle",
  disabled: "idle",
  idle: "idle",
};

/** The mark of a run's state. */
export function runMark(status: JobRunStatus): MarkState | "done" {
  switch (status) {
    case "running":
      return "running";
    case "succeeded":
      return "done";
    case "failed":
    case "interrupted":
      return "failed";
    default:
      return "idle";
  }
}

/** One row of `omfp:jobs` (UI-742, UI-743). */
export interface JobRow {
  readonly name: string;
  readonly description: string | undefined;
  /** `manual`, the cron expression with its next time, or `service` with its restart policy. */
  readonly trigger: string;
  /** The command, or `prompt` with `new`/`reuse`. */
  readonly action: string;
  readonly status: JobView["status"];
  readonly mark: MarkState;
  /** The state's word: running with its duration, waiting with the time of the restart, … */
  readonly statusText: string;
  /** The last run: when, how it ended; `undefined` before the first. */
  readonly lastRun: string | undefined;
  readonly lastRunStatus: JobRunStatus | undefined;
  /** The running run, which Stop kills. */
  readonly run: RunInfo | undefined;
  /** Run starts it now: `start` for a stopped or waiting service. */
  readonly start: "run" | "start";
  readonly enabled: boolean;
}

export function jobRow(job: JobView, now: number): JobRow {
  const { job: definition } = job;
  const next = job.nextAt === undefined || job.nextAt === null ? undefined : formatWhen(job.nextAt, now);
  const trigger =
    definition.trigger === "manual"
      ? text.manual
      : definition.trigger === "schedule"
        ? text.schedule(definition.schedule ?? "", next)
        : text.service(definition.restart);
  const prompt = definition.prompt;
  const action =
    definition.command !== undefined && definition.command !== null
      ? definition.command
      : text.prompt(prompt?.thread ?? "new");
  const run = job.run ?? undefined;
  const statusText =
    job.status === "running"
      ? text.status.running(run === undefined ? undefined : formatDuration(now - run.startedAt))
      : job.status === "waiting"
        ? text.status.waiting(next)
        : text.status[job.status];
  const last = job.lastRun ?? undefined;
  return {
    name: job.name,
    description: definition.description ?? undefined,
    trigger,
    action,
    status: job.status,
    mark: STATUS_MARKS[job.status],
    statusText,
    lastRun:
      last === undefined ? undefined : text.lastRun(formatWhen(last.endedAt ?? last.startedAt, now), runOutcome(last)),
    lastRunStatus: last?.status,
    run,
    start: definition.trigger === "service" && (job.status === "stopped" || job.status === "waiting") ? "start" : "run",
    enabled: definition.enabled,
  };
}

/** Who asked for a run, in words; `undefined` for the scheduler and the supervisor. */
export function startedByText(
  createdBy: CreatedBy | null | undefined,
  threads: ReadonlyMap<string, ThreadSummary>,
): string | undefined {
  switch (createdBy?.kind) {
    case undefined:
      return undefined;
    case "user":
      return text.startedBy(text.who.user);
    case "mcp":
      return text.startedBy(text.who.mcp);
    case "job":
      return text.startedBy(text.who.job(createdBy.job));
    case "thread": {
      const thread = threads.get(createdBy.threadId);
      return text.startedBy(text.who.agent(thread === undefined ? createdBy.threadId : displayTitle(thread)));
    }
  }
}

/** One run under an expanded row of `omfp:jobs` (UI-743). */
export interface RunRow {
  readonly id: string;
  /** What started it, and who asked. */
  readonly trigger: string;
  readonly startedBy: string | undefined;
  /** When it started, and ended once it did. */
  readonly times: string;
  readonly status: JobRunStatus;
  readonly mark: MarkState | "done";
  readonly outcome: string;
  readonly reason: string | undefined;
  /** A prompt run's thread. */
  readonly threadId: string | undefined;
  /** A command run that started: its output is `job/<id>` (PRT-201). */
  readonly output: boolean;
}

export function runRow(run: RunInfo, threads: ReadonlyMap<string, ThreadSummary>, now: number): RunRow {
  const start = formatWhen(run.startedAt, now);
  const end = run.endedAt === undefined || run.endedAt === null ? undefined : formatWhen(run.endedAt, now);
  return {
    id: run.id,
    trigger: text.trigger[run.trigger],
    startedBy: startedByText(run.startedBy, threads),
    times: end === undefined ? start : text.ended(start, end),
    status: run.status,
    mark: runMark(run.status),
    outcome: runOutcome(run),
    reason: run.reason ?? undefined,
    threadId: run.threadId ?? undefined,
    output: run.command !== undefined && run.command !== null && run.status !== "skipped",
  };
}

/** The init files' events of declared runs (UI-748). */
export type JobEventKind = "job_started" | "job_ended" | "job_failed";

/** The init events one `run_started` / `run_ended` event of `jobs/<id>` raises: `job_failed` after `job_ended` for a
 * failed run. */
export function runEventKinds(kind: "run_started" | "run_ended", run: RunInfo): readonly JobEventKind[] {
  if (kind === "run_started") {
    return ["job_started"];
  }
  return run.status === "failed" ? ["job_ended", "job_failed"] : ["job_ended"];
}
