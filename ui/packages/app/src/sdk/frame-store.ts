/**
 * Per-frame snapshots of the client model (UI-012): any number of model changes between two animation frames end in
 * one snapshot, and React renders from snapshots only, so streaming deltas cost at most one render per frame.
 */

import type { ConnectionStatus, Dirty, Index, ModelView, ThreadView, TopicState, WorkspaceJobs } from "@omfp/client";
import type { JobState, RepoStatus, TerminalState, ViewsState } from "@omfp/protocol";

/** What the store reads from the client. */
export interface ModelSource {
  readonly model: ModelView;
  /** Like `Client.changes()`: `next()` resolves once the model changed since the previous call. */
  changes(): { next(): Promise<void> };
  takeDirty(): Dirty;
}

/** The model as of one frame. Every field is an immutable reference taken from the model, so selectors can compare
 * by identity across frames. */
export interface Snapshot {
  /** Increases by one per committed frame. */
  readonly frame: number;
  /** The model's revision at this frame. */
  readonly revision: number;
  /** What changed since the previous frame. */
  readonly dirty: Dirty;
  readonly connection: ConnectionStatus;
  readonly index: Index;
  readonly threads: ReadonlyMap<string, ThreadView>;
  readonly workspaces: ReadonlyMap<string, readonly RepoStatus[]>;
  /** Views state of followed workspaces (PRT-040), by workspace id. */
  readonly views: ReadonlyMap<string, ViewsState>;
  /** Followed jobs (PRT-041), by job id. */
  readonly jobs: ReadonlyMap<string, JobState>;
  /** Followed PTY sessions (TRM-100), by terminal id. */
  readonly terminals: ReadonlyMap<string, TerminalState>;
  /** Followed declared jobs (PRT-200) and the run events applied live, by workspace id. */
  readonly workspaceJobs: ReadonlyMap<string, WorkspaceJobs>;
  readonly topics: readonly TopicState[];
}

/** Runs `commit` before the next paint: `requestAnimationFrame` in the browser, a manual queue in tests. */
export type FrameScheduler = (commit: () => void) => void;

export const animationFrame: FrameScheduler = (commit) => {
  requestAnimationFrame(() => commit());
};

function capture(model: ModelView, dirty: Dirty, frame: number): Snapshot {
  return {
    frame,
    revision: model.revision,
    dirty,
    connection: model.connection,
    index: model.index,
    threads: model.threads,
    workspaces: model.workspaces,
    views: model.views,
    jobs: model.jobs,
    terminals: model.terminals,
    workspaceJobs: model.workspaceJobs,
    topics: model.topics,
  };
}

/** Holds the current {@link Snapshot} and wakes subscribers once per frame with changes (`useSyncExternalStore`). */
export class FrameStore {
  readonly #source: ModelSource;
  readonly #schedule: FrameScheduler;
  readonly #listeners = new Set<() => void>();
  #snapshot: Snapshot;
  #running = false;

  constructor(source: ModelSource, schedule: FrameScheduler = animationFrame) {
    this.#source = source;
    this.#schedule = schedule;
    this.#snapshot = capture(source.model, source.takeDirty(), 0);
  }

  /** The latest committed snapshot; stable between frames. */
  readonly getSnapshot = (): Snapshot => this.#snapshot;

  /** Calls `listener` after each committed frame; returns the unsubscribe function. */
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  /** Follows the model until {@link FrameStore.stop}: after each change, waits for the next frame, then commits. */
  start(): void {
    if (this.#running) {
      return;
    }
    this.#running = true;
    void this.#loop(this.#source.changes());
  }

  stop(): void {
    this.#running = false;
  }

  /** Takes a snapshot of the model now and wakes subscribers when anything changed since the last one. */
  commit(): void {
    const { model } = this.#source;
    if (model.revision === this.#snapshot.revision) {
      this.#source.takeDirty();
      return;
    }
    this.#snapshot = capture(model, this.#source.takeDirty(), this.#snapshot.frame + 1);
    for (const listener of [...this.#listeners]) {
      listener();
    }
  }

  async #loop(changes: { next(): Promise<void> }): Promise<void> {
    while (this.#running) {
      await changes.next();
      const frame = Promise.withResolvers<void>();
      this.#schedule(() => {
        if (this.#running) {
          this.commit();
        }
        frame.resolve();
      });
      await frame.promise;
    }
  }
}
