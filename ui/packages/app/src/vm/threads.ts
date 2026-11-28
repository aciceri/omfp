/**
 * The left column (UI-020..023, UI-026): state badges, one section per workspace with its threads and
 * its archived ones, the full-text filter (local fields plus `threads.search`), collapsed sections, unread marks and
 * the "next thread needing attention" and next/previous thread shortcuts.
 */

import type { ThreadState as MarkState } from "@omfp/kit";
import type { CreateWorkspaceParams, ThreadSummary, Workspace, WorkspaceRepo } from "@omfp/protocol";
import { strings } from "../strings/index.ts";

/** What a thread's badge says; also its `data-status` token. */
export type ThreadStatus = "awaiting" | "failed" | "running" | "starting" | "idle" | "stopped";

/** Attention first, then the process, then omp's settled flag. */
export function threadStatus(thread: ThreadSummary): ThreadStatus {
  if (thread.needsAttention) {
    return "awaiting";
  }
  switch (thread.process.state) {
    case "failed":
      return "failed";
    case "starting":
      return "starting";
    case "running":
      return thread.settled ? "idle" : "running";
    case "stopped":
      return "stopped";
  }
}

/** The kit state mark of a thread (UI-137, UI-157): running, awaiting and failed win; else unread or idle. */
export function threadMark(status: ThreadStatus, unread = false): MarkState {
  switch (status) {
    case "running":
    case "starting":
      return "running";
    case "awaiting":
      return "awaiting";
    case "failed":
      return "failed";
    default:
      return unread ? "unread" : "idle";
  }
}

export function statusLabel(status: ThreadStatus): string {
  return strings.shell.status[status];
}

/** The title to show: the thread's, or "New thread" while it has none (DMN-073). */
export function displayTitle(thread: Pick<ThreadSummary, "title">): string {
  return thread.title.trim() === "" ? strings.shell.untitled : thread.title;
}

export type RepoKind = WorkspaceRepo["kind"];

/** `name:branch`, or `name` on a detached HEAD. */
function repoLabel(repo: Pick<WorkspaceRepo, "name" | "branch">): string {
  return repo.branch === undefined || repo.branch === null ? repo.name : `${repo.name}:${repo.branch}`;
}

/** The filter of the column (UI-022, UI-240): the query, what `threads.search` found for it, and a spawned tree. */
export interface ThreadFilter {
  readonly query: string;
  /** Snippet by thread id of the conversation matches of `query` (DMN-076); empty while unknown. */
  readonly conversation: ReadonlyMap<string, string>;
  /** Only these threads: the spawned tree of the "spawned by" filter (UI-240). */
  readonly spawned?: ReadonlySet<string>;
}

/** A query is typed or a spawned tree is chosen: sections without matching threads are hidden and collapsed ones are
 * shown. */
export function isFiltering(filter: ThreadFilter): boolean {
  return filter.query.trim() !== "" || filter.spawned !== undefined;
}

/** `query` (case-insensitive) is in the title, the workspace's name or one of its repositories' names. */
function matchesLocally(thread: ThreadSummary, workspace: Workspace | undefined, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (needle === "") {
    return true;
  }
  const fields = [
    displayTitle(thread),
    ...(workspace === undefined
      ? []
      : [workspace.name, ...workspace.repos.flatMap((repo) => [repo.name, repoLabel(repo)])]),
  ];
  return fields.some((field) => field.toLowerCase().includes(needle));
}

/** The thread passes `filter`: in its spawned tree, if any, and matching the query locally or through its
 * conversation. */
function matchesFilter(thread: ThreadSummary, workspace: Workspace | undefined, filter: ThreadFilter): boolean {
  if (filter.spawned !== undefined && !filter.spawned.has(thread.id)) {
    return false;
  }
  return (
    matchesLocally(thread, workspace, filter.query) ||
    (filter.query.trim() !== "" && filter.conversation.has(thread.id))
  );
}

/** The thread whose agent created `thread` (ARC-009), if an agent did. */
export function spawnerOf(thread: ThreadSummary): string | undefined {
  const { createdBy } = thread.provenance;
  return createdBy.kind === "thread" ? createdBy.threadId : undefined;
}

/** The spawned tree of `rootId` (AGT-070): the thread and every thread it spawned, directly or through threads it
 * spawned, archived ones included. */
