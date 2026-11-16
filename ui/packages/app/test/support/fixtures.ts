/** Hand-built protocol DTOs for view-model and widget tests (the TS twin of the Dioxus `test_support`). */

import type {
  JobDefinition,
  JobView,
  Provenance,
  RunInfo,
  ThreadSummary,
  Workspace,
  WorkspaceRepo,
} from "@omfp/protocol";

export const provenance: Provenance = { createdBy: { kind: "user" }, client: "omfp/web" };

/** Entry `name` of the folder `/work/<workspace>`: a worktree of `/src/<name>` on `branch`. */
export function repo(workspace: string, name: string, branch: string | null): WorkspaceRepo {
  return {
    name,
    path: `/work/${workspace}/${name}`,
    kind: "worktree",
    main: `/src/${name}`,
    branch,
    head: null,
    isRoot: false,
    canonicalPath: `/work/${workspace}/${name}`,
    sharedWith: [],
  };
}

/** A folder `/work/<id>` named `id` holding `repos`. */
export function workspace(id: string, repos: WorkspaceRepo[] = [], threadIds: string[] = []): Workspace {
  return { id, name: id, path: `/work/${id}`, repos, threadIds, provenance, createdAt: 1, position: 0 };
}

/** An idle running thread in `workspaceId`. */
export function thread(
  id: string,
  title: string,
  workspaceId: string,
  extra: Partial<ThreadSummary> = {},
): ThreadSummary {
  return {
    id,
    title,
    titleSource: title === "" ? "placeholder" : "set",
    workspaceId,
    process: { state: "running" },
    settled: true,
    needsAttention: false,
    provenance,
    createdAt: 1,
    updatedAt: 1,
    archivedAt: null,
    position: 0,
    ...extra,
  };
}

/** A run of the declared job `job` of `w1` running `make <job>` (`sleep 60` for a transient one), running since 1000
 * (PRT-201). */
export function runInfo(id: string, job: string | null, extra: Partial<RunInfo> = {}): RunInfo {
  return {
    id,
    workspaceId: "w1",
    job,
    command: job === null ? "sleep 60" : `make ${job}`,
    trigger: "manual",
    startedBy: null,
    threadId: null,
    startedAt: 1000,
    endedAt: null,
    status: "running",
    reason: null,
    exitCode: null,
    signal: null,
    ...extra,
  };
}

/** An idle manual job `name` running `make <name>`, its defaults filled in (PRT-200). */
export function jobView(name: string, job: Partial<JobDefinition> = {}, extra: Partial<JobView> = {}): JobView {
  return {
    name,
    job: {
      trigger: "manual",
      missed: "skip",
      restart: "always",
      enabled: true,
      command: `make ${name}`,
      ...job,
    },
    status: "idle",
    run: null,
    lastRun: null,
    nextAt: null,
    ...extra,
  };
}
