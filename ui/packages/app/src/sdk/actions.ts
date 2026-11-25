/**
 * Thread actions shared by widgets, commands and the frame (UI-023, UI-026, UI-030): each is one protocol call plus
 * what the window must forget. Failures land in the {@link ERRORS} slot the frame shows.
 */

import { CallError } from "@omfp/client";
import { compactNumber } from "@omfp/kit";
import type { CloseJobs, RunInfo } from "@omfp/protocol";
import { useMemo } from "react";
import { strings } from "../strings/index.ts";
import { type CompactMode, compactPrompt, tokensBefore, UNFOCUSED_MODE } from "../vm/compaction.ts";
import { expandPastes } from "../vm/paste.ts";
import { adoptDraft, dropDraft } from "../vm/tab-choices.ts";
import { displayTitle } from "../vm/threads.ts";
import { messageEntryId } from "../vm/tree.ts";
import { adoptDraftView, dropDraftView } from "../vm/view-choices.ts";
import { currentWorkspaceId as workspaceOf } from "../vm/workspaces.ts";
import { useThreadSummary, useWindowState } from "./hooks.ts";
import { type OmfpHost, useHost } from "./host.tsx";
import { isObject } from "./json.ts";
import {
  CLOSE_THREAD,
  type CloseRequest,
  DRAFT_CREATING,
  DRAFT_MESSAGE,
  DRAFT_THREAD,
  type DraftThread,
  draftKey,
  ERRORS,
  FOCUS_COMPOSER,
  LOGIN,
  RENAMING_THREAD,
  SELECTED_THREAD,
  SUCCESSES,
  TAB_CHOICES,
  VIEW_CHOICES,
} from "./keys.ts";

export interface ThreadActions {
  /** Shows `threadId` in this window (`undefined`: none); leaving an empty draft thread discards it (UI-023). */
  select(threadId: string | undefined): void;
  /** Shows the draft thread of `workspaceId` (UI-023); a draft started in another workspace moves there with its
   * text. */
  openDraft(workspaceId: string): void;
  /** Drops the draft thread and its text. */
  discardDraft(): void;
  /** Creates the daemon thread with the draft's first message (text, images or both), then selects it. A failed
   * creation keeps the draft; a failed first prompt carrying a surviving thread ID transfers the unsent draft there
   * for retry. */
  sendDraft(): Promise<boolean>;
  /**
   * The draft thread's mic (UI-023, UI-180): opens the microphone at once, then, once `ready` resolves (the images
   * being read are in), creates the daemon thread as {@link ThreadActions.sendDraft} does — the draft's message, if
   * any, its first one — and starts a live session on it (`useLive`'s `start`). A draft left or replaced by another
   * workspace's meanwhile creates nothing. A failure before the thread exists is reported and keeps the draft; once it
   * exists, the thread's live panel shows it (UI-186).
   */
  startDraftLive(ready?: Promise<unknown>): Promise<void>;
  archive(threadId: string): Promise<void>;
  unarchive(threadId: string): Promise<void>;
  /** Immediately deletes the thread and its history, keeping its workspace (UI-026). */
  remove(threadId: string): Promise<void>;
  rename(threadId: string, title: string): Promise<void>;
  stop(threadId: string): Promise<void>;
  restart(threadId: string): Promise<void>;
  /** Continues the turn a daemon restart interrupted (`thread.resume`, UI-933); a refusal or a failed start is
   * reported. */
  resume(threadId: string): Promise<void>;
  /**
   * Forks `threadId` into a new thread of its workspace holding the conversation through entry `entryId` (the whole
   * session without one, `threads.fork`), then shows it with a success toast (UI-666). Resolves to the new thread's
   * id, `undefined` when it failed (reported).
   */
  fork(threadId: string, entryId?: string): Promise<string | undefined>;
  /**
   * {@link ThreadActions.fork} at a transcript message: its entry on the session's current branch, found in
   * `thread.tree` by role and timestamp (UI-661); a message the branch lacks is reported.
   */
  forkAtMessage(threadId: string, message: { role: string; timestamp?: number | null }): Promise<string | undefined>;
  /**
   * Has omp compact the thread's context, with `instructions` (trimmed; blank: none) as the focus of its summary, then
   * shows a success toast (UI-910, UI-911). Without `mode` through `thread.compact`; with one (`soft`, `remote`,
   * `snapcompact`, which takes no focus) by sending omp's `/compact <mode> [focus]` as a prompt, whose outcome omp
   * reports in the transcript: a neutral toast then says it started (UI-913). Refused with an error toast while the
   * thread works (its summary is not settled); a failure is reported. Resolves to whether omp compacted (with a mode:
   * took the command).
   */
  compact(threadId: string, instructions?: string, mode?: CompactMode): Promise<boolean>;
}

