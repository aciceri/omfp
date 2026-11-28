/**
 * Opens a window on what its address bar named (UI-214, UI-218; read by `takeLaunch`): the thread at once, the
 * workspace's draft thread once the live index lists the workspace, the view chosen with either, the prompt in the
 * draft's composer. An unknown workspace is reported in the window's errors. Until the frame shows the named thread's
 * or workspace's views, {@link LAUNCHING} keeps the window from showing the views standing in meanwhile (UI-223).
 */

import { INDEX_TOPIC } from "@omfp/protocol";
import { strings } from "../strings/index.ts";
import { findWorkspace, type Launch } from "../vm/launch.ts";
import { reportError, threadActions } from "./actions.ts";
import { chooseView } from "./active-view.ts";
import type { OmfpHost } from "./host.tsx";
import { DRAFT_MESSAGE, SELECTED_THREAD } from "./keys.ts";
import { windowKey } from "./window-store.ts";

/** The window opens on a thread or a workspace whose views it does not show yet (UI-223): the frame hides its shell
 * and clears this once it shows them; a thread or workspace the daemon lacks clears it too. */
export const LAUNCHING = windowKey<boolean>("launching", false);

export function applyLaunch(host: OmfpHost, launch: Launch): void {
  const { thread, workspace, view, prompt } = launch;
  if (thread === undefined && workspace === undefined) {
    if (view !== undefined) {
      chooseView(host, view);
    }
    return;
  }
  host.window.set(LAUNCHING, true);
  if (workspace === undefined) {
    host.window.set(SELECTED_THREAD, thread);
    if (view !== undefined) {
      chooseView(host, view);
    }
  }
  const settle = (): boolean => {
    const snapshot = host.frames.getSnapshot();
    if (snapshot.topics.find((state) => state.topic === INDEX_TOPIC)?.sync.kind !== "live") {
      return false;
    }
    if (workspace === undefined) {
      // A thread the index lacks has no workspace whose views could show.
      if (thread !== undefined && !snapshot.index.threads.has(thread)) {
        host.window.set(LAUNCHING, false);
      }
      return true;
    }
    const found = findWorkspace(snapshot.index.workspaces.values(), workspace);
    if (found === undefined) {
      host.window.set(LAUNCHING, false);
      reportError(host, strings.shell.launch.unknownWorkspace(workspace));
      return true;
    }
    threadActions(host).openDraft(found.id);
    if (view !== undefined) {
      chooseView(host, view);
    }
    if (prompt !== undefined) {
      host.window.set(DRAFT_MESSAGE, (draft) => ({ ...draft, text: prompt }));
    }
    return true;
  };
  if (settle()) {
    return;
  }
  const stop = host.frames.subscribe(() => {
    if (settle()) {
      stop();
    }
  });
}
