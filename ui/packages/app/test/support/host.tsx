/**
 * Test host for app and widget tests: a fake protocol client over the real client `Model`, manual animation frames,
 * and helpers to feed index, thread, views and job state. Tests render widgets inside {@link renderWithHost}.
 */

import { type CallMethod, Model } from "@omfp/client";
import { replay, type ThreadEvent } from "@omfp/core";
import {
  type ErrorBody,
  type EventFrame,
  INDEX_TOPIC,
  type IndexEvent,
  type IndexSnapshot,
  type JobEvent,
  type JobState,
  jobTopic,
  type ParamsOf,
  type ResultOf,
  type TerminalEvent,
  type TerminalState,
  type Topic,
  type TransientFrame,
  terminalTopic,
  threadTopic,
  type ViewsEvent,
  type ViewsState,
  viewsTopic,
  type WindowRequestFrame,
} from "@omfp/protocol";
import { act, type RenderResult, render } from "@testing-library/react";
import type { ReactNode } from "react";
import { createHost, type HostOptions, type OmfpHost, OmfpProvider, type SdkClient } from "../../src/sdk/index.ts";

/** One `call` the code under test made; answer it with {@link CallRecord.resolve} or {@link CallRecord.reject}. */
export interface CallRecord {
  readonly method: CallMethod;
  readonly params: unknown;
  resolve(result: unknown): void;
  reject(error: unknown): void;
}

/** A client whose model the test drives directly. */
export class FakeClient implements SdkClient {
  readonly model = new Model();
  readonly calls: CallRecord[] = [];
  readonly subscribed: Topic[] = [];
  readonly unsubscribed: Topic[] = [];
  /** Answers for calls by method, applied as soon as the call is made (else the call stays pending). */
  readonly answers = new Map<CallMethod, (params: unknown) => unknown>();
  #waiters: Array<() => void> = [];
  #positions = new Map<Topic, number>();

  changes(): { next(): Promise<void> } {
    let seen = this.model.revision;
    return {
      next: async () => {
        while (this.model.revision === seen) {
          const { promise, resolve } = Promise.withResolvers<void>();
          this.#waiters.push(resolve);
          await promise;
        }
        seen = this.model.revision;
      },
    };
  }

  takeDirty() {
    return this.model.takeDirty();
  }

  call<M extends CallMethod>(method: M, params: ParamsOf<M>): Promise<ResultOf<M>> {
    const { promise, resolve, reject } = Promise.withResolvers<ResultOf<M>>();
    this.calls.push({ method, params, resolve: resolve as (result: unknown) => void, reject });
    const answer = this.answers.get(method);
    if (answer !== undefined) {
      resolve(answer(params) as ResultOf<M>);
    }
    return promise;
  }

  /** Like the session: a subscribe request (recorded in {@link FakeClient.subscribed}) only for a topic not followed
   * yet, or whose subscription failed. */
  subscribe(topic: Topic): void {
    if (this.model.track(topic)) {
      this.subscribed.push(topic);
    }
    this.notify();
  }

  readonly #windowRequests = new Set<(frame: WindowRequestFrame) => void>();

  onWindowRequest(listener: (frame: WindowRequestFrame) => void): () => void {
    this.#windowRequests.add(listener);
    return () => {
      this.#windowRequests.delete(listener);
    };
  }