export function spawnedTree(threads: readonly ThreadSummary[], rootId: string): ReadonlySet<string> {
  return treeOf(spawnedChildren(threads), rootId);
}

/** How many threads each thread spawned, transitively (its spawned tree without itself); threads that spawned none
 * are left out. */
export function spawnedCounts(threads: readonly ThreadSummary[]): ReadonlyMap<string, number> {
  const children = spawnedChildren(threads);
  return new Map([...children.keys()].map((id) => [id, treeOf(children, id).size - 1]));
}

function spawnedChildren(threads: readonly ThreadSummary[]): ReadonlyMap<string, readonly string[]> {
  const children = new Map<string, string[]>();
  for (const thread of threads) {
    const spawner = spawnerOf(thread);
    if (spawner !== undefined) {
      children.set(spawner, [...(children.get(spawner) ?? []), thread.id]);
    }
  }
  return children;
}

function treeOf(children: ReadonlyMap<string, readonly string[]>, rootId: string): ReadonlySet<string> {
  const tree = new Set([rootId]);
  const pending = [rootId];
  for (let id = pending.pop(); id !== undefined; id = pending.pop()) {
    for (const child of children.get(id) ?? []) {
      if (!tree.has(child)) {
        tree.add(child);
        pending.push(child);
      }
    }
  }
  return tree;
}

/** "+ workspace" input starting with `/` or `~` is a repository path, anything else a name (UI-023). */
export function isWorkspacePath(input: string): boolean {
  const trimmed = input.trim();
  return trimmed.startsWith("/") || trimmed.startsWith("~");
}

/** `workspaces.create` params for the "+ workspace" input: `{ link }` for a path (without trailing slashes), else
 * `{ name }`, or `{}` when empty (the daemon picks the placeholder name). */
export function newWorkspaceParams(input: string): CreateWorkspaceParams {
  const trimmed = input.trim();
  if (isWorkspacePath(trimmed)) {
    return { link: trimmed.length > 1 ? trimmed.replace(/\/+$/, "") : trimmed };
  }
  return trimmed === "" ? {} : { name: trimmed };
}

/** One workspace of the column. */
export interface Section {
  readonly workspace: Workspace;
  /** Its non-archived threads that pass the filter, by `position` (UI-620); without a filter, not those listed under
   * its jobs (UI-992). */
  readonly threadIds: readonly string[];
  /** Its archived threads that pass the filter, most recently archived first (`archivedAt` descending, ties by
   * `position`, then id; UI-026). */
  readonly archivedIds: readonly string[];
  /** Its threads listed under their job's row rather than among its threads, filter or not (UI-992). */
  readonly jobThreadIds: readonly string[];
}

/** The names of the declared jobs by workspace id, for the workspaces whose `jobs/<id>` (PRT-200) the window holds. */
export type DeclaredJobs = ReadonlyMap<string, ReadonlySet<string>>;

const NO_DECLARED_JOBS: DeclaredJobs = new Map();

/**
 * `thread` is listed under its job's row in the left column rather than among its workspace's threads (UI-992): a
 * non-archived thread a job made (PRT-209) in a workspace with jobs, while that workspace declares the job, or its jobs
 * are not read yet. A thread of a job no longer declared stays in the list, where nothing else would reach it.
 */
export function isListedUnderJob(thread: ThreadSummary, workspace: Workspace, declared: DeclaredJobs): boolean {
  const { createdBy } = thread.provenance;
  if (createdBy.kind !== "job" || isArchived(thread) || workspace.jobs === undefined || workspace.jobs === null) {
    return false;
  }
  return declared.get(workspace.id)?.has(createdBy.job) ?? true;
}

/** What takes the focus when the thread or workspace under it leaves the column (UI-285). */
export type RemovalFocus =
  | { readonly kind: "thread"; readonly threadId: string; readonly workspaceId: string }
  | { readonly kind: "addThread"; readonly workspaceId: string }
  | { readonly kind: "workspace"; readonly workspaceId: string }
  | { readonly kind: "newWorkspace" };

/** The first of `before` missing from `after`'s neighbour: the next one still there, else the previous one; `null`
 * when none is left, `undefined` when nothing went. */
function neighbour(before: readonly string[], after: readonly string[]): string | null | undefined {
  const kept = new Set(after);
  const gone = before.findIndex((id) => !kept.has(id));
  if (gone === -1) {
    return undefined;
  }
  return (
    before.slice(gone + 1).find((id) => kept.has(id)) ?? before.slice(0, gone).findLast((id) => kept.has(id)) ?? null
  );
}

