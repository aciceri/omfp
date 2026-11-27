/**
 * Opens a window on what its address bar named (UI-214, UI-218; read by `takeLaunch`): the thread at once, the
 * workspace's draft thread once the live index lists the workspace, the view chosen with either, the prompt in the
 * draft's composer. An unknown workspace is reported in the window's errors.
 */

import { INDEX_TOPIC } from "@omfp/protocol";
import { strings } from "../strings/index.ts";
import { findWorkspace, type Launch } from "../vm/launch.ts";
import { reportError, threadActions } from "./actions.ts";
import { chooseView } from "./active-view.ts";
import type { OmfpHost } from "./host.tsx";
import { DRAFT_MESSAGE, SELECTED_THREAD } from "./keys.ts";

export function applyLaunch(host: OmfpHost, launch: Launch): void {
  const { thread, workspace, view, prompt } = launch;
  if (workspace === undefined) {
    if (thread !== undefined) {
      host.window.set(SELECTED_THREAD, thread);
    }
    if (view !== undefined) {
      chooseView(host, view);
    }
    return;
  }
  const open = (): boolean => {
    const snapshot = host.frames.getSnapshot();
    if (snapshot.topics.find((state) => state.topic === INDEX_TOPIC)?.sync.kind !== "live") {
      return false;
    }
    const found = findWorkspace(snapshot.index.workspaces.values(), workspace);
    if (found === undefined) {
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
  if (open()) {
    return;
  }
  const stop = host.frames.subscribe(() => {
    if (open()) {
      stop();
    }
  });
}