/** The message of a failed call or any thrown value; a daemon error carrying omp's stderr tail (a failed start,
 * `thread_failed`, DMN-212) ends with it on the lines after, so omp's own words show (UI-381). */
export function errorMessage(error: unknown): string {
  if (!(error instanceof Error)) {
    return String(error);
  }
  const tail = error instanceof CallError ? stderrTail(error) : undefined;
  return tail === undefined ? error.message : `${error.message}\n${tail}`;
}

/** The `stderrTail` of a daemon error's details, trimmed; `undefined` when it has none. */
function stderrTail({ failure }: CallError): string | undefined {
  if (failure.kind !== "daemon") {
    return undefined;
  }
  const details = failure.error.details;
  const tail = isObject(details) ? details.stderrTail : undefined;
  return typeof tail === "string" && tail.trim() !== "" ? tail.trim() : undefined;
}

/** The last key {@link reportError} gave: never reused, so a new error is never taken for one that just went. */
let lastErrorKey = 0;

/** Adds `error` to the window's error list, as its own entry even when the message repeats, and its messages (UI-521). */
export function reportError(host: OmfpHost, error: unknown): void {
  const text = errorMessage(error);
  lastErrorKey += 1;
  const key = lastErrorKey;
  host.window.set(ERRORS, (errors) => [...errors, { key, text }]);
  host.notices.remember("error", text);
}

/**
 * The outcome of a user action (UI-385, UI-520): a success toast that goes by itself, also announced through the live
 * region and kept in the window's messages (UI-521); with `undo`, the toast offers to take the action back (UI-376).
 */
export function reportSuccess(host: OmfpHost, text: string, undo?: () => void): void {
  host.window.set(SUCCESSES, (shown) => [
    ...shown,
    { key: (shown.at(-1)?.key ?? 0) + 1, text, ...(undo === undefined ? {} : { undo }) },
  ]);
  host.announcer.announce(text);
  host.notices.remember("success", text);
}

/** A user action omp took whose outcome shows elsewhere (UI-913): as {@link reportSuccess}, but a neutral toast. */
export function reportTaken(host: OmfpHost, text: string): void {
  host.window.set(SUCCESSES, (shown) => [...shown, { key: (shown.at(-1)?.key ?? 0) + 1, text, tone: "info" as const }]);
  host.announcer.announce(text);
  host.notices.remember("info", text);
}

/** Asks the frame to log in (UI-382): to `providerId` at once, else through its provider picker. */
export function requestLogin(host: OmfpHost, providerId?: string): void {
  host.window.set(LOGIN, { providerId });
}

export function useReportError(): (error: unknown) => void {
  const host = useHost();
  return useMemo(() => (error: unknown) => reportError(host, error), [host]);
}

/** `reportSuccess` for widgets: the outcome of a user action as a success toast (UI-520). */
export function useReportSuccess(): (text: string) => void {
  const host = useHost();
  return useMemo(() => (text: string) => reportSuccess(host, text), [host]);
}

/** Opening and copying a URL omp opened on the daemon's host (UI-346, UI-347), and relaying the address its OAuth
 * sign-in ended on (UI-761). */
export interface UrlActions {
  /** Opens `url` in a new tab of this browser, without access to this window. */
  open(url: string): void;
  /** Copies `url` to the clipboard with a success toast (UI-520); a refusal (or no clipboard: a page served over plain
   * HTTP from another machine) goes to the error list. */
  copy(url: string): void;
  /** Hands `pasted`, the address a browser ended on after an OAuth sign-in omp started (thread `threadId`'s, else the
   * running login's), to omp's loopback listener through the daemon (`oauth.callback`, PRT-212): `true` with a
   * success toast once omp took it, else `false` with the daemon's refusal in the error list. */
  relay(threadId: string | undefined, pasted: string): Promise<boolean>;
}