/**
 * Where the focus goes when the column changes from `before` to `after` under it (UI-285): a workspace that went
 * hands it to the next workspace's head, else the previous one's, else "+ workspace"; a thread that left a list
 * (archived, deleted) to the next row of that list, else the previous one, else its workspace's "+ thread".
 */
export function focusAfterRemoval(before: readonly Section[], after: readonly Section[]): RemovalFocus | undefined {
  const workspace = neighbour(
    before.map((section) => section.workspace.id),
    after.map((section) => section.workspace.id),
  );
  if (workspace !== undefined) {
    return workspace === null ? { kind: "newWorkspace" } : { kind: "workspace", workspaceId: workspace };
  }
  for (const section of after) {
    const workspaceId = section.workspace.id;
    const previous = before.find((candidate) => candidate.workspace.id === workspaceId);
    if (previous === undefined) {
      continue;
    }
    const active = neighbour(previous.threadIds, section.threadIds);
    const thread = active === undefined ? neighbour(previous.archivedIds, section.archivedIds) : active;
    if (thread !== undefined) {
      return thread === null ? { kind: "addThread", workspaceId } : { kind: "thread", threadId: thread, workspaceId };
    }
  }
  return undefined;
}

/**
 * Every workspace with its threads (UI-020), in the daemon's order (`position`, UI-620; equal ones by name); while
 * filtering, only threads that match and only workspaces with one (UI-022). Threads of an unknown workspace are not
 * listed; the threads of a declared job (`declared`) are listed under its row instead, unless they match a filter
 * (UI-992).
 */
export function sidebarSections(
  threads: readonly ThreadSummary[],
  workspaces: readonly Workspace[],
  filter: ThreadFilter,
  declared: DeclaredJobs = NO_DECLARED_JOBS,
): Section[] {
  const filtering = isFiltering(filter);
  const ordered = [...threads].sort((a, b) => a.position - b.position);
  return [...workspaces]
    .sort(
      (a, b) => a.position - b.position || compare(a.name.toLowerCase(), b.name.toLowerCase()) || compare(a.id, b.id),
    )
    .map((workspace) => {
      const own = ordered.filter((thread) => thread.workspaceId === workspace.id);
      const underJobs = own.filter((thread) => isListedUnderJob(thread, workspace, declared));
      const shown = own.filter((thread) => matchesFilter(thread, workspace, filter));
      return {
        workspace,
        threadIds: shown
          .filter((thread) => !isArchived(thread) && (filtering || !underJobs.includes(thread)))
          .map((thread) => thread.id),
        archivedIds: shown
          .filter(isArchived)
          .sort((a, b) => (b.archivedAt ?? 0) - (a.archivedAt ?? 0) || a.position - b.position || compare(a.id, b.id))
          .map((thread) => thread.id),
        jobThreadIds: underJobs.map((thread) => thread.id),
      };
    })
    .filter((section) => !filtering || section.threadIds.length + section.archivedIds.length > 0);
}

/** The section's threads are hidden: collapsed by the user, unless a filter is typed. */
export function isCollapsed(workspaceId: string, collapsed: readonly string[], filter: ThreadFilter): boolean {
  return !isFiltering(filter) && collapsed.includes(workspaceId);
}

/** `ids` with `id` added, or removed when it was there. */
export function toggled(ids: readonly string[], id: string): readonly string[] {
  return ids.includes(id) ? ids.filter((candidate) => candidate !== id) : [...ids, id];
}

/** A stored list of ids (the collapsed sections, kept across reloads); `undefined` when it is not one. */
export function decodeIds(stored: unknown): readonly string[] | undefined {
  return Array.isArray(stored) && stored.every((entry) => typeof entry === "string") ? stored : undefined;
}

/** Queries shorter than this are not sent to `threads.search` (DMN-076). */
const MIN_SEARCH_LENGTH = 2;
/** Pause after the last keystroke before `threads.search` is called. */
export const SEARCH_DELAY_MS = 200;

/** One `threads.search` hit. */
export interface SearchHit {
  readonly threadId: string;
  readonly snippet: string;
}

/** Runs `run` after `ms` milliseconds; returns the cancel function. */
export type Schedule = (run: () => void, ms: number) => () => void;

