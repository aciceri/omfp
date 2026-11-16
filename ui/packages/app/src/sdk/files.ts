/**
 * Files in the SDK (AGT-027, UI-110..117): `useFile` reads a workspace file through the daemon and follows its changes,
 * `useFileUrl` gives the URL of its raw bytes (UI-540), `useOpenFile` opens one in a file tab of the view's `main` slot
 * (or the frame's floating panel), `useMissingFile` finds the file a path that does not exist names
 * (`workspaces.find_file`, DMN-092), and the window keys the file tabs, the floating panel, the file picker and the
 * mode line read.
 */

import { CallError } from "@omfp/client";
import type { VimMode } from "@omfp/kit";
import type { FileCandidate, FileContent } from "@omfp/protocol";
import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { strings } from "../strings/index.ts";
import { MAIN_SLOT } from "../view/layout.ts";
import {
  closeOpenedTab,
  decodeFloatingPanel,
  decodeOpenFiles,
  type FileTabs,
  FLOATING_PANEL_DEFAULT,
  type FloatingPanel,
  fileTabId,
  NO_FILE_TABS,
  neighbourTab,
  type OpenFileOptions,
  type OpenFiles,
  openTab,
  retargetTab,
} from "../vm/files.ts";
import { replaceTab } from "../vm/tab-choices.ts";
import { currentWorkspaceId, errorMessage, reportError, useCurrentWorkspaceId } from "./actions.ts";
import { useCall, useSnapshot, useWindowState, useWorkspace } from "./hooks.ts";
import { type OmfpHost, useHost } from "./host.tsx";
import { PENDING_TAB, SELECTED_THREAD, SHOWN_VIEW, TAB_CHOICES } from "./keys.ts";
import { type ResultCache, resultKey } from "./results.ts";
import { chosenTabOf, selectTab, useChosenTab } from "./tab-choices.ts";
import { WidgetScope } from "./widget.ts";
import { type WindowKey, windowKey } from "./window-store.ts";

/**
 * The open file tabs of every workspace in this window, with the widget tabs reveals opened in the `main` slot, kept
 * in `localStorage` (UI-115, UI-373).
 */
export const OPEN_FILES = windowKey<OpenFiles>("openFiles", {}, decodeOpenFiles);

/** Where the floating file panel sits and whether it is minimised, kept in `localStorage` (UI-115). */
export const FLOATING_FILES = windowKey<FloatingPanel>("floatingFiles", FLOATING_PANEL_DEFAULT, decodeFloatingPanel);

/** Where a floating widget panel (UI-230) of a workspace sits and whether it is minimised, kept in `localStorage`. */
export function floatingWidgetKey(workspaceId: string, panelId: string): WindowKey<FloatingPanel> {
  const name = `floatingWidget/${workspaceId}/${panelId}`;
  let key = FLOATING_WIDGET_KEYS.get(name);
  if (key === undefined) {
    key = windowKey<FloatingPanel>(name, FLOATING_PANEL_DEFAULT, decodeFloatingPanel);
    FLOATING_WIDGET_KEYS.set(name, key);
  }
  return key;
}

/** One key object per panel, so hooks see a stable key. */
const FLOATING_WIDGET_KEYS = new Map<string, WindowKey<FloatingPanel>>();

/** A path that does not exist, with the files it may name, for the frame's file picker (UI-117). */
export interface FilePicker {
  readonly workspaceId: string;
  /** The path as it was opened. */
  readonly path: string;
  readonly options: OpenFileOptions;
  readonly candidates: readonly FileCandidate[];
}

/** The file picker the frame shows; `undefined` when none is open (UI-117). */
export const FILE_PICKER = windowKey<FilePicker | undefined>("filePicker", undefined);

/** vim's mode while a file editor has the focus, for the mode line (UI-112); `undefined` otherwise. */
export const EDITOR_MODE = windowKey<VimMode | undefined>("editorMode", undefined);

/** How often an open file is checked for changes (UI-110). */
export const FILE_POLL_MS = 2000;

/** Runs `check` periodically until the returned function is called; the host's default is every 2 s. */
export type FilePoll = (check: () => void) => () => void;

export const intervalPoll: FilePoll = (check) => {
  const timer = setInterval(check, FILE_POLL_MS);
  return () => clearInterval(timer);
};

/** Opens `path` in this window's current workspace, or in `options.workspaceId`'s (UI-115); without a workspace it
 * reports why. */
