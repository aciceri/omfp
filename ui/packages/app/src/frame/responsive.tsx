/**
 * The frame on narrow and phone-sized frames (UI-200..UI-205) and the left column's splitter (UI-207): the strip the
 * left column folds into and the drawer it opens, the phone's bottom bar of areas, the `threads.toggle` and
 * `threads.collapse` commands with the buttons that hide the wide column and show it again (UI-985..UI-987), and the
 * visual viewport's height that keeps the composer above a phone's keyboard.
 */

import {
  AreaBar,
  type AreaItem,
  Icon,
  IconButton,
  ModeButton,
  type StripItem,
  ThreadStrip,
  useMediaQuery,
  useWidthClass,
} from "@omfp/kit";
import { type ReactNode, type RefObject, useEffect, useLayoutEffect, useMemo, useState } from "react";
import { flushSync } from "react-dom";
import { THREADS_ATTRIBUTE } from "../keymap/handler.ts";
import { DRAFT_THREAD, SELECTED_THREAD, useCommand, useThreads, useWindowState } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import type { Area } from "../view/layout.ts";
import { LEFT_WIDTH, resizeLeft } from "../view/sizes.ts";
import { Resizer } from "../view/splitter.tsx";
import { threadMark } from "../vm/threads.ts";
import { CommandsSheet } from "./commands-sheet.tsx";
import {
  type Breakpoint,
  breakpointOf,
  COARSE_POINTER,
  LEFT_COLLAPSED,
  LEFT_OPEN,
  MOBILE_AREA,
  SHORT_TOUCH_SCREEN,
  stripThreads,
} from "./responsive.ts";

const text = strings.shell.responsive;

/** `id` of the left column, which the strip's toggle controls. */
export const LEFT_COLUMN_ID = "omfp-left-column";

/** Marks the frame's own controls of the drawer (strip, bottom bar): a press there is not an outside click. */
const DRAWER_CONTROL = "data-omfp-drawer";

/** The root font size in px (`rem`). */
function remPx(): number {
  return Number.parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
}

/** Prefer the selected visible row; a collapsed or empty column still has a keyboard focus target. */
function focusColumn(column: HTMLElement | null): void {
  for (const selector of [
    '[data-testid="thread-row"][aria-current="true"]',
    '[data-testid="thread-row"]',
    '[data-testid="workspace-collapse"]',
    '[data-testid="new-workspace-input"]',
  ]) {
    const target = column?.querySelector<HTMLElement>(selector);
    if (target?.checkVisibility()) {
      target.focus();
      return;
    }
  }
}
/**
 * The left column as a drawer (UI-201, UI-204): it closes when a thread or a draft is selected, on Escape, on a press
 * in the frame outside it (overlays such as menus and dialogs are not in the frame), and when the breakpoint changes.
 * Opening moves the focus to the selected thread (else the first row); closing with the focus inside returns it to the
 * strip's toggle.
 */
export function useLeftDrawer(breakpoint: Breakpoint, aside: RefObject<HTMLElement | null>): void {
  const [open, setOpen] = useWindowState(LEFT_OPEN);
  const [, setArea] = useWindowState(MOBILE_AREA);
  const [selected] = useWindowState(SELECTED_THREAD);
  const [draft] = useWindowState(DRAFT_THREAD);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new selection (or draft) closes the drawer and shows the chat.
  useEffect(() => {
    setOpen(false);
    setArea("chat");
  }, [selected, draft, setOpen, setArea]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new breakpoint closes the drawer.
  useEffect(() => setOpen(false), [breakpoint, setOpen]);
  const drawer = open && breakpoint !== "wide";
  useEffect(() => {
    const column = aside.current;
    if (!drawer || column === null) {
      return;
    }
    if (!column.contains(document.activeElement)) {
      focusColumn(column);
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
      }
    };
    const onOutside = (event: Event) => {
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest(".omfp-app") !== null &&
        !column.contains(target) &&
        target.closest(`[${DRAWER_CONTROL}]`) === null
      ) {
        setOpen(false);
      }
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("pointerdown", onOutside, true);
    document.addEventListener("focusin", onOutside);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("pointerdown", onOutside, true);
      document.removeEventListener("focusin", onOutside);
      if (column.contains(document.activeElement)) {
        document.querySelector<HTMLElement>('[data-testid="left-toggle"]')?.focus();
      }
    };
  }, [drawer, aside, setOpen]);
}

