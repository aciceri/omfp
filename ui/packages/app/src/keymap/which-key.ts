/**
 * The which-key panel's model (UI-320..UI-326): when a started key sequence waits for its next key, the keys that
 * complete it, from the live binding table, grouped by what their commands act on. The key handler (handler.ts)
 * decides when the panel shows and publishes the waiting sequence through a {@link WhichKeyState}; the frame renders it
 * (frame/which-key.tsx).
 */

import type { Binding, Mode } from "./keymap.ts";

/** How long a started sequence waits for its next key before the panel shows it (UI-321). */
export const WHICH_KEY_DELAY_MS = 300;

/** The leader: alone in normal mode it opens the panel at once (UI-324). */
export const LEADER = "Space";

/** Timers of the panel's delay, so tests drive it (the global timers in the browser). */
export interface KeyTimers {
  setTimeout(run: () => void, ms: number): void;
}

export const globalKeyTimers: KeyTimers = {
  setTimeout: (run, ms) => {
    globalThis.setTimeout(run, ms);
  },
};

/** A sequence waiting for its next key, as the panel shows it. */
export interface PendingKeys {
  /** The keys typed so far, tokens separated by spaces (`Space h`). */
  readonly keys: string;
  readonly mode: Mode;
  /** The command scopes around the focused element, innermost first: whose commands the keys would run. */
  readonly scopes: readonly string[];
}

/** Whether `pending` opens the panel without the delay: the leader alone in normal mode (UI-324). */
export function opensAtOnce(pending: Pick<PendingKeys, "keys" | "mode">): boolean {
  return pending.mode === "normal" && pending.keys === LEADER;
}

/** The sequence the panel shows, `undefined` while it is closed; the frame subscribes to it. */
export class WhichKeyState {
  #pending: PendingKeys | undefined;
  readonly #listeners = new Set<() => void>();

  readonly get = (): PendingKeys | undefined => this.#pending;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  set(pending: PendingKeys | undefined): void {
    if (pending === this.#pending) {
      return;
    }
    this.#pending = pending;
    for (const listener of [...this.#listeners]) {
      listener();
    }
  }
}

/** The panel's sections, in this order. */
export const KEY_GROUPS = [
  "threads",
  "views",
  "workspace",
  "files",
  "help",
  "navigation",
  "commands",
  "other",
] as const;

export type KeyGroup = (typeof KEY_GROUPS)[number];

/** Command ids whose group is not their namespace's. */
const COMMAND_GROUPS: Readonly<Record<string, KeyGroup>> = {
  "threads.toggle": "views",
  "threads.collapse": "views",
  "live.voice": "commands",
};

/** The group of a command id's namespace (the part before its first dot). */
const NAMESPACE_GROUPS: Readonly<Record<string, KeyGroup>> = {
  thread: "threads",
  threads: "threads",
  composer: "threads",
  view: "views",
  widget: "views",
  workspace: "workspace",
  file: "files",
  help: "help",
  nav: "navigation",
  notice: "navigation",
  focus: "navigation",
  cmdline: "commands",
  window: "commands",
  density: "commands",
  settings: "commands",
};

/** The group a command belongs to; `other` for commands of widgets and init files outside the built-in namespaces. */
export function commandGroup(command: string): KeyGroup {
  const namespace = command.split(".", 1)[0] ?? command;
  return COMMAND_GROUPS[command] ?? NAMESPACE_GROUPS[namespace] ?? "other";
}

/** The leader's groups (UI-326): a prefix of normal mode → its group. */
const PREFIX_GROUPS: Readonly<Record<string, KeyGroup>> = {
  "Space h": "help",
  "Space p": "workspace",
  "Space f": "files",
  "Space v": "views",
};

/** What a binding's command is now: its title and whether a command answers to it. */
export interface CommandTitle {
  readonly title: string;
  readonly available: boolean;
}

/** One key of the panel: the next token and what it does. */
export type WhichKeyEntry =
  | {
      readonly kind: "command";
      /** The next key (one token). */
      readonly key: string;
      readonly binding: Binding;
      readonly title: string;
      readonly available: boolean;
    }
  | {
      readonly kind: "prefix";
      readonly key: string;
      /** The keys it starts, the typed ones included (`Space h`). */
      readonly keys: string;
      /** Their group, when they have one (known prefix, or every binding under it in one group). */
      readonly group: KeyGroup | undefined;
      /** How many bindings it starts. */
      readonly count: number;
    };

export interface WhichKeySection {
  readonly group: KeyGroup;
  readonly entries: readonly WhichKeyEntry[];
}

/**
 * The keys completing `pending` in its mode, from `bindings` (the window's effective table, so whatever is bound now),
 * in sections by {@link KEY_GROUPS}, each in binding order. A key whose sequence is bound is a command, like the
 * matcher where a complete binding wins over longer ones; a key that only starts longer bindings is a prefix.
 */
export function whichKeySections(
  bindings: readonly Binding[],
  pending: Pick<PendingKeys, "keys" | "mode">,
  titleOf: (binding: Binding) => CommandTitle,
): WhichKeySection[] {
  const start = `${pending.keys} `;
  const below = bindings.filter((binding) => binding.mode === pending.mode && binding.keys.startsWith(start));
  const order: string[] = [];
  const byKey = new Map<string, Binding[]>();
  for (const binding of below) {
    const next = binding.keys.slice(start.length).split(" ", 1)[0] ?? "";
    const found = byKey.get(next);
    if (found === undefined) {
      order.push(next);
      byKey.set(next, [binding]);
    } else {
      found.push(binding);
    }
  }
  const sections = new Map<KeyGroup, WhichKeyEntry[]>();
  for (const key of order) {
    const under = byKey.get(key) ?? [];
    const keys = `${start}${key}`;
    const exact = under.find((binding) => binding.keys === keys);
    const entry: WhichKeyEntry =
      exact === undefined
        ? { kind: "prefix", key, keys, group: prefixGroup(pending.mode, keys, under), count: under.length }
        : { kind: "command", key, binding: exact, ...titleOf(exact) };
    const group = entry.kind === "command" ? commandGroup(entry.binding.command) : (entry.group ?? "other");
    sections.set(group, [...(sections.get(group) ?? []), entry]);
  }
  return KEY_GROUPS.flatMap((group) => {
    const entries = sections.get(group);
    return entries === undefined ? [] : [{ group, entries }];
  });
}

function prefixGroup(mode: Mode, keys: string, under: readonly Binding[]): KeyGroup | undefined {
  const known = mode === "normal" ? PREFIX_GROUPS[keys] : undefined;
  if (known !== undefined) {
    return known;
  }
  const groups = new Set(under.map((binding) => commandGroup(binding.command)));
  const [only] = groups;
  return groups.size === 1 && only !== "other" ? only : undefined;
}
