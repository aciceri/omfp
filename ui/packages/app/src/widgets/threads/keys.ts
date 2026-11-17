/** The left column's window-local state (UI-020, UI-022, UI-026, UI-240, UI-740). */
import { windowKey } from "../../sdk/index.ts";
import { decodeIds, type Seen } from "../../vm/threads.ts";

/** The filter query of this window. */
export const THREAD_QUERY = windowKey<string>("threads/query", "");
/** Workspace ids whose section is collapsed, kept across reloads (UI-022). */
export const COLLAPSED_WORKSPACES = windowKey<readonly string[]>("threads/collapsed", [], decodeIds);
/** Workspace ids whose archived threads are listed (UI-026); collapsed by default. */
export const ARCHIVED_OPEN = windowKey<readonly string[]>("threads/archivedOpen", []);
/** Workspace ids whose declared jobs are listed under `Jobs (N)` (UI-740); collapsed by default. */
export const JOBS_OPEN = windowKey<readonly string[]>("threads/jobsOpen", []);
/** `updatedAt` of each thread when this window last showed it (UI-020). */
export const SEEN = windowKey<Seen>("threads/seen", new Map());
/** The thread whose spawned tree the column shows (UI-240); `undefined` shows every thread. */
export const SPAWNED_BY = windowKey<string | undefined>("threads/spawnedBy", undefined);
