/**
 * `omfp:transcript`: the selected thread's transcript, virtualized with `@tanstack/react-virtual` (UI-042): only the
 * items around the viewport are in the DOM, heights are measured as they render, and the view follows the bottom
 * while pinned. The transcript handles PageUp/PageDown/Home/End itself (WebKitGTK does not scroll a focused
 * container, S-11) and the transcript-scope `nav.*` commands (UI-095): `j`/`k` page through an item taller than the
 * viewport before moving on (UI-370), `Ctrl-d`/`Ctrl-u` and PageDown/PageUp scroll and keep the focus while the
 * focused item shows (UI-371). Every scroll is instant, so no motion setting changes them. A user message offers
 * "Fork from here" (UI-660), `:fork` forks from the current message (UI-663) and `:copy` copies it as Markdown
 * (UI-790). A message sent on another day than
 * the one before it shows a day separator inside its item, so `j`/`k` never stop on one (UI-750). `/` searches the
 * conversation, visiting the query's occurrences and highlighting them in the rendered rows (UI-730, UI-736,
 * search.ts). While the user scrolls, a minimap of the whole conversation shows beside the scrollbar (UI-860,
 * minimap.tsx): only on the scrolls their input caused (`ScrollIntent`), never on a follow of the bottom, a restore or
 * the jump to the bottom.
 */

import type { ThreadState } from "@omfp/core";
import { Button, Icon, IconButton, SearchField } from "omfp/kit";
import {
  arrivalScroll,
  type DayMark,
  daySeparator,
  farFromBottom,
  halfPageScrollTop,
  highlightRegistry,
  type ItemEdges,
  isScrollUpKey,
  transcriptItems as items,
  keyScrollTop,
  type MessageView,
  type MoveCommand,
  mountScrollTop,
  movedUpByUser,
  type NavCommand,
  navTarget,
  type PinEvent,
  pagerScroll,
  pinReducer,
  type TranscriptRowContent as RowContent,
  refocusAfterScroll,
  transcriptRowContent as rowContent,
  SCOPE_ATTRIBUTE,
  ScrollIntent,
  type ScrollPosition,
  type SearchCursor,
  type SearchMarks,
  scrollPosition,
  searchMatchingBlocks,
  searchOccurrenceIndex,
  searchPattern,
  searchRevealScroll,
  startOfDay,
  stepSearch,
  strings,
  type TranscriptItem,
  type Turn,
  transcriptTurn as turnOf,
  useAnnounce,
  useCommand,
  useConversationActions,
  useReportError,
  useSettings,
  useThreadActions,
  useVirtualizer,
  useWindowState,
} from "omfp/sdk";
import {
  type KeyboardEvent,
  memo,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ImageViewer } from "./image-viewer.tsx";
import { InterruptedTurn } from "./interrupted.tsx";
import { blockCardKey, MessageRow, NoticeRowView, SystemRowView } from "./message.tsx";
import { type MinimapHandle, TranscriptMinimap } from "./minimap.tsx";
import type { WidgetProps } from "./props.ts";
import { collapsedThinkingHints, NO_MARKS, revealInline, textRanges, useSearchHighlights } from "./search.ts";
import { measurementsKey, pinKey, useClock, useExpandCards, useSelectedThread } from "./state.ts";

/** Height assumed for an item before it is measured. */
const ESTIMATED_ITEM_PX = 120;
const estimateSize = () => ESTIMATED_ITEM_PX;
const neverAdjust = () => false;
/** Items rendered beyond each edge of the viewport. */
const OVERSCAN = 6;
/** No search matches for the minimap while the search is closed. */
const NO_MATCHES: readonly number[] = [];

const NAV: readonly { id: NavCommand; title: string }[] = [
  { id: "nav.next", title: strings.chat.nav.next },
  { id: "nav.prev", title: strings.chat.nav.prev },
  { id: "nav.top", title: strings.chat.nav.top },
  { id: "nav.bottom", title: strings.chat.nav.bottom },
  { id: "nav.halfDown", title: strings.chat.nav.halfDown },
  { id: "nav.halfUp", title: strings.chat.nav.halfUp },
];

