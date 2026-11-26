/**
 * Reordering the left column (UI-620..UI-627): the pointer drag of thread rows and workspace sections — a mouse or a
 * pen drags once it moved 4 px, a finger after a long press (UI-623) —, the move keys on the focused row or section,
 * else on the selected thread (UI-624, UI-095), and the call, announced once done (UI-625) or reported as an error.
 * Nothing drags or moves while a filter is typed (UI-626).
 */

import {
  type PointerEvent as ReactPointerEvent,
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { reportError, SELECTED_THREAD, useAnnounce, useHost } from "../../sdk/index.ts";
import { strings } from "../../strings/index.ts";
import {
  announcement,
  isNoop,
  type KeyMove,
  keyTarget,
  type ReorderSource,
  type ReorderTarget,
  reorderCall,
  type SectionBox,
  threadTargetAt,
  workspaceTargetAt,
} from "../../vm/order.ts";
import type { Section } from "../../vm/threads.ts";

/** How far a mouse or pen moves before a press becomes a drag, in CSS pixels. */
const MOUSE_SLOP = 4;
/** How long a finger rests before its press becomes a drag (UI-623). */
export const LONG_PRESS_MS = 500;
/** How far a finger may move during the long press; farther, it scrolls. */
const TOUCH_SLOP = 8;
/** The band at the column's top and bottom edges where a drag scrolls it, and the step. */
const EDGE = 32;
const EDGE_STEP = 16;

/** Moves `source` to `target`; `focus` puts the focus back on the moved row or section head (the keys). */
export type Reorder = (source: ReorderSource, target: ReorderTarget, focus?: boolean) => void;

/**
 * The moves of the column (UI-625): a move that leaves everything in place does nothing; else the method runs, then
 * the live region says where the thread or workspace went and, for the keys, the focus stays on it while the index
 * moves it (until the user focuses or presses elsewhere). A refusal (a conflict with another window, a working
 * thread) goes to the error toasts.
 */
export function useReorder(sections: readonly Section[], name: (source: ReorderSource) => string): Reorder {
  const host = useHost();
  const announce = useAnnounce();
  const latest = useRef({ sections, name });
  latest.current = { sections, name };
  const refocus = useRef<string | undefined>(undefined);
  const focusMoved = useCallback(() => {
    const selector = refocus.current;
    const element = selector === undefined ? null : document.querySelector<HTMLElement>(selector);
    if (element !== null && element !== document.activeElement) {
      element.focus();
      element.scrollIntoView?.({ block: "nearest" });
    }
  }, []);
  // The moved row is rendered again (in another section, or moved in its list), which may drop its focus.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs whenever the sections change.
  useEffect(focusMoved, [sections, focusMoved]);
  useEffect(() => {
    const elsewhere = (event: Event) => {
      const selector = refocus.current;
      if (selector !== undefined && !(event.target instanceof Element && event.target.matches(selector))) {
        refocus.current = undefined;
      }
    };
    document.addEventListener("focusin", elsewhere);
    document.addEventListener("pointerdown", elsewhere);
    return () => {
      document.removeEventListener("focusin", elsewhere);
      document.removeEventListener("pointerdown", elsewhere);
    };
  }, []);
  return useCallback(
    (source, target, focus = false) => {
      const { sections: now, name: nameOf } = latest.current;
      if (isNoop(now, source, target)) {
        return;
      }
      const said = announcement(now, source, target, nameOf(source));
      const call = reorderCall(source, target);
      // The keys keep the focus on what moved: its row, or its section's head.
      refocus.current = !focus
        ? undefined
        : source.kind === "thread"
          ? `[data-testid="thread-row"][data-thread-id="${CSS.escape(source.id)}"]`
          : `[data-testid="workspace-section"][data-workspace-id="${CSS.escape(source.id)}"] [data-testid="workspace-collapse"]`;
      const done =
        call.method === "threads.reorder"
          ? host.client.call(call.method, call.params)
          : host.client.call(call.method, call.params);
      done.then(
        () => {
          announce(said);
          focusMoved();
        },
        (error: unknown) => {
          refocus.current = undefined;
          reportError(host, error);
        },
      );
    },
    [host, announce, focusMoved],
  );
}

/** What is being dragged and where it would land now. */
export interface DragState {
  readonly source: ReorderSource;
  readonly target: ReorderTarget | undefined;
}

/** The pointer-down handler starting a drag of `source`, a thread row or a section head. */
export type DragStart = (source: ReorderSource) => (event: ReactPointerEvent) => void;

/** Whether a press on `target` may drag its row or section: on the select button, the section's toggle or name, never
 * on another control (delete, menu, a field). */
function fromHandle(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) {
    return false;
  }
  const control = target.closest("button, a, input, textarea, select, [role='menuitem']");
  return control === null || control.hasAttribute("data-reorder-handle");
}

