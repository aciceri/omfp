/**
 * `omfp:jobs` (UI-742, UI-743): the jobs a workspace declares in `.omfp/jobs.json`, live from `jobs/<id>` through
 * `useJobs` (AGT-090), one row each — name (its description as tooltip), trigger, action, state and last run — with
 * Run (Start for a stopped or waiting service), Stop while a run runs, and Enable/Disable; an error is a toast. One row
 * at a time expands to its runs (`jobs.runs`, newest first, Load older): a command run to its output (`job/<id>`), a
 * prompt run to its thread. A refused file's error shows above the rows, with Open jobs.json.
 */

import type { JobRuns, JobView, RunInfo } from "@omfp/protocol";
import { Button, EmptyState, Icon, Log, StateBadge, StateMark } from "omfp/kit";
import {
  type JobRow,
  jobRow,
  type RunRow,
  runRow,
  strings,
  useCall,
  useJobs,
  useOpenFile,
  useReportError,
  useSnapshot,
  useThreadActions,
  useThreads,
  useTopic,
  useWidget,
  type WidgetProps,
} from "omfp/sdk";
import { type ReactNode, useEffect, useId, useRef, useState } from "react";

const text = strings.shell.jobs;

/** The workspace's jobs file, which Open jobs.json opens. */
const JOBS_FILE = ".omfp/jobs.json";

/** Props: `workspaceId?`, the workspace whose jobs to show (default: the widget's, the current one). */
export function JobsWidget(props: WidgetProps): ReactNode {
  const widget = useWidget();
  const workspaceId = typeof props.workspaceId === "string" ? props.workspaceId : widget.workspaceId;
  if (workspaceId === undefined) {
    return <EmptyState icon="inbox" title={text.noWorkspace} data-testid="jobs-no-workspace" />;
  }
  return <Jobs key={workspaceId} workspaceId={workspaceId} />;
}

/** `Date.now()`, again every second while `ticking` (running durations). */
function useNow(ticking: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!ticking) {
      return undefined;
    }
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [ticking]);
  return now;
}