/**
 * The conversation part of the filter (UI-022): `threads.search` once typing pauses, reporting only the answer to the
 * latest query (an older answer arriving late is dropped); short queries report no match at once.
 */
export class ConversationSearch {
  readonly #search: (query: string) => Promise<readonly SearchHit[]>;
  readonly #schedule: Schedule;
  readonly #onResults: (query: string, snippets: ReadonlyMap<string, string>) => void;
  #latest = 0;
  #cancel: (() => void) | undefined;

  constructor(
    search: (query: string) => Promise<readonly SearchHit[]>,
    schedule: Schedule,
    onResults: (query: string, snippets: ReadonlyMap<string, string>) => void,
  ) {
    this.#search = search;
    this.#schedule = schedule;
    this.#onResults = onResults;
  }

  /** The query changed. */
  update(query: string): void {
    this.#cancel?.();
    this.#cancel = undefined;
    this.#latest += 1;
    const request = this.#latest;
    const trimmed = query.trim();
    if (trimmed.length < MIN_SEARCH_LENGTH) {
      this.#onResults(query, new Map());
      return;
    }
    const report = (hits: readonly SearchHit[]) => {
      if (request === this.#latest) {
        this.#onResults(query, new Map(hits.map((hit) => [hit.threadId, hit.snippet])));
      }
    };
    this.#cancel = this.#schedule(() => {
      this.#search(trimmed).then(report, () => report([]));
    }, SEARCH_DELAY_MS);
  }

  /** Drops the pending call and any answer still to come. */
  dispose(): void {
    this.#cancel?.();
    this.#cancel = undefined;
    this.#latest += 1;
  }
}

/** A move through the threads: by `delta` threads, or to one end. */
export type Move = number | "top" | "bottom";

/**
 * The thread a move selects in Threads mode, the column shown or not (UI-989, UI-095): one step (`1`, `-1`) is
 * {@link adjacentThread}'s, wrapping around; a longer one goes that many threads and stops at the ends; `top` and
 * `bottom` are the first and last thread. Each counts only the threads {@link adjacentThread} stops on. From a thread
 * the column does not list, the first step is {@link adjacentThread}'s.
 */
export function steppedThread(
  sections: readonly Section[],
  collapsed: readonly string[],
  filter: ThreadFilter,
  current: string | undefined,
  workspaceId: string | undefined,
  move: Move,
): string | undefined {
  if (move === 1 || move === -1) {
    return adjacentThread(sections, collapsed, filter, current, workspaceId, move);
  }
  const selectable = sections.flatMap((section) =>
    isCollapsed(section.workspace.id, collapsed, filter) ? [] : section.threadIds,
  );
  if (move === "top" || move === "bottom") {
    return move === "top" ? selectable[0] : selectable[selectable.length - 1];
  }
  const direction = move > 0 ? 1 : -1;
  let from = current === undefined ? -1 : selectable.indexOf(current);
  if (from < 0) {
    const first = adjacentThread(sections, collapsed, filter, current, workspaceId, direction);
    if (first === undefined) {
      return undefined;
    }
    from = selectable.indexOf(first) - direction;
  }
  return selectable[Math.min(selectable.length - 1, Math.max(0, from + move))];
}

/** A place of the left column for {@link adjacentThread}: a thread row, or the end of a workspace's section. */
interface ColumnPlace {
  readonly id?: string;
  readonly end?: string;
  readonly selectable: boolean;
}

/**
 * The thread `thread.next` (`step` 1) or `thread.prev` (-1) selects from anywhere (UI-989, UI-990): the next or
 * previous one in the left column's order across workspaces, wrapping around at both ends; archived threads, the
 * threads of collapsed sections and, while filtering, those that do not match (the sections leave them out) are
 * skipped. The start is `current` where the column lists it, else the end of `workspaceId`'s section (a draft thread,
 * a thread the filter hides), else the top (next) or the bottom (previous). `current` itself only when it is the
 * only one.
 */
