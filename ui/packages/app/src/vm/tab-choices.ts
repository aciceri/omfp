/**
 * The tab a window selected in each `tabs` container (UI-372) as data: per thread (the draft thread's per workspace),
 * per view and per container. Pure functions, unit tested.
 */

import { isObject } from "../sdk/json.ts";
import { VIEW_CHOICE_THREADS } from "./view-choices.ts";

/** The selected tab id of each `tabs` container of a view, by container (`main` for the `main` slot, else the
 * container's layout path). */
export type ContainerTabs = Readonly<Record<string, string>>;

/** The selected tabs of each view, by view id. */
export type ViewTabs = Readonly<Record<string, ContainerTabs>>;

/** The tabs selected with a thread. */
export interface ThreadTabs {
  readonly threadId: string;
  readonly views: ViewTabs;
}

export interface TabChoices {
  /** Oldest first, at most {@link VIEW_CHOICE_THREADS} (as many as keep a view). */
  readonly threads: readonly ThreadTabs[];
  /** The draft thread's (UI-023), by the workspace it is started in. */
  readonly drafts: Readonly<Record<string, ViewTabs>>;
}

export const NO_TAB_CHOICES: TabChoices = { threads: [], drafts: {} };

/** Whose selections they are: a thread's, or the draft thread's of a workspace. */
export type TabOwner =
  | { readonly kind: "thread"; readonly threadId: string }
  | { readonly kind: "draft"; readonly workspaceId: string };

const NO_VIEW_TABS: ViewTabs = {};

function own<T>(record: Readonly<Record<string, T>>, key: string): T | undefined {
  return Object.hasOwn(record, key) ? record[key] : undefined;
}

function viewTabsOf(choices: TabChoices, owner: TabOwner): ViewTabs | undefined {
  return owner.kind === "thread"
    ? choices.threads.find((entry) => entry.threadId === owner.threadId)?.views
    : own(choices.drafts, owner.workspaceId);
}

/** The tab `owner` last selected in `container` of view `viewId`; `undefined` when none (the container's first). */
export function chosenTab(choices: TabChoices, owner: TabOwner, viewId: string, container: string): string | undefined {
  const views = viewTabsOf(choices, owner);
  const containers = views === undefined ? undefined : own(views, viewId);
  return containers === undefined ? undefined : own(containers, container);
}

/** `views` with `tabId` selected in `container` of `viewId` (`undefined`: the container's first). */
function withTab(views: ViewTabs, viewId: string, container: string, tabId: string | undefined): ViewTabs {
  const containers = own(views, viewId) ?? {};
  if (own(containers, container) === tabId) {
    return views;
  }
  const { [container]: _dropped, ...rest } = containers;
  return { ...views, [viewId]: tabId === undefined ? rest : { ...rest, [container]: tabId } };
}

/** `choices` with `owner`'s selections `views`, a thread's as the most recent; the oldest threads over the bound go. */
function withViews(choices: TabChoices, owner: TabOwner, views: ViewTabs): TabChoices {
  if (owner.kind === "draft") {
    return { ...choices, drafts: { ...choices.drafts, [owner.workspaceId]: views } };
  }
  const threads = [
    ...choices.threads.filter((entry) => entry.threadId !== owner.threadId),
    { threadId: owner.threadId, views },
  ].slice(-VIEW_CHOICE_THREADS);
  return { ...choices, threads };
}

/** `choices` after `owner` selected `tabId` in `container` of view `viewId` (`undefined`: back to the first tab); a
 * thread that changed its selection becomes the most recent. */
export function chooseTab(
  choices: TabChoices,
  owner: TabOwner,
  viewId: string,
  container: string,
  tabId: string | undefined,
): TabChoices {
  const views = viewTabsOf(choices, owner) ?? NO_VIEW_TABS;
  const next = withTab(views, viewId, container, tabId);
  return next === views ? choices : withViews(choices, owner, next);
}

/** `views` with every selection `matches` accepts moved to `to` (none: dropped); the same object when none matched. */
function withTabReplaced(views: ViewTabs, matches: (selected: string) => boolean, to: string | undefined): ViewTabs {
  let changed = false;
  const kept = Object.entries(views).map(([viewId, containers]): [string, ContainerTabs] => {
    const entries = Object.entries(containers);
    if (!entries.some(([, selected]) => matches(selected))) {
      return [viewId, containers];
    }
    changed = true;
    const replaced = entries.flatMap(([container, selected]): [string, string][] =>
      !matches(selected) ? [[container, selected]] : to === undefined ? [] : [[container, to]],
    );
    return [viewId, Object.fromEntries(replaced)];
  });
  return changed ? Object.fromEntries(kept) : views;
}

