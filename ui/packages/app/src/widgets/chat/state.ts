/**
 * Window state private to the conversation widgets (kept outside React so it survives virtualized rows unmounting and
 * thread switches, UI-010), the selected thread, and the clock the countdowns read (injected in tests, UI-073).
 */

import type { ThreadView } from "@omfp/client";
import type { AskForm, PendingQueueOp, VirtualItem } from "omfp/sdk";
import {
  type Draft,
  INITIAL_PIN,
  type PinState,
  SELECTED_THREAD,
  useHost,
  useThread,
  useWindowState,
  type WindowKey,
  windowKey,
} from "omfp/sdk";
import { createContext, useCallback, useContext, useEffect, useState } from "react";

/** Queue actions sent and not yet answered (UI-062). */
export const PENDING_QUEUE = windowKey<readonly PendingQueueOp[]>("chat/pendingQueue", []);

/** `<threadId>/<interactionId>` of requests answered from this window (no "answered elsewhere" row, UI-072). */
export const ANSWERED_HERE = windowKey<readonly string[]>("chat/answeredHere", []);

/** `<threadId>/<interactionId>` of closed rows the user dismissed. */
export const DISMISSED = windowKey<readonly string[]>("chat/dismissed", []);

/** When each `<threadId>/<interactionId>` was first shown, for its countdown (UI-073). */
export const SHOWN_AT = windowKey<Readonly<Record<string, number>>>("chat/shownAt", {});
/** When live tools and composer activities were first observed in this window (UI-693, UI-695). */
const OBSERVED_AT = windowKey<Readonly<Record<string, number>>>("chat/observedAt", {});

/** `ask` forms being filled, by `<threadId>/<interactionId>` (UI-071). */
export const ASK_FORMS = windowKey<Readonly<Record<string, AskForm>>>("chat/askForms", {});

/** Expanded state of cards and disclosures the user toggled, by card key; absent = its default. */
const EXPANDED = windowKey<Readonly<Record<string, boolean>>>("chat/expanded", {});

/** Scrolling of each thread's transcript, so switching threads keeps the position (UI-042). */
export function pinKey(threadId: string): WindowKey<PinState> {
  let key = PIN_KEYS.get(threadId);
  if (key === undefined) {
    key = windowKey<PinState>(`chat/pin/${threadId}`, INITIAL_PIN);
    PIN_KEYS.set(threadId, key);
  }
  return key;
}

const PIN_KEYS = new Map<string, WindowKey<PinState>>();

/** Measured item sizes of each thread's transcript, so a remount lays out (and restores its position) as before. */
export function measurementsKey(threadId: string): WindowKey<readonly VirtualItem[]> {
  let key = MEASUREMENT_KEYS.get(threadId);
  if (key === undefined) {
    key = windowKey<readonly VirtualItem[]>(`chat/measurements/${threadId}`, []);
    MEASUREMENT_KEYS.set(threadId, key);
  }
  return key;
}

const MEASUREMENT_KEYS = new Map<string, WindowKey<readonly VirtualItem[]>>();

/** The draft `Ctrl-C` last cleared in each thread's composer, which `u` restores (UI-063). */
export function clearedDraftKey(threadId: string): WindowKey<Draft | undefined> {
  let key = CLEARED_DRAFT_KEYS.get(threadId);
  if (key === undefined) {
    key = windowKey<Draft | undefined>(`chat/clearedDraft/${threadId}`, undefined);
    CLEARED_DRAFT_KEYS.set(threadId, key);
  }
  return key;
}

const CLEARED_DRAFT_KEYS = new Map<string, WindowKey<Draft | undefined>>();

/** `<threadId>/<interactionId>`. */
export function interactionKey(threadId: string, interactionId: string): string {
  return `${threadId}/${interactionId}`;
}

/** The selected thread's id and view (`undefined` until its snapshot arrives). */
export function useSelectedThread(): { threadId: string | undefined; view: ThreadView | undefined } {
  const [threadId] = useWindowState(SELECTED_THREAD);
  return { threadId, view: useThread(threadId) };
}

/** A card's expanded state and its toggle, kept in {@link EXPANDED}. */
export function useExpanded(cardKey: string, byDefault: boolean): [boolean, (expanded: boolean) => void] {
  const [expanded, setExpanded] = useWindowState(EXPANDED);
  const set = useCallback(
    (value: boolean) => setExpanded((previous) => ({ ...previous, [cardKey]: value })),
    [cardKey, setExpanded],
  );
  return [expanded[cardKey] ?? byDefault, set];
}

/** Expands the cards and disclosures `keys` name, as if the user had (conversation search, UI-736); the caller does
 * not render again when a card changes. */
export function useExpandCards(): (keys: readonly string[]) => void {
  const { window } = useHost();
  return useCallback(
    (keys: readonly string[]) =>
      window.set(EXPANDED, (previous) =>
        keys.every((key) => previous[key] === true)
          ? previous
          : { ...previous, ...Object.fromEntries(keys.map((key) => [key, true])) },
      ),
    [window],
  );
}

/** Wall-clock time and a ticker; tests inject a manual one. */
export interface Clock {
  now(): number;
  /** Calls `tick` every `ms` milliseconds until the returned function is called. */
  every(ms: number, tick: () => void): () => void;
}

const systemClock: Clock = {
  now: () => Date.now(),
  every: (ms, tick) => {
    const id = setInterval(tick, ms);
    return () => clearInterval(id);
  },
};

export const ClockContext = createContext<Clock>(systemClock);

export function useClock(): Clock {
  return useContext(ClockContext);
}

/**
 * Milliseconds since `key` was first observed active in this window. A settled item never observed live has no
 * elapsed time; an observed item keeps its last elapsed value when it settles (UI-693, UI-695).
 */
export function useObservedElapsed(key: string, active: boolean): number | undefined {
  const clock = useClock();
  const [observedAt, setObservedAt] = useWindowState(OBSERVED_AT);
  const startedAt = observedAt[key];
  const [now, setNow] = useState(() => clock.now());
  useEffect(() => {
    if (!active || startedAt !== undefined) {
      return;
    }
    const observed = clock.now();
    setNow(observed);
    setObservedAt((previous) => (previous[key] === undefined ? { ...previous, [key]: observed } : previous));
  }, [active, clock, key, setObservedAt, startedAt]);
  useEffect(() => {
    if (!active) {
      return undefined;
    }
    setNow(clock.now());
    return clock.every(100, () => setNow(clock.now()));
  }, [active, clock]);
  if (startedAt === undefined) {
    return active ? 0 : undefined;
  }
  return Math.max(0, now - startedAt);
}

/**
 * Records when each of the live items `keys` was first observed in this window, as {@link useObservedElapsed} does,
 * for items not rendered yet: the reads of a collapsed read group (UI-765), whose elapsed time then counts from there.
 */
export function useObserved(keys: readonly string[]): void {
  const clock = useClock();
  const [observedAt, setObservedAt] = useWindowState(OBSERVED_AT);
  // One string, so the effect runs again only when the set of keys not observed yet changes.
  const unobserved = keys.filter((key) => observedAt[key] === undefined).join("\n");
  useEffect(() => {
    if (unobserved === "") {
      return;
    }
    const observed = clock.now();
    setObservedAt((previous) => {
      const added = unobserved.split("\n").filter((key) => previous[key] === undefined);
      return added.length === 0
        ? previous
        : { ...previous, ...Object.fromEntries(added.map((key) => [key, observed])) };
    });
  }, [clock, unobserved, setObservedAt]);
}
