/** Window state slots shared by several widgets and the frame. Widget-private slots live next to their widget. */

import type { ImageContent } from "@omfp/core";
import type { RunInfo } from "@omfp/protocol";
import { decodeTabChoices, NO_TAB_CHOICES, type TabChoices } from "../vm/tab-choices.ts";
import { decodeViewChoices, NO_VIEW_CHOICES, type ViewChoices } from "../vm/view-choices.ts";
import { type WindowKey, windowKey } from "./window-store.ts";

/** The thread this window shows; `undefined` for none. */
export const SELECTED_THREAD = windowKey<string | undefined>("selectedThread", undefined);

/** A large paste kept as a chip (UI-357): its number in the draft and its text. */
export interface Paste {
  readonly id: number;
  readonly text: string;
}

/** A composer draft: text, pasted or dropped images (UI-060, UI-064) and large pastes, each standing in `text` as
 * its placeholder token (UI-357). */
export interface Draft {
  readonly text: string;
  readonly images: readonly ImageContent[];
  readonly pastes: readonly Paste[];
}

export const EMPTY_DRAFT: Draft = { text: "", images: [], pastes: [] };

/** The draft of one thread, kept across thread switches and reconnects (UI-064); dropped when the thread is archived
 * or deleted (UI-026). */
export function draftKey(threadId: string): WindowKey<Draft> {
  let key = DRAFT_KEYS.get(threadId);
  if (key === undefined) {
    key = windowKey<Draft>(`draft/${threadId}`, EMPTY_DRAFT);
    DRAFT_KEYS.set(threadId, key);
  }
  return key;
}

/** One key object per thread, so hooks see a stable key. */
const DRAFT_KEYS = new Map<string, WindowKey<Draft>>();

/** Manual colour scheme override (UI-091), per window; `system` follows `prefers-color-scheme`. Density is a daemon
 * setting (`useSettings`, UI-097). */
export type ColorScheme = "system" | "light" | "dark";

export const COLOR_SCHEME = windowKey<ColorScheme>("colorScheme", "system", (stored) =>
  stored === "light" || stored === "dark" || stored === "system" ? stored : undefined,
);

/**
 * The views this window chose (UI-302, UI-367): per thread the view last shown with it, per workspace the view last
 * chosen in it. Kept in `localStorage`, so a reload (and the next window) shows each thread with its view; read it with
 * `useActiveView`, change it with `chooseView`. The window-wide choice of earlier versions (`omfp.activeView`) is not
 * carried over.
 */
export const VIEW_CHOICES = windowKey<ViewChoices>("viewChoices", NO_VIEW_CHOICES, decodeViewChoices);

/**
 * The tab this window selected in each `tabs` container (UI-372): per thread (the draft thread's per workspace), per
 * view and per container, the `main` slot's file and widget tabs included. Kept in `localStorage` like
 * {@link VIEW_CHOICES}; read and change it through `useChosenTab`, `selectOpenedTab` and `closeOpened`.
 */
export const TAB_CHOICES = windowKey<TabChoices>("tabChoices", NO_TAB_CHOICES, decodeTabChoices);

/** The view the frame renders now and the thread (none: the draft thread) and workspace it renders it for (UI-372);
 * not `ready` while it is the built-in view standing in until the workspace's views arrive. */
export interface ShownView {
  readonly threadId: string | undefined;
  readonly workspaceId: string | undefined;
  readonly viewId: string;
  readonly ready: boolean;
}

/** Where opening a file or a widget tab selects it (UI-372); `undefined` before the frame rendered a view. */
export const SHOWN_VIEW = windowKey<ShownView | undefined>("shownView", undefined);

/** An opened tab to select once the frame renders `threadId` (none: the draft thread of `workspaceId`) in its
 * workspace's own view (UI-372): a file opened right after a thread switch, or while the workspace's views load. */
export interface PendingTab {
  readonly threadId: string | undefined;
  readonly workspaceId: string;
  readonly tabId: string;
}

export const PENDING_TAB = windowKey<PendingTab | undefined>("pendingTab", undefined);

/** A thread being started in a workspace (UI-023): shown while no thread is selected; the daemon thread is created
 * when its first message is sent. One per window. */