export function useUrlActions(): UrlActions {
  const host = useHost();
  return useMemo(
    () => ({
      open: (url) => {
        window.open(url, "_blank", "noopener");
      },
      copy: (url) => {
        // `navigator.clipboard` is missing outside secure contexts: the throw becomes the rejection.
        Promise.resolve()
          .then(() => navigator.clipboard.writeText(url))
          .then(
            () => reportSuccess(host, strings.chat.notice.copied),
            (error: unknown) => reportError(host, error),
          );
      },
      relay: (threadId, pasted) =>
        host.client.call("oauth.callback", threadId === undefined ? { url: pasted } : { threadId, url: pasted }).then(
          () => {
            reportSuccess(host, strings.chat.notice.relayed);
            return true;
          },
          (error: unknown) => {
            reportError(host, error);
            return false;
          },
        ),
    }),
    [host],
  );
}

/** The workspace this window works in, live: the selected thread's, else the draft thread's (UI-100). */
export function useCurrentWorkspaceId(): string | undefined {
  const [selected] = useWindowState(SELECTED_THREAD);
  const [draft] = useWindowState(DRAFT_THREAD);
  return workspaceOf(selected, useThreadSummary(selected), draft);
}

/** The workspace this window works in: the selected thread's, else the draft thread's (UI-100). */
export function currentWorkspaceId(host: OmfpHost): string | undefined {
  const selected = host.window.get(SELECTED_THREAD);
  const summary = selected === undefined ? undefined : host.frames.getSnapshot().index.threads.get(selected);
  return workspaceOf(selected, summary, host.window.get(DRAFT_THREAD));
}

/** Drops what the window keeps for a thread that is archived or deleted: selection, draft, open editors (UI-026). */
function forget(host: OmfpHost, threadId: string): void {
  const { window } = host;
  for (const key of [SELECTED_THREAD, RENAMING_THREAD]) {
    if (window.get(key) === threadId) {
      window.set(key, undefined);
    }
  }
  window.reset(draftKey(threadId));
}

/** Drops the draft thread with its message, the tabs it selected (UI-372) and the view chosen on it (UI-367). */
function discardDraft(host: OmfpHost): void {
  const draft = host.window.get(DRAFT_THREAD);
  host.window.set(DRAFT_THREAD, undefined);
  host.window.reset(DRAFT_MESSAGE);
  host.window.set(VIEW_CHOICES, dropDraftView);
  if (draft !== undefined) {
    host.window.set(TAB_CHOICES, (choices) => dropDraft(choices, draft.workspaceId));
  }
}

/**
 * Creates the daemon thread of the draft with its message (text, images or both) as the first one, then selects it;
 * resolves to its id. Without a message nothing is created, unless `empty` (the draft's mic). A failed creation keeps
 * the draft; a failed first prompt carrying a surviving thread ID transfers the unsent draft there for retry. Failures
 * are reported. {@link DRAFT_CREATING} holds the draft read-only meanwhile, also once the mic's live start ended.
 * `created` hears of the thread before it is selected (the mic's start keeps its session for it).
 */
async function createFromDraft(
  host: OmfpHost,
  empty: boolean,
  created?: (threadId: string) => void,
): Promise<string | undefined> {
  const { client, window } = host;
  const draft = window.get(DRAFT_THREAD);
  const message = window.get(DRAFT_MESSAGE);
  const prompt = expandPastes(message.text, message.pastes).trim();
  const images = message.images;
  // A first message may be images only, as in a thread (UI-060).
  if (draft === undefined || (!empty && prompt === "" && images.length === 0)) {
    return undefined;
  }
  window.set(DRAFT_CREATING, true);
  try {
    const thread = await client.call("threads.create", {
      workspace: { id: draft.workspaceId },
      ...(prompt === "" ? {} : { prompt }),
      ...(images.length === 0 ? {} : { images: [...images] }),
      ...(draft.model === undefined ? {} : { model: draft.model }),
    });
    created?.(thread.id);
    // The thread keeps the tabs its draft selected (UI-372) and the view chosen on it (UI-367).
    window.set(TAB_CHOICES, (choices) => adoptDraft(choices, draft.workspaceId, thread.id));
    adoptView(host, draft, thread.id);
    if (window.get(DRAFT_THREAD)?.workspaceId === draft.workspaceId) {
      discardDraft(host);
    }
    window.set(SELECTED_THREAD, thread.id);
    return thread.id;
  } catch (error) {
    if (error instanceof CallError && error.failure.kind === "daemon") {
      const details = error.failure.error.details;
      const threadId = isObject(details) ? details.threadId : undefined;
      if (typeof threadId === "string" && threadId.trim() !== "") {
        window.set(draftKey(threadId), message);
        window.set(TAB_CHOICES, (choices) => adoptDraft(choices, draft.workspaceId, threadId));
        adoptView(host, draft, threadId);
        if (window.get(DRAFT_THREAD) === draft) {
          discardDraft(host);
        }
        window.set(SELECTED_THREAD, threadId);
        window.set(FOCUS_COMPOSER, threadId);
      }
    }
    reportError(host, error);
    return undefined;
  } finally {
    window.set(DRAFT_CREATING, false);
  }
}