/** Keys typed into these edit or move within them; the transcript leaves them alone. */
const CONTROLS = 'input, textarea, select, button, [contenteditable="true"], [role="radio"], [role="checkbox"]';

export function TranscriptWidget(_props: WidgetProps) {
  const { threadId, view } = useSelectedThread();
  if (threadId === undefined || view === undefined) {
    return <ThreadPlaceholder threadId={threadId} />;
  }
  return (
    <Transcript
      key={threadId}
      threadId={threadId}
      state={view.state}
      footer={<InterruptedTurn threadId={threadId} />}
    />
  );
}

/** Shown while no thread is selected or its snapshot has not arrived yet. */
export function ThreadPlaceholder({ threadId }: { threadId: string | undefined }) {
  return threadId === undefined ? (
    <div className="omfp-chat__empty" data-testid="no-thread">
      {strings.chat.noThread}
    </div>
  ) : (
    <div className="omfp-chat__empty" data-testid="thread-loading" data-thread-id={threadId}>
      {strings.chat.loading}
    </div>
  );
}

/** Announces when the agent starts and finishes working (UI-092). */
function useSettledAnnouncements(settled: boolean): void {
  const announce = useAnnounce();
  const previous = useRef(settled);
  useEffect(() => {
    if (previous.current !== settled) {
      previous.current = settled;
      announce(settled ? strings.chat.announce.settled : strings.chat.announce.working);
    }
  }, [announce, settled]);
}

/**
 * The start of today in the local time zone; it moves on just after local midnight, so the day separators' `Today` and
 * `Yesterday` stay true in a window left open overnight (UI-750).
 */
function useToday(): number {
  const [today, setToday] = useState(() => startOfDay(Date.now()));
  useEffect(() => {
    const tomorrow = new Date(today);
    tomorrow.setDate(tomorrow.getDate() + 1);
    // A second past midnight: a timer never fires early, but the new day must have begun when it does.
    const timer = setTimeout(() => setToday(startOfDay(Date.now())), tomorrow.getTime() - Date.now() + 1000);
    return () => clearTimeout(timer);
  }, [today]);
  return today;
}

/**
 * One thread's transcript; mount it with `key={threadId}` so a fresh scroll container never shows another thread's
 * position. `footer` (the pending interactions, UI-070) sits after the list inside the same scroll container.
 */