export function openFile(host: OmfpHost, path: string, options: OpenFileOptions = {}): void {
  const workspaceId = options.workspaceId ?? currentWorkspaceId(host);
  if (workspaceId === undefined) {
    reportError(host, new Error(strings.shell.files.noWorkspace(path)));
    return;
  }
  host.window.set(OPEN_FILES, (files) => ({
    ...files,
    [workspaceId]: openTab(files[workspaceId] ?? NO_FILE_TABS, path, options),
  }));
  selectOpenedTab(host, workspaceId, fileTabId(path));
}

/** The view the frame renders for the selected thread; `undefined` while it renders another thread (a switch not
 * rendered yet) or none, and, with `ready`, while it is the built-in view standing in until the workspace's views
 * arrive. */
function shownViewId(host: OmfpHost, ready: boolean): string | undefined {
  const shown = host.window.get(SHOWN_VIEW);
  return shown !== undefined && (shown.ready || !ready) && shown.threadId === host.window.get(SELECTED_THREAD)
    ? shown.viewId
    : undefined;
}

/**
 * Selects the opened tab `tabId` of `workspaceId`'s `main` slot (a file's or a revealed widget's, UI-115, UI-373) in
 * the view shown, for the selected thread only (else the draft thread, UI-372); right after a thread switch, or while
 * the workspace's views load, once the frame renders that thread's own view ({@link PENDING_TAB}). Another
 * workspace's tab selects nothing: no thread of it is shown.
 */
export function selectOpenedTab(host: OmfpHost, workspaceId: string, tabId: string): void {
  if (workspaceId !== currentWorkspaceId(host)) {
    return;
  }
  const viewId = shownViewId(host, true);
  if (viewId === undefined) {
    host.window.set(PENDING_TAB, { threadId: host.window.get(SELECTED_THREAD), tabId });
  } else {
    selectTab(host, viewId, MAIN_SLOT, tabId);
  }
}

/**
 * Closes the opened tab `tabId` of `workspaceId` (a file's or a revealed widget's): when the shown thread had it
 * selected in `viewId` (the view, or floating panel, whose tab bar closed it; by default the view shown), its
 * neighbour is (UI-366), and the other threads of the workspace that had it selected go back to their first tab
 * (UI-372).
 */
export function closeOpened(
  host: OmfpHost,
  workspaceId: string,
  tabId: string,
  viewId: string | undefined = shownViewId(host, false),
): void {
  const tabs = host.window.get(OPEN_FILES)[workspaceId];
  if (tabs === undefined || closeOpenedTab(tabs, tabId) === tabs) {
    return;
  }
  const shown = workspaceId === currentWorkspaceId(host) ? viewId : undefined;
  const selected = shown !== undefined && chosenTabOf(host, shown, MAIN_SLOT) === tabId;
  host.window.set(OPEN_FILES, (files) => ({ ...files, [workspaceId]: closeOpenedTab(tabs, tabId) }));
  host.window.set(TAB_CHOICES, (choices) =>
    replaceTab(choices, workspaceId, threadsOf(host, workspaceId), tabId, undefined),
  );
  if (selected) {
    selectTab(host, shown, MAIN_SLOT, neighbourTab(tabs, tabId));
  }
}

/** Whether a thread is one of `workspaceId`'s, by the live index. */
function threadsOf(host: OmfpHost, workspaceId: string): (threadId: string) => boolean {
  const { threads } = host.frames.getSnapshot().index;
  return (threadId) => threads.get(threadId)?.workspaceId === workspaceId;
}

/** `openFile(path, { line?, diff? })` for this window (AGT-027); an absolute path outside the workspace folder opens
 * too, read-only (UI-700, UI-701). */
export function useOpenFile(): (path: string, options?: OpenFileOptions) => void {
  const host = useHost();
  return useMemo(() => (path: string, options?: OpenFileOptions) => openFile(host, path, options), [host]);
}

/** What became of a path that does not exist (UI-117): `opened` its only candidate, `choosing` among several in the
 * frame's picker, `none` found. */
export type MissingOutcome = "opened" | "choosing" | "none";

/**
 * Looks for the file `path` names in the current workspace (`workspaces.find_file`, DMN-092; the selected thread's tool
 * cards rank first): one candidate takes the place of `path`'s tab (or opens), several open the frame's picker.
 */