/** Gives `threadId` the view chosen on `draft`, while the window's draft is still that workspace's: reopening it
 * meanwhile (a new object, for the focus) keeps its view (UI-367). */
function adoptView(host: OmfpHost, draft: DraftThread, threadId: string): void {
  if (host.window.get(DRAFT_THREAD)?.workspaceId === draft.workspaceId) {
    host.window.set(VIEW_CHOICES, (choices) => adoptDraftView(choices, threadId, choices.draft));
  }
}

/** The runs a `conflict` of `threads.archive` or `threads.delete` lists (PRT-208); `undefined` for another failure. */
function conflictRuns(error: unknown): readonly RunInfo[] | undefined {
  if (!(error instanceof CallError) || error.failure.kind !== "daemon" || error.failure.error.code !== "conflict") {
    return undefined;
  }
  const { details } = error.failure.error;
  const runs = isObject(details) ? details.runs : undefined;
  // The daemon's `RunInfo`s (PRT-208).
  return Array.isArray(runs) && runs.length > 0 ? (runs as RunInfo[]) : undefined;
}

/**
 * Archives or deletes `threadId` (UI-026) — every entry point goes through here — with `jobs` saying what becomes of
 * the commands its agent started (PRT-208). Without `jobs`, a `conflict` listing them asks the frame's dialog
 * ({@link CLOSE_THREAD}, UI-744), which calls again with the choice; any other failure is reported. The window forgets
 * the thread once it is closed.
 */
export async function closeThread(
  host: OmfpHost,
  method: CloseRequest["method"],
  threadId: string,
  jobs?: CloseJobs,
): Promise<void> {
  try {
    await host.client.call(method, jobs === undefined ? { threadId } : { threadId, jobs });
    forget(host, threadId);
  } catch (error) {
    const runs = jobs === undefined ? conflictRuns(error) : undefined;
    if (runs === undefined) {
      reportError(host, error);
    } else {
      host.window.set(CLOSE_THREAD, { method, threadId, runs });
    }
  }
}

/** Deletes `threadIds` first (the daemon refuses a workspace that still has threads), then removes the workspace,
 * which deletes its folder (DMN-028), stopping at the first failure (reported to `ERRORS`); drops the draft thread
 * started in it. Resolves to whether everything worked. */
export async function removeWorkspace(
  host: OmfpHost,
  workspaceId: string,
  threadIds: readonly string[],
): Promise<boolean> {
  if (host.window.get(DRAFT_THREAD)?.workspaceId === workspaceId) {
    discardDraft(host);
  }
  try {
    for (const threadId of threadIds) {
      forget(host, threadId);
      // The workspace goes with its folder: what the threads' agents started there goes too (PRT-208).
      await host.client.call("threads.delete", { threadId, jobs: "kill" });
    }
    await host.client.call("workspaces.remove", { workspaceId });
    return true;
  } catch (error) {
    reportError(host, error);
    return false;
  }
}

