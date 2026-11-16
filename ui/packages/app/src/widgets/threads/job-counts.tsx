/**
 * The job counts after a workspace's name in the left column (UI-740): running runs with the running mark, failing
 * jobs with the failed mark, the warn mark for a refused `jobs.json`, else the muted total with the jobs icon; its
 * tooltip and name spell everything out. Pressing it reveals `omfp:jobs` for that workspace (UI-741).
 */

import { Button, Icon, StateMark } from "@omfp/kit";
import type { JobCounts as Counts } from "@omfp/protocol";
import type { ReactNode } from "react";
import { REVEAL_WIDGET, useSnapshot, useWindowState } from "../../sdk/index.ts";
import { jobCountsView } from "../../vm/jobs.ts";

export function JobCounts({ workspaceId, counts }: { workspaceId: string; counts: Counts }): ReactNode {
  const [, reveal] = useWindowState(REVEAL_WIDGET);
  // The window follows the jobs of every counted workspace (UI-748): the refused file's reason is at hand.
  const error = useSnapshot((snapshot) => snapshot.workspaceJobs.get(workspaceId)?.state.error);
  const view = jobCountsView(counts, error);
  return (
    <Button
      size="sm"
      variant="quiet"
      className="omfp-job-counts"
      aria-label={view.label}
      data-testid="job-counts"
      data-workspace-id={workspaceId}
      onPress={() => reveal({ reference: "omfp:jobs", nonce: Date.now(), workspaceId })}
    >
      <span className="omfp-job-counts__parts" title={view.label}>
        {view.running > 0 && (
          <span className="omfp-job-counts__part" data-part="running">
            <StateMark state="running" />
            {view.running}
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
        {view.total !== undefined && (
          <span className="omfp-job-counts__part" data-part="total">
            <Icon name="clock" />
            {view.total}
          </span>
        )}
      </span>
    </Button>
  );
}
