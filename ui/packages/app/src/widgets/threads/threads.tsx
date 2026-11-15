/**
 * The left column the frame renders outside any view (UI-020..023, UI-026, UI-100, UI-169, UI-170, UI-240): the
 * brand, the full-text filter (`/`), the "+ workspace" field, the "spawned by" filter in force, then one section per
 * workspace in the daemon's order (UI-620) — `▾`/`▸`, name (the current one in
 * gradient text), path, count, a menu with rename and remove, a delete button, its threads (state mark, title, time;
 * unread titles bold), its draft thread, a collapsed "Archived (N)" row and "+ thread". Sections collapse (`za`); the
 * collapsed set is window-local and survives reloads. Threads and sections are dragged (and moved by keys) into
 * another place, a thread into another workspace (UI-621..UI-627, `reorder.ts`). Its
 * `threads` scope answers `nav.*` (j/k, gg/G, Ctrl-d/u, UI-095), which are also the global fallback, `dd` and `za` on
 * the focused row or section; it registers `nav.search` (`/`) and `workspace.new` (`Space w`).
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
  useMediaQuery,
  WorkspaceSection,
} from "@omfp/kit";
import { INDEX_TOPIC, type ThreadSummary, type Workspace } from "@omfp/protocol";
import { type ReactNode, type RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  DELETE_WORKSPACE,
  DRAFT_THREAD,
  SELECTED_THREAD,
  threadActions,
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
  displayTitle,
  focusAfterRemoval,
  isCollapsed,
  isFiltering,
  isUnread,
  type Move,
  moveSelection,
  type Section,
  type Seen,
  sidebarSections,
  spawnedCounts,
  spawnedTree,
  type ThreadFilter,
  toggled,
  updateSeen,
  visibleOrder,
} from "../../vm/threads.ts";
import { ARCHIVED_OPEN, COLLAPSED_WORKSPACES, SEEN, SPAWNED_BY, THREAD_QUERY } from "./keys.ts";
import { type DragStart, type DragState, useMoveCommands, useReorder, useReorderDrag } from "./reorder.ts";
import { SummaryRow } from "./summary-row.tsx";
import { NewWorkspace, RenameWorkspace } from "./workspace-fields.tsx";

const text = strings.shell.threads;

/** Rows moved by Ctrl-d / Ctrl-u. */
const HALF_PAGE = 10;
const NO_SNIPPETS: ReadonlyMap<string, string> = new Map();