/**
 * The left column's commands. `threads.toggle` (Space t) focuses the wide column, shown first if it was collapsed
 * (UI-987), or toggles the narrow drawer and focuses its selected row (UI-201). `threads.collapse` (Space b) hides
 * the wide column or shows it again (UI-985); narrow and phone frames have nothing to collapse.
 */
export function LeftColumnCommands({ breakpoint }: { breakpoint: Breakpoint }): ReactNode {
  const [, setOpen] = useWindowState(LEFT_OPEN);
  const [collapsed] = useWindowState(LEFT_COLLAPSED);
  const setCollapsed = useSetCollapsed();
  const reveal = useRevealColumn(breakpoint);
  useCommand({
    id: "threads.toggle",
    title: strings.shell.commands.toggleThreads,
    run: () => {
      if (breakpoint !== "wide") {
        setOpen((open) => !open);
        return;
      }
      reveal();
      focusColumn(document.getElementById(LEFT_COLUMN_ID));
    },
  });
  useCommand({
    id: "threads.collapse",
    title: strings.shell.commands.collapseThreads,
    enabled: () => breakpoint === "wide",
    run: () => setCollapsed(!collapsed),
  });
  return null;
}

/**
 * Shows the wide column at once when it is collapsed, so a command can focus inside it: `threads.toggle` and the
 * column's `workspace.new` (UI-987).
 */
export function useRevealColumn(breakpoint: Breakpoint): () => void {
  const [collapsed] = useWindowState(LEFT_COLLAPSED);
  const setCollapsed = useSetCollapsed();
  return () => {
    if (breakpoint === "wide" && collapsed) {
      setCollapsed(false);
    }
  };
}

const COLLAPSE_BUTTON = "left-collapse";
const EXPAND_BUTTON = "left-expand";

/**
 * Collapses the wide column or shows it again, rendered at once so the focus can move: from inside the column to the
 * mode line's Show threads, from that button to the column's Hide threads (UI-985, UI-986).
 */
function useSetCollapsed(): (collapse: boolean) => void {
  const [, setCollapsed] = useWindowState(LEFT_COLLAPSED);
  return (collapse) => {
    const from = collapse
      ? document.getElementById(LEFT_COLUMN_ID)
      : document.querySelector(`[data-testid="${EXPAND_BUTTON}"]`);
    const moveFocus = from?.contains(document.activeElement) ?? false;
    flushSync(() => setCollapsed(collapse));
    if (moveFocus) {
      document.querySelector<HTMLElement>(`[data-testid="${collapse ? EXPAND_BUTTON : COLLAPSE_BUTTON}"]`)?.focus();
    }
  };
}

/** Hide threads, on the wide column's brand row (UI-985). */
export function HideThreadsButton(): ReactNode {
  const setCollapsed = useSetCollapsed();
  return (
    <IconButton
      label={text.hideThreads}
      icon={<Icon name="panel-left-close" />}
      aria-controls={LEFT_COLUMN_ID}
      data-testid={COLLAPSE_BUTTON}
      onPress={() => setCollapsed(true)}
    />
  );
}

/** Show threads, at the mode line's start while the wide column is collapsed (UI-986). */
export function ShowThreadsButton(): ReactNode {
  const setCollapsed = useSetCollapsed();
  return (
    <ModeButton
      label={text.showThreads}
      icon="panel-left-open"
      aria-controls={LEFT_COLUMN_ID}
      data-testid={EXPAND_BUTTON}
      onPress={() => setCollapsed(false)}
    />
  );
}

/** The strip of a narrow frame (UI-201): brand, toggle, and the threads needing the user or running. */
export function LeftStrip(): ReactNode {
  const [open, setOpen] = useWindowState(LEFT_OPEN);
  const [selected, select] = useWindowState(SELECTED_THREAD);
  const threads = useThreads();
  const items = useMemo(
    (): StripItem[] =>
      stripThreads([...threads.values()]).map((thread) => ({
        id: thread.id,
        title: thread.title,
        state: threadMark(thread.status),
        selected: thread.id === selected,
      })),
    [threads, selected],
  );
  return (
    <ThreadStrip
      {...{ [DRAWER_CONTROL]: "", [THREADS_ATTRIBUTE]: "" }}
      aria-label={text.strip}
      data-testid="left-strip"
      open={open}
      onToggle={() => setOpen(!open)}
      toggleLabel={open ? text.hideThreads : text.showThreads}
      controls={LEFT_COLUMN_ID}
      threads={items}
      onSelect={(id) => select(id)}
    />
  );
}