export async function findMissingFile(
  host: OmfpHost,
  path: string,
  options: OpenFileOptions = {},
): Promise<MissingOutcome> {
  const workspaceId = currentWorkspaceId(host);
  if (workspaceId === undefined) {
    return "none";
  }
  const threadId = host.window.get(SELECTED_THREAD);
  const found = await host.client.call("workspaces.find_file", {
    workspaceId,
    query: path,
    ...(threadId === undefined ? {} : { threadId }),
  });
  const [only] = found.candidates;
  if (only === undefined) {
    return "none";
  }
  if (found.candidates.length === 1) {
    chooseFile(host, workspaceId, path, only.path, options);
    return "opened";
  }
  host.window.set(FILE_PICKER, { workspaceId, path, options, candidates: found.candidates });
  return "choosing";
}

/**
 * Opens `chosen` in place of the tab of `path` (UI-117), closing the picker: the threads that selected `path`'s tab
 * select `chosen`'s (UI-372), whichever thread is shown when the lookup answers. Without a tab of `path`, `chosen`
 * opens as `openFile` opens it.
 */
export function chooseFile(
  host: OmfpHost,
  workspaceId: string,
  path: string,
  chosen: string,
  options: OpenFileOptions = {},
): void {
  host.window.set(FILE_PICKER, undefined);
  const tabbed = (host.window.get(OPEN_FILES)[workspaceId] ?? NO_FILE_TABS).tabs.some((tab) => tab.path === path);
  host.window.set(OPEN_FILES, (files) => ({
    ...files,
    [workspaceId]: retargetTab(files[workspaceId] ?? NO_FILE_TABS, path, chosen, options),
  }));
  if (tabbed) {
    host.window.set(TAB_CHOICES, (choices) =>
      replaceTab(choices, workspaceId, threadsOf(host, workspaceId), fileTabId(path), fileTabId(chosen)),
    );
  } else {
    selectOpenedTab(host, workspaceId, fileTabId(chosen));
  }
}

/** Closes the picker without a choice: the opening is cancelled, so the tab of the missing `path` closes (UI-117). */
export function cancelFilePicker(host: OmfpHost): void {
  const picker = host.window.get(FILE_PICKER);
  if (picker === undefined) {
    return;
  }
  host.window.set(FILE_PICKER, undefined);
  closeOpened(host, picker.workspaceId, fileTabId(picker.path));
}

/**
 * While `missing` (the file `path` does not exist as given), looks for the file it names once (UI-117): `undefined`
 * until it is missing, then `finding`, then the {@link MissingOutcome}.
 */
export function useMissingFile(
  path: string,
  missing: boolean,
  options: OpenFileOptions = {},
): "finding" | MissingOutcome | undefined {
  const host = useHost();
  const [outcome, setOutcome] = useState<{ readonly path: string; readonly found: MissingOutcome }>();
  const { line, diff } = options;
  useEffect(() => {
    if (!missing) {
      return;
    }
    let live = true;
    findMissingFile(host, path, { line, diff }).then(
      (found) => {
        if (live) {
          setOutcome({ path, found });
        }
      },
      (error: unknown) => {
        if (live) {
          setOutcome({ path, found: "none" });
          reportError(host, error);
        }
      },
    );
    return () => {
      live = false;
    };
  }, [host, path, missing, line, diff]);
  if (!missing) {
    return undefined;
  }
  return outcome?.path === path ? outcome.found : "finding";
}

/** A workspace's opened tabs in this window, with the one selected in the view shown and what changes them. */
export interface FileTabsState extends FileTabs {
  /** The tab the selected thread (else the draft thread) selected in the `main` slot (or the floating file panel) of
   * the view shown (UI-372); `undefined`: the slot's first tab. */
  readonly active: string | undefined;
  /** Selects a tab by id (a file's `fileTabId`, a widget's `widgetTabId`, a slot child's layout path) for the
   * selected thread in the view shown; `undefined`: the slot's first tab. */
  select(id: string | undefined): void;
  /** Closes the file `path`. */
  close(path: string): void;
  /** Closes an opened tab by id (a file's `fileTabId` or a widget's `widgetTabId`). */
  closeTab(id: string): void;
}