export function Transcript({ threadId, state, footer }: { threadId: string; state: ThreadState; footer?: ReactNode }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [pin, setPin] = useWindowState(pinKey(threadId));
  const [searching, setSearching] = useState(false);
  const [query, setQuery] = useState("");
  const searchInput = useRef<HTMLInputElement>(null);
  useCommand({
    id: "nav.search",
    title: strings.shell.commands.search,
    aliases: ["search"],
    run: () => {
      setSearching(true);
      queueMicrotask(() => searchInput.current?.focus());
    },
  });
  const pinned = useRef(pin.pinned);
  pinned.current = pin.pinned;
  // Updated as soon as an event changes it, not at the next render: a follow in between must not pull an unpinned
  // view back to the bottom.
  const dispatch = useCallback(
    (event: PinEvent) =>
      setPin((previous) => {
        const next = pinReducer(previous, event);
        pinned.current = next.pinned;
        return next;
      }),
    [setPin],
  );
  /** Where the last scroll event or follow left the view; a scroll up from there unpins (UI-042). */
  const lastPosition = useRef<ScrollPosition | undefined>(undefined);
  /** The view is unpinned far enough above the end to offer jumping back to it (UI-044). */
  const [far, setFar] = useState(false);
  // The same list (and so the same item keys) while only the streaming reply changes (UI-191).
  const rows = items(state, useSettings().ui.showXdevNotices === true);
  const pattern = useMemo(() => searchPattern(query), [query]);
  const matches = useMemo(
    () =>
      pattern === undefined
        ? []
        : rows.flatMap((row, index) => {
            const content = rowContent(state, row);
            return content?.type === "message" && searchMatchingBlocks(content.view, pattern).length > 0 ? [index] : [];
          }),
    [pattern, rows, state],
  );
  // Mounting returns to where the user had scrolled up to, else to the bottom. Until the content is tall enough to
  // reach that position (items are measured after they render), the target is re-applied and the clamped scroll
  // events it causes are not stored; any user scrolling ends the restore.
  const restoring = useRef<number | undefined>(mountScrollTop(pin));
  const [measurements, setMeasurements] = useWindowState(measurementsKey(threadId));
  const [initialMeasurements] = useState(() => [...measurements]);
  // A new `getItemKey` makes the virtualizer lay out every item again: keep it while the rows stay the same.
  const getItemKey = useCallback((index: number) => rows[index]?.key ?? index, [rows]);
  const virtualizer = useVirtualizer({
    initialMeasurementsCache: initialMeasurements,
    count: rows.length,
    getScrollElement: () => containerRef.current,
    estimateSize,
    overscan: OVERSCAN,
    getItemKey,
    ...(restoring.current === undefined ? {} : { initialOffset: restoring.current }),
  });
  // Pinned, the bottom is followed anyway: the virtualizer must not shift the scroll position when an item above
  // changes size, so the only scrolls up away from the bottom are the user's.
  virtualizer.shouldAdjustScrollPositionOnItemSizeChange = pin.pinned ? neverAdjust : undefined;
  // Unmounting (a thread switch) keeps the measured sizes for the next mount.
  useEffect(() => () => setMeasurements(virtualizer.measurementsCache), [setMeasurements, virtualizer]);
  const measurementsOf = useCallback(() => virtualizer.measurementsCache, [virtualizer]);
  const minimap = useRef<MinimapHandle>(null);
  const clock = useClock();
  /** Which scroll events the user caused: those show the minimap (UI-860). */
  const [intent] = useState(() => new ScrollIntent());
  const [current, setCurrent] = useState<number | undefined>(undefined);
  const today = useToday();
  useSettledAnnouncements(state.settled);
  const threadActions = useThreadActions();
  const reportError = useReportError();
  const latestState = useRef(state);
  latestState.current = state;
  // One stable function for every row, so a render does not render the rows again (UI-191).
  const forkAt = useCallback(
    (index: number) => {
      const message = latestState.current.transcript[index]?.message;
      if (message !== undefined) {
        void threadActions.forkAtMessage(threadId, message);
      }
    },
    [threadActions, threadId],
  );
  const latestCurrent = useRef(current);
  latestCurrent.current = current;
  /** The row holding the focus, else the current one (`j`/`k`); `undefined` without either. */
  const focusedRow = (): TranscriptItem | undefined => {
    const focused = document.activeElement?.closest<HTMLElement>("[data-index]")?.dataset.index;
    const position = focused === undefined ? latestCurrent.current : Number(focused);
    return position === undefined ? undefined : rows[position];
  };
  useCommand({
    id: "thread.fork",
    title: strings.shell.commands.fork,
    description: strings.shell.commands.forkDescription,
    aliases: ["fork"],
    scope: "transcript",
    run: () => {
      const row = focusedRow();
      const message = row?.kind.type === "message" ? state.transcript[row.kind.index]?.message : undefined;
      if (row?.kind.type === "message" && (message?.role === "user" || message?.role === "assistant")) {
        forkAt(row.kind.index);
      } else {
        reportError(strings.shell.fork.noMessage);
      }
    },
  });
  const conversation = useConversationActions();
  // `:copy` copies the focused message, else the whole conversation (UI-790).
  useCommand({
    id: "thread.copy",
    title: strings.shell.commands.copy,
    description: strings.shell.commands.copyDescription,
    aliases: ["copy"],
    scope: "transcript",
    run: () => {
      const row = focusedRow();
      const content = row === undefined ? undefined : rowContent(state, row);
      void (content?.type === "message" ? conversation.copyMessage(content.view) : conversation.copy(threadId));
    },
  });

  /** Scrolls to the restore target or, while pinned, to the bottom. A pinned view the user moved up since the last
   * scroll or follow unpins instead, even before its scroll event arrives (WebKitGTK sends it at the next frame,
   * after the render a streamed delta caused). */
  const follow = useCallback(
    (container: HTMLElement) => {
      const target = restoring.current;
      if (target !== undefined) {
        container.scrollTop = target;
        if (container.scrollTop >= target - 1) {
          restoring.current = undefined;
        }
      } else if (pinned.current && movedUpByUser(lastPosition.current, container)) {
        dispatch({ type: "userScrollUp" });
      } else if (pinned.current) {
        // Instant: a smooth follow keeps animating while the user scrolls up, and its scroll events near the bottom pin
        // the view again.
        container.scrollTop = container.scrollHeight;
      }
      lastPosition.current = scrollPosition(container);
      setFar(farFromBottom(lastPosition.current));
    },
    [dispatch],
  );
  /** The user scrolls the view: the restore ends and the scroll shows the minimap (UI-860). */
  const userScrolls = () => {
    restoring.current = undefined;
    intent.input(clock.now());
  };
  /**
   * A press on the container itself is on its scrollbar: what it drags or pages is the user's until the press ends,
   * wherever the button is released, or at the first move with no button held (an engine may send no `pointerup` after
   * a scrollbar drag).
   */
  const pressScrollbar = () => {
    intent.hold();
    const done = new AbortController();
    const end = () => {
      done.abort();
      intent.release(clock.now(), true);
    };
    const options = { capture: true, signal: done.signal };
    window.addEventListener("pointerup", end, options);
    window.addEventListener("pointercancel", end, options);
    window.addEventListener("pointermove", (event) => event.buttons === 0 && end(), options);
  };
  /**
   * A touch scrolls the view while it lasts, and one that moved flings on a while. Its moves and its end go to the
   * element it started on, listened to there: once a row leaves the rendered window, they no longer bubble up here.
   */
  const touch = (target: EventTarget) => {
    intent.hold();
    let moved = false;
    const done = new AbortController();
    const end = () => {
      done.abort();
      intent.release(clock.now(), moved);
    };
    const options = { signal: done.signal };
    target.addEventListener(
      "touchmove",
      () => {
        moved = true;
      },
      options,
    );
    target.addEventListener("touchend", end, options);
    target.addEventListener("touchcancel", end, options);
  };
  /** Back to the end, following it again (UI-044): instant, like a follow, at every motion level. */
  const jumpToBottom = () => {
    const container = containerRef.current;
    if (container === null) {
      return;
    }
    restoring.current = undefined;
    dispatch({ type: "pin" });
    container.scrollTop = container.scrollHeight;
    lastPosition.current = scrollPosition(container);
    setFar(false);
    // The button goes away: the keyboard stays on the transcript.
    container.focus({ preventScroll: true });
  };
  // Every render (new items, measured heights, streaming deltas) follows the bottom or the restore target.
  useLayoutEffect(() => {
    if (containerRef.current !== null) {
      follow(containerRef.current);
    }
  });
  // Growth outside this component's renders (the interactions footer) too.
  useEffect(() => {
    const container = containerRef.current;
    const content = contentRef.current;
    if (container === null || content === null || typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(() => follow(container));
    observer.observe(content);
    return () => observer.disconnect();
  }, [follow]);
  // A navigation command moved the current item: focus it, now if it is rendered, else once it is (a far move renders
  // it after its scroll).
  const focusPending = useRef(false);
  useEffect(() => {
    if (!focusPending.current || current === undefined) {
      return;
    }
    const element = containerRef.current?.querySelector<HTMLElement>(`[data-index="${current}"]`);
    if (element !== null && element !== undefined) {
      focusPending.current = false;
      element.focus({ preventScroll: true });
    }
  });
  const focusItem = (index: number) => {
    const element = containerRef.current?.querySelector<HTMLElement>(`[data-index="${index}"]`);
    focusPending.current = element === null || element === undefined;
    element?.focus({ preventScroll: true });
    setCurrent(index);
  };

  /** The rendered item `index`'s edges relative to the viewport's top; `undefined` when it is not in the DOM. */
  const edges = (container: HTMLElement, index: number): ItemEdges | undefined => {
    const element = container.querySelector(`[data-index="${index}"]`);
    if (element === null) {
      return undefined;
    }
    const view = container.getBoundingClientRect().top;
    const rect = element.getBoundingClientRect();
    return { top: rect.top - view, bottom: rect.bottom - view };
  };

  /** Scrolls to `top` for a user's command; a move up unpins (moving down, the scroll event pins at the bottom). */
  const userScrollTo = (container: HTMLElement, top: number) => {
    userScrolls();
    if (top < container.scrollTop) {
      dispatch({ type: "userScrollUp" });
    }
    container.scrollTop = top;
  };

  /** Scrolls to where the user pressed or dragged on the minimap (UI-862). */
  const jumpTo = (top: number) => {
    const container = containerRef.current;
    if (container !== null) {
      userScrollTo(container, top);
    }
  };

  /** After a scroll by a paging key, focuses the item at the viewport's top if the focused one left it (UI-371). */
  const refocus = (container: HTMLElement) => {
    const list = listRef.current;
    if (list === null) {
      return;
    }
    // Item positions are the virtualizer's, from the list's top; the list sits below the content's padding.
    const offset = list.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop;
    const extents = virtualizer.measurementsCache.map((item) => ({
      start: item.start + offset,
      end: item.end + offset,
    }));
    const target = refocusAfterScroll(current, extents, container.scrollTop, container.clientHeight);
    if (target !== undefined) {
      focusItem(target);
    }
  };

  const move = (command: MoveCommand, container: HTMLElement) => {
    // Paging and arrival apply to `j`/`k` from a focused item; with none, the first press focuses the last as before.
    const direction =
      current === undefined ? undefined : command === "nav.next" ? 1 : command === "nav.prev" ? -1 : undefined;
    const view = container.clientHeight;
    const focused = direction === undefined || current === undefined ? undefined : edges(container, current);
    // A focused item taller than the viewport pages before the move (UI-370).
    const step = direction === undefined || focused === undefined ? undefined : pagerScroll(direction, focused, view);
    if (step !== undefined) {
      userScrollTo(container, container.scrollTop + step);
      return;
    }
    const target = navTarget(command, current, rows.length);
    if (target < 0) {
      return;
    }
    const arrival = direction === undefined ? undefined : edges(container, target);
    const scroll =
      direction === undefined || arrival === undefined ? undefined : arrivalScroll(direction, arrival, view);
    userScrolls();
    if (scroll === undefined) {
      dispatch(target === rows.length - 1 ? { type: "pin" } : { type: "userScrollUp" });
      virtualizer.scrollToIndex(target, { align: "auto" });
    } else {
      // Reading on into a long item: the view is not at the end, the growth of a streaming reply must not pull it.
      dispatch({ type: "userScrollUp" });
      container.scrollTop += scroll;
    }
    focusItem(target);
  };

  const navigate = (command: NavCommand) => {
    const container = containerRef.current;
    if (container === null) {
      return;
    }
    if (command === "nav.halfDown" || command === "nav.halfUp") {
      const direction = command === "nav.halfDown" ? 1 : -1;
      userScrollTo(
        container,
        halfPageScrollTop(direction, container.scrollTop, container.clientHeight, container.scrollHeight),
      );
      refocus(container);
      return;
    }
    move(command, container);
  };
  // The registrations get one stable function that runs the latest `navigate`, so a render does not render them.
  const latestNavigate = useRef(navigate);
  latestNavigate.current = navigate;
  const runNav = useCallback((command: NavCommand) => latestNavigate.current(command), []);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const container = event.currentTarget;
    const target = event.target;
    if (
      !fromInside(event) ||
      (target instanceof Element && target !== container && target.closest(CONTROLS) !== null)
    ) {
      return;
    }
    const top = keyScrollTop(event.key, container.scrollTop, container.clientHeight, container.scrollHeight);
    if (isScrollUpKey(event.key) || top !== undefined || event.key === "ArrowDown" || event.key === " ") {
      userScrolls();
    }
    if (isScrollUpKey(event.key)) {
      dispatch({ type: "userScrollUp" });
    }
    if (top !== undefined) {
      event.preventDefault();
      if (event.key === "End") {
        dispatch({ type: "pin" });
      }
      container.scrollTop = top;
      if (event.key === "PageUp" || event.key === "PageDown") {
        refocus(container);
      }
    }
  };

  // Conversation search (UI-730, UI-736): where it is, how many occurrences each rendered matching row showed, and
  // whether the next paint brings the current occurrence into view.
  const [cursor, setCursor] = useState<SearchCursor | undefined>(undefined);
  const renderedOccurrences = useRef(new Map<number, number>());
  const revealPending = useRef(false);
  const expandCards = useExpandCards();
  const overlayRef = useRef<HTMLDivElement>(null);
  const visitMatch = (target: SearchCursor | undefined) => {
    if (target === undefined) return;
    userScrolls();
    dispatch({ type: "userScrollUp" });
    // A match in a collapsed thinking block shows: the message's matching thinking blocks expand.
    const row = rows[target.row];
    const content = row === undefined ? undefined : rowContent(state, row);
    if (row !== undefined && content?.type === "message" && pattern !== undefined) {
      const { view } = content;
      expandCards(
        searchMatchingBlocks(view, pattern).flatMap((index) => {
          const block = view.blocks[index];
          return block?.type === "thinking" ? [blockCardKey(`${threadId}/${row.key}`, block, index)] : [];
        }),
      );
    }
    revealPending.current = true;
    // A row not rendered yet is scrolled to first; the paint once it renders shows the occurrence.
    if (containerRef.current?.querySelector(`[data-index="${target.row}"]`) == null) {
      virtualizer.scrollToIndex(target.row, { align: "start" });
    }
    setCursor(target);
    setCurrent(target.row);
  };
  const firstMatch = matches[0];
  const visitLatest = useRef(visitMatch);
  visitLatest.current = visitMatch;
  // A new query, or a first match elsewhere, starts again at the first occurrence (expanding its thinking blocks).
  useEffect(() => {
    if (searching) {
      visitLatest.current(
        firstMatch === undefined || pattern === undefined ? undefined : { row: firstMatch, occurrence: 0 },
      );
    }
  }, [firstMatch, pattern, searching]);
  const searchAt = cursor === undefined ? -1 : matches.indexOf(cursor.row);
  /** The occurrences in the rendered matching rows, the current one apart; scrolls it into view after a visit. */
  const searchMarks = (): SearchMarks => {
    const list = listRef.current;
    const container = containerRef.current;
    if (pattern === undefined || list === null || container === null) {
      return NO_MARKS;
    }
    const matching = new Set(matches);
    const found: Range[] = [];
    let mark: Range | undefined;
    let markItem: Element | undefined;
    for (const item of Array.from(list.children)) {
      const index = Number(item.getAttribute("data-index") ?? Number.NaN);
      const row = matching.has(index) ? rows[index] : undefined;
      const content = row === undefined ? undefined : rowContent(state, row);
      if (content?.type !== "message") {
        continue;
      }
      const ranges = textRanges(item, pattern);
      renderedOccurrences.current.set(index, ranges.length);
      found.push(...collapsedThinkingHints(item, content.view, pattern));
      if (cursor?.row === index) {
        markItem = item;
        mark = ranges[searchOccurrenceIndex(cursor.occurrence, ranges.length)];
      }
      found.push(...ranges.filter((range) => range !== mark));
    }
    if (revealPending.current && markItem !== undefined) {
      revealPending.current = false;
      if (mark !== undefined) {
        revealInline(mark, markItem);
      }
      const top = container.getBoundingClientRect().top;
      const edges = (rect: DOMRect): ItemEdges => ({ top: rect.top - top, bottom: rect.bottom - top });
      const scroll = searchRevealScroll(
        edges(markItem.getBoundingClientRect()),
        mark === undefined ? undefined : edges(mark.getBoundingClientRect()),
        container.clientHeight,
      );
      if (scroll !== undefined) {
        container.scrollTop += scroll;
      }
    }
    return { matches: found, current: mark };
  };
  const highlighting = searching && pattern !== undefined;
  const repaint = useSearchHighlights(listRef, overlayRef, highlighting, searchMarks);
  // A render may change the cursor or the matches without changing the rows' DOM.
  useLayoutEffect(() => {
    repaint();
  });
  const closeSearch = () => {
    setSearching(false);
    revealPending.current = false;
  };

  return (
    <ImageViewer fallback={containerRef}>
      {searching && (
        <div className="omfp-transcript__search" data-testid="conversation-search">
          <SearchField
            label={strings.chat.search}
            value={query}
            onChange={setQuery}
            inputRef={searchInput}
            inputProps={{
              "data-testid": "conversation-search-input",
              onKeyDown: (event) => {
                if (event.key === "Escape") {
                  closeSearch();
                  if (current !== undefined) focusItem(current);
                  else containerRef.current?.focus();
                } else if (event.key === "Enter") {
                  visitMatch(
                    stepSearch(
                      cursor,
                      event.shiftKey ? -1 : 1,
                      matches,
                      cursor === undefined ? undefined : renderedOccurrences.current.get(cursor.row),
                    ),
                  );
                } else return;
                event.preventDefault();
                event.stopPropagation();
              },
            }}
          />
          <span role="status">
            {searchAt < 0 ? 0 : searchAt + 1} / {matches.length}
          </span>
          <Button
            onPress={() => {
              closeSearch();
              containerRef.current?.focus();
            }}
          >
            {strings.chat.closeSearch}
          </Button>
        </div>
      )}
      <div className="omfp-transcript-frame">
        <div
          ref={containerRef}
          className="omfp-transcript omfp-focus-inset"
          data-testid="transcript"
          data-thread-id={threadId}
          data-pinned={pin.pinned}
          role="log"
          aria-label={strings.chat.transcript}
          // biome-ignore lint/a11y/noNoninteractiveTabindex: the scroll container takes focus for keyboard scrolling (UI-092)
          tabIndex={0}
          {...{ [SCOPE_ATTRIBUTE]: "transcript" }}
          onScroll={(event) => {
            // Scroll events while restoring come from clamping, not from the user.
            if (restoring.current !== undefined) {
              return;
            }
            const now = scrollPosition(event.currentTarget);
            const previous = lastPosition.current;
            lastPosition.current = now;
            dispatch({ type: "scroll", ...now, previous });
            setFar(farFromBottom(now));
            // Only a scroll the user caused shows the minimap; a follow, a restore or the jump to the bottom only moves
            // its box.
            minimap.current?.scrolled(now, intent.byUser(clock.now()));
          }}
          onWheel={(event) => {
            // Ctrl with the wheel zooms the page and scrolls nothing (UI-283): the pin and the restore stay.
            if (!fromInside(event) || event.ctrlKey) {
              return;
            }
            userScrolls();
            if (event.deltaY < 0) {
              dispatch({ type: "userScrollUp" });
            }
          }}
          onKeyDown={onKeyDown}
          onPointerDown={(event) => {
            if (fromInside(event)) {
              restoring.current = undefined;
              if (event.target === event.currentTarget) {
                pressScrollbar();
              }
            }
          }}
          onTouchStart={(event) => {
            if (fromInside(event)) {
              restoring.current = undefined;
              touch(event.target);
            }
          }}
        >
          {NAV.map(({ id, title }) => (
            <NavRegistration key={id} id={id} title={title} run={runNav} />
          ))}
          <TranscriptMinimap
            ref={minimap}
            container={containerRef}
            list={listRef}
            measurements={measurementsOf}
            rows={rows}
            state={state}
            matches={searching ? matches : NO_MATCHES}
            current={current}
            onJump={jumpTo}
          />
          <div ref={contentRef} className="omfp-transcript__content">
            <div
              ref={listRef}
              className="omfp-transcript__list"
              style={{ height: virtualizer.getTotalSize(), position: "relative" }}
            >
              {virtualizer.getVirtualItems().map((virtual) => {
                const row = rows[virtual.index];
                const content = row === undefined ? undefined : rowContent(state, row);
                return row === undefined || content === undefined ? null : (
                  <div
                    key={virtual.key}
                    ref={virtualizer.measureElement}
                    data-index={virtual.index}
                    data-key={row.key}
                    data-testid="transcript-item"
                    aria-current={current === virtual.index ? true : undefined}
                    tabIndex={-1}
                    className="omfp-transcript__item omfp-focus-item"
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      width: "100%",
                      transform: `translateY(${virtual.start}px)`,
                    }}
                  >
                    <Row
                      threadId={threadId}
                      cardKey={`${threadId}/${row.key}`}
                      content={content}
                      forkIndex={forkIndex(state, row)}
                      turn={turnOf(state, row)}
                      onFork={forkAt}
                      {...(row.kind.type === "message" && row.kind.day !== undefined
                        ? { day: row.kind.day, today }
                        : {})}
                    />
                  </div>
                );
              })}
              {highlighting && highlightRegistry() === undefined && (
                <div
                  ref={overlayRef}
                  className="omfp-transcript__marks"
                  data-testid="search-marks"
                  aria-hidden="true"
                />
              )}
            </div>
            {footer}
          </div>
        </div>
        {!pin.pinned && far && (
          <IconButton
            className="omfp-transcript__jump"
            variant="secondary"
            size="md"
            label={strings.chat.jumpToBottom}
            icon={<Icon name="arrow-down" />}
            onPress={jumpToBottom}
            data-testid="jump-to-bottom"
          />
        )}
      </div>
    </ImageViewer>
  );
}