/** The sections of the column as the pointer sees them, without `source` (its own place is no target). */
function measure(root: HTMLElement, source: ReorderSource): SectionBox[] {
  const sections = [...root.querySelectorAll<HTMLElement>('[data-testid="workspace-section"]')];
  return sections
    .filter((section) => source.kind === "thread" || section.dataset.workspaceId !== source.id)
    .map((section) => {
      const box = section.getBoundingClientRect();
      const rows =
        source.kind === "workspace"
          ? []
          : [...section.querySelectorAll<HTMLElement>("[data-reorder-thread]")]
              .filter((row) => row.dataset.reorderThread !== source.id)
              .map((row) => {
                const rect = row.getBoundingClientRect();
                return { id: row.dataset.reorderThread ?? "", middle: (rect.top + rect.bottom) / 2 };
              });
      return { workspaceId: section.dataset.workspaceId ?? "", top: box.top, bottom: box.bottom, rows };
    });
}

/** The nearest scrolling ancestor of `element` (the column's scroller), if any. */
function scroller(element: HTMLElement): HTMLElement | undefined {
  for (let current: HTMLElement | null = element; current !== null; current = current.parentElement) {
    const overflow = getComputedStyle(current).overflowY;
    if ((overflow === "auto" || overflow === "scroll") && current.scrollHeight > current.clientHeight) {
      return current;
    }
  }
  return undefined;
}

/** What a followed press tells its hook: what is dragged and where it would land (`undefined`: no drag), the drop. */
interface PressEvents {
  readonly show: (drag: DragState | undefined) => void;
  readonly drop: (source: ReorderSource, target: ReorderTarget) => void;
}

/**
 * Follows the press `event` on `source` (UI-621..UI-623): a mouse or pen drags once it moved {@link MOUSE_SLOP}, a
 * finger after resting {@link LONG_PRESS_MS} (moving sooner, it scrolls); the drag shows where it would land, scrolls
 * the column near its edges, keeps a finger from scrolling the page and drops on release; Escape or a cancelled
 * pointer drops nothing. Returns the function that stops following it.
 */