export function useFileTabs(workspaceId: string | undefined): FileTabsState {
  const host = useHost();
  const [files] = useWindowState(OPEN_FILES);
  const [shown] = useWindowState(SHOWN_VIEW);
  const [active, select] = useChosenTab(shown?.viewId, MAIN_SLOT);
  const tabs = workspaceId === undefined ? NO_FILE_TABS : (files[workspaceId] ?? NO_FILE_TABS);
  return useMemo(
    () => ({
      ...tabs,
      active,
      select,
      close: (path) => {
        if (workspaceId !== undefined) {
          closeOpened(host, workspaceId, fileTabId(path));
        }
      },
      closeTab: (id) => {
        if (workspaceId !== undefined) {
          closeOpened(host, workspaceId, id);
        }
      },
    }),
    [host, tabs, active, select, workspaceId],
  );
}

export type FileLoadStatus = "loading" | "ready" | "missing" | "error";

/** The outcome of {@link FileState.save}. */
export type SaveResult =
  | { readonly ok: true; readonly file: FileContent }
  /** `conflict`: the file no longer has the expected hash (DMN-090); nothing was written. */
  | { readonly ok: false; readonly conflict: boolean; readonly error: string };

export interface FileState {
  readonly status: FileLoadStatus;
  /** The latest version read (`undefined` while loading, missing or failed). */
  readonly file: FileContent | undefined;
  readonly error: string | undefined;
  /** The first read since the widget mounted (or the path changed) is on its way: `file`, when `status` is `ready`,
   * is the window's last copy, read by an earlier mount (UI-392). */
  readonly refreshing: boolean;
  /** Reads the file again now, `HEAD` text included. */
  reload(): void;
  /** Writes `content` if the file still has `expectedHash` (default: the hash of `file`). */
  save(content: string, expectedHash?: string): Promise<SaveResult>;
}

/** The daemon's error code of a failed call (`not_found`, `conflict`, …); `undefined` for other failures. */
export function errorCode(error: unknown): string | undefined {
  return error instanceof CallError && error.failure.kind === "daemon" ? error.failure.error.code : undefined;
}

interface Loaded {
  /** The file's cache key ({@link resultKey}): the workspace, widget and path the state is of. */
  readonly key: string;
  readonly status: FileLoadStatus;
  readonly file: FileContent | undefined;
  readonly error: string | undefined;
  readonly refreshing: boolean;
}

/** What a mount shows before its first read answers: the window's last copy of the file, else `loading`. */
function startOf(results: ResultCache, key: string): Loaded {
  const cached = results.get(key)?.value as FileContent | undefined;
  return cached === undefined
    ? { key, status: "loading", file: undefined, error: undefined, refreshing: true }
    : { key, status: "ready", file: cached, error: undefined, refreshing: true };
}

/**
 * A file of the current workspace (`workspaces.read_file`, DMN-089), or, by an absolute path, a file outside its folder,
 * read-only (`file.outside`, DMN-310). With `follow` (the default) it is checked again
 * by the host's poll (`knownHash`, so an unchanged file costs no transfer) and read again whenever the workspace's git
 * status changes (`workspace/<id>`), which also refreshes its `HEAD` text. Until the first read answers, the window's
 * last copy read by the same widget instance shows, `refreshing` (UI-392).
 */