/**
 * Whether a transcript event came from its own DOM, not from a portal a row renders (an image's dialog, a menu), whose
 * events React still bubbles through the row: those never scroll the transcript.
 */
function fromInside(event: { currentTarget: EventTarget; target: EventTarget }): boolean {
  return (
    event.currentTarget instanceof Node && event.target instanceof Node && event.currentTarget.contains(event.target)
  );
}

/** Registers one transcript-scope navigation command while mounted. */
const NavRegistration = memo(function NavRegistration({
  id,
  title,
  run,
}: {
  id: NavCommand;
  title: string;
  run: (command: NavCommand) => void;
}) {
  useCommand({ id, title, scope: "transcript", run: () => run(id) });
  return null;
});

/** The transcript index of a user message's row, which offers "Fork from here" (UI-660); `undefined` for others. */
function forkIndex(state: ThreadState, row: TranscriptItem): number | undefined {
  return row.kind.type === "message" && state.transcript[row.kind.index]?.message.role === "user"
    ? row.kind.index
    : undefined;
}

/** The first line of a message's text, what its fork button names. */
function firstLine(view: MessageView): string {
  const block = view.blocks.find((candidate) => candidate.type === "text");
  const line = block?.type === "text" ? (block.text.trim().split("\n", 1)[0] ?? "") : "";
  return line.length > 80 ? `${line.slice(0, 79)}…` : line;
}

