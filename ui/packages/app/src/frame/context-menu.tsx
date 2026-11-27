/**
 * omfp's own context menu (UI-314..UI-319) on messages, files and threads, instead of the browser's or WebKit's: a
 * kit `Menu` in a `Popover` at the pointer, with the items of `sdk/context-menu.ts` (the built-in ones, then those of
 * init files and widgets) whose commands answer for the target. A right click opens it, a finger resting on the target
 * (UI-318), and `menu.open` (`Shift-F10`, the menu key, `:menu`) on the focused target (UI-317). Shift-right-click, a
 * text field, selected text and a target without items keep the native menu, as does a widget that handled the event.
 */

import { Menu, MenuItem, Popover } from "@omfp/kit";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import {
  type ContextMenuEntry,
  type ContextTarget,
  contextMenuEntries,
  contextTargetAt,
  reportError,
  runContextEntry,
  useCommand,
  useHost,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";

const text = strings.shell.contextMenu;

/** How long a finger rests on a target before its menu opens (the left column's drag waits as long, UI-623). */
export const MENU_LONG_PRESS_MS = 500;
/** How far a finger may move while resting; farther, it scrolls (or, once the menu opened, drags). */
const TOUCH_SLOP = 8;

interface OpenMenu {
  /** Which opening this is: a new one (another right click while open) places a new popover. */
  readonly serial: number;
  readonly target: ContextTarget;
  readonly entries: readonly ContextMenuEntry[];
  readonly x: number;
  readonly y: number;
}

/** Whether `element` is in a text field, whose own menu (paste, spelling) stays. */
function editable(element: Element): boolean {
  return element.closest("input, textarea, select, [contenteditable]:not([contenteditable='false'])") !== null;
}

/** Whether a right click on `element` keeps the browser's menu: Shift held, a text field, or selected text there. */
function keepsNativeMenu(event: MouseEvent, element: Element): boolean {
  if (event.shiftKey || editable(element)) {
    return true;
  }
  const selection = window.getSelection();
  return selection !== null && !selection.isCollapsed && selection.containsNode(element, true);
}

/** Where a menu opened without a pointer shows: at the start of the element's first visible line. */
function elementPoint(element: Element): { x: number; y: number } {
  const box = element.getBoundingClientRect();
  return { x: box.left, y: Math.min(Math.max(box.top, 0), window.innerHeight) };
}

/** The window's context menu: listens to the page, holds the open menu and registers `menu.open`. */
export function ContextMenuLayer(): ReactNode {
  const host = useHost();
  const [open, setOpen] = useState<OpenMenu | undefined>(undefined);
  const anchor = useRef<HTMLSpanElement>(null);
  const serial = useRef(0);
  /** Opens `target`'s menu at `x`, `y`; `false` when it has no item now. */
  const show = useCallback(
    (target: ContextTarget, x: number, y: number): boolean => {
      const entries = contextMenuEntries(host.commands, host.contextMenus, target);
      if (entries.length === 0) {
        return false;
      }
      serial.current += 1;
      setOpen({ serial: serial.current, target, entries, x, y });
      return true;
    },
    [host],
  );
  const close = useCallback(() => setOpen(undefined), []);

  useEffect(() => {
    /** The finger resting on a target now: opens its menu at once (the native long-press menu came first). */
    let press: { readonly open: () => void; readonly stop: () => void; opened: boolean } | undefined;
    const onContextMenu = (event: MouseEvent) => {
      if (event.defaultPrevented || !(event.target instanceof Element)) {
        return;
      }
      // The native menu of a long press (Android's) opens ours instead, once.
      if (press !== undefined) {
        press.open();
        if (press.opened) {
          event.preventDefault();
        }
        return;
      }
      const target = contextTargetAt(event.target);
      if (target === undefined || keepsNativeMenu(event, event.target)) {
        return;
      }
      // The keyboard's menu key fires it without a pointer position.
      const point =
        event.clientX === 0 && event.clientY === 0
          ? elementPoint(target.element)
          : { x: event.clientX, y: event.clientY };
      if (show(target, point.x, point.y)) {
        event.preventDefault();
      }
    };
    const onPointerDown = (event: PointerEvent) => {
      if (event.pointerType !== "touch" || !event.isPrimary || !(event.target instanceof Element)) {
        return;
      }
      const target = contextTargetAt(event.target);
      if (target === undefined || editable(event.target)) {
        return;
      }
      press?.stop();
      const { pointerId, clientX: x, clientY: y } = event;
      const move = (moved: PointerEvent) => {
        if (moved.pointerId === pointerId && Math.hypot(moved.clientX - x, moved.clientY - y) > TOUCH_SLOP) {
          // Sooner it scrolls; once the menu opened, the finger drags (a thread row, UI-623) and the menu goes.
          if (current.opened) {
            close();
          }
          current.stop();
        }
      };
      const up = (released: PointerEvent) => {
        if (released.pointerId !== pointerId) {
          return;
        }
        current.stop();
        if (current.opened) {
          // The click that follows the release is part of the press, not a press on the target.
          const click = (followed: MouseEvent) => {
            followed.preventDefault();
            followed.stopPropagation();
          };
          window.addEventListener("click", click, { capture: true, once: true });
          setTimeout(() => window.removeEventListener("click", click, { capture: true }), 0);
        }
      };
      const cancel = (cancelled: PointerEvent) => {
        if (cancelled.pointerId === pointerId) {
          current.stop();
        }
      };
      const current = {
        opened: false,
        open: () => {
          clearTimeout(timer);
          if (!current.opened) {
            current.opened = show(target, x, y);
          }
        },
        stop: () => {
          clearTimeout(timer);
          if (press === current) {
            press = undefined;
          }
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", up);
          window.removeEventListener("pointercancel", cancel);
        },
      };
      const timer = setTimeout(current.open, MENU_LONG_PRESS_MS);
      press = current;
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      window.addEventListener("pointercancel", cancel);
    };
    document.addEventListener("contextmenu", onContextMenu);
    // Captured: file links stop the propagation of their presses (a tool card's path, `omfp:changes`).
    document.addEventListener("pointerdown", onPointerDown, { capture: true });
    return () => {
      press?.stop();
      document.removeEventListener("contextmenu", onContextMenu);
      document.removeEventListener("pointerdown", onPointerDown, { capture: true });
    };
  }, [show, close]);

  useCommand({
    id: "menu.open",
    title: strings.shell.commands.menu,
    aliases: ["menu"],
    run: () => {
      const target = contextTargetAt(document.activeElement);
      const point = target === undefined ? undefined : elementPoint(target.element);
      // With nothing such focused, or a target without items now, it says what to focus.
      if (target === undefined || point === undefined || !show(target, point.x, point.y)) {
        reportError(host, text.noTarget);
      }
    },
  });

  if (open === undefined) {
    return null;
  }
  return (
    <>
      <span
        key={`anchor-${open.serial}`}
        ref={anchor}
        aria-hidden="true"
        className="omfp-context-menu__anchor"
        style={{ left: open.x, top: open.y }}
      />
      <Popover
        key={`menu-${open.serial}`}
        triggerRef={anchor}
        isOpen
        onOpenChange={(isOpen) => !isOpen && close()}
        placement="bottom start"
        offset={0}
        className="omfp-context-menu"
      >
        <Menu
          aria-label={text.label(open.target.kind)}
          data-testid="context-menu"
          data-target={open.target.kind}
          autoFocus="first"
          items={open.entries}
          onAction={(key) => {
            const entry = open.entries.find((candidate) => candidate.key === key);
            close();
            if (entry !== undefined) {
              runContextEntry(host.commands, entry, open.target);
            }
          }}
        >
          {(entry) => (
            <MenuItem
              id={entry.key}
              textValue={entry.title}
              data-testid="context-menu-item"
              data-command={entry.command}
            >
              {entry.title}
            </MenuItem>
          )}
        </Menu>
      </Popover>
    </>
  );
}