export function useFile(path: string, { follow = true }: { readonly follow?: boolean } = {}): FileState {
  const host = useHost();
  const call = useCall();
  const workspaceId = useCurrentWorkspaceId();
  const { status: gitStatus } = useWorkspace(follow ? workspaceId : undefined);
  const key = resultKey(useContext(WidgetScope), "workspaces.read_file", `${workspaceId ?? ""}\u0000${path}`);
  const [loaded, setLoaded] = useState<Loaded>(() => startOf(host.results, key));
  const current = useRef<FileContent | undefined>(undefined);
  /** The key {@link current} belongs to. */
  const currentKey = useRef<string | undefined>(undefined);
  // Results of an older path or workspace are dropped.
  const generation = useRef(0);
  const read = useCallback(
    async (check: boolean) => {
      const asked = generation.current;
      if (workspaceId === undefined) {
        setLoaded({
          key,
          status: "error",
          file: undefined,
          error: strings.shell.files.noWorkspace(path),
          refreshing: false,
        });
        return;
      }
      const known = check ? current.current?.hash : undefined;
      try {
        const file = await call("workspaces.read_file", {
          workspaceId,
          path,
          ...(known === undefined ? {} : { knownHash: known }),
        });
        if (asked !== generation.current || file.unchanged === true) {
          return;
        }
        current.current = file;
        host.results.set(key, file);
        setLoaded({ key, status: "ready", file, error: undefined, refreshing: false });
      } catch (error) {
        if (asked !== generation.current) {
          return;
        }
        if (errorCode(error) === "not_found") {
          current.current = undefined;
          host.results.delete(key);
          setLoaded({ key, status: "missing", file: undefined, error: errorMessage(error), refreshing: false });
        } else if (!check) {
          setLoaded({ key, status: "error", file: current.current, error: errorMessage(error), refreshing: false });
        }
      }
    },
    [call, host, key, workspaceId, path],
  );

  // On mount, a new path or workspace, and when a hidden view shows again (UI-390): read, showing meanwhile what was
  // shown (the same file) or the window's last copy (another one).
  useEffect(() => {
    generation.current++;
    if (currentKey.current !== key) {
      currentKey.current = key;
      current.current = host.results.get(key)?.value as FileContent | undefined;
    }
    setLoaded((previous) =>
      previous.key !== key
        ? startOf(host.results, key)
        : previous.refreshing
          ? previous
          : { ...previous, refreshing: true },
    );
    void read(false);
  }, [host, key, read]);

  useEffect(() => (follow ? host.filePoll(() => void read(true)) : undefined), [host, follow, read]);

  // A new git status (an agent's edit, a commit, a rescan): read again, `HEAD` text included.
  const seenStatus = useRef(gitStatus);
  useEffect(() => {
    const seen = seenStatus.current;
    seenStatus.current = gitStatus;
    // The topic's first snapshot is no change: the file was just read.
    if (gitStatus !== seen && seen !== undefined) {
      void read(false);
    }
  }, [gitStatus, read]);

  const save = useCallback(
    async (content: string, expectedHash?: string): Promise<SaveResult> => {
      const file = current.current;
      if (workspaceId === undefined || file === undefined) {
        return { ok: false, conflict: false, error: strings.shell.file.loading };
      }
      try {
        const written = await call("workspaces.write_file", {
          workspaceId,
          path: file.path,
          content,
          expectedHash: expectedHash ?? file.hash,
        });
        const saved: FileContent = {
          ...file,
          content,
          hash: written.hash,
          size: written.size,
          unchanged: false,
          truncated: false,
        };
        current.current = saved;
        host.results.set(key, saved);
        setLoaded({ key, status: "ready", file: saved, error: undefined, refreshing: false });
        return { ok: true, file: saved };
      } catch (error) {
        return { ok: false, conflict: errorCode(error) === "conflict", error: errorMessage(error) };
      }
    },
    [call, host, key, workspaceId],
  );

  return useMemo(() => {
    const { status, file, error, refreshing } = loaded;
    return { status, file, error, refreshing, reload: () => void read(false), save };
  }, [loaded, read, save]);
}

/**
 * The URL of the raw bytes of `path` in workspace `workspaceId` (DMN-250): `<fileBase>/<workspaceId>/<path>`, each
 * path segment percent-encoded and the slashes kept, so an absolute path keeps its leading one.
 */
export function fileUrl(fileBase: string, workspaceId: string, path: string): string {
  const encoded = path.split("/").map(encodeURIComponent).join("/");
  return `${fileBase}/${encodeURIComponent(workspaceId)}/${encoded}`;
}

/** The `fileBase` of the latest `system.hello` (PRT-150), kept while reconnecting. */
function useFileBase(): string | undefined {
  const current = useSnapshot((snapshot) =>
    snapshot.connection.kind === "connected" ? snapshot.connection.hello.fileBase : undefined,
  );
  const latest = useRef(current);
  if (current !== undefined) {
    latest.current = current;
  }
  return latest.current;
}

/**
 * The URL of the raw bytes of a file of the current workspace (UI-540, DMN-250), for `<video>`, `<audio>`, `<img>` or
 * `fetch`: `path` as `useFile` takes it (an absolute path outside the workspace folder too, DMN-314), served with its
 * type, byte ranges and cache validators. `undefined` without
 * a current workspace or before the first `system.hello`. The URL carries a secret of this daemon's run instead of
 * the token, so it works where no header can be set; keep it inside the window.
 */
export function useFileUrl(path: string): string | undefined {
  const workspaceId = useCurrentWorkspaceId();
  const base = useFileBase();
  return base === undefined || workspaceId === undefined || path === "" ? undefined : fileUrl(base, workspaceId, path);
}
