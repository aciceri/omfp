/**
 * The global commands of the frame (UI-095), shared by the keymap and the `:` line: a draft thread in the current
 * workspace and thread actions on the selected thread (UI-021, UI-023, UI-026, UI-030), a rescan of the current
 * workspace (DMN-023), colour scheme (UI-091, window-local), density and `:set` (daemon settings, UI-094, UI-097), the
 * view switch (UI-101), the next and previous view (UI-302) and `:view-reset` (UI-107, UI-430), adding a widget to the view
 * (UI-300), copying a widget into the workspace (UI-106) and saving a workspace widget to the user library (UI-233).
 */

import type { ReactNode } from "react";
import {
  COLOR_SCHEME,
  chooseView,
  currentWorkspaceId,
  FOCUS_COMPOSER,
  type OpenFileOptions,
  openFile,
  RENAMING_THREAD,
  RESET_VIEWS,
  reportError,
  reportSuccess,
  SELECTED_THREAD,
  useCommand,
  useHost,
  useSetSettings,
  useThreadActions,
  useWindowState,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { displayTitle, isArchived, nextNeedingAttention, spawnerOf } from "../vm/threads.ts";
import { DENSITIES, parseSet, parseTheme, SET_KEYS, themeArgs } from "./set.ts";

/** The path the focused element or its nearest ancestor carries for `:open` (`data-omfp-path`, UI-116). */
function focusedPath(element: Element | null): { path: string; options: OpenFileOptions } | undefined {
  const holder = element?.closest("[data-omfp-path]");
  const path = holder?.getAttribute("data-omfp-path");
  if (holder === null || holder === undefined || path === null || path === undefined || path === "") {
    return undefined;
  }
  const line = Number(holder.getAttribute("data-omfp-line"));
  return {
    path,
    options: {
      line: Number.isInteger(line) && line > 0 ? line : undefined,
      diff: holder.hasAttribute("data-omfp-diff"),
    },
  };
}

export interface GlobalCommandsProps {
  /** The views of the window, in the order the mode line lists them. */
  readonly viewIds: readonly string[];
  /** The view the window shows. */
  readonly currentView: string | undefined;
  /** What `:widget add` completes (UI-300). */
  readonly widgets: readonly string[];
  /** Adds a widget to the current view (UI-300); resolves to whether `views.json` was written. */
  readonly addWidget: (reference: string) => Promise<boolean>;
}

/** `widget` actions, completed as the first argument of `:widget`. */
const WIDGET_ACTIONS = ["add", "copy", "save", "save!"];

export function GlobalCommands({ viewIds, currentView, widgets, addWidget }: GlobalCommandsProps): ReactNode {
  const host = useHost();
  const actions = useThreadActions();
  const setSettings = useSetSettings();
  const [selected] = useWindowState(SELECTED_THREAD);
  const summary = () => (selected === undefined ? undefined : host.frames.getSnapshot().index.threads.get(selected));
  const live = () => {
    const thread = summary();
    return thread !== undefined && !isArchived(thread);
  };
  const archived = () => {
    const thread = summary();
    return thread !== undefined && isArchived(thread);
  };
  const onSelected = (run: (threadId: string) => unknown) => () => {
    if (selected !== undefined) {
      void run(selected);
    }
  };
  const titles = strings.shell.commands;
  useCommand({
    id: "thread.new",
    title: titles.new,
    aliases: ["new"],
    // Without a current workspace, the "+ workspace" field of the left column takes the focus.
    run: () => {
      const workspaceId = currentWorkspaceId(host);
      if (workspaceId === undefined) {
        host.commands.run("workspace.new", [], []);
      } else {
        actions.openDraft(workspaceId);
      }
    },
  });
  useCommand({
    id: "file.open",
    title: titles.open,
    aliases: ["open"],
    enabled: () => currentWorkspaceId(host) !== undefined,
    // `:open <path>[:<line>]`, else the path the focused element (or an ancestor) carries (UI-116).
    run: (args) => {
      if (args.length > 0) {
        // Explicit filenames are not inline prose: unknown extensions and spaces still name files.
        const argument = args.join(" ");
        const suffix = /^(.*?):(\d+)(?::\d+)?$/.exec(argument);
        openFile(host, suffix?.[1] ?? argument, {
          line: suffix?.[2] === undefined ? undefined : Number(suffix[2]),
        });
        return;
      }
      const target = focusedPath(document.activeElement);
      if (target !== undefined) {
        openFile(host, target.path, target.options);
      }
    },
  });
  useCommand({
    id: "workspace.rescan",
    title: titles.rescan,
    aliases: ["rescan"],
    enabled: () => currentWorkspaceId(host) !== undefined,
    run: () => {
      const workspaceId = currentWorkspaceId(host);
      if (workspaceId !== undefined) {
        host.client.call("workspaces.rescan", { workspaceId }).catch((error: unknown) => reportError(host, error));
      }
    },
  });
  useCommand({
    id: "thread.archive",
    title: titles.archive,
    aliases: ["archive"],
    enabled: live,
    run: onSelected(actions.archive),
  });
  useCommand({
    id: "thread.unarchive",
    title: titles.unarchive,
    aliases: ["unarchive"],
    enabled: archived,
    run: onSelected(actions.unarchive),
  });
  useCommand({
    id: "thread.delete",
    title: titles.delete,
    description: titles.deleteDescription,
    aliases: ["delete"],
    enabled: () => summary() !== undefined,
    run: onSelected(actions.remove),
  });
  useCommand({
    id: "thread.rename",
    title: titles.rename,
    aliases: ["rename"],
    enabled: () => summary() !== undefined,
    run: (args) =>
      onSelected((threadId) =>
        args.length > 0 ? actions.rename(threadId, args.join(" ")) : host.window.set(RENAMING_THREAD, threadId),
      )(),
  });
  useCommand({
    id: "thread.stop",
    title: titles.stop,
    description: titles.stopDescription,
    aliases: ["stop"],
    enabled: () => ["running", "starting"].includes(summary()?.process.state ?? ""),
    run: onSelected(actions.stop),
  });
  useCommand({
    id: "thread.stopSpawned",
    title: titles.stopSpawned,
    aliases: ["stop!"],
    // Offered when the selected thread spawned threads (UI-242).
    enabled: () =>
      selected !== undefined &&
      [...host.frames.getSnapshot().index.threads.values()].some((thread) => spawnerOf(thread) === selected),
    run: onSelected(async (threadId) => {
      try {
        const { threadIds } = await host.client.call("thread.stop", { threadId, cascade: true });
        const thread = summary();
        const stopped = strings.shell.threads.stoppedSpawned(threadIds.length);
        reportSuccess(host, thread === undefined ? stopped : `${displayTitle(thread)}: ${stopped}`);
      } catch (error) {
        reportError(host, error);
      }
    }),
  });
  useCommand({
    id: "thread.restart",
    title: titles.restart,
    aliases: ["restart"],
    enabled: () => ["stopped", "failed"].includes(summary()?.process.state ?? ""),
    run: onSelected(actions.restart),
  });
  useCommand({
    id: "thread.nextAttention",
    title: titles.nextAttention,
    aliases: ["next"],
    run: () => {
      const threads = [...host.frames.getSnapshot().index.threads.values()];
      const next = nextNeedingAttention(threads, selected);
      if (next !== undefined) {
        actions.select(next);
        // The jump lands in the thread's composer (UI-286).
        host.window.set(FOCUS_COMPOSER, next);
      }
    },
  });
  useCommand({
    id: "window.theme",
    title: titles.theme,
    aliases: ["theme"],
    // UI-513: the window's colour scheme and the daemon's built-in theme, either or both.
    args: themeArgs,
    run: (args) => {
      const parsed = parseTheme(args);
      if (!parsed.ok) {
        reportError(host, new Error(parsed.error));
        return;
      }
      if (parsed.scheme !== undefined) {
        host.window.set(COLOR_SCHEME, parsed.scheme);
      }
      if (parsed.theme !== undefined) {
        void setSettings({ ui: { theme: parsed.theme } });
      }
    },
  });
  useCommand({
    id: "density.set",
    title: titles.density,
    aliases: ["density"],
    args: () => DENSITIES,
    run: ([density]) => {
      const chosen = DENSITIES.find((candidate) => candidate === density);
      if (chosen !== undefined) {
        void setSettings({ ui: { density: chosen } });
      }
    },
  });
  useCommand({
    id: "settings.set",
    title: titles.set,
    aliases: ["set"],
    args: () => SET_KEYS,
    run: (args) => {
      const parsed = parseSet(args.join(" "));
      if (parsed.ok) {
        void setSettings(parsed.patch);
      } else {
        reportError(host, new Error(parsed.error));
      }
    },
  });
  useCommand({
    id: "view.switch",
    title: titles.view,
    aliases: ["view"],
    args: () => viewIds,
    run: ([view]) => {
      if (view !== undefined && viewIds.includes(view)) {
        chooseView(host, view);
      } else if (view !== undefined) {
        reportError(host, strings.shell.layout.noView(view));
      }
    },
  });
  // `] v` / `[ v` (UI-302): the next or previous view, round the list.
  const stepView = (step: number) => () => {
    const index = viewIds.indexOf(currentView ?? "");
    const next = viewIds[(index + step + viewIds.length) % viewIds.length];
    if (next !== undefined) {
      chooseView(host, next);
    }
  };
  useCommand({
    id: "view.next",
    title: titles.nextView,
    enabled: () => viewIds.length > 1,
    run: stepView(1),
  });
  useCommand({
    id: "view.prev",
    title: titles.prevView,
    enabled: () => viewIds.length > 1,
    run: stepView(-1),
  });
  // UI-430: the reset of views.json is its own command and asks first (`ResetViewsDialog`).
  useCommand({
    id: "view.reset",
    title: titles.resetViews,
    aliases: ["view-reset"],
    run: () => {
      const workspaceId = currentWorkspaceId(host);
      if (workspaceId === undefined) {
        reportError(host, strings.shell.layout.noWorkspace);
      } else {
        host.window.set(RESET_VIEWS, workspaceId);
      }
    },
  });
  useCommand({
    id: "widget.manage",
    title: titles.widget,
    aliases: ["widget"],
    // `:widget add ` completes the widgets it can add; the other actions take a reference or a name freely.
    args: ([action, ...rest] = []) =>
      action === undefined ? WIDGET_ACTIONS : action === "add" && rest.length === 0 ? widgets : [],
    run: async ([action, widget, name, ...rest]) => {
      const text = strings.shell.widgetCopy;
      if (action === "add" && widget !== undefined && name === undefined) {
        if (await addWidget(widget)) {
          reportSuccess(host, text.added(widget, currentView ?? ""));
        }
        return;
      }
      const known = action === "copy" || action === "save" || action === "save!";
      if (!known || widget === undefined || rest.length > 0) {
        reportError(host, text.usage);
        return;
      }
      const workspaceId = currentWorkspaceId(host);
      if (workspaceId === undefined) {
        reportError(host, strings.shell.layout.noWorkspace);
        return;
      }
      try {
        if (action === "copy") {
          const copied = await host.client.call("widgets.copy", {
            workspaceId,
            widget,
            ...(name === undefined ? {} : { name }),
          });
          reportSuccess(host, text.copied(copied.name));
        } else {
          const saved = await host.client.call("widgets.save", {
            workspaceId,
            widget,
            name: name ?? widget.replace(/^\.\/widgets\//, ""),
            replace: action === "save!",
          });
          reportSuccess(host, text.saved(saved.reference));
        }
      } catch (error) {
        reportError(host, error);
      }
    },
  });
  return null;
}