export interface DraftThread {
  readonly workspaceId: string;
  /** Explicit `provider/id` for the first prompt; absent means omp's default. */
  readonly model?: string;
}

/** Threads whose model change is awaiting acknowledgement; their composers cannot send yet (UI-030). */
export const MODEL_CHANGING = windowKey<readonly string[]>("modelChanging", []);

export const DRAFT_THREAD = windowKey<DraftThread | undefined>("draftThread", undefined);

/** The composer text of the {@link DRAFT_THREAD}; dropped with it. */
export const DRAFT_MESSAGE = windowKey<Draft>("draft/draftThread", EMPTY_DRAFT);

/** The workspace whose delete confirmation is open (UI-020, DMN-028). */
export const DELETE_WORKSPACE = windowKey<string | undefined>("deleteWorkspace", undefined);

/** The workspace whose `views.json` reset confirmation is open (UI-430). */
export const RESET_VIEWS = windowKey<string | undefined>("resetViews", undefined);

/** The thread whose title is being edited in place (UI-030). */
export const RENAMING_THREAD = windowKey<string | undefined>("renamingThread", undefined);

/** The thread whose composer takes the focus once shown, e.g. after `:notice` (UI-286); the composer clears it. */
export const FOCUS_COMPOSER = windowKey<string | undefined>("focusComposer", undefined);

/** Errors of failed actions, oldest first, shown by the frame until dismissed. */
export const ERRORS = windowKey<readonly string[]>("errors", []);

/** A message of an action that worked (UI-385), e.g. "Logged in to Anthropic"; `key` is unique per window. */
export interface Success {
  readonly key: number;
  readonly text: string;
}

/** Messages of actions that worked, oldest first, shown by the frame as success toasts that go by themselves. */
export const SUCCESSES = windowKey<readonly Success[]>("successes", []);

/** A request to log in to a provider (UI-382): the frame starts `auth.login` for `providerId` at once, or opens its
 * provider picker when it is `undefined`, then clears the request. Set it with `requestLogin(host, providerId?)`. */
export interface LoginRequest {
  readonly providerId: string | undefined;
}

export const LOGIN = windowKey<LoginRequest | undefined>("login", undefined);

/** Counts the logins and logouts this window made (UI-385): what reads the models omp offers (the header's model
 * list) reads them again when it changes. */
export const AUTH_REVISION = windowKey<number>("authRevision", 0);

/** A request to show a built-in widget (UI-274): the frame selects the tab holding `reference` in the current view,
 * else floats it in a panel of this window; `nonce` tells one request from the next. With `workspaceId` of another
 * workspace than the current one, it floats `reference { workspaceId }` (UI-741). */
export interface RevealRequest {
  readonly reference: string;
  readonly nonce: number;
  readonly workspaceId?: string;
}

export const REVEAL_WIDGET = windowKey<RevealRequest | undefined>("revealWidget", undefined);

/** A thread close the daemon refused while commands its agent started still run (UI-744, PRT-208): the frame asks
 * what to do with them, then repeats the call. */
export interface CloseRequest {
  readonly method: "threads.archive" | "threads.delete";
  readonly threadId: string;
  readonly runs: readonly RunInfo[];
}

export const CLOSE_THREAD = windowKey<CloseRequest | undefined>("closeThread", undefined);

/**
 * A request to open a subagent's detail (transcript and steer) under its row of `omfp:background` (UI-492); reveal
 * `omfp:background` with it to show it. `nonce` tells one request from the next.
 */
export interface OpenSubagentRequest {
  readonly threadId: string;
  readonly subagentId: string;
  readonly nonce: number;
}

export const OPEN_SUBAGENT = windowKey<OpenSubagentRequest | undefined>("openSubagent", undefined);

/**
 * A request to open a declared job's row (its runs) in `omfp:jobs` showing `workspaceId` (UI-742); reveal `omfp:jobs`
 * with it to show it. `nonce` tells one request from the next.
 */
export interface OpenJobRequest {
  readonly workspaceId: string;
  readonly job: string;
  readonly nonce: number;
}

export const OPEN_JOB = windowKey<OpenJobRequest | undefined>("openJob", undefined);