export function adjacentThread(
  sections: readonly Section[],
  collapsed: readonly string[],
  filter: ThreadFilter,
  current: string | undefined,
  workspaceId: string | undefined,
  step: 1 | -1,
): string | undefined {
  // Every place of the column in order, each section's end included (where its draft row and "+ thread" sit).
  const places = sections.flatMap((section): ColumnPlace[] => {
    const open = !isCollapsed(section.workspace.id, collapsed, filter);
    return [
      ...section.threadIds.map((id) => ({ id, selectable: open })),
      ...section.archivedIds.map((id) => ({ id, selectable: false })),
      { end: section.workspace.id, selectable: false },
    ];
  });
  const listed = current === undefined ? -1 : places.findIndex((place) => place.id === current);
  const end = places.findIndex((place) => place.end !== undefined && place.end === workspaceId);
  const start = listed >= 0 ? listed : end >= 0 ? end : step > 0 ? -1 : places.length;
  for (let offset = 1; offset <= places.length; offset += 1) {
    const place = places[(((start + step * offset) % places.length) + places.length) % places.length];
    if (place?.selectable === true && place.id !== undefined) {
      return place.id;
    }
  }
  return undefined;
}

/**
 * The next thread awaiting input after `current` in index order, wrapping around; `current` itself only when it is the
 * only one (UI-021).
 */
export function nextNeedingAttention(
  threads: readonly ThreadSummary[],
  current: string | undefined,
): string | undefined {
  const candidates = threads.filter((thread) => !isArchived(thread));
  const position = current === undefined ? -1 : candidates.findIndex((thread) => thread.id === current);
  const start = position + 1;
  for (let offset = 0; offset < candidates.length; offset += 1) {
    const thread = candidates[(start + offset) % candidates.length];
    if (thread?.needsAttention === true) {
      return thread.id;
    }
  }
  return undefined;
}

/** Non-archived threads awaiting input. */
export function waitingCount(threads: readonly ThreadSummary[]): number {
  return threads.filter((thread) => !isArchived(thread) && thread.needsAttention).length;
}

/** `updatedAt` of each thread when this window last showed it (UI-020). */
export type Seen = ReadonlyMap<string, number>;

/**
 * A thread has news the user has not seen: it changed after the last time it was shown (UI-020). The selected thread
 * is never unread; a thread the window has never observed has no baseline yet and is not unread.
 */
export function isUnread(thread: ThreadSummary, seen: Seen, selected: string | undefined): boolean {
  const baseline = seen.get(thread.id);
  return selected !== thread.id && baseline !== undefined && thread.updatedAt > baseline;
}

/**
 * `seen` after the index changed: threads observed for the first time get their current `updatedAt` as baseline, and
 * the selected thread is always seen. The same object when nothing changes, so callers can skip the write.
 */
export function updateSeen(seen: Seen, threads: readonly ThreadSummary[], selected: string | undefined): Seen {
  const updates = threads.filter((thread) => {
    const baseline = seen.get(thread.id);
    return baseline === undefined || (thread.id === selected && baseline < thread.updatedAt);
  });
  if (updates.length === 0) {
    return seen;
  }
  const next = new Map(seen);
  for (const thread of updates) {
    next.set(thread.id, thread.updatedAt);
  }
  return next;
}

export function isArchived(thread: ThreadSummary): boolean {
  return thread.archivedAt !== undefined && thread.archivedAt !== null;
}

function compare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Where a forked thread comes from, as its header and row show it (UI-664, UI-665). */
export interface ForkLineage {
  readonly sourceId: string;
  /** The source's title now, else the one it had when it was forked (a deleted source). */
  readonly sourceTitle: string;
  /** The source still exists: its title selects it. */
  readonly sourceExists: boolean;
  /** The first line of the message the fork ends at; `undefined` for a copy of the whole session. */
  readonly message: string | undefined;
  /** `Forked from <title> at “<message>”`. */
  readonly label: string;
}

/** The lineage of `thread`, `undefined` when it is no fork (PRT-181). */
export function forkLineage(
  thread: Pick<ThreadSummary, "forkedFrom">,
  threads: ReadonlyMap<string, Pick<ThreadSummary, "title">>,
): ForkLineage | undefined {
  const origin = thread.forkedFrom;
  if (origin === undefined || origin === null) {
    return undefined;
  }
  const source = threads.get(origin.threadId);
  const sourceTitle = source === undefined ? displayTitle(origin) : displayTitle(source);
  const message = origin.message ?? undefined;
  return {
    sourceId: origin.threadId,
    sourceTitle,
    sourceExists: source !== undefined,
    message,
    label: strings.shell.fork.lineage(sourceTitle, message),
  };
}
