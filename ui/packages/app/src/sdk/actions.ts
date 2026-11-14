/**
 * Thread actions shared by widgets, commands and the frame (UI-023, UI-026, UI-030): each is one protocol call plus
 * what the window must forget. Failures land in the {@link ERRORS} slot the frame shows.
 */

import { CallError } from "@omfp/client";
import { useMemo } from "react";
import { strings } from "../strings/index.ts";
import { expandPastes } from "../vm/paste.ts";
import { displayTitle } from "../vm/threads.ts";
import { messageEntryId } from "../vm/tree.ts";
import { currentWorkspaceId as workspaceOf } from "../vm/workspaces.ts";
import { useThreadSummary, useWindowState } from "./hooks.ts";
import { type OmfpHost, useHost } from "./host.tsx";
import { isObject } from "./json.ts";
import {
  DELETE_THREAD,
  DRAFT_MESSAGE,
  DRAFT_THREAD,
  draftKey,
  ERRORS,
  LOGIN,
  RENAMING_THREAD,
  SELECTED_THREAD,
  SUCCESSES,
} from "./keys.ts";

export interface ThreadActions {
  /** Shows `threadId` in this window (`undefined`: none); leaving an empty draft thread discards it (UI-023). */
  select(threadId: string | undefined): void;
  /** Shows the draft thread of `workspaceId` (UI-023); a draft started in another workspace moves there with its
   * text. */
  openDraft(workspaceId: string): void;
  /** Drops the draft thread and its text. */
  discardDraft(): void;
  /** Creates the daemon thread of the draft with its text as first message (`threads.create { workspace, prompt }`),
   * then selects it and drops the draft; on failure the draft and its text stay. Resolves to whether it worked. */
  sendDraft(): Promise<boolean>;
  archive(threadId: string): Promise<void>;
  unarchive(threadId: string): Promise<void>;
  /** Opens the delete confirmation; {@link ThreadActions.remove} is what it confirms. */
  requestDelete(threadId: string): void;
  /** Deletes without asking. */
  remove(threadId: string): Promise<void>;
  rename(threadId: string, title: string): Promise<void>;
  stop(threadId: string): Promise<void>;
  restart(threadId: string): Promise<void>;
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

/** Adds `error` to the window's error list and its messages (UI-521). */
export function reportError(host: OmfpHost, error: unknown): void {
  const message = errorMessage(error);
  host.window.set(ERRORS, (errors) => [...errors, message]);
  host.notices.remember("error", message);
}

/**
 * The outcome of a user action (UI-385, UI-520): a success toast that goes by itself, also announced through the live
 * region and kept in the window's messages (UI-521).
 */
export function reportSuccess(host: OmfpHost, text: string): void {
  host.window.set(SUCCESSES, (shown) => [...shown, { key: (shown.at(-1)?.key ?? 0) + 1, text }]);
  host.announcer.announce(text);
  host.notices.remember("success", text);
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

/** Opening and copying a URL omp opened on the daemon's host (UI-346, UI-347). */
export interface UrlActions {
  /** Opens `url` in a new tab of this browser, without access to this window. */
  open(url: string): void;
  /** Copies `url` to the clipboard with a success toast (UI-520); a refusal (or no clipboard: a page served over plain
   * HTTP from another machine) goes to the error list. */
  copy(url: string): void;
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
  for (const key of [SELECTED_THREAD, RENAMING_THREAD, DELETE_THREAD]) {
    if (window.get(key) === threadId) {
      window.set(key, undefined);
    }
  }
  window.reset(draftKey(threadId));
}

function discardDraft(host: OmfpHost): void {
  host.window.set(DRAFT_THREAD, undefined);
  host.window.reset(DRAFT_MESSAGE);
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
      await host.client.call("threads.delete", { threadId });
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
      // A new object each time: the draft's composer takes the focus again.
      window.set(DRAFT_THREAD, { ...window.get(DRAFT_THREAD), workspaceId });
      window.set(SELECTED_THREAD, undefined);
    },
    discardDraft: () => discardDraft(host),
    sendDraft: async () => {
      const draft = window.get(DRAFT_THREAD);
      const message = window.get(DRAFT_MESSAGE);
      const prompt = expandPastes(message.text, message.pastes).trim();
      if (draft === undefined || prompt === "") {
        return false;
      }
      try {
        const thread = await client.call("threads.create", {
          workspace: { id: draft.workspaceId },
          prompt,
          ...(draft.model === undefined ? {} : { model: draft.model }),
        });
        if (window.get(DRAFT_THREAD)?.workspaceId === draft.workspaceId) {
          discardDraft(host);
        }
        window.set(SELECTED_THREAD, thread.id);
        return true;
      } catch (error) {
        reportError(host, error);
        return false;
      }
    },
    archive: (threadId) => {
      forget(host, threadId);
      return attempt(() => client.call("threads.archive", { threadId }));
    },
    unarchive: (threadId) => attempt(() => client.call("threads.unarchive", { threadId })),
    requestDelete: (threadId) => window.set(DELETE_THREAD, threadId),
    remove: (threadId) => {
      forget(host, threadId);
      return attempt(() => client.call("threads.delete", { threadId }));
    },
    rename: (threadId, title) => attempt(() => client.call("threads.update", { threadId, title })),
    stop: (threadId) => attempt(() => client.call("thread.stop", { threadId })),
    restart: (threadId) => attempt(() => client.call("thread.restart", { threadId })),
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

/** {@link ThreadActions} of this window. */
export function useThreadActions(): ThreadActions {
  const host = useHost();
  return useMemo(() => threadActions(host), [host]);
}