/** One row; renders again only when its content changes (a streaming delta changes the streaming row's only) or it
 * moves in its thread's conversation (`turn`, UI-922). */
const Row = memo(function Row({
  threadId,
  cardKey,
  content,
  forkIndex,
  turn,
  onFork,
  day,
  today,
}: {
  threadId: string;
  cardKey: string;
  content: RowContent;
  forkIndex: number | undefined;
  turn: Turn;
  onFork: (index: number) => void;
  /** The message's day mark, with the start of today to label it (UI-750). */
  day?: DayMark;
  today?: number;
}) {
  const separator = day === undefined || today === undefined ? undefined : daySeparator(day, today);
  const body = rowBody(threadId, cardKey, content, forkIndex, turn, onFork);
  return separator === undefined ? (
    body
  ) : (
    <>
      {/* biome-ignore lint/a11y/useSemanticElements: an `<hr>` cannot hold the visible day label (UI-750) */}
      <div className="omfp-day" role="separator" aria-label={separator.label} data-testid="day-separator">
        <time dateTime={separator.date}>{separator.label}</time>
      </div>
      {body}
    </>
  );
});

function rowBody(
  threadId: string,
  cardKey: string,
  content: RowContent,
  forkIndex: number | undefined,
  turn: Turn,
  onFork: (index: number) => void,
): ReactNode {
  switch (content.type) {
    case "message": {
      const actions =
        forkIndex === undefined ? undefined : (
          <Button
            size="sm"
            variant="quiet"
            icon={<Icon name="git-fork" />}
            aria-label={strings.shell.fork.actionLabel(firstLine(content.view))}
            data-testid="message-fork"
            onPress={() => onFork(forkIndex)}
          >
            {strings.shell.fork.action}
          </Button>
        );
      return (
        <MessageRow
          threadId={threadId}
          cardKey={cardKey}
          view={content.view}
          streaming={content.streaming}
          continues={content.continues}
          turn={turn}
          {...(actions === undefined ? {} : { actions })}
        />
      );
    }
    case "system":
      return <SystemRowView cardKey={cardKey} row={content.row} />;
    case "notice":
      return <NoticeRowView threadId={threadId} cardKey={cardKey} row={content.row} />;
  }
}
