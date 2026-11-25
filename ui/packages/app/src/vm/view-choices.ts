/**
 * The views a window chose (UI-302, UI-367) as data: per thread the view last shown with it, and the view chosen on
 * the draft thread, which the thread created from it keeps. Pure functions, unit tested.
 */

/** The view last shown with a thread. */
export interface ThreadViewChoice {
  readonly threadId: string;
  readonly viewId: string;
}

export interface ViewChoices {
  /** Oldest first, at most {@link VIEW_CHOICE_THREADS}. */
  readonly threads: readonly ThreadViewChoice[];
  /** The view chosen on the draft thread since it opened; none: the workspace's default view. */
  readonly draft?: string;
}

export const NO_VIEW_CHOICES: ViewChoices = { threads: [] };

/** How many threads keep their view; the least recently chosen goes first. */
export const VIEW_CHOICE_THREADS = 200;

/**
 * The view to show among `viewIds` (the current workspace's): the thread's (none: the draft thread's), else
 * `undefined` (the workspace's default view). A choice the workspace no longer has is skipped, not forgotten.
 */
export function activeView(
  choices: ViewChoices,
  threadId: string | undefined,
  viewIds: readonly string[],
): string | undefined {
  const own =
    threadId === undefined ? choices.draft : choices.threads.find((choice) => choice.threadId === threadId)?.viewId;
  return own !== undefined && viewIds.includes(own) ? own : undefined;
}

/** `threads` with `threadId` on `viewId`, as the most recent choice; the oldest over the bound go. */
function withThread(
  threads: readonly ThreadViewChoice[],
  threadId: string,
  viewId: string,
): readonly ThreadViewChoice[] {
  const last = threads.at(-1);
  if (last?.threadId === threadId && last.viewId === viewId) {
    return threads;
  }
  return [...threads.filter((choice) => choice.threadId !== threadId), { threadId, viewId }].slice(
    -VIEW_CHOICE_THREADS,
  );
}

/** `choices` after switching to `viewId` with `threadId` (none: the draft thread). */
export function chooseView(choices: ViewChoices, threadId: string | undefined, viewId: string): ViewChoices {
  if (threadId === undefined) {
    return choices.draft === viewId ? choices : { ...choices, draft: viewId };
  }
  const threads = withThread(choices.threads, threadId, viewId);
  return threads === choices.threads ? choices : { ...choices, threads };
}

/** `choices` without the draft thread's view: a draft opens on the workspace's default view. */
export function dropDraftView(choices: ViewChoices): ViewChoices {
  return choices.draft === undefined ? choices : { threads: choices.threads };
}

/** `choices` once `threadId` is created from a draft thread on `draftView` (none: the default view): it keeps it. */
export function adoptDraftView(choices: ViewChoices, threadId: string, draftView: string | undefined): ViewChoices {
  return draftView === undefined ? choices : chooseView(choices, threadId, draftView);
}

/** `choices` without the threads the index no longer lists (deleted); the same object when none. */
export function keepListed(choices: ViewChoices, threads: { has(id: string): boolean }): ViewChoices {
  const threadsKept = choices.threads.filter((choice) => threads.has(choice.threadId));
  return threadsKept.length === choices.threads.length ? choices : { ...choices, threads: threadsKept };
}

/** Reads persisted {@link ViewChoices}, skipping malformed entries; `undefined` when it is not an object. */
export function decodeViewChoices(stored: unknown): ViewChoices | undefined {
  if (typeof stored !== "object" || stored === null || Array.isArray(stored)) {
    return undefined;
  }
  const { threads, draft } = stored as { threads?: unknown; draft?: unknown };
  const decodedThreads = (Array.isArray(threads) ? threads : []).flatMap((entry: unknown): ThreadViewChoice[] => {
    if (typeof entry !== "object" || entry === null) {
      return [];
    }
    const { threadId, viewId } = entry as { threadId?: unknown; viewId?: unknown };
    return typeof threadId === "string" && typeof viewId === "string" ? [{ threadId, viewId }] : [];
  });
  const kept = { threads: decodedThreads.slice(-VIEW_CHOICE_THREADS) };
  return typeof draft === "string" ? { ...kept, draft } : kept;
}
