/**
 * The left column the frame renders outside any view (UI-020..023, UI-026, UI-100, UI-169, UI-170, UI-240): the
 * brand, the full-text filter (`/`), the "spawned by" filter in force, then one section per
 * workspace in the daemon's order (UI-620) — `▾`/`▸`, name (the current one in gradient text), path as its tooltip,
 * the thread count flush right, which a menu with rename and remove and a delete button replace on hover and focus,
 * its threads (state mark, title, time; unread titles bold) but those listed under their job (UI-992), its draft
 * thread, "+ thread", then the collapsed "Jobs (N)" (UI-740, `job-rows.tsx`) and "Archived (N)" rows, a light rule
 * between sections, and "+ workspace" below the last one, a row turning into its field (`workspace-fields.tsx`). Sections
 * collapse (`za`); the collapsed set is window-local and survives reloads. Threads and sections are dragged (and moved
 * by keys) into another place, a thread into another workspace (UI-621..UI-627, `reorder.ts`). It registers Threads
 * mode's moves (UI-095), which work with the column shown, collapsed or hidden: `thread.next` / `thread.prev` (`j` /
 * `k`, and `Alt-j` / `Alt-k` from any mode, UI-989..UI-991), `thread.first` / `thread.last` (`gg` / `G`) and
 * `thread.halfDown` / `thread.halfUp` (`Ctrl-d` / `Ctrl-u`) following its order; `dd` and `za` on the focused row or
 * section, else on the selected thread and its workspace; `workspace.new` (`Space w`).
 */

