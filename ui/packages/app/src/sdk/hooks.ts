/**
 * The widget SDK's hooks (`omfp/sdk`, docs/specs/08-agent-control.md): every widget, built-in or not,
 * reads daemon state and acts through these, never through the client directly.
 *
 * - Model: {@link useSnapshot} and its shorthands read the per-frame snapshot (UI-012); results are immutable
 *   references, stable while unchanged.
 * - Topics: {@link useThread}, {@link useWorkspace}, {@link useViews} and {@link useTopic} follow their topic while mounted
 *   (reference-counted across widgets), with the protocol's replay/snapshot semantics.
 * - Calls: {@link useCall} is the full protocol (`call("thread.prompt", { threadId, text })`); `useCall(method, params)`
 *   its result, shown again from the window's last results while a new mount refreshes it (UI-392).
 * - Window: {@link useWindowState} for window-local state shared by key; {@link useCommand} registers a command for
 *   the keymap and the `:` line; {@link useContextMenu} adds items to the context menus (UI-319); {@link useAnnounce}
 *   speaks through the live region.
 */

import type { CallMethod, ConnectionStatus, Index, ThreadView, TopicState } from "@omfp/client";
import {
  type ParamsOf,
  type RepoStatus,
  type ResultOf,
  type ThreadSummary,
  type Topic,
  threadTopic,
  type ViewsState,
  viewsTopic,
  type Workspace,
  workspaceTopic,
} from "@omfp/protocol";
import { useCallback, useContext, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { Command } from "./commands.ts";
import type { ContextMenu } from "./context-menu.ts";
import type { Snapshot } from "./frame-store.ts";
import { type OmfpHost, type SdkClient, useHost } from "./host.tsx";
import type { LiveClient, LiveSnapshot } from "./live.ts";
import { useSlotReport } from "./reports.ts";
import { type ResultCache, resultKey } from "./results.ts";
import { WidgetScope } from "./widget.ts";
import type { Update, WindowKey } from "./window-store.ts";

/** `select` applied to the latest frame's snapshot; re-renders only when its result changes (by identity). */
export function useSnapshot<T>(select: (snapshot: Snapshot) => T): T {
  const { frames } = useHost();
  return useSyncExternalStore(frames.subscribe, () => select(frames.getSnapshot()));
}

export function useConnection(): ConnectionStatus {
  return useSnapshot((snapshot) => snapshot.connection);
}

/** The `index` topic (the frame follows it for the whole window). */
export function useIndex(): Index {
  return useSnapshot((snapshot) => snapshot.index);
}

/** Thread summaries by id, in creation order. */
export function useThreads(): ReadonlyMap<string, ThreadSummary> {
  return useSnapshot((snapshot) => snapshot.index.threads);
}

export function useThreadSummary(threadId: string | undefined): ThreadSummary | undefined {
  return useSnapshot((snapshot) => (threadId === undefined ? undefined : snapshot.index.threads.get(threadId)));
}

export function useWorkspaces(): ReadonlyMap<string, Workspace> {
  return useSnapshot((snapshot) => snapshot.index.workspaces);
}

/** Follows `topic` while mounted; its subscription state. */
export function useTopic(topic: Topic | undefined): TopicState | undefined {
  const { topics } = useHost();
  useEffect(() => (topic === undefined ? undefined : topics.retain(topic)), [topics, topic]);
  return useSnapshot((snapshot) =>
    topic === undefined ? undefined : snapshot.topics.find((state) => state.topic === topic),
  );
}

/** One thread's reducer state (UI-010), following `thread/<id>` while mounted (and a while after, among the recent
 * threads, UI-335); `undefined` until its snapshot. */
export function useThread(threadId: string | undefined): ThreadView | undefined {
  useTopic(threadId === undefined ? undefined : threadTopic(threadId));
  return useSnapshot((snapshot) => (threadId === undefined ? undefined : snapshot.threads.get(threadId)));
}

/** A workspace from the index with the git status of its repositories (`workspace/<id>`, followed while mounted). */
export interface WorkspaceData {
  readonly workspace: Workspace | undefined;
  /** `undefined` until the topic's snapshot arrives. */
  readonly status: readonly RepoStatus[] | undefined;
}

export function useWorkspace(workspaceId: string | undefined): WorkspaceData {
  useTopic(workspaceId === undefined ? undefined : workspaceTopic(workspaceId));
  const workspace = useSnapshot((snapshot) =>
    workspaceId === undefined ? undefined : snapshot.index.workspaces.get(workspaceId),
  );
  const status = useSnapshot((snapshot) =>
    workspaceId === undefined ? undefined : snapshot.workspaces.get(workspaceId),
  );
  return { workspace, status };
}

/** A workspace's views manifest, widget folders and widget state (PRT-040), following `views/<id>` while mounted;
 * `undefined` until its snapshot. */
export function useViews(workspaceId: string | undefined): ViewsState | undefined {
  useTopic(workspaceId === undefined ? undefined : viewsTopic(workspaceId));
  return useSnapshot((snapshot) => (workspaceId === undefined ? undefined : snapshot.views.get(workspaceId)));
}

/** What `useCall(method, params)` gives (UI-392). */
export interface CallState<T> {
  /** The latest result: this mount's, else the window's last one of the same widget, method and params while the
   * request runs; `undefined` before the first. A failure keeps it. */
  readonly data: T | undefined;
  /** Why the latest request failed (a `CallError` for the daemon's errors), `undefined` once one succeeds. */
  readonly error: unknown;
  /** A request is on its way: `data`, when there is one, is the previous result. */
  readonly refreshing: boolean;
  /** Calls again, keeping `data` until the new result arrives. */
  refresh(): void;
}

interface Fetched {
  /** The request the state is of ({@link resultKey}); `undefined` without params. */
  readonly key: string | undefined;
  readonly data: unknown;
  readonly error: unknown;
  readonly refreshing: boolean;
}

/** The state a request starts in: the window's last result for `key`, refreshing. */
function startOf(results: ResultCache, key: string | undefined): Fetched {
  return {
    key,
    data: key === undefined ? undefined : results.get(key)?.value,
    error: undefined,
    refreshing: key !== undefined,
  };
}

/**
 * The protocol. `useCall()` gives `call(method, params)`, which resolves to the typed result or rejects with
 * `CallError`. `useCall(method, params)` calls `method` whenever the params change (by JSON), on every mount and on
 * `refresh()`, and gives its {@link CallState}: on a new mount the window's last result of the same widget instance,
 * method and params shows at once, `refreshing`, until the new one replaces it (UI-392). `params` `null`: no call.
 */
export function useCall(): SdkClient["call"];
export function useCall<M extends CallMethod>(method: M, params: ParamsOf<M> | null): CallState<ResultOf<M>>;
export function useCall<M extends CallMethod>(
  method?: M,
  params?: ParamsOf<M> | null,
): SdkClient["call"] | CallState<ResultOf<M>> {
  const { client, results } = useHost();
  const call = useCallback<SdkClient["call"]>((name, args) => client.call(name, args), [client]);
  const scope = useContext(WidgetScope);
  const request = method === undefined || params === null || params === undefined ? undefined : JSON.stringify(params);
  const key = method === undefined || request === undefined ? undefined : resultKey(scope, method, request);
  const [fetched, setFetched] = useState<Fetched>(() => startOf(results, key));
  let state = fetched;
  if (fetched.key !== key) {
    // New params: their last result, if any, rather than the previous params' result.
    state = startOf(results, key);
    setFetched(state);
  }
  const [round, setRound] = useState(0);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `round` (refresh) calls again.
  useEffect(() => {
    if (method === undefined || request === undefined || key === undefined) {
      return;
    }
    let live = true;
    // Shown again (a hidden view, UI-390) or refreshed: the shown result is the previous one until this one answers.
    setFetched((previous) =>
      previous.key !== key || previous.refreshing ? previous : { ...previous, refreshing: true },
    );
    // `request` is the JSON of the params the key was made of.
    client.call(method, JSON.parse(request) as ParamsOf<M>).then(
      (data) => {
        results.set(key, data);
        if (live) {
          setFetched({ key, data, error: undefined, refreshing: false });
        }
      },
      (error: unknown) => {
        if (live) {
          setFetched((previous) => ({
            ...(previous.key === key ? previous : startOf(results, key)),
            error,
            refreshing: false,
          }));
        }
      },
    );
    return () => {
      live = false;
    };
  }, [client, results, method, request, key, round]);
  const refresh = useCallback(() => setRound((count) => count + 1), []);
  const callState = useMemo(
    (): CallState<ResultOf<M>> => ({
      // The result of `method` (or its cached copy), kept untyped.
      data: state.data as ResultOf<M> | undefined,
      error: state.error,
      refreshing: state.refreshing,
      refresh,
    }),
    [state, refresh],
  );
  return method === undefined ? call : callState;
}

/** A slot of window-local state and its setter, shared by every widget using the same key. Pass a stable key
 * (module constant, or a memoized factory such as `draftKey`). */
export function useWindowState<T>(key: WindowKey<T>): [T, (update: Update<T>) => void] {
  const { window } = useHost();
  const value = useSyncExternalStore(
    useCallback((listener: () => void) => window.subscribe(key, listener), [window, key]),
    () => window.get(key),
  );
  const set = useCallback((update: Update<T>) => window.set(key, update), [window, key]);
  return [value, set];
}

/**
 * Registers `command` while mounted. The latest `command` object is the one run, so it may close over fresh state;
 * the registration itself changes only with `id`, `scope`, `title` or `description`. Inside a workspace or user widget
 * the command carries the widget's reference (UI-351). Init files and other code outside React use its sibling
 * {@link registerCommand} (UI-261).
 */
export function useCommand(command: Command): void {
  const { commands } = useHost();
  const widget = useSlotReport()?.identity.reference;
  const latest = useRef(command);
  latest.current = command;
  const { id, scope, title, description } = command;
  useEffect(() => {
    const { aliases } = latest.current;
    return commands.register({
      id,
      title,
      ...(description === undefined ? {} : { description }),
      ...(scope === undefined ? {} : { scope }),
      ...(aliases === undefined ? {} : { aliases }),
      ...(widget === undefined ? {} : { widget }),
      args: (before) => latest.current.args?.(before) ?? [],
      enabled: (args) => latest.current.enabled?.(args) ?? true,
      run: (args, element) => latest.current.run(args, element),
    });
  }, [commands, id, scope, title, description, widget]);
}

/** Registers `command` in `host`'s registry until the returned function is called: {@link useCommand} outside React,
 * for init files (UI-261) and frame-level code. */
export function registerCommand(host: OmfpHost, command: Command): () => void {
  return host.commands.register(command);
}

/**
 * Adds `menu`'s items to the context menus of its target kind while mounted (UI-319): `{ on: "message" | "file" |
 * "thread", items: [<command id or alias> | { command, title? }] }`, after the built-in items and those added before.
 * The registration changes only when the menu's content does.
 */
export function useContextMenu(menu: ContextMenu): void {
  const { contextMenus } = useHost();
  const content = JSON.stringify(menu);
  useEffect(() => contextMenus.register(JSON.parse(content) as ContextMenu), [contextMenus, content]);
}

/** Announces `message` through the window's ARIA live region (UI-092). */
export function useAnnounce(): (message: string) => void {
  const { announcer } = useHost();
  return useCallback((message: string) => announcer.announce(message), [announcer]);
}

/** This window's live voice session (UI-180..UI-186) and its latest snapshot. */
export function useLive(): [LiveClient, LiveSnapshot] {
  const { live } = useHost();
  return [live, useSyncExternalStore(live.subscribe, live.getSnapshot)];
}