/**
 * `choices` with the selections of the opened tab `from` by the draft thread of `workspaceId` and by the threads
 * `inWorkspace` says are of it moved to the tab `to` (a missing path's tab showing the file it names, UI-117), or,
 * without `to` (the tab closed), dropped: they go back to their container's first tab, and a tab opened again later
 * is not selected by them. A selection is `from`'s when `normalize` makes it `from` (a file's tab id kept with another
 * spelling of its path, UI-119). The same object when none selected `from`.
 */
export function replaceTab(
  choices: TabChoices,
  workspaceId: string,
  inWorkspace: (threadId: string) => boolean,
  from: string,
  to: string | undefined,
  normalize: (tabId: string) => string = (tabId) => tabId,
): TabChoices {
  const matches = (selected: string) => normalize(selected) === from;
  let changed = false;
  const threads = choices.threads.map((entry) => {
    if (!inWorkspace(entry.threadId)) {
      return entry;
    }
    const views = withTabReplaced(entry.views, matches, to);
    if (views === entry.views) {
      return entry;
    }
    changed = true;
    return { threadId: entry.threadId, views };
  });
  const draft = own(choices.drafts, workspaceId);
  const draftViews = draft === undefined ? draft : withTabReplaced(draft, matches, to);
  const drafts =
    draftViews === draft ? choices.drafts : { ...choices.drafts, [workspaceId]: draftViews ?? NO_VIEW_TABS };
  return changed || drafts !== choices.drafts ? { threads: changed ? threads : choices.threads, drafts } : choices;
}

/** `choices` without the draft thread's selections in `workspaceId` (discarded); the same object when it had none. */
export function dropDraft(choices: TabChoices, workspaceId: string): TabChoices {
  if (!Object.hasOwn(choices.drafts, workspaceId)) {
    return choices;
  }
  const { [workspaceId]: _dropped, ...drafts } = choices.drafts;
  return { ...choices, drafts };
}

/** `choices` once the draft thread of `workspaceId` became thread `threadId`: the thread keeps the draft's selections
 * (unless it has its own), and the next draft starts from the first tabs. */
export function adoptDraft(choices: TabChoices, workspaceId: string, threadId: string): TabChoices {
  const draft = own(choices.drafts, workspaceId);
  const dropped = dropDraft(choices, workspaceId);
  if (draft === undefined || choices.threads.some((entry) => entry.threadId === threadId)) {
    return dropped;
  }
  return withViews(dropped, { kind: "thread", threadId }, draft);
}

/** `choices` without the threads and the drafts of workspaces the index no longer lists (deleted); the same object
 * when none. */
export function keepListedTabs(
  choices: TabChoices,
  threads: { has(id: string): boolean },
  workspaces: { has(id: string): boolean },
): TabChoices {
  const threadsKept = choices.threads.filter((entry) => threads.has(entry.threadId));
  const entries = Object.entries(choices.drafts);
  const draftsKept = entries.filter(([id]) => workspaces.has(id));
  if (threadsKept.length === choices.threads.length && draftsKept.length === entries.length) {
    return choices;
  }
  return { threads: threadsKept, drafts: Object.fromEntries(draftsKept) };
}

/** Reads persisted {@link ViewTabs}, skipping malformed entries. */
function decodeViewTabs(stored: unknown): ViewTabs {
  if (!isObject(stored)) {
    return NO_VIEW_TABS;
  }
  return Object.fromEntries(
    Object.entries(stored).flatMap(([viewId, containers]): [string, ContainerTabs][] =>
      isObject(containers)
        ? [
            [
              viewId,
              Object.fromEntries(
                Object.entries(containers).filter((entry): entry is [string, string] => typeof entry[1] === "string"),
              ),
            ],
          ]
        : [],
    ),
  );
}

/** Reads persisted {@link TabChoices}, skipping malformed entries; `undefined` when it is not an object. */
export function decodeTabChoices(stored: unknown): TabChoices | undefined {
  if (!isObject(stored)) {
    return undefined;
  }
  const { threads, drafts } = stored;
  const decodedThreads = (Array.isArray(threads) ? threads : []).flatMap((entry: unknown): ThreadTabs[] => {
    if (!isObject(entry) || typeof entry.threadId !== "string") {
      return [];
    }
    return [{ threadId: entry.threadId, views: decodeViewTabs(entry.views) }];
  });
  const decodedDrafts = isObject(drafts)
    ? Object.fromEntries(Object.entries(drafts).map(([workspaceId, views]) => [workspaceId, decodeViewTabs(views)]))
    : {};
  return { threads: decodedThreads.slice(-VIEW_CHOICE_THREADS), drafts: decodedDrafts };
}