export function ThreadsWidget(_props: WidgetProps): ReactNode {
  const threads = useThreadList();
  const workspaces = useWorkspaceList();
  const [selected] = useWindowState(SELECTED_THREAD);
  const [query] = useWindowState(THREAD_QUERY);
  const [collapsed] = useWindowState(COLLAPSED_WORKSPACES);
  const [archivedOpen] = useWindowState(ARCHIVED_OPEN);
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
  const sections = useMemo(() => sidebarSections(threads, workspaces, filter), [threads, workspaces, filter]);
  const order = useMemo(
    () => visibleOrder(sections, collapsed, archivedOpen, filter),
    [sections, collapsed, archivedOpen, filter],
  );
  const root = useRef<HTMLElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const newWorkspace = useRef<HTMLInputElement>(null);
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
  useNavCommands(order, root);
  useSectionCommands();
  useFocusAfterRemoval(sections, root, newWorkspace);
  useCommand({
    id: "nav.search",
    title: strings.shell.commands.search,
    aliases: ["search", "filter"],
    run: () => search.current?.focus(),
  });
  useCommand({
    id: "workspace.new",
    title: strings.shell.commands.newWorkspace,
    aliases: ["workspace"],
    run: () => newWorkspace.current?.focus(),
  });
  useCommand({
    id: "threads.spawnedBy",
    title: strings.shell.commands.spawnedBy,
    aliases: ["spawned"],
    enabled: () => spawnedRoot !== undefined || (selected !== undefined && counts.has(selected)),
    run: () => setSpawnedBy((current) => (current !== undefined || selected === undefined ? undefined : selected)),
  });
  if (workspaces.length === 0) {
    return (
      <nav
        ref={root}
        className="omfp-threads"
        aria-label={text.label}
        aria-busy={!loaded}
        data-testid="sidebar"
        data-omfp-scope="threads"
      >
        <Brand host={location.hostname} />
        <NewWorkspace inputRef={newWorkspace} focus={loaded} />
        {loaded && (
          <p className="omfp-muted" data-testid="no-workspaces-hint">
            {strings.shell.newWorkspace.hint}
          </p>
        )}
      </nav>
    );
  }
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
      <ThreadFilterField search={search} />
      <NewWorkspace inputRef={newWorkspace} focus={false} />
      {spawnedRoot !== undefined && (
        <SpawnedByBar thread={byId.get(spawnedRoot)} clear={() => setSpawnedBy(undefined)} />
      )}
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
      {sections.length === 0 && <HintRow data-testid="no-threads">{text.noMatch}</HintRow>}
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

/** `nav.*` in the `threads` scope and as the global fallback. */
function useNavCommands(order: readonly string[], root: RefObject<HTMLElement | null>): void {
  const host = useHost();
  const latest = useRef(order);
  latest.current = order;
  useEffect(() => {
    const actions = threadActions(host);
    const move = (step: Move) => {
      const target = moveSelection(latest.current, host.window.get(SELECTED_THREAD), step);
      if (target === undefined) {
        return;
      }
      actions.select(target);
      const row = root.current?.querySelector<HTMLElement>(
        `[data-testid="thread-row"][data-thread-id="${CSS.escape(target)}"]`,
      );
      if (root.current?.contains(document.activeElement) === true) {
        row?.focus();
      }
      row?.scrollIntoView?.({ block: "nearest" });
    };
    const removals = NAV_MOVES.flatMap(([id, title, step]) => [
      host.commands.register({ id, title, scope: "threads", run: () => move(step) }),
      host.commands.register({ id, title, run: () => move(step) }),
    ]);
    return () => {
      for (const remove of removals) {
        remove();
      }
    };
  }, [host, root]);
}

const NAV_MOVES: ReadonlyArray<readonly [string, string, Move]> = [
  ["nav.next", strings.shell.commands.next, 1],
  ["nav.prev", strings.shell.commands.prev, -1],
  ["nav.top", strings.shell.commands.top, "top"],
  ["nav.bottom", strings.shell.commands.bottom, "bottom"],
  ["nav.halfDown", strings.shell.commands.halfDown, HALF_PAGE],
  ["nav.halfUp", strings.shell.commands.halfUp, -HALF_PAGE],
];

/** `dd` (delete the focused thread, draft or workspace) and `za` (collapse the focused section), UI-020, UI-022. */
function useSectionCommands(): void {
  const host = useHost();
  const actions = useThreadActions();
  const sectionOf = (element: Element | null) =>
    element?.closest<HTMLElement>('[data-testid="workspace-section"]')?.dataset.workspaceId;
  useCommand({
    id: "nav.delete",
    title: strings.shell.commands.deleteFocused,
    scope: "threads",
    run: () => {
      const element = document.activeElement;
      const threadId = element?.closest<HTMLElement>("[data-thread-id]")?.dataset.threadId;
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
    scope: "threads",
    run: () => {
      const workspaceId = sectionOf(document.activeElement);
      if (workspaceId !== undefined) {
        host.window.set(COLLAPSED_WORKSPACES, (previous) => toggled(previous, workspaceId));
      }
    },
  });
}

/**
 * Keeps the focus in the column when the thread or workspace that had it goes (UI-285): once the element focused
 * last has left the page and the focus fell back to it, {@link focusAfterRemoval} picks the row, head or field that
 * takes the focus (a collapsed section's head when its rows are hidden).
 */
function useFocusAfterRemoval(
  sections: readonly Section[],
  root: RefObject<HTMLElement | null>,
  newWorkspace: RefObject<HTMLInputElement | null>,
): void {
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
      newWorkspace.current?.focus();
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
  }, [sections, root, newWorkspace]);
}

/** A screen without hover whose pointer is a finger: no key hints (UI-418). */
const TOUCH_ONLY = "(hover: none) and (pointer: coarse)";

/** The one filter of the column (UI-022); `/` focuses it (named in the placeholder, except on touch, UI-418). */
function ThreadFilterField({ search }: { search: RefObject<HTMLInputElement | null> }): ReactNode {
  const [query, setQuery] = useWindowState(THREAD_QUERY);
  const touch = useMediaQuery(TOUCH_ONLY);
  return (
    <div className="omfp-threads__filter">
      <SearchField
        label={text.filter}
        hideLabel
        placeholder={touch ? text.filter : text.filterPlaceholder}
        value={query}
        onChange={setQuery}
        inputRef={search}
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

/** A section's threads, its draft thread, its archived threads behind their toggle, then "+ thread" (UI-020, UI-023);
 * its threads drag (UI-621), its archived ones do not. */
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
      {section.archivedIds.length > 0 && (
        <>
          <ActionRow
            kind="archived"
            icon="archive"
            data-testid="archived-toggle"
            aria-expanded={showArchived}
            onPress={() => setArchivedOpen((previous) => toggled(previous, workspace.id))}
          >
            {text.archived(section.archivedIds.length)}
          </ActionRow>
          {showArchived && (
            <RowList data-testid="archived-list">{section.archivedIds.map((id) => row(id, true))}</RowList>
          )}
        </>
      )}
      <ActionRow
        data-testid="add-thread"
        data-workspace-id={workspace.id}
        onPress={() => actions.openDraft(workspace.id)}
      >
        {text.addThread}
      </ActionRow>
    </>
  );
}
