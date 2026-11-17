/** Change notification for coalesced rendering (UI-012). */

/** What {@link Changes} watches: the model's revision, and a promise settled at the next notification. */
export class ChangeHub {
  readonly #revision: () => number;
  #waiting: Array<() => void> = [];

  constructor(revision: () => number) {
    this.#revision = revision;
  }

  get revision(): number {
    return this.#revision();
  }

  /** Settles at the next {@link ChangeHub.notify}. */
  changed(): Promise<void> {
    return new Promise((resolve) => this.#waiting.push(resolve));
  }

  /** Wakes every waiter. */
  notify(): void {
    const waiting = this.#waiting;
    this.#waiting = [];
    for (const wake of waiting) {
      wake();
    }
  }
}

/**
 * Wakes the host when the model changed. Any number of changes in between wake it once: the revision is read when the
 * host resumes, so a host awaiting {@link Changes.next} before each animation frame renders at most once per frame.
 */
export class Changes {
  readonly #hub: ChangeHub;
  #seen: number;

  constructor(hub: ChangeHub) {
    this.#hub = hub;
    this.#seen = hub.revision;
  }

  /** Resolves once the model changed since the previous call (or since this `Changes` was created). */
  async next(): Promise<void> {
    while (this.#hub.revision === this.#seen) {
      await this.#hub.changed();
    }
    this.#seen = this.#hub.revision;
  }
}

/** Adds `listener` to `listeners`; returns its removal. */
export function listen<T>(listeners: Set<T>, listener: T): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