import {
  ActionRow,
  Brand,
  HintRow,
  Icon,
  IconButton,
  Menu,
  MenuItem,
  MenuTrigger,
  Popover,
  RowList,
  SearchField,
  ThreadRow,
  ToggleRow,
  WorkspaceSection,
} from "@omfp/kit";
import { INDEX_TOPIC, type ThreadSummary, type Workspace } from "@omfp/protocol";
import { type ReactNode, type RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { modeAt } from "../../keymap/handler.ts";
import {
  currentWorkspaceId,
  DELETE_WORKSPACE,
  DRAFT_THREAD,
  FOCUS_COMPOSER,
  SELECTED_THREAD,
  useCall,
  useCommand,
  useCurrentWorkspaceId,
  useHost,
  useSnapshot,
  useThreadActions,
  useThreads,
  useWindowState,
  useWorkspaces,
  type WidgetProps,
} from "../../sdk/index.ts";
import { strings } from "../../strings/index.ts";
import type { ReorderSource } from "../../vm/order.ts";
import {
  ConversationSearch,
  type DeclaredJobs,
  displayTitle,
  focusAfterRemoval,
  isCollapsed,
  isFiltering,
  isUnread,
  type Move,
  type Section,
  type Seen,
  sidebarSections,
  spawnedCounts,
  spawnedTree,
  steppedThread,
  type ThreadFilter,
  toggled,
  updateSeen,
} from "../../vm/threads.ts";
import { WorkspaceJobs } from "./job-rows.tsx";
import { ARCHIVED_OPEN, COLLAPSED_WORKSPACES, SEEN, SPAWNED_BY, THREAD_QUERY } from "./keys.ts";
import { type DragStart, type DragState, useMoveCommands, useReorder, useReorderDrag } from "./reorder.ts";
import { SummaryRow } from "./summary-row.tsx";
import { AddWorkspace, RenameWorkspace } from "./workspace-fields.tsx";

const text = strings.shell.threads;

/** Threads moved by Ctrl-d / Ctrl-u. */
const HALF_PAGE = 10;
const NO_SNIPPETS: ReadonlyMap<string, string> = new Map();

/** The frame's hook into its column: `reveal` showing a collapsed column before a command focuses inside it
 * (UI-987). */
export function ThreadsWidget({ reveal }: WidgetProps & { readonly reveal?: () => void }): ReactNode {
  const threads = useThreadList();
  const workspaces = useWorkspaceList();
  const [selected] = useWindowState(SELECTED_THREAD);
  const [query] = useWindowState(THREAD_QUERY);
  const [collapsed] = useWindowState(COLLAPSED_WORKSPACES);
  const [spawnedBy, setSpawnedBy] = useWindowState(SPAWNED_BY);
  // The column is `aria-busy` until the index is live (again while it resubscribes after a reconnect).
  const loaded = useSnapshot((snapshot) =>
    snapshot.topics.some((topic) => topic.topic === INDEX_TOPIC && topic.sync.kind === "live"),
  );
  const byId = useMemo(() => new Map(threads.map((thread) => [thread.id, thread])), [threads]);
  const spawnedRoot = spawnedBy !== undefined && byId.has(spawnedBy) ? spawnedBy : undefined;
  const spawned = useMemo(
    () => (spawnedRoot === undefined ? undefined : spawnedTree(threads, spawnedRoot)),
    [threads, spawnedRoot],
  );
  const counts = useMemo(() => spawnedCounts(threads), [threads]);
  // The filter of a deleted thread goes with it.
  useEffect(() => {
    if (loaded && spawnedBy !== undefined && !byId.has(spawnedBy)) {
      setSpawnedBy(undefined);
    }
  }, [loaded, spawnedBy, byId, setSpawnedBy]);
  const conversation = useConversationMatches(query);
  const filter = useMemo<ThreadFilter>(
    () => ({ query, conversation, ...(spawned !== undefined && { spawned }) }),
    [query, conversation, spawned],
  );
  const seen = useSeen(threads, selected);
  const declared = useDeclaredJobs();
  const sections = useMemo(
    () => sidebarSections(threads, workspaces, filter, declared),
    [threads, workspaces, filter, declared],
  );
  const root = useRef<HTMLElement>(null);
  const newWorkspace = useRef<HTMLInputElement>(null);
  const [addingWorkspace, setAddingWorkspace] = useState(false);
  const filtering = isFiltering(filter);
  const nameOf = useCallback(
    (source: ReorderSource) => {
      if (source.kind === "workspace") {
        return workspaces.find((workspace) => workspace.id === source.id)?.name ?? "";
      }
      const thread = byId.get(source.id);
      return thread === undefined ? "" : displayTitle(thread);
    },
    [workspaces, byId],
  );
  // Before the focus after a removal: a thread moved to another workspace keeps the focus (UI-624).
  const reorder = useReorder(sections, nameOf);
  const { drag, start } = useReorderDrag(root, !filtering, reorder);
  useMoveCommands(sections, filtering, reorder);
  useStepCommands(sections, collapsed, filter, root);
  useSectionCommands(root);
  useFocusAfterRemoval(sections, root);
  useCommand({
    id: "workspace.new",
    title: strings.shell.commands.newWorkspace,
    aliases: ["workspace"],
    run: () => {
      reveal?.();
      if (newWorkspace.current === null) {
        setAddingWorkspace(true);
      } else {
        newWorkspace.current.focus();
      }
    },
  });
  useCommand({
    id: "threads.spawnedBy",
    title: strings.shell.commands.spawnedBy,
    aliases: ["spawned"],
    enabled: () => spawnedRoot !== undefined || (selected !== undefined && counts.has(selected)),
    run: () => setSpawnedBy((current) => (current !== undefined || selected === undefined ? undefined : selected)),
  });
  return (
    <nav
      ref={root}
      className="omfp-threads"
      aria-label={text.label}
      aria-busy={!loaded}
      data-testid="sidebar"
      data-omfp-scope="threads"
      data-reordering={drag?.source.kind}
    >
      <Brand host={location.hostname} />
      <ThreadFilterField />
      {spawnedRoot !== undefined && (
        <SpawnedByBar thread={byId.get(spawnedRoot)} clear={() => setSpawnedBy(undefined)} />
      )}
      <div className="omfp-threads__list">
        {sections.map((section, index) => (
          <WorkspaceGroup
            key={section.workspace.id}
            section={section}
            filter={filter}
            byId={byId}
            seen={seen}
            selected={selected}
            counts={counts}
            drag={drag}
            start={start}
            last={index === sections.length - 1}
          />
        ))}
        {workspaces.length > 0 && sections.length === 0 && <HintRow data-testid="no-threads">{text.noMatch}</HintRow>}
        <AddWorkspace open={addingWorkspace} onOpenChange={setAddingWorkspace} inputRef={newWorkspace} />
      </div>
    </nav>
  );
}

function useThreadList(): readonly ThreadSummary[] {
  const threads = useThreads();
  return useMemo(() => [...threads.values()], [threads]);
}

function useWorkspaceList(): readonly Workspace[] {
  const workspaces = useWorkspaces();
  return useMemo(() => [...workspaces.values()], [workspaces]);
}

/** The declared jobs' names of the workspaces whose `jobs/<id>` the window holds (UI-992). */
function useDeclaredJobs(): DeclaredJobs {
  const held = useSnapshot((snapshot) => snapshot.workspaceJobs);
  return useMemo(
    () => new Map([...held].map(([id, jobs]) => [id, new Set(jobs.state.jobs.map((job) => job.name))])),
    [held],
  );
}

/** Unread baselines, kept in step with the index; showing a thread marks it seen (UI-020). */
function useSeen(threads: readonly ThreadSummary[], selected: string | undefined): Seen {
  const [seen, setSeen] = useWindowState(SEEN);
  useEffect(() => {
    setSeen((previous) => updateSeen(previous, threads, selected));
  }, [threads, selected, setSeen]);
  return seen;
}

/** Conversation matches of `query` by `threads.search` (UI-022, DMN-076): debounced, only for the current query. */
function useConversationMatches(query: string): ReadonlyMap<string, string> {
  const call = useCall();
  const [found, setFound] = useState<{ query: string; snippets: ReadonlyMap<string, string> }>({
    query: "",
    snippets: NO_SNIPPETS,
  });
  const search = useMemo(
    () =>
      new ConversationSearch(
        async (wanted) => (await call("threads.search", { query: wanted })).results,
        (run, ms) => {
          const timer = setTimeout(run, ms);
          return () => clearTimeout(timer);
        },
        (answered, snippets) => setFound({ query: answered, snippets }),
      ),
    [call],
  );
  useEffect(() => () => search.dispose(), [search]);
  useEffect(() => search.update(query), [search, query]);
  return found.query === query ? found.snippets : NO_SNIPPETS;
}

/** Threads mode's moves besides `thread.next` / `thread.prev` (UI-095). */
const MOVES: ReadonlyArray<readonly [string, string, Move, readonly string[]]> = [
  ["thread.first", strings.shell.commands.firstThread, "top", ["first-thread"]],
  ["thread.last", strings.shell.commands.lastThread, "bottom", ["last-thread"]],
  ["thread.halfDown", strings.shell.commands.halfThreadsDown, HALF_PAGE, []],
  ["thread.halfUp", strings.shell.commands.halfThreadsUp, -HALF_PAGE, []],
];

/**
 * `thread.next` / `thread.prev` (UI-989..UI-991) and Threads mode's other moves (UI-095), one implementation
 * ({@link steppedThread}): global, so they work from the composer, the transcript and with the column hidden. Pressed
 * in a text field outside the column, the new thread's composer takes the focus; on a row of the column, the new row
 * does; anywhere else the focus stays.
 */
function useStepCommands(
  sections: readonly Section[],
  collapsed: readonly string[],
  filter: ThreadFilter,
  root: RefObject<HTMLElement | null>,
): void {
  const host = useHost();
  const actions = useThreadActions();
  const latest = useRef({ sections, collapsed, filter, actions });
  latest.current = { sections, collapsed, filter, actions };
  useEffect(() => {
    const step = (move: Move) => {
      const current = latest.current;
      const target = steppedThread(
        current.sections,
        current.collapsed,
        current.filter,
        host.window.get(SELECTED_THREAD),
        currentWorkspaceId(host),
        move,
      );
      if (target === undefined) {
        return;
      }
      const focused = document.activeElement;
      const inColumn = root.current?.contains(focused) === true;
      current.actions.select(target);
      const row = root.current?.querySelector<HTMLElement>(
        `[data-testid="thread-row"][data-thread-id="${CSS.escape(target)}"]`,
      );
      if (inColumn && focused?.closest("[data-thread-id]") !== null) {
        row?.focus();
      } else if (!inColumn && modeAt(focused) === "insert") {
        host.window.set(FOCUS_COMPOSER, target);
      }
      row?.scrollIntoView?.({ block: "nearest" });
    };
    const removals = [
      host.commands.register({
        id: "thread.next",
        title: strings.shell.commands.nextThread,
        aliases: ["next-thread"],
        run: () => step(1),
      }),
      host.commands.register({
        id: "thread.prev",
        title: strings.shell.commands.prevThread,
        aliases: ["prev-thread"],
        run: () => step(-1),
      }),
      ...MOVES.map(([id, title, move, aliases]) =>
        host.commands.register({ id, title, ...(aliases.length === 0 ? {} : { aliases }), run: () => step(move) }),
      ),
    ];
    return () => {
      for (const remove of removals) {
        remove();
      }
    };
  }, [host, root]);
}

/**
 * `dd` (delete the focused thread, draft or workspace) and `za` (collapse the focused section), UI-020, UI-022; with
 * the focus outside the column (Threads mode with nothing focused, the column collapsed or hidden, UI-095), the
 * selected thread and the current workspace's section.
 */
function useSectionCommands(root: RefObject<HTMLElement | null>): void {
  const host = useHost();
  const actions = useThreadActions();
  /** The focused element when it is in the column. */
  const inColumn = () => {
    const element = document.activeElement;
    return element !== null && root.current?.contains(element) === true ? element : null;
  };
  const sectionOf = (element: Element | null) =>
    element?.closest<HTMLElement>('[data-testid="workspace-section"]')?.dataset.workspaceId;
  useCommand({
    id: "nav.delete",
    title: strings.shell.commands.deleteFocused,
    run: () => {
      const element = inColumn();
      const threadId =
        element === null
          ? host.window.get(SELECTED_THREAD)
          : element.closest<HTMLElement>("[data-thread-id]")?.dataset.threadId;
      const workspaceId = sectionOf(element);
      if (threadId !== undefined) {
        void actions.remove(threadId);
      } else if (element?.closest('[data-testid="draft-thread"]')) {
        actions.discardDraft();
      } else if (workspaceId !== undefined) {
        host.window.set(DELETE_WORKSPACE, workspaceId);
      }
    },
  });
  useCommand({
    id: "nav.toggleSection",
    title: strings.shell.commands.toggleSection,
    run: () => {
      const element = inColumn();
      const workspaceId = element === null ? currentWorkspaceId(host) : sectionOf(element);
      if (workspaceId === undefined) {
        return;
      }
      host.window.set(COLLAPSED_WORKSPACES, (previous) => toggled(previous, workspaceId));
      element
        ?.closest('[data-testid="workspace-section"]')
        ?.querySelector<HTMLElement>('[data-testid="workspace-collapse"]')
        ?.focus();
    },
  });
}

/**
 * Keeps the focus in the column when the thread or workspace that had it goes (UI-285): once the element focused
 * last has left the page and the focus fell back to it, {@link focusAfterRemoval} picks the row, head or field that
 * takes the focus (a collapsed section's head when its rows are hidden).
 */
function useFocusAfterRemoval(sections: readonly Section[], root: RefObject<HTMLElement | null>): void {
  const last = useRef<Element | null>(null);
  useEffect(() => {
    const track = (event: FocusEvent) => {
      last.current = event.target instanceof Element ? event.target : null;
    };
    document.addEventListener("focusin", track);
    return () => document.removeEventListener("focusin", track);
  }, []);
  const previous = useRef(sections);
  useEffect(() => {
    const before = previous.current;
    previous.current = sections;
    const active = document.activeElement;
    const lost = (active === null || active === document.body) && last.current?.isConnected === false;
    const target = lost ? focusAfterRemoval(before, sections) : undefined;
    if (target === undefined) {
      return;
    }
    if (target.kind === "newWorkspace") {
      root.current?.querySelector<HTMLElement>('[data-testid="add-workspace"]')?.focus();
      return;
    }
    const section = root.current?.querySelector(
      `[data-testid="workspace-section"][data-workspace-id="${CSS.escape(target.workspaceId)}"]`,
    );
    const candidates = [
      target.kind === "thread" && `[data-testid="thread-row"][data-thread-id="${CSS.escape(target.threadId)}"]`,
      target.kind !== "workspace" && '[data-testid="add-thread"]',
      '[data-testid="workspace-collapse"]',
    ];
    for (const selector of candidates) {
      const element = selector === false ? null : section?.querySelector<HTMLElement>(selector);
      if (element !== null && element !== undefined) {
        element.focus();
        return;
      }
    }
  }, [sections, root]);
}

/** The column's persistent filter (UI-022); Space / opens the separate live thread picker. */
function ThreadFilterField(): ReactNode {
  const [query, setQuery] = useWindowState(THREAD_QUERY);
  return (
    <div className="omfp-threads__filter">
      <SearchField
        label={text.filter}
        hideLabel
        placeholder={text.filter}
        value={query}
        onChange={setQuery}
        inputProps={{ "data-testid": "thread-filter" }}
        clearTestId="clear-thread-filter"
      />
    </div>
  );
}

/** The "spawned by" filter in force (UI-240): the thread it names and the clear button. */
function SpawnedByBar({ thread, clear }: { thread: ThreadSummary | undefined; clear: () => void }): ReactNode {
  return (
    <div className="omfp-threads__spawned" data-testid="spawned-by">
      <Icon name="git-fork" />
      <span className="omfp-threads__spawned-title">
        {text.spawnedBy(thread === undefined ? "" : displayTitle(thread))}
      </span>
      <IconButton label={text.clearSpawned} icon={<Icon name="x" />} data-testid="clear-spawned-by" onPress={clear} />
    </div>
  );
}

/** The drag props of the column's sections and rows (UI-621, UI-622, UI-627). */
interface Dragging {
  /** The drag in progress, if any. */
  drag: DragState | undefined;
  /** Starts dragging a thread row or a section head. */
  start: DragStart;
}

/** A workspace of the column: the kit's section (UI-170, UI-442) with its menu, delete, rename and rows; its head
 * drags it (UI-622), and it shows where a dragged section or thread lands (UI-627). */
function WorkspaceGroup({
  section,
  filter,
  byId,
  seen,
  selected,
  counts,
  drag,
  start,
  last,
}: {
  section: Section;
  filter: ThreadFilter;
  byId: ReadonlyMap<string, ThreadSummary>;
  seen: Seen;
  selected: string | undefined;
  /** How many threads each thread spawned (UI-240). */
  counts: ReadonlyMap<string, number>;
  /** The last section: a section dropped after every other one lands under it. */
  last: boolean;
} & Dragging): ReactNode {
  const host = useHost();
  const { workspace } = section;
  const [collapsedIds, setCollapsed] = useWindowState(COLLAPSED_WORKSPACES);
  const [renaming, setRenaming] = useState(false);
  const head = useRef<HTMLButtonElement>(null);
  const closeRename = () => {
    setRenaming(false);
    // The focus stays on the renamed workspace: its head (UI-286).
    head.current?.focus();
  };
  const collapsed = isCollapsed(workspace.id, collapsedIds, filter);
  const current = useCurrentWorkspaceId() === workspace.id;
  const target = drag?.target;
  const placedHere =
    target?.kind !== "workspace"
      ? undefined
      : target.beforeId === workspace.id
        ? "before"
        : target.beforeId === undefined && last
          ? "after"
          : undefined;
  return (
    <WorkspaceSection
      data-testid="workspace-section"
      data-workspace-id={workspace.id}
      data-dragging={drag?.source.kind === "workspace" && drag.source.id === workspace.id ? "true" : undefined}
      data-drop={placedHere}
      data-drop-into={target?.kind === "thread" && target.workspaceId === workspace.id ? "true" : undefined}
      headProps={{ onPointerDown: start({ kind: "workspace", id: workspace.id }) }}
      name={workspace.name}
      path={workspace.path}
      count={section.threadIds.length}
      expanded={!collapsed}
      onExpandedChange={() => setCollapsed((previous) => toggled(previous, workspace.id))}
      toggleLabel={collapsed ? text.expand(workspace.name) : text.collapse(workspace.name)}
      toggleProps={{ ref: head, "data-testid": "workspace-collapse", "data-reorder-handle": true }}
      nameProps={{ "data-testid": "workspace-section-name" }}
      current={current}
      currentLabel={text.current}
      {...(renaming ? { editor: <RenameWorkspace workspace={workspace} close={closeRename} /> } : {})}
      actions={
        <>
          <MenuTrigger>
            <IconButton
              label={text.menu(workspace.name)}
              icon={<Icon name="settings-2" />}
              data-testid="workspace-menu"
            />
            <Popover placement="bottom end">
              <Menu
                aria-label={text.menu(workspace.name)}
                onAction={(key) => {
                  if (key === "rename") {
                    setRenaming(true);
                  } else if (key === "remove") {
                    host.window.set(DELETE_WORKSPACE, workspace.id);
                  }
                }}
              >
                <MenuItem id="rename" data-testid="workspace-rename">
                  {text.rename}
                </MenuItem>
                <MenuItem id="remove" data-testid="workspace-remove">
                  {text.remove}
                </MenuItem>
              </Menu>
            </Popover>
          </MenuTrigger>
          <IconButton
            label={text.deleteWorkspace(workspace.name)}
            icon={<Icon name="trash-2" />}
            data-testid="delete-workspace"
            onPress={() => host.window.set(DELETE_WORKSPACE, workspace.id)}
          />
        </>
      }
    >
      <WorkspaceRows
        section={section}
        filter={filter}
        byId={byId}
        seen={seen}
        selected={selected}
        counts={counts}
        drag={drag}
        start={start}
      />
    </WorkspaceSection>
  );
}

/** A section's threads, its draft thread, "+ thread", then its jobs and its archived threads behind their toggles
 * (UI-020, UI-023, UI-026, UI-740); its threads drag (UI-621), its archived ones do not. */
function WorkspaceRows({
  section,
  filter,
  byId,
  seen,
  selected,
  counts,
  drag,
  start,
}: {
  section: Section;
  filter: ThreadFilter;
  byId: ReadonlyMap<string, ThreadSummary>;
  seen: Seen;
  selected: string | undefined;
  counts: ReadonlyMap<string, number>;
} & Dragging): ReactNode {
  const actions = useThreadActions();
  const { workspace } = section;
  const [archivedOpen, setArchivedOpen] = useWindowState(ARCHIVED_OPEN);
  const [draft] = useWindowState(DRAFT_THREAD);
  const showArchived = archivedOpen.includes(workspace.id);
  const draftHere = draft?.workspaceId === workspace.id && !isFiltering(filter);
  const snippets = isFiltering(filter) ? filter.conversation : NO_SNIPPETS;
  const target = drag?.target?.kind === "thread" && drag.target.workspaceId === workspace.id ? drag.target : undefined;
  const row = (id: string, archived: boolean) => {
    const thread = byId.get(id);
    return thread === undefined ? null : (
      <SummaryRow
        key={id}
        thread={thread}
        selected={selected === id}
        unread={!archived && isUnread(thread, seen, selected)}
        snippet={snippets.get(id)}
        spawned={counts.get(id)}
        archived={archived}
        {...(!archived && {
          reorder: {
            onPointerDown: start({ kind: "thread", id, workspaceId: workspace.id }),
            dragging: drag?.source.kind === "thread" && drag.source.id === id,
            dropBefore: target?.beforeId === id,
          },
        })}
      />
    );
  };
  return (
    <>
      <RowList data-drop={target !== undefined && target.beforeId === undefined ? "end" : undefined}>
        {section.threadIds.map((id) => row(id, false))}
        {draftHere && (
          <ThreadRow
            state="idle"
            title={strings.shell.untitled}
            placeholder
            meta={text.draft}
            selected={selected === undefined}
            onSelect={() => actions.openDraft(workspace.id)}
            selectProps={{ "data-testid": "draft-thread", "data-workspace-id": workspace.id }}
          />
        )}
        {section.threadIds.length === 0 && !draftHere && (
          <li>
            <HintRow>{text.empty}</HintRow>
          </li>
        )}
      </RowList>
      <ActionRow
        data-testid="add-thread"
        data-workspace-id={workspace.id}
        onPress={() => actions.openDraft(workspace.id)}
      >
        {text.addThread}
      </ActionRow>
      {workspace.jobs !== undefined && workspace.jobs !== null && (
        <WorkspaceJobs
          workspaceId={workspace.id}
          counts={workspace.jobs}
          threads={section.jobThreadIds.flatMap((id) => byId.get(id) ?? [])}
        />
      )}
      {section.archivedIds.length > 0 && (
        <>
          <ToggleRow
            expanded={showArchived}
            data-testid="archived-toggle"
            onPress={() => setArchivedOpen((previous) => toggled(previous, workspace.id))}
          >
            {text.archived(section.archivedIds.length)}
          </ToggleRow>
          {showArchived && (
            <RowList data-testid="archived-list">{section.archivedIds.map((id) => row(id, true))}</RowList>
          )}
        </>
      )}
    </>
  );
}
