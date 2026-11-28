/**
 * Window-local state (UI-005, UI-010): what only this window knows — selection, drafts, expanded cards, list grouping,
 * colour scheme — kept outside React so it survives a widget unmounting (thread switches, virtualized rows) and is
 * shared by every widget that names the same key. Never sent to the daemon.
 */

/** A typed slot of window state with its initial value. Keys with the same `name` are the same slot. */
export interface WindowKey<T> {
  readonly name: string;
  readonly initial: T;
  /** Kept in `localStorage` as JSON so the next window starts from it (e.g. the colour scheme); decodes a stored
   * value, `undefined` when it is not valid. */
  readonly persist?: (stored: unknown) => T | undefined;
}

/** Declares a slot; a `persist` decoder makes it survive reloads (its values must be JSON). */
export function windowKey<T>(name: string, initial: T, persist?: (stored: unknown) => T | undefined): WindowKey<T> {
  return persist === undefined ? { name, initial } : { name, initial, persist };
}

/** A new value, or a function of the previous one. */
export type Update<T> = T | ((previous: T) => T);

/** Where persisted slots live; `localStorage` in the browser. */
export interface KeyValueStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const STORAGE_PREFIX = "omfp.";

export class WindowStore {
  readonly #values = new Map<string, unknown>();
  readonly #listeners = new Map<string, Set<() => void>>();
  readonly #storage: KeyValueStorage | undefined;

  constructor(storage?: KeyValueStorage) {
    this.#storage = storage;
  }

  get<T>(key: WindowKey<T>): T {
    if (this.#values.has(key.name)) {
      return this.#values.get(key.name) as T;
    }
    const stored = key.persist === undefined ? undefined : this.#load(key.name, key.persist);
    const value = stored === undefined ? key.initial : stored;
    this.#values.set(key.name, value);
    return value;
  }

  set<T>(key: WindowKey<T>, update: Update<T>): void {
    const previous = this.get(key);
    const next = typeof update === "function" ? (update as (previous: T) => T)(previous) : update;
    if (Object.is(next, previous)) {
      return;
    }
    this.#values.set(key.name, next);
    if (key.persist !== undefined) {
      this.#storage?.setItem(STORAGE_PREFIX + key.name, JSON.stringify(next));
    }
    for (const listener of [...(this.#listeners.get(key.name) ?? [])]) {
      listener();
    }
  }

  /** Back to the initial value (e.g. a deleted thread's draft). */
  reset<T>(key: WindowKey<T>): void {
    this.set(key, key.initial);
  }

  /**
   * The value of a persisted `key` as stored now, which another window on the same storage may have changed since
   * this one read it; `undefined` for a key not persisted, without storage or without a stored value.
   */
  stored<T>(key: WindowKey<T>): T | undefined {
    return key.persist === undefined ? undefined : this.#load(key.name, key.persist);
  }

  subscribe<T>(key: WindowKey<T>, listener: () => void): () => void {
    let listeners = this.#listeners.get(key.name);
    if (listeners === undefined) {
      listeners = new Set();
      this.#listeners.set(key.name, listeners);
    }
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }

  #load<T>(name: string, decode: (stored: unknown) => T | undefined): T | undefined {
    const raw = this.#storage?.getItem(STORAGE_PREFIX + name);
    if (raw === null || raw === undefined) {
      return undefined;
    }
    try {
      return decode(JSON.parse(raw));
    } catch {
      return undefined;
    }
  }
}
