/**
 * The Workspace widget (UI-025): the current workspace (the selected thread's, else the draft's) with its repositories
 * and their git status, a warning when several threads work in it or a checkout is in another workspace with live
 * threads (DMN-027), and the sources the "add repository" control sends.
 */

import type { GitStatus, RepoSource, RepoStatus, ThreadSummary, Workspace, WorkspaceRepo } from "@omfp/protocol";
import { strings } from "../strings/index.ts";
import { isArchived, type RepoKind } from "./threads.ts";

const text = strings.shell.workspace;

/** One repository of a workspace. */
export interface RepoRow {
  /** Entry name in the folder. */
  readonly name: string;
  readonly kind: RepoKind;
  /** Checked-out branch (live status first, else the index), or `detached`. */
  readonly branch: string;
  /** `→ /src/be` for a link, `of /src/fe` for a worktree; nothing for a clone. */
  readonly origin: string | undefined;
  /** `↑2 ↓0 · dirty · 2 files +10 −1` once the status is known. */
  readonly git: string | undefined;
  /** Uncommitted changes; `undefined` while the status is unknown. */
  readonly dirty: boolean | undefined;
  /** Names of other workspaces with live threads holding the same checkout (DMN-027). */
  readonly alsoIn: readonly string[];
}

/** One workspace. */
export interface WorkspaceRow {
  readonly id: string;
  readonly name: string;
  readonly path: string;
  /** Non-archived threads living in it. */
  readonly threadCount: number;
  /** More than one thread works in it (DMN-027). */
  readonly shared: boolean;
  readonly repos: readonly RepoRow[];
}

/**
 * The window's current workspace (UI-100): the selected thread's (`selected` is its summary in the index), else the
 * draft thread's (UI-023), else none. While the selected thread is not in the index yet, none: never the draft's.
 */
export function currentWorkspaceId(
  selectedId: string | undefined,
  selected: ThreadSummary | undefined,
  draft: { readonly workspaceId: string } | undefined,
): string | undefined {
  return selectedId === undefined ? draft?.workspaceId : selected?.workspaceId;
}

/** The row of `workspace`; `status` is the `workspace/<id>` topic's git status (`undefined` until it arrives). */
export function workspaceRow(
  workspace: Workspace,
  threads: readonly ThreadSummary[],
  workspaces: readonly Workspace[],
  status: readonly RepoStatus[] | undefined,
): WorkspaceRow {
  const threadCount = livingThreads(workspace.id, threads);
  return {
    id: workspace.id,
    name: workspace.name,
    path: workspace.path,
    threadCount,
    shared: threadCount > 1,
    repos: workspace.repos.map((repo) => {
      const entry = status?.find((candidate) => candidate.name === repo.name);
      // A status the daemon could not read (DMN-383) is no status: the row keeps the index's branch.
      const git = (entry?.error ?? undefined) === undefined ? entry?.status : undefined;
      return {
        name: repo.name,
        kind: repo.kind,
        branch: (git === undefined ? repo.branch : git.branch) ?? text.detached,
        origin: repoOrigin(repo),
        git: git === undefined ? undefined : gitSummary(git),
        dirty: git?.dirty,
        alsoIn: sharedWithLive(repo, workspaces, threads),
      };
    }),
  };
}

function livingThreads(workspaceId: string, threads: readonly ThreadSummary[]): number {
  return threads.filter((thread) => !isArchived(thread) && thread.workspaceId === workspaceId).length;
}

/** Where a repository comes from: a link's target or a worktree's main repository. */
function repoOrigin(repo: WorkspaceRepo): string | undefined {
  switch (repo.kind) {
    case "link":
      return text.linkTarget(repo.target);
    case "worktree":
      return text.worktreeOf(repo.main);
    case "clone":
      return undefined;
  }
}

/** The names of the workspaces in `repo.sharedWith` that have live (non-archived) threads (DMN-027). */
function sharedWithLive(
  repo: WorkspaceRepo,
  workspaces: readonly Workspace[],
  threads: readonly ThreadSummary[],
): string[] {
  return workspaces
    .filter((workspace) => repo.sharedWith.includes(workspace.id) && livingThreads(workspace.id, threads) > 0)
    .map((workspace) => workspace.name);
}

/** One-line git summary next to the branch: ahead/behind, dirty, changed files. */
export function gitSummary(status: GitStatus): string {
  const parts: string[] = [];
  if (status.ahead > 0 || status.behind > 0) {
    parts.push(text.aheadBehind(status.ahead, status.behind));
  }
  if (status.dirty) {
    parts.push(text.dirty);
  }
  if (status.files.length > 0) {
    const additions = status.files.reduce((sum, file) => sum + file.additions, 0);
    const deletions = status.files.reduce((sum, file) => sum + file.deletions, 0);
    parts.push(text.files(status.files.length, additions, deletions));
  } else if (!status.dirty) {
    parts.push(text.clean);
  }
  return parts.join(" · ");
}

/** Removing a link only deletes the symlink; anything else asks first (UI-025, DMN-025). */
export function removalNeedsConfirmation(kind: RepoKind): boolean {
  return kind !== "link";
}

/** What the "add repository" control adds (its kind switch, UI-025): worktrees are only made by the agent. */
export type AddKind = "link" | "clone";

export const ADD_KINDS: readonly AddKind[] = ["link", "clone"];

/** The fields of the "add repository" control; each kind reads its own. */
export interface AddFields {
  /** Folder to link. */
  readonly path: string;
  readonly url: string;
}

/** The `workspaces.add_repo` source for `kind`, from trimmed fields; `undefined` while its required field is empty. */
export function addRepoSource(kind: AddKind, fields: AddFields): RepoSource | undefined {
  switch (kind) {
    case "link": {
      const path = fields.path.trim();
      return path === "" ? undefined : { kind: "link", path };
    }
    case "clone": {
      const url = fields.url.trim();
      return url === "" ? undefined : { kind: "clone", url };
    }
  }
}
