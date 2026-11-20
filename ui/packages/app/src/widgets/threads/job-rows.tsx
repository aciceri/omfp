/**
 * A workspace's declared jobs in the left column (UI-740, UI-741, UI-743, UI-992..UI-995): `Jobs (N)`, collapsed by
 * default, with at its right end the running count with the running mark, the count of the job threads awaiting the
 * user with the awaiting mark, the failing count with the failed mark and the warn mark for a refused `jobs.json`
 * (`job-counts`, spelled out in its tooltip and name). Expanded, one row per job, current while one of its threads is
 * selected: its state mark, name, and state or next time, which Open thread (while it has a thread) and Run (Start) or
 * Stop replace on hover and focus; pressing a row reveals `omfp:jobs` with that job's row open (`OPEN_JOB`).
 */

import { HintRow, Icon, IconButton, NestedRow, RowList, StateMark, ToggleRow } from "@omfp/kit";
import type { JobCounts, ThreadSummary } from "@omfp/protocol";
import type { ReactNode } from "react";
import {
  OPEN_JOB,
  REVEAL_WIDGET,
  SELECTED_THREAD,
  useHost,
  useJobs,
  useReportError,
  useSnapshot,
  useThreadActions,
  useWindowState,
} from "../../sdk/index.ts";
import { strings } from "../../strings/index.ts";
import { jobCountsView, jobSideRow } from "../../vm/jobs.ts";
import { toggled } from "../../vm/threads.ts";
import { JOBS_OPEN } from "./keys.ts";

const text = strings.shell.jobs;

/** `threads`: the workspace's threads listed under its jobs rather than among its threads (UI-992). */
export function WorkspaceJobs({
  workspaceId,
  counts,
  threads,
}: {
  workspaceId: string;
  counts: JobCounts;
  threads: readonly ThreadSummary[];
}): ReactNode {
  const [open, setOpen] = useWindowState(JOBS_OPEN);
  const expanded = open.includes(workspaceId);
  // The window follows the jobs of every counted workspace (UI-748): the refused file's reason is at hand.
  const error = useSnapshot((snapshot) => snapshot.workspaceJobs.get(workspaceId)?.state.error);
  const view = jobCountsView(counts, error, threads);
  const marks = view.running > 0 || view.awaiting > 0 || view.failing > 0 || view.error;
  return (
    <>
      <ToggleRow
        expanded={expanded}
        data-testid="jobs-toggle"
        data-workspace-id={workspaceId}
        onPress={() => setOpen((previous) => toggled(previous, workspaceId))}
        end={
          marks ? (
            <span
              className="omfp-job-counts"
              role="img"
              aria-label={view.label}
              title={view.label}
              data-testid="job-counts"
            >
              {view.running > 0 && (
                <span className="omfp-job-counts__part" data-part="running">
                  <StateMark state="running" />
                  {view.running}
                </span>
              )}
              {view.awaiting > 0 && (
                <span className="omfp-job-counts__part" data-part="awaiting">
                  <StateMark state="awaiting" />
                  {view.awaiting}
                </span>
              )}
              {view.failing > 0 && (
                <span className="omfp-job-counts__part" data-part="failing">
                  <StateMark state="failed" />
                  {view.failing}
                </span>
              )}
              {view.error && (
                <span className="omfp-job-counts__part" data-part="error">
                  <Icon name="circle-alert" />
                </span>
              )}
            </span>
          ) : undefined
        }
      >
        <span title={view.label}>{strings.shell.threads.jobs(counts.total)}</span>
      </ToggleRow>
      {expanded && <JobList workspaceId={workspaceId} threads={threads} />}
    </>
  );
}

/** The workspace's jobs under the expanded `Jobs (N)`, from its `jobs/<id>` (`useJobs`). */
function JobList({ workspaceId, threads }: { workspaceId: string; threads: readonly ThreadSummary[] }): ReactNode {
  const host = useHost();
  const { jobs, error, loaded, run, kill } = useJobs(workspaceId);
  const actions = useThreadActions();
  const [selected] = useWindowState(SELECTED_THREAD);
  const report = useReportError();
  const now = Date.now();
  const reveal = (job: string) => {
    const nonce = Date.now();
    host.window.set(OPEN_JOB, { workspaceId, job, nonce });
    host.window.set(REVEAL_WIDGET, { reference: "omfp:jobs", nonce, workspaceId });
  };
  const hint = !loaded ? text.loading : jobs.length > 0 ? undefined : error !== null ? text.invalid(error) : text.none;
  return (
    <RowList data-testid="jobs-list">
      {jobs.map((job) => {
        const row = jobSideRow(job, now, threads, selected);
        const { threadId } = row;
        return (
          <NestedRow
            key={row.name}
            state={row.mark}
            stateLabel={row.meta}
            title={row.name}
            meta={row.meta}
            selected={row.current}
            data-testid="job-side-row"
            data-job={row.name}
            onSelect={() => reveal(row.name)}
            selectProps={{
              "data-testid": "job-side-open",
              ...(row.description !== undefined && { title: row.description }),
            }}
            actions={
              <>
                {threadId !== undefined && (
                  <IconButton
                    label={text.openThreadLabel(row.name)}
                    icon={<Icon name="message-square" />}
                    data-testid="job-side-thread"
                    onPress={() => actions.select(threadId)}
                  />
                )}
                {row.run === undefined ? (
                  <IconButton
                    label={row.start === "start" ? text.startLabel(row.name) : text.runLabel(row.name)}
                    icon={<Icon name="play" />}
                    isDisabled={!row.enabled}
                    data-testid="job-side-run"
                    onPress={() => void run(row.name).catch(report)}
                  />
                ) : (
                  <IconButton
                    label={text.stopLabel(row.name)}
                    icon={<Icon name="square" />}
                    data-testid="job-side-stop"
                    onPress={() => void kill(row.name).catch(report)}
                  />
                )}
              </>
            }
          />
        );
      })}
      {hint !== undefined && (
        <li>
          <HintRow>{hint}</HintRow>
        </li>
      )}
    </RowList>
  );
}
