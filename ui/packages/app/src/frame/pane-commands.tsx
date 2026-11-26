/**
 * View mode's commands (UI-095): `view.focus` (`v`, `Enter` in Threads mode) enters the view where it was focused last,
 * `view.leave` (`Esc` in View mode) leaves it for Threads mode, `pane.left` … `pane.right` (`Ctrl-w h/j/k/l`) move
 * between its panes, `tab.next` / `tab.prev` (`g t` / `g T`) between the tabs around the focus. The frame mounts it
 * once; it follows the focus to know where each view was focused last (keymap/panes.ts).
 */

import { type ReactNode, useEffect } from "react";
import { focusPaneToward, focusView, leaveView, rememberFocus, stepTab } from "../keymap/panes.ts";
import { useCommand } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";

const text = strings.shell.commands;

export function PaneCommands(): ReactNode {
  useEffect(() => {
    const remember = (event: FocusEvent) => {
      if (event.target instanceof Element) {
        rememberFocus(event.target);
      }
    };
    document.addEventListener("focusin", remember);
    return () => document.removeEventListener("focusin", remember);
  }, []);
  useCommand({
    id: "view.focus",
    title: text.focusView,
    run: () => {
      focusView();
    },
  });
  useCommand({ id: "view.leave", title: text.leaveView, run: leaveView });
  useCommand({
    id: "pane.left",
    title: text.paneLeft,
    run: () => {
      focusPaneToward("left");
    },
  });
  useCommand({
    id: "pane.down",
    title: text.paneDown,
    run: () => {
      focusPaneToward("down");
    },
  });
  useCommand({
    id: "pane.up",
    title: text.paneUp,
    run: () => {
      focusPaneToward("up");
    },
  });
  useCommand({
    id: "pane.right",
    title: text.paneRight,
    run: () => {
      focusPaneToward("right");
    },
  });
  useCommand({
    id: "tab.next",
    title: text.nextTab,
    run: () => {
      stepTab(1);
    },
  });
  useCommand({
    id: "tab.prev",
    title: text.prevTab,
    run: () => {
      stepTab(-1);
    },
  });
  return null;
}
