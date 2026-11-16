/**
 * Declared jobs (DMN-320..DMN-336) for widgets, commands and init files: {@link useJobs} (AGT-090) follows a
 * workspace's `jobs/<id>` (PRT-200), and {@link jobActions} acts on its jobs by name (UI-743, UI-745, UI-748).
 */

import { type JobView, jobsTopic, type RunInfo } from "@omfp/protocol";
import { useContext, useMemo } from "react";
import { strings } from "../strings/index.ts";
import { useSnapshot, useTopic } from "./hooks.ts";
import { type SdkClient, useHost } from "./host.tsx";
import { WidgetScope } from "./widget.ts";

const text = strings.shell.jobs;

/** What acts on a workspace's jobs by name; each call rejects with the method's error. */
export interface JobActions {
  /** Starts a run now (`jobs.run`, PRT-204); a stopped or waiting service starts again. */
  run(name: string): Promise<RunInfo>;
  /** Kills the job's running run (`jobs.kill`); rejects when it runs none. */
  kill(name: string): Promise<void>;
  /** Sets the job's `enabled` in `.omfp/jobs.json` (`jobs.enable`, PRT-207). */
  enable(name: string, enabled: boolean): Promise<JobView>;
}

/**
 * The {@link JobActions} of `workspaceId` (without one, every call rejects); the workspace's `jobs/<id>` state, which
 * `kill` reads for the running run, is the model's as the call is made (`jobs.list` while the window has none).
 */
export function jobActions(client: SdkClient, workspaceId: string | undefined): JobActions {
  if (workspaceId === undefined) {
    const none = () => Promise.reject(new Error(text.noWorkspace));
    return { run: none, kill: none, enable: none };
  }
  return {
    run: (name) => client.call("jobs.run", { workspaceId, job: name }),
    kill: async (name) => {
      const { jobs } =
        client.model.workspaceJobs.get(workspaceId)?.state ?? (await client.call("jobs.list", { workspaceId }));
      const job = jobs.find((candidate) => candidate.name === name);
      if (job === undefined) {
        throw new Error(text.unknown(name));
      }
      if (job.run === undefined || job.run === null) {
        throw new Error(text.notRunning(name));
      }
      await client.call("jobs.kill", { jobId: job.run.id });
    },
    enable: (name, enabled) => client.call("jobs.enable", { workspaceId, name, enabled }),
  };
}

/** What {@link useJobs} gives (AGT-090). */
export interface Jobs extends JobActions {
  /** The declared jobs by name; `[]` until the topic's state arrived. */
  readonly jobs: readonly JobView[];
  /** Why the current `jobs.json` is refused, `null` when it is valid or absent. */
  readonly error: string | null;
  /** The topic's state arrived. */
  readonly loaded: boolean;
}

const NO_JOBS: readonly JobView[] = [];

/**
 * The declared jobs of the widget's workspace (or `workspaceId`), live from `jobs/<id>` (PRT-200) while mounted, with
 * the {@link JobActions} on them. A job's runs: `useCall("jobs.runs", { workspaceId, job })` (PRT-203).
 */
export function useJobs(workspaceId?: string): Jobs {
  const host = useHost();
  // Outside a widget slot (the frame), only `workspaceId` names a workspace.
  const scope = useContext(WidgetScope);
  const id = workspaceId ?? scope?.workspaceId;
  useTopic(id === undefined ? undefined : jobsTopic(id));
  const state = useSnapshot((snapshot) => (id === undefined ? undefined : snapshot.workspaceJobs.get(id)?.state));
  const actions = useMemo(() => jobActions(host.client, id), [host, id]);
  return { jobs: state?.jobs ?? NO_JOBS, error: state?.error ?? null, loaded: state !== undefined, ...actions };
}