const AREA_ITEMS: readonly AreaItem[] = [
  { id: "threads", label: text.threads, icon: "panel-left" },
  { id: "chat", label: text.chat, icon: "message-square" },
  { id: "side", label: text.side, icon: "panel-right" },
];

/**
 * The phone's bottom bar (UI-204): Threads · Chat · Side (Side only when the view has a side area), then Commands,
 * which opens the commands sheet (UI-460).
 */
export function Areas({ side }: { side: boolean }): ReactNode {
  const [open, setOpen] = useWindowState(LEFT_OPEN);
  const [area, setArea] = useWindowState(MOBILE_AREA);
  const [commands, setCommands] = useState(false);
  const items = side ? AREA_ITEMS : AREA_ITEMS.filter((item) => item.id !== "side");
  return (
    <>
      <AreaBar
        {...{ [DRAWER_CONTROL]: "" }}
        label={text.areas}
        data-testid="area-bar"
        items={items}
        actions={[
          {
            id: "commands",
            label: strings.shell.commandsSheet.title,
            icon: "command",
            onPress: () => setCommands(true),
          },
        ]}
        selected={open ? "threads" : side ? area : "chat"}
        onSelect={(id) => {
          if (id === "threads") {
            setOpen(true);
          } else {
            setOpen(false);
            setArea(id as Area);
          }
        }}
      />
      {commands && <CommandsSheet onClose={() => setCommands(false)} />}
    </>
  );
}

/** The width of the frame (the column's parent) in `rem`; 0 before layout. */
function frameRemOf(aside: HTMLElement | null): number {
  const shell = aside?.parentElement;
  return shell === null || shell === undefined ? 0 : shell.getBoundingClientRect().width / remPx();
}

/** Sets the column's width (`undefined`: the stylesheet's default, from `--omfp-side-w` up to 26rem, UI-486). */
function showWidth(aside: HTMLElement | null, rem: number | undefined): void {
  aside?.style.setProperty("flex-basis", rem === undefined ? "" : `${rem}rem`);
}

/**
 * The splitter between the left column and the view (UI-207): drags, arrow keys (5 % of the frame) and double click
 * (back to the default width) set this window's width of the column, which only it applies to the column: a drag moves
 * the column directly, without rendering the frame, and writes the width once released. Without a width of its own
 * the column moves from the width it shows.
 */
export function LeftSplitter({ aside }: { aside: RefObject<HTMLElement | null> }): ReactNode {
  const [width, setWidth] = useWindowState(LEFT_WIDTH);
  const current = width ?? (aside.current?.getBoundingClientRect().width ?? 0) / remPx();
  const [frameRem, setFrameRem] = useState(0);
  useLayoutEffect(() => {
    const column = aside.current;
    showWidth(column, width);
    setFrameRem(frameRemOf(column));
    return () => showWidth(column, undefined);
  }, [aside, width]);
  return (
    <Resizer
      orientation="vertical"
      label={text.leftSplitter}
      value={frameRem > 0 ? (current / frameRem) * 100 : 0}
      data-testid="left-splitter"
      onPreview={(delta) =>
        showWidth(aside.current, delta === undefined ? width : resizeLeft(current, delta, frameRemOf(aside.current)))
      }
      onResize={(delta) => setWidth(resizeLeft(current, delta, frameRemOf(aside.current)))}
      onReset={() => setWidth(undefined)}
    />
  );
}

/** The frame's breakpoint: its own width's (UI-200), `mobile` on a short touch screen (UI-414), both followed live. */
export function useFrameBreakpoint(app: RefObject<Element | null>): Breakpoint {
  const byWidth = useWidthClass(app, breakpointOf);
  const shortTouch = useMediaQuery(SHORT_TOUCH_SCREEN);
  return shortTouch ? "mobile" : byWidth;
}

/**
 * On a phone or any touch screen, the app is as tall as the visual viewport (`--vvh` on `<html>`, a runtime value, not
 * a theme token), so the composer stays above the on-screen keyboard (UI-205, UI-414); elsewhere it fills the window.
 */
export function useVisualViewport(breakpoint: Breakpoint): void {
  const follow = useMediaQuery(COARSE_POINTER) || breakpoint === "mobile";
  useEffect(() => {
    const viewport = window.visualViewport;
    const root = document.documentElement;
    if (!follow || viewport === null || viewport === undefined) {
      return;
    }
    const update = () => root.style.setProperty("--vvh", `${viewport.height}px`);
    update();
    viewport.addEventListener("resize", update);
    return () => {
      viewport.removeEventListener("resize", update);
      root.style.removeProperty("--vvh");
    };
  }, [follow]);
}
