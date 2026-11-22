/**
 * Closing a tab of a view's layout (UI-375): the tab is a node of `views.json`, so closing it writes the file without
 * it (every window then shows the new layout), and the success toast's Undo writes back the manifest it replaced
 * while the file is still what the close wrote (UI-376). A window's closes and Undos run one after the other, each
 * once the window shows what the one before wrote, so each starts from the current file; each write names the file it
 * was made from, so the daemon refuses it when another window or an agent changed the file meanwhile (DMN-364).
 */

import { errorCode } from "../sdk/files.ts";
import { type OmfpHost, reportError, reportSuccess } from "../sdk/index.ts";
import { sameJson } from "../sdk/json.ts";
import { strings } from "../strings/index.ts";
import { manifestWithoutTab } from "./edit.ts";
import type { LayoutTree } from "./layout.ts";
import { DEFAULT_MANIFEST } from "./views.ts";

const text = strings.shell.layout;

/** How long a write waits for the window to show the file it wrote before the next edit runs anyway (the daemon
 * publishes before it answers, the frame store shows it at its next frame). */
const SETTLE_MS = 3000;

/** The tab to close: the view, the identity keys of the tab's node from the view's root (`keysAt`) and the node as
 * the window shows it, with the title its toast names. */
export interface LayoutTab {
  readonly workspaceId: string;
  readonly viewId: string;
  readonly keys: readonly string[];
  readonly node: LayoutTree;
  readonly title: string;
}

/** Each window's last layout edit; the next one waits for it. */
const queues = new WeakMap<OmfpHost, Promise<unknown>>();

/** Runs `edit` once this window's earlier layout edits are done. */
function enqueue(host: OmfpHost, edit: () => Promise<boolean>): Promise<boolean> {
  const run = (queues.get(host) ?? Promise.resolve()).then(edit);
  queues.set(host, run);
  return run;
}

/** The manifest the window shows for `workspaceId`: `null` without a file (or before the views topic answers). */
function shown(host: OmfpHost, workspaceId: string): unknown {
  return host.frames.getSnapshot().views.get(workspaceId)?.manifest ?? null;
}

/** Resolves once the window no longer shows `before` for `workspaceId`, or after {@link SETTLE_MS}. */
function settled(host: OmfpHost, workspaceId: string, before: unknown): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  const moved = () => !sameJson(shown(host, workspaceId), before);
  if (moved()) {
    resolve();
    return promise;
  }
  const unsubscribe = host.frames.subscribe(() => {
    if (moved()) {
      done();
    }
  });
  const timer = setTimeout(() => done(), SETTLE_MS);
  function done() {
    clearTimeout(timer);
    unsubscribe();
    resolve();
  }
  return promise;
}

/**
 * Writes `after` as `workspaceId`'s `views.json` if it still holds `before` (`null`: while there is no file), the file
 * `after` was made from, then waits until the window shows the change; a refusal because the file changed meanwhile
 * reports `stale`, other failures their own error.
 */
async function write(
  host: OmfpHost,
  workspaceId: string,
  edit: { readonly before: unknown; readonly after: unknown; readonly stale: string },
): Promise<boolean> {
  const { before, after, stale } = edit;
  try {
    await host.client.call("views.set", { workspaceId, manifest: after, expected: before });
  } catch (error) {
    reportError(host, errorCode(error) === "conflict" ? stale : error);
    return false;
  }
  await settled(host, workspaceId, before);
  return true;
}

async function close(host: OmfpHost, tab: LayoutTab): Promise<boolean> {
  const source = host.frames.getSnapshot().views.get(tab.workspaceId);
  if (source === undefined) {
    reportError(host, text.loading);
    return false;
  }
  const manifest = manifestWithoutTab(source, tab.viewId, tab.keys, tab.node);
  if (!manifest.ok) {
    reportError(host, manifest.error);
    return false;
  }
  const before = source.manifest ?? null;
  const closed = { before, after: manifest.value, stale: text.closeStale(tab.title) };
  if (!(await write(host, tab.workspaceId, closed))) {
    return false;
  }
  const undo = (): Promise<boolean> => {
    // Restoring over a file changed since would undo someone else's change too.
    if (!sameJson(shown(host, tab.workspaceId), manifest.value)) {
      reportError(host, text.undoStale(tab.title));
      return Promise.resolve(false);
    }
    const restored = { before: manifest.value, after: before ?? DEFAULT_MANIFEST, stale: text.undoStale(tab.title) };
    return write(host, tab.workspaceId, restored);
  };
  reportSuccess(host, text.closed(tab.title), () => void enqueue(host, undo));
  return true;
}

/**
 * Closes `tab` once this window's earlier layout edits are done: writes `views.json` without it with `views.set`, then
 * shows a success toast whose Undo writes the file back the same way. Refusals and failures go to the frame's error
 * list. Resolves to whether the file was written.
 */
export function closeLayoutTab(host: OmfpHost, tab: LayoutTab): Promise<boolean> {
  return enqueue(host, () => close(host, tab));
}
