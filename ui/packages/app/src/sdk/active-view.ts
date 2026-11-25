/**
 * The view a window shows per thread (UI-302, UI-367): read from and written to {@link VIEW_CHOICES}. Every way of
 * switching view (mode-line tabs, `:view`, `] v` / `[ v`, init files' `omfp.run("view.switch")`) goes through
 * {@link chooseView}.
 */

import { INDEX_TOPIC } from "@omfp/protocol";
import { useEffect, useLayoutEffect } from "react";
import { MAIN_SLOT } from "../view/layout.ts";
import { keepListedTabs } from "../vm/tab-choices.ts";
import { activeView, chooseView as choose, keepListed } from "../vm/view-choices.ts";
import { useCurrentWorkspaceId } from "./actions.ts";
import { useWindowState } from "./hooks.ts";
import { type OmfpHost, useHost } from "./host.tsx";
import { PENDING_TAB, SELECTED_THREAD, SHOWN_VIEW, TAB_CHOICES, VIEW_CHOICES } from "./keys.ts";
import { selectTab } from "./tab-choices.ts";

/** The id among `viewIds` (the current workspace's views) this window shows for the selected thread (else the draft
 * thread): the thread's view, else `undefined` (the workspace's default view). */
export function useActiveView(viewIds: readonly string[]): string | undefined {
  const [choices] = useWindowState(VIEW_CHOICES);
  const [selected] = useWindowState(SELECTED_THREAD);
  return activeView(choices, selected, viewIds);
}

/** Switches this window to `viewId`: the selected thread's view (none: the draft thread's). */
export function chooseView(host: OmfpHost, viewId: string): void {
  const selected = host.window.get(SELECTED_THREAD);
  host.window.set(VIEW_CHOICES, (choices) => choose(choices, selected, viewId));
}

/**
 * Keeps {@link VIEW_CHOICES} up to date, for the frame: the threads the live index no longer lists (deleted) are
 * dropped, with their tabs and those of the workspaces it no longer lists from {@link TAB_CHOICES} (UI-372).
 */
export function useViewChoicesUpkeep(): void {
  const host = useHost();
  useEffect(() => {
    const prune = () => {
      const snapshot = host.frames.getSnapshot();
      if (snapshot.topics.find((state) => state.topic === INDEX_TOPIC)?.sync.kind !== "live") {
        return;
      }
      const { threads, workspaces } = snapshot.index;
      // A thread just created is selected, with its draft's view and tabs, before the index may list it.
      const selectedThread = host.window.get(SELECTED_THREAD);
      const kept = { has: (id: string) => id === selectedThread || threads.has(id) };
      host.window.set(VIEW_CHOICES, (choices) => keepListed(choices, kept));
      host.window.set(TAB_CHOICES, (choices) => keepListedTabs(choices, kept, workspaces));
    };
    prune();
    return host.frames.subscribe(() => {
      if (host.frames.getSnapshot().dirty.index) {
        prune();
      }
    });
  }, [host]);
}

/**
 * Publishes the view the frame renders, with the thread and workspace it renders it for, as {@link SHOWN_VIEW}
 * (UI-372), before the passive effects of its widgets run; then, once `ready` (the workspace's views arrived, so the
 * view is not the built-in one standing in while they load), selects the {@link PENDING_TAB} opened for that thread
 * (a draft thread: of that workspace) in it.
 */
export function useShownView(viewId: string | undefined, ready: boolean): void {
  const host = useHost();
  const [threadId] = useWindowState(SELECTED_THREAD);
  const workspaceId = useCurrentWorkspaceId();
  const [pending] = useWindowState(PENDING_TAB);
  useLayoutEffect(
    () => host.window.set(SHOWN_VIEW, viewId === undefined ? undefined : { threadId, workspaceId, viewId, ready }),
    [host, threadId, workspaceId, viewId, ready],
  );
  useLayoutEffect(() => {
    if (
      ready &&
      pending !== undefined &&
      viewId !== undefined &&
      pending.threadId === threadId &&
      pending.workspaceId === workspaceId
    ) {
      host.window.set(PENDING_TAB, undefined);
      selectTab(host, viewId, MAIN_SLOT, pending.tabId);
    }
  }, [host, ready, pending, threadId, workspaceId, viewId]);
}
