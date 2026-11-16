/**
 * The tab selected in each `tabs` container per thread and view (UI-372): read from and written to
 * {@link TAB_CHOICES} for the selected thread, else for the draft thread of the current workspace.
 */

import { useCallback, useState } from "react";
import { chooseTab, chosenTab, type TabOwner } from "../vm/tab-choices.ts";
import { currentWorkspaceId, useCurrentWorkspaceId } from "./actions.ts";
import { useWindowState } from "./hooks.ts";
import type { OmfpHost } from "./host.tsx";
import { SELECTED_THREAD, TAB_CHOICES } from "./keys.ts";

/** Whose selections this window shows: the selected thread's, else the draft thread's of the current workspace;
 * `undefined` without either. */
function ownerOf(threadId: string | undefined, workspaceId: string | undefined): TabOwner | undefined {
  if (threadId !== undefined) {
    return { kind: "thread", threadId };
  }
  return workspaceId === undefined ? undefined : { kind: "draft", workspaceId };
}

/** Selects `tabId` (`undefined`: the first tab) in `container` of view `viewId` for this window's selected thread,
 * else its draft thread. */
export function selectTab(host: OmfpHost, viewId: string, container: string, tabId: string | undefined): void {
  const owner = ownerOf(host.window.get(SELECTED_THREAD), currentWorkspaceId(host));
  if (owner !== undefined) {
    host.window.set(TAB_CHOICES, (choices) => chooseTab(choices, owner, viewId, container, tabId));
  }
}

/** The tab this window's selected thread, else its draft thread, selected in `container` of view `viewId`. */
export function chosenTabOf(host: OmfpHost, viewId: string, container: string): string | undefined {
  const owner = ownerOf(host.window.get(SELECTED_THREAD), currentWorkspaceId(host));
  return owner === undefined ? undefined : chosenTab(host.window.get(TAB_CHOICES), owner, viewId, container);
}

/**
 * The tab selected in `container` of view `viewId` (`undefined` before any: the container's first) for the selected
 * thread, else the draft thread, and what selects one. Without a thread, a workspace or a view, the selection is the
 * component's own.
 */
export function useChosenTab(
  viewId: string | undefined,
  container: string,
): readonly [string | undefined, (tabId: string | undefined) => void] {
  const [choices, setChoices] = useWindowState(TAB_CHOICES);
  const [threadId] = useWindowState(SELECTED_THREAD);
  const workspaceId = useCurrentWorkspaceId();
  const [local, setLocal] = useState<string>();
  const select = useCallback(
    (tabId: string | undefined) => {
      const owner = ownerOf(threadId, workspaceId);
      if (owner === undefined || viewId === undefined) {
        setLocal(tabId);
      } else {
        setChoices((all) => chooseTab(all, owner, viewId, container, tabId));
      }
    },
    [setChoices, threadId, workspaceId, viewId, container],
  );
  const owner = ownerOf(threadId, workspaceId);
  return [owner === undefined || viewId === undefined ? local : chosenTab(choices, owner, viewId, container), select];
}