export function threadActions(host: OmfpHost): ThreadActions {
  const { client, window } = host;
  const attempt = async (call: () => Promise<unknown>): Promise<void> => {
    try {
      await call();
    } catch (error) {
      reportError(host, error);
    }
  };
  return {
    select: (threadId) => {
      const draft = window.get(DRAFT_MESSAGE);
      if (threadId !== undefined && draft.text.trim() === "" && draft.images.length === 0) {
        discardDraft(host);
      }
      window.set(SELECTED_THREAD, threadId);
    },
    openDraft: (workspaceId) => {
      const current = window.get(DRAFT_THREAD);
      // A new draft opens on the default view; returning to the workspace's draft keeps the view chosen on it (UI-367).
      if (current?.workspaceId !== workspaceId) {
        window.set(VIEW_CHOICES, dropDraftView);
      }
      // A new object each time: the draft's composer takes the focus again.
      window.set(DRAFT_THREAD, { ...current, workspaceId });
      window.set(SELECTED_THREAD, undefined);
    },
    discardDraft: () => discardDraft(host),
    sendDraft: async () => (await createFromDraft(host, false)) !== undefined,
    startDraftLive: (ready) => {
      const workspaceId = window.get(DRAFT_THREAD)?.workspaceId;
      return host.live.start({
        create: async ({ running, created }) => {
          await ready;
          // A start ended meanwhile (the mic pressed again, another thread or draft shown, the page left) creates nothing.
          return running() ? createFromDraft(host, true, created) : undefined;
        },
        fail: (message) => reportError(host, message),
        // The draft the mic was pressed in: not another workspace's opened since, nor dismissed.
        shown: () => workspaceId !== undefined && window.get(DRAFT_THREAD)?.workspaceId === workspaceId,
      });
    },
    archive: (threadId) => closeThread(host, "threads.archive", threadId),
    unarchive: (threadId) => attempt(() => client.call("threads.unarchive", { threadId })),
    remove: (threadId) => closeThread(host, "threads.delete", threadId),
    rename: (threadId, title) => attempt(() => client.call("threads.update", { threadId, title })),
    stop: (threadId) => attempt(() => client.call("thread.stop", { threadId })),
    restart: (threadId) => attempt(() => client.call("thread.restart", { threadId })),
    resume: (threadId) => attempt(() => client.call("thread.resume", { threadId })),
    fork: (threadId, entryId) => forkThread(host, threadId, entryId),
    forkAtMessage: async (threadId, { role, timestamp }) => {
      if (typeof timestamp !== "number") {
        reportError(host, strings.shell.fork.noTimestamp);
        return undefined;
      }
      try {
        const entryId = messageEntryId(await client.call("thread.tree", { threadId }), role, timestamp);
        if (entryId === undefined) {
          reportError(host, strings.shell.fork.notOnBranch);
          return undefined;
        }
        return await forkThread(host, threadId, entryId);
      } catch (error) {
        reportError(host, error);
        return undefined;
      }
    },
    compact: (threadId, instructions, mode) => compactThread(host, threadId, instructions, mode),
  };
}

async function forkThread(host: OmfpHost, threadId: string, entryId: string | undefined): Promise<string | undefined> {
  try {
    const thread = await host.client.call("threads.fork", { threadId, ...(entryId === undefined ? {} : { entryId }) });
    host.window.set(SELECTED_THREAD, thread.id);
    reportSuccess(host, strings.shell.fork.forked(displayTitle(thread)));
    return thread.id;
  } catch (error) {
    reportError(host, error);
    return undefined;
  }
}

async function compactThread(
  host: OmfpHost,
  threadId: string,
  instructions: string | undefined,
  mode: CompactMode | undefined,
): Promise<boolean> {
  const text = strings.shell.compaction;
  // omp would abort the running turn and resume it after the summary: the window leaves the turn alone (UI-910).
  if (host.frames.getSnapshot().index.threads.get(threadId)?.settled === false) {
    reportError(host, text.busy);
    return false;
  }
  const focus = instructions?.trim() ?? "";
  if (mode === UNFOCUSED_MODE && focus !== "") {
    reportError(host, text.noFocus(mode));
    return false;
  }
  try {
    if (mode !== undefined) {
      // omp's RPC `compact` takes no mode; its `/compact` builtin, sent as a prompt, does (UI-913).
      await host.client.call("thread.prompt", { threadId, text: compactPrompt(mode, focus) });
      reportTaken(host, text.started(mode));
      return true;
    }
    const result = await host.client.call("thread.compact", {
      threadId,
      ...(focus === "" ? {} : { instructions: focus }),
    });
    const before = tokensBefore(result);
    reportSuccess(host, text.compacted(before === undefined ? undefined : compactNumber(before)));
    return true;
  } catch (error) {
    reportError(host, error);
    return false;
  }
}

/** {@link ThreadActions} of this window. */
export function useThreadActions(): ThreadActions {
  const host = useHost();
  return useMemo(() => threadActions(host), [host]);
}