function Jobs({ workspaceId }: { workspaceId: string }): ReactNode {
  const { jobs, error, loaded, run, kill, enable } = useJobs(workspaceId);
  const report = useReportError();
  const openFile = useOpenFile();
  const [open, setOpen] = useState<string | undefined>(undefined);
  const now = useNow(jobs.some((job) => job.status === "running" || job.status === "waiting"));
  const openButton = (
    <Button
      size="sm"
      icon={<Icon name="file" />}
      onPress={() => openFile(JOBS_FILE, { workspaceId })}
      data-testid="jobs-open-file"
    >
      {text.openFile}
    </Button>
  );
  if (!loaded) {
    return <EmptyState icon="clock" title={text.loading} data-testid="jobs-loading" />;
  }
  return (
    <section className="omfp-jobs" data-testid="jobs" aria-label={text.label}>
      {error !== null && (
        <div className="omfp-banner omfp-jobs__error" data-tone="error" role="alert" data-testid="jobs-error">
          <span>{text.invalid(error)}</span>
          {openButton}
        </div>
      )}
      {jobs.length === 0 ? (
        error === null && (
          <EmptyState icon="clock" title={text.none} action={openButton} data-testid="jobs-none">
            {text.noneHint}
          </EmptyState>
        )
      ) : (
        <ul className="omfp-jobs__list">
          {jobs.map((job) => (
            <Row
              key={job.name}
              workspaceId={workspaceId}
              job={job}
              row={jobRow(job, now)}
              now={now}
              open={open === job.name}
              onToggle={() => setOpen((current) => (current === job.name ? undefined : job.name))}
              run={() => void run(job.name).catch(report)}
              kill={() => void kill(job.name).catch(report)}
              enable={(enabled) => void enable(job.name, enabled).catch(report)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

interface RowProps {
  readonly workspaceId: string;
  readonly job: JobView;
  readonly row: JobRow;
  readonly now: number;
  readonly open: boolean;
  readonly onToggle: () => void;
  readonly run: () => void;
  readonly kill: () => void;
  readonly enable: (enabled: boolean) => void;
}

function Row({ workspaceId, job, row, now, open, onToggle, run, kill, enable }: RowProps): ReactNode {
  const runsId = useId();
  return (
    <li
      className="omfp-jobs__item"
      data-testid="job-row"
      data-job={row.name}
      data-status={row.status}
      data-enabled={row.enabled}
    >
      <div className="omfp-jobs__head">
        <button
          type="button"
          className="omfp-jobs__row omfp-focus-item"
          title={row.description}
          aria-expanded={open}
          aria-controls={open ? runsId : undefined}
          aria-label={text.runs(row.name)}
          data-testid="job-toggle"
          onClick={onToggle}
        >
          <span className="omfp-jobs__name">{row.name}</span>
          <StateBadge bare state={row.mark} data-testid="job-status">
            {row.statusText}
          </StateBadge>
          <span className="omfp-jobs__trigger" data-testid="job-trigger">
            {row.trigger}
          </span>
          <span className="omfp-jobs__action" data-testid="job-action" title={row.action}>
            {row.action}
          </span>
          <span className="omfp-jobs__last" data-testid="job-last-run" data-status={row.lastRunStatus}>
            {row.lastRun ?? text.never}
          </span>
        </button>
        <div className="omfp-jobs__actions">
          {row.run === undefined ? (
            <Button
              size="sm"
              icon={<Icon name="play" />}
              isDisabled={!row.enabled}
              aria-label={row.start === "start" ? text.startLabel(row.name) : text.runLabel(row.name)}
              data-testid="job-run"
              onPress={run}
            >
              {row.start === "start" ? text.start : text.run}
            </Button>
          ) : (
            <Button
              size="sm"
              icon={<Icon name="square" />}
              aria-label={text.stopLabel(row.name)}
              data-testid="job-stop"
              onPress={kill}
            >
              {text.stop}
            </Button>
          )}
          <Button
            size="sm"
            variant="quiet"
            aria-label={row.enabled ? text.disableLabel(row.name) : text.enableLabel(row.name)}
            data-testid="job-enable"
            onPress={() => enable(!row.enabled)}
          >
            {row.enabled ? text.disable : text.enable}
          </Button>
        </div>
      </div>
      {open && <Runs id={runsId} workspaceId={workspaceId} job={job} now={now} />}
    </li>
  );
}

/** The older runs loaded under a first page (`base`, the `jobs.runs` answer they continue). */
interface Page {
  readonly base: JobRuns | undefined;
  readonly runs: readonly RunInfo[];
  readonly more: boolean;
}

/** A job's runs (UI-743): `jobs.runs` newest first, again when a run starts or ends, then older pages on request. */
function Runs({
  id,
  workspaceId,
  job,
  now,
}: {
  id: string;
  workspaceId: string;
  job: JobView;
  now: number;
}): ReactNode {
  const call = useCall();
  const report = useReportError();
  const threads = useThreads();
  const first = useCall("jobs.runs", { workspaceId, job: job.name });
  const [older, setOlder] = useState<Page | undefined>(undefined);
  const [loading, setLoading] = useState(false);
  // Bumped by every refresh: an older page asked for before it continues a first page no longer shown.
  const generation = useRef(0);
  // A run of the job started or ended — skipped ones too, which change neither `run` nor `lastRun` — or its state
  // changed: the first page again, without the older ones.
  const events = useSnapshot((snapshot) => snapshot.workspaceJobs.get(workspaceId)?.runs);
  const heard = events?.findLast((event) => event.run.job === job.name)?.seq ?? 0;
  const changed = `${heard} ${job.run?.id ?? ""} ${job.lastRun?.id ?? ""} ${job.lastRun?.status ?? ""}`;
  const seen = useRef(changed);
  const { refresh } = first;
  useEffect(() => {
    if (seen.current !== changed) {
      seen.current = changed;
      generation.current += 1;
      setOlder(undefined);
      setLoading(false);
      refresh();
    }
  }, [changed, refresh]);
  // `useCall` drops a first page answered after a newer refresh; older pages hang on the first page they continue.
  const base = first.data;
  const shownOlder = older !== undefined && older.base === base ? older : undefined;
  const rows = [...(base?.runs ?? []), ...(shownOlder?.runs ?? [])];
  const more = shownOlder?.more ?? base?.more ?? false;
  const loadOlder = () => {
    const last = rows.at(-1);
    if (last === undefined) {
      return;
    }
    const asked = generation.current;
    setLoading(true);
    call("jobs.runs", { workspaceId, job: job.name, before: last.id })
      .then((page) => {
        if (generation.current === asked) {
          setOlder((previous) => ({
            base,
            runs: [...(previous !== undefined && previous.base === base ? previous.runs : []), ...page.runs],
            more: page.more,
          }));
        }
      })
      .catch(report)
      .finally(() => {
        if (generation.current === asked) {
          setLoading(false);
        }
      });
  };
  return (
    <div id={id} className="omfp-jobs__runs" data-testid="job-runs">
      {first.data === undefined ? (
        <p className="omfp-jobs__muted">{first.error === undefined ? text.readingRuns : String(first.error)}</p>
      ) : rows.length === 0 ? (
        <p className="omfp-jobs__muted" data-testid="job-no-runs">
          {text.noRuns}
        </p>
      ) : (
        <ul className="omfp-jobs__run-list">
          {rows.map((run) => (
            <RunItem key={run.id} row={runRow(run, threads, now)} />
          ))}
        </ul>
      )}
      {more && (
        <Button size="sm" variant="quiet" isPending={loading} onPress={loadOlder} data-testid="job-runs-older">
          {text.loadOlder}
        </Button>
      )}
    </div>
  );
}

function RunItem({ row }: { row: RunRow }): ReactNode {
  const actions = useThreadActions();
  const threads = useThreads();
  const [output, setOutput] = useState(false);
  const outputId = useId();
  const thread = row.threadId === undefined ? undefined : threads.get(row.threadId);
  return (
    <li className="omfp-jobs__run" data-testid="job-run-row" data-run={row.id} data-status={row.status}>
      <div className="omfp-jobs__run-head">
        <StateMark state={row.mark} />
        <span data-testid="job-run-trigger">
          {row.startedBy === undefined ? row.trigger : `${row.trigger} ${row.startedBy}`}
        </span>
        <span className="omfp-jobs__muted" data-testid="job-run-times">
          {row.times}
        </span>
        <span data-testid="job-run-outcome">{row.outcome}</span>
        {row.reason !== undefined && (
          <span className="omfp-jobs__muted" data-testid="job-run-reason">
            {row.reason}
          </span>
        )}
        {row.threadId !== undefined && (
          <Button
            size="sm"
            variant="quiet"
            icon={<Icon name="message-square" />}
            data-testid="job-run-thread"
            onPress={() => {
              if (row.threadId !== undefined) {
                actions.select(row.threadId);
              }
            }}
          >
            {text.openThread(thread?.title ?? row.threadId)}
          </Button>
        )}
        {row.output && (
          <Button
            size="sm"
            variant="quiet"
            aria-expanded={output}
            {...(output ? { "aria-controls": outputId } : {})}
            data-testid="job-run-output-toggle"
            onPress={() => setOutput((shown) => !shown)}
          >
            {text.output(row.id)}
          </Button>
        )}
      </div>
      {output && <RunOutput id={outputId} runId={row.id} />}
    </li>
  );
}

/** A command run's output, followed on `job/<id>` while shown. */
function RunOutput({ id, runId }: { id: string; runId: string }): ReactNode {
  useTopic(`job/${runId}`);
  const state = useSnapshot((snapshot) => snapshot.jobs.get(runId));
  return (
    <Log
      id={id}
      className="omfp-jobs__output"
      lines={state === undefined ? [] : state.lines.map((line) => line.text)}
      aria-label={text.output(runId)}
      data-testid="job-run-output"
    />
  );
}