  /** Delivers a request of the daemon to this window (PRT-124), as the session does. */
  windowRequest(frame: WindowRequestFrame): void {
    for (const listener of [...this.#windowRequests]) {
      listener(frame);
    }
  }

  unsubscribe(topic: Topic): void {
    this.unsubscribed.push(topic);
    this.model.untrack(topic);
    this.notify();
  }

  readonly #transient = new Set<(frame: TransientFrame) => void>();

  onTransient(listener: (frame: TransientFrame) => void): () => void {
    this.#transient.add(listener);
    return () => {
      this.#transient.delete(listener);
    };
  }

  /** Delivers a transient frame (PRT-054) to the listeners, as the session does for a followed topic. */
  transient(frame: TransientFrame): void {
    if (this.model.topic(frame.topic) !== undefined) {
      for (const listener of [...this.#transient]) {
        listener(frame);
      }
    }
  }

  /** Calls to `method` so far. */
  callsTo(method: CallMethod): CallRecord[] {
    return this.calls.filter((call) => call.method === method);
  }

  /** Marks the connection ready (`system.hello` answered). */
  connected(): void {
    this.model.setConnection({
      kind: "connected",
      hello: {
        protocol: 1,
        daemon: "0.1.0",
        omp: "18.18.6",
        warnings: [],
        widgetBase: "/widgets/test",
        fileBase: "/files/test",
        connection: 1,
      },
    });
    this.notify();
  }

  /** Answers the index subscription with `snapshot`. */
  index(snapshot: Partial<IndexSnapshot>): void {
    this.model.track(INDEX_TOPIC);
    this.model.applySubscribed(INDEX_TOPIC, {
      mode: "snapshot",
      epoch: "e1",
      seq: 0,
      state: {
        workspaces: [],
        threads: [],
        theme: { css: null },
        settings: { ui: {}, keymap: [], error: null },
        ...snapshot,
      },
    });
    this.#positions.set(INDEX_TOPIC, 0);
    this.notify();
  }

  /** Applies index events after {@link FakeClient.index}. */
  indexEvents(...events: IndexEvent[]): void {
    for (const event of events) {
      this.#apply(INDEX_TOPIC, event);
    }
    this.notify();
  }

  /** Answers `thread/<id>` with the state after `events` (the thread must be followed, e.g. by `useThread`). */
  thread(threadId: string, events: readonly ThreadEvent[]): void {
    const topic = threadTopic(threadId);
    this.model.track(topic);
    this.model.applySubscribed(topic, { mode: "snapshot", epoch: "e1", seq: 0, state: replay([...events]) });
    this.#positions.set(topic, 0);
    this.notify();
  }

  /** Applies live thread events after {@link FakeClient.thread}. */
  threadEvents(threadId: string, ...events: ThreadEvent[]): void {
    for (const event of events) {
      this.#apply(threadTopic(threadId), event);
    }
    this.notify();
  }

  /** Answers `views/<workspaceId>` with `state` (empty manifest, widgets and stored values by default). */
  views(workspaceId: string, state: Partial<ViewsState> = {}): void {
    this.#snapshot(viewsTopic(workspaceId), { manifest: null, error: null, widgets: [], stored: {}, ...state });
  }

  /** Applies live views events after {@link FakeClient.views}. */
  viewsEvents(workspaceId: string, ...events: ViewsEvent[]): void {
    for (const event of events) {
      this.#apply(viewsTopic(workspaceId), event);
    }
    this.notify();
  }

  /** Answers `job/<id>` with `state`. */
  job(state: JobState): void {
    this.#snapshot(jobTopic(state.job.id), state);
  }

  /** Applies live job events after {@link FakeClient.job}. */
  jobEvents(jobId: string, ...events: JobEvent[]): void {
    for (const event of events) {
      this.#apply(jobTopic(jobId), event);
    }
    this.notify();
  }

  /** Answers `terminal/<id>` with `state`. */
  terminal(state: TerminalState): void {
    this.#snapshot(terminalTopic(state.terminal.id), state);
  }

  /** Applies live terminal events after {@link FakeClient.terminal}. */
  terminalEvents(terminalId: string, ...events: TerminalEvent[]): void {
    for (const event of events) {
      this.#apply(terminalTopic(terminalId), event);
    }
    this.notify();
  }

  /** The daemon refuses the subscription of `topic` (e.g. `not_found` for a forgotten job). */
  failTopic(topic: Topic, error: ErrorBody): void {
    this.model.track(topic);
    this.model.failTopic(topic, error);
    this.notify();
  }

  #snapshot(topic: Topic, state: unknown): void {
    this.model.track(topic);
    this.model.applySubscribed(topic, { mode: "snapshot", epoch: "e1", seq: 0, state });
    this.#positions.set(topic, 0);
    this.notify();
  }

  /** Wakes the frame store after a direct model change. */
  notify(): void {
    const waiters = this.#waiters;
    this.#waiters = [];
    for (const wake of waiters) {
      wake();
    }
  }

  #apply(topic: Topic, event: EventFrame["event"]): void {
    const seq = (this.#positions.get(topic) ?? 0) + 1;
    this.#positions.set(topic, seq);
    this.model.applyEvent({ type: "event", topic, epoch: "e1", seq, event });
  }
}

/** Animation frames run by the test. */
export class ManualFrames {
  #pending: Array<() => void> = [];

  readonly schedule = (commit: () => void): void => {
    this.#pending.push(commit);
  };

  get pending(): number {
    return this.#pending.length;
  }

  /** Runs the frames requested so far. */
  run(): void {
    const pending = this.#pending;
    this.#pending = [];
    for (const commit of pending) {
      commit();
    }
  }
}

/** Report and notice timers run by the test (`widgets.report` batching, AGT-026; notices, UI-192), and the notices'
 * clock: `time`, which the test sets. */
export class ManualTimers {
  #pending: Array<() => void> = [];
  time = 0;

  readonly setTimeout = (run: () => void, _ms: number): void => {
    this.#pending.push(run);
  };

  readonly now = (): number => this.time;

  get pending(): number {
    return this.#pending.length;
  }

  /** Runs the timers set so far. */
  run(): void {
    const pending = this.#pending;
    this.#pending = [];
    for (const run of pending) {
      run();
    }
  }
}

export interface TestHost {
  readonly client: FakeClient;
  readonly host: OmfpHost;
  /** Commits the model to the snapshot now (as an animation frame would) inside `act`. */
  flush(): void;
}

/** A host over a {@link FakeClient}; frames commit only on {@link TestHost.flush}. */
export function testHost(client = new FakeClient(), options: Omit<HostOptions, "schedule"> = {}): TestHost {
  const host = createHost(client, { ...options, schedule: () => {} });
  return {
    client,
    host,
    flush: () => act(() => host.frames.commit()),
  };
}

/** Lets the released topics go: `TopicRefs` unsubscribes a topic in a microtask after its last release (UI-335). */
export async function released(): Promise<void> {
  await Promise.resolve();
}

/** Renders `ui` inside the test host's provider. */
export function renderWithHost(ui: ReactNode, test: TestHost = testHost()): TestHost & { view: RenderResult } {
  const view = render(<OmfpProvider host={test.host}>{ui}</OmfpProvider>);
  return { ...test, view };
}