function follow(column: HTMLElement, source: ReorderSource, event: ReactPointerEvent, events: PressEvents): () => void {
  const touch = event.pointerType === "touch";
  const { pointerId, clientX: x, clientY: y } = event;
  const scrolling = scroller(column);
  let started = false;
  let target: ReorderTarget | undefined;
  const aim = (pointerY: number) => {
    if (scrolling !== undefined) {
      const edges = scrolling.getBoundingClientRect();
      if (pointerY < edges.top + EDGE) {
        scrolling.scrollTop -= EDGE_STEP;
      } else if (pointerY > edges.bottom - EDGE) {
        scrolling.scrollTop += EDGE_STEP;
      }
    }
    const boxes = measure(column, source);
    const next = source.kind === "thread" ? threadTargetAt(boxes, pointerY) : workspaceTargetAt(boxes, pointerY);
    if (JSON.stringify(next) !== JSON.stringify(target)) {
      target = next;
      events.show({ source, target });
    }
  };
  const begin = (pointerY: number) => {
    started = true;
    events.show({ source, target: undefined });
    aim(pointerY);
  };
  const move = (moved: PointerEvent) => {
    if (moved.pointerId !== pointerId) {
      return;
    }
    if (started) {
      moved.preventDefault();
      aim(moved.clientY);
      return;
    }
    const distance = Math.hypot(moved.clientX - x, moved.clientY - y);
    if (touch && distance > TOUCH_SLOP) {
      stop();
    } else if (!touch && distance >= MOUSE_SLOP) {
      begin(moved.clientY);
    }
  };
  const up = (released: PointerEvent) => {
    if (released.pointerId !== pointerId) {
      return;
    }
    const dropped = started ? target : undefined;
    const dragged = started;
    stop();
    if (dragged) {
      // The click that may follow the release is part of the drag, not a press on a row.
      const swallow = (click: MouseEvent) => {
        click.preventDefault();
        click.stopPropagation();
      };
      window.addEventListener("click", swallow, { capture: true, once: true });
      setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
    }
    if (dropped !== undefined) {
      events.drop(source, dropped);
    }
  };
  const cancel = (cancelled: PointerEvent) => {
    if (cancelled.pointerId === pointerId) {
      stop();
    }
  };
  const giveUp = (key: KeyboardEvent) => {
    if (key.key === "Escape" && started) {
      key.preventDefault();
      key.stopPropagation();
      stop();
    }
  };
  const holdTouch = (moved: TouchEvent) => {
    if (started && moved.cancelable) {
      moved.preventDefault();
    }
  };
  const noMenu = (menu: Event) => menu.preventDefault();
  const timer = touch ? setTimeout(() => begin(y), LONG_PRESS_MS) : undefined;
  const stop = () => {
    clearTimeout(timer);
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("keydown", giveUp, { capture: true });
    window.removeEventListener("touchmove", holdTouch);
    window.removeEventListener("contextmenu", noMenu, { capture: true });
    if (started) {
      started = false;
      events.show(undefined);
    }
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  window.addEventListener("keydown", giveUp, { capture: true });
  window.addEventListener("touchmove", holdTouch, { passive: false });
  if (touch) {
    window.addEventListener("contextmenu", noMenu, { capture: true });
  }
  return stop;
}

/**
 * The pointer drag of the column (UI-621..UI-623): `start(source)` is the pointer-down handler of a thread row or a
 * section head; while dragging, `drag` says what moves and where it would land.
 */
export function useReorderDrag(
  root: RefObject<HTMLElement | null>,
  enabled: boolean,
  reorder: Reorder,
): { readonly drag: DragState | undefined; readonly start: DragStart } {
  const [drag, setDrag] = useState<DragState | undefined>(undefined);
  const following = useRef<(() => void) | undefined>(undefined);
  const latest = useRef(reorder);
  latest.current = reorder;
  useEffect(() => () => following.current?.(), []);
  const start = useCallback<DragStart>(
    (source) => (event) => {
      const column = root.current;
      if (
        !enabled ||
        column === null ||
        !event.isPrimary ||
        (event.pointerType === "mouse" && event.button !== 0) ||
        !fromHandle(event.target)
      ) {
        return;
      }
      following.current?.();
      following.current = follow(column, source, event, {
        show: setDrag,
        drop: (dragged, target) => latest.current(dragged, target),
      });
    },
    [root, enabled],
  );
  return { drag, start };
}

/** The thread or workspace the move keys act on: the focused thread row (not an archived one), else the section of
 * the focused element. */
function focusedSource(element: Element | null): ReorderSource | undefined {
  const section = element?.closest<HTMLElement>('[data-testid="workspace-section"]');
  const workspaceId = section?.dataset.workspaceId;
  if (element === null || workspaceId === undefined) {
    return undefined;
  }
  const row = element.closest<HTMLElement>("[data-reorder-thread]");
  if (row !== null) {
    return { kind: "thread", id: row.dataset.reorderThread ?? "", workspaceId };
  }
  return element.closest("[data-thread-id]") === null ? { kind: "workspace", id: workspaceId } : undefined;
}

/** The thread the move keys act on outside the column's sections (UI-095): the selected one, where a section lists it. */
function selectedSource(sections: readonly Section[], selected: string | undefined): ReorderSource | undefined {
  if (selected === undefined) {
    return undefined;
  }
  const section = sections.find((candidate) => candidate.threadIds.includes(selected));
  return section === undefined ? undefined : { kind: "thread", id: selected, workspaceId: section.workspace.id };
}

const MOVES: ReadonlyArray<readonly [string, string, KeyMove]> = [
  ["threads.moveUp", strings.shell.commands.moveUp, "up"],
  ["threads.moveDown", strings.shell.commands.moveDown, "down"],
  ["threads.moveToPrevWorkspace", strings.shell.commands.moveToPrevWorkspace, "prevWorkspace"],
  ["threads.moveToNextWorkspace", strings.shell.commands.moveToNextWorkspace, "nextWorkspace"],
];

/** The move keys of Threads mode (UI-624, UI-095): `Alt-ArrowUp`/`Alt-ArrowDown` move the focused thread or workspace,
 * `Alt-Shift-ArrowUp`/`Alt-Shift-ArrowDown` the focused thread into the previous or next workspace; with the focus
 * outside the column's sections, the selected thread. */
export function useMoveCommands(sections: readonly Section[], filtering: boolean, reorder: Reorder): void {
  const host = useHost();
  const announce = useAnnounce();
  const latest = useRef({ sections, filtering, reorder, announce });
  latest.current = { sections, filtering, reorder, announce };
  useEffect(() => {
    const removals = MOVES.map(([id, title, move]) =>
      host.commands.register({
        id,
        title,
        run: () => {
          const current = latest.current;
          const element = document.activeElement;
          const section = element === null ? null : element.closest('[data-testid="workspace-section"]');
          const source =
            section === null
              ? selectedSource(current.sections, host.window.get(SELECTED_THREAD))
              : focusedSource(element);
          if (source === undefined) {
            return;
          }
          if (current.filtering) {
            current.announce(strings.shell.reorder.filtering);
            return;
          }
          const target = keyTarget(current.sections, source, move);
          if (target !== undefined) {
            current.reorder(source, target, true);
          }
        },
      }),
    );
    return () => {
      for (const remove of removals) {
        remove();
      }
    };
  }, [host]);
}
