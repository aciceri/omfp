/**
 * What the window tells the daemon about itself for agents (UI-350, UI-351, PRT-122), with its widget report: the
 * selected thread, whether it has the focus, its views after `views.json`, the init files and their transforms (where
 * each comes from, the widgets of its layout), the view it shows, and the commands init files and
 * workspace or user widgets registered beyond the built-ins, with their keys. Agents read it with `windows.state`
 * (`omfp_windows`).
 */

import type { ViewSource, WindowCommand, WindowReport, WindowView } from "@omfp/protocol";
import { useEffect, useState, useSyncExternalStore } from "react";
import type { Binding } from "../keymap/keymap.ts";
import { type Command, useHost } from "../sdk/index.ts";
import type { LayoutTree } from "../view/layout.ts";
import { BUILTIN_VIEW, type ResolvedView, type Views } from "../view/views.ts";

/** The widget references of `tree`, each once, in layout order. */
export function widgetsOf(tree: LayoutTree): string[] {
  const found = new Set<string>();
  const visit = (node: LayoutTree): void => {
    if (node.kind === "widget") {
      found.add(node.name);
    } else {
      for (const child of node.children) {
        visit(child);
      }
    }
  };
  visit(tree);
  return [...found];
}

/** Where a view of the switcher comes from. */
export function viewSource(view: ResolvedView): ViewSource {
  if (view === BUILTIN_VIEW) {
    return "builtin";
  }
  switch (view.init) {
    case "user":
      return "user_init";
    case "workspace":
      return "workspace_init";
    default:
      return "views_json";
  }
}

/** What the window state is made of. */
export interface WindowStateInput {
  readonly workspaceId: string | undefined;
  readonly threadId: string | undefined;
  readonly focused: boolean;
  /** The window's views (after the init files). */
  readonly views: Views;
  /** The view of the switcher the window shows. */
  readonly shown: ResolvedView | undefined;
  readonly commands: readonly Command[];
  /** The window's effective bindings. */
  readonly bindings: readonly Binding[];
}

function windowView(view: ResolvedView): WindowView {
  return { id: view.id, title: view.title, source: viewSource(view), widgets: widgetsOf(view.tree) };
}

function windowCommand(command: Command, bindings: readonly Binding[]): WindowCommand {
  return {
    id: command.id,
    title: command.title,
    aliases: [...(command.aliases ?? [])],
    source: command.init === "user" ? "user_init" : command.init === "workspace" ? "workspace_init" : "widget",
    widget: command.widget ?? null,
    enabled: command.enabled?.() ?? true,
    bindings: bindings
      .filter((binding) => binding.command === command.id)
      .map(({ mode, keys, args }) => ({ mode, keys, args: [...(args ?? [])] })),
  };
}

/** The window state (PRT-122): without a workspace, no views. */
export function windowReport(input: WindowStateInput): WindowReport {
  const { workspaceId, threadId, focused, views, shown } = input;
  const commands = input.commands
    .filter((command) => command.init !== undefined || command.widget !== undefined)
    .map((command) => windowCommand(command, input.bindings));
  if (workspaceId === undefined) {
    return { threadId: threadId ?? null, focused, activeView: null, defaultView: null, views: [], commands };
  }
  return {
    threadId: threadId ?? null,
    focused,
    activeView: shown?.id ?? null,
    defaultView: views.defaultId,
    views: views.views.map(windowView),
    commands,
  };
}

/** Whether the window has the focus, live: `hasFocus` read again on every focus change of the window. */
export function useWindowFocus(hasFocus: () => boolean): boolean {
  return useSyncExternalStore((changed) => {
    window.addEventListener("focus", changed);
    window.addEventListener("blur", changed);
    return () => {
      window.removeEventListener("focus", changed);
      window.removeEventListener("blur", changed);
    };
  }, hasFocus);
}

/** Sends the window state with the window's reports (UI-350, UI-351) whenever it or the registered commands change. */
export function useWindowStateReport(input: Omit<WindowStateInput, "commands">): void {
  const { reports, commands } = useHost();
  const [registered, setRegistered] = useState(() => commands.list());
  useEffect(() => {
    setRegistered(commands.list());
    return commands.subscribe(() => setRegistered(commands.list()));
  }, [commands]);
  const { workspaceId, threadId, focused, views, shown, bindings } = input;
  useEffect(() => {
    reports.setWindow(windowReport({ workspaceId, threadId, focused, views, shown, bindings, commands: registered }));
  }, [reports, workspaceId, threadId, focused, views, shown, bindings, registered]);
}
