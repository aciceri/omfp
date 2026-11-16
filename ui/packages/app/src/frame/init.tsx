/**
 * The init files in the window (UI-260..UI-263): imports the user's `init.ts` (its
 * version on the `index` topic) and the current workspace's `.omfp/init.ts` (on its `views/<id>` topic) from the
 * daemon's widget base, again when a version or the current workspace changes; gives them the current workspace, its
 * `views.json` and repositories (UI-267); hands thread changes to their listeners; follows the `jobs/<id>` of every
 * workspace whose jobs the index counts and hands each run event to their listeners once (UI-748); and shows the
 * banner of a file that failed. What the files add (bindings, views, panels) the frame reads with {@link useInit}.
 */

import { jobsTopic, type ThreadSummary, type ViewsState } from "@omfp/protocol";
import { type ReactNode, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import {
  type InitError,
  type InitRepo,
  type InitSnapshot,
  runEventKinds,
  useHost,
  useSnapshot,
  useThreads,
  useWorkspaces,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { useWidgetBase } from "../view/slot.tsx";
import { threadNotices } from "./notifications.ts";

const text = strings.init;

/** The URL of an init file (DMN-185): `<widgetBase>/init/<user | workspace id>/<version>/init.ts`. */
export function initUrl(widgetBase: string, scope: string, version: string): string {
  return `${widgetBase}/init/${encodeURIComponent(scope)}/${encodeURIComponent(version)}/init.ts`;
}

/** What the init files add to the window, live. */
export function useInit(): InitSnapshot {
  const { init } = useHost();
  return useSyncExternalStore(init.subscribe, init.getSnapshot);
}

/** Loads the init files of the window and of `workspaceId` (its views topic: `views`), and feeds their listeners. */
export function InitLoader({
  workspaceId,
  views,
}: {
  readonly workspaceId: string | undefined;
  readonly views: ViewsState | undefined;
}): null {
  const { init } = useHost();
  const base = useWidgetBase();
  const user = useSnapshot((snapshot) => snapshot.index.init);
  const workspace = views?.init ?? null;
  const repos = useSnapshot((snapshot) =>
    workspaceId === undefined ? undefined : snapshot.index.workspaces.get(workspaceId)?.repos,
  );
  // Only what `omfp.workspace.repos` gives: a new branch or commit must not run the files again.
  const listed = (repos ?? []).map(({ name, path, kind }): InitRepo => ({ name, path, kind }));
  const reposKey = JSON.stringify(listed);
  const manifest = views?.manifest;
  const manifestError = views?.error;
  // biome-ignore lint/correctness/useExhaustiveDependencies: `reposKey` stands for `listed`
  const context = useMemo(
    () => ({ workspaceId, manifest, manifestError, repos: listed }),
    [workspaceId, manifest, manifestError, reposKey],
  );
  // Before the loads, so a run reads this context; the user's file first: loads run in call order, and effects in
  // declaration order.
  useEffect(() => init.setContext(context), [init, context]);
  useEffect(() => {
    if (base !== undefined) {
      void init.load("user", user === null ? undefined : initUrl(base, "user", user));
    }
  }, [init, base, user]);
  useEffect(() => {
    if (base !== undefined) {
      const url = workspaceId === undefined || workspace === null ? undefined : initUrl(base, workspaceId, workspace);
      void init.load("workspace", url, workspaceId);
    }
  }, [init, base, workspaceId, workspace]);
  const threads = useThreads();
  const previous = useRef<ReadonlyMap<string, ThreadSummary> | undefined>(undefined);
  useEffect(() => {
    const before = previous.current;
    previous.current = threads;
    if (before === undefined) {
      return;
    }
    for (const { threadId, tone } of threadNotices(before, threads)) {
      const thread = threads.get(threadId);
      if (thread !== undefined) {
        init.emit(tone, thread);
      }
    }
  }, [threads, init]);
  useRunEvents();
  return null;
}

/**
 * Follows `jobs/<id>` of every workspace whose `jobs` the index carries (PRT-210) and hands each run event to the init
 * files' listeners (UI-748) as the model applies it — once, never one of a snapshot, and without waiting for a frame
 * (a background tab renders none).
 */
function useRunEvents(): void {
  const { init, topics, client } = useHost();
  const workspaces = useWorkspaces();
  const counted = [...workspaces.values()]
    .filter((workspace) => workspace.jobs !== undefined && workspace.jobs !== null)
    .map((workspace) => workspace.id)
    .join(" ");
  useEffect(() => {
    const releases = counted === "" ? [] : counted.split(" ").map((id) => topics.retain(jobsTopic(id)));
    return () => {
      for (const release of releases) {
        release();
      }
    };
  }, [topics, counted]);
  useEffect(
    () =>
      client.model.onRunEvent((_workspaceId, { kind, run }) => {
        for (const event of runEventKinds(kind, run)) {
          init.emit(event, run);
        }
      }),
    [client, init],
  );
}

/** One banner per init file that failed (UI-263); nothing while they work. */
export function InitErrors({ errors }: { readonly errors: readonly InitError[] }): ReactNode {
  return errors.map(({ file, message }) => (
    <div key={file} className="omfp-banner" data-tone="error" role="alert" data-testid="init-error" data-file={file}>
      {text.banner(text.files[file], message)}
    </div>
  ));
}
