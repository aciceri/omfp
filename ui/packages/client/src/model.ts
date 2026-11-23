/**
 * Pure client state kept from the daemon's frames: connection status, the `index` topic, per-thread `ThreadState`,
 * workspace git status, per-workspace views state (PRT-040), per-job output (PRT-041) and per-workspace declared jobs
 * (PRT-200), plus the position `{epoch, seq}` of every subscribed topic.
 *
 * Thread state comes only from the shared reducer (UI-010): snapshots are the daemon's reducer output, events are
 * applied with `reduce` from `@omfp/core` (PRT-021). Exposed data is immutable: a change replaces the objects on its
 * path (a new `index`, a new `ThreadView`, …), bumps {@link Model.revision} and lands in a {@link Dirty} set the host
 * drains once per frame (UI-012).
 */

import { emptyThreadState, type Resolution, reduce, type ThreadEvent, type ThreadState } from "@omfp/core";
import {
  type ErrorBody,
  type EventFrame,
  type HelloResult,
  type IndexEvent,
  type IndexSnapshot,
  type JobEvent,
  type JobState,
  type JobsState,
  type JobView,
  type LoginActivity,
  type ParsedTopic,
  type Position,
  parseTopic,
  type RepoStatus,
  type RunInfo,
  type Settings,
  type SubscribeParams,
  type SubscribeResult,
  type TerminalEvent,
  type TerminalState,
  type Theme,
  type ThreadSummary,
  type Topic,
  type ViewsEvent,
  type ViewsState,
  type Workspace,
} from "@omfp/protocol";
import { listen } from "./changes.ts";
import {
  type Decoded,
  decodeIndexSnapshot,
  decodeJobState,
  decodeJobsState,
  decodeRepoStatusList,
  decodeTerminalState,
  decodeThreadState,
  decodeViewsState,
  NO_SETTINGS,
  NO_THEME,
} from "./decode.ts";
import { jsonEqual } from "./json.ts";

/** Closed interactions kept per thread (newest last). */
export const CLOSED_KEPT = 32;

/** State of the connection to the daemon. */
export type ConnectionStatus =
  /** First connection attempt. */
  | { kind: "connecting" }
  /** `system.hello` succeeded. */
  | { kind: "connected"; hello: HelloResult }
  /** The connection failed or was lost; attempt `attempt` starts after `retryInMs` (UI-003). */
  | { kind: "reconnecting"; attempt: number; retryInMs: number; error: string }
  /** The daemon refused the token (`unauthorized`, close 4401); the client does not retry. */
  | { kind: "unauthenticated" }
  /** The daemon does not speak this client's protocol version (PRT-004); the client does not retry. */
  | { kind: "version_mismatch"; supported: number[]; message: string }
  /** The client was closed by the host. */
  | { kind: "closed" };

/** The `index` topic: workspaces and thread summaries keyed by id, in id order (ids are ULIDs, so this
 * is creation order), the user's theme (DMN-075; `{ css: null }` until the daemon sends one) and the UI settings of
 * the daemon's config file (DMN-009, UI-097; `{ ui: {}, keymap: [], error: null }` until the daemon sends them), the
 * version of the user's init file (PRT-095; `null` without one), how many models omp can use (PRT-141; `null`
 * until the daemon checked, which `auth.status` asks for) and the running login (DMN-290; `null` when none runs). */
export interface Index {
  readonly workspaces: ReadonlyMap<string, Workspace>;
  readonly threads: ReadonlyMap<string, ThreadSummary>;
  readonly theme: Theme;
  readonly settings: Settings;
  readonly init: string | null;
  readonly models: number | null;
  readonly login: LoginActivity | null;
}

/** A pending dialog request of a thread. */
export type InteractionRequest = ThreadState["interactions"][number];

/** An interactive request that is no longer pending. */
export interface ClosedInteraction {
  readonly request: InteractionRequest;
  /** Who answered (UI-072); `undefined` when omp cancelled it or the process went away. */
  readonly resolution: Resolution | undefined;
}

/** One subscribed thread. */
export interface ThreadView {
  /** The reducer's state (UI-010). */
  readonly state: ThreadState;
  /** Requests that left `state.interactions` while live events were applied, newest last, at most
   * {@link CLOSED_KEPT}. A snapshot carries no such history and starts the list afresh. */
  readonly closed: readonly ClosedInteraction[];
}

/** A `run_started` or `run_ended` event of a `jobs/<workspaceId>` topic (PRT-200), as the model applied it. */
export interface RunEvent {
  /** Increases by one per run event the model applies, across workspaces: what a reader saw ends at a `seq`. */
  readonly seq: number;
  readonly kind: "run_started" | "run_ended";
  readonly run: RunInfo;
}

/** Run events a `jobs/<id>` topic keeps (newest last). */
export const RUN_EVENTS_KEPT = 64;

/** One subscribed `jobs/<workspaceId>` topic. */
export interface WorkspaceJobs {
  /** The topic's state (PRT-200): the declared jobs by name and why the current file is refused. */
  readonly state: JobsState;
  /** The run events applied live, oldest first, at most {@link RUN_EVENTS_KEPT}; a snapshot carries none (its runs
   * were never announced to this client) and keeps the ones before it. */
  readonly runs: readonly RunEvent[];
}

/** How a subscription started (PRT-020): the daemon replayed the missed events, or sent the full state. */
export type StartMode = "replay" | "snapshot";

/** Where a subscribed topic stands. */
export type TopicSync =
  /** A subscribe request is due or in flight; events are ignored until it is answered. */
  | { kind: "subscribing" }
  /** Events apply in order. */
  | { kind: "live"; mode: StartMode }
  /** The daemon refused the subscription (retried on request and on the next connection). */
  | { kind: "failed"; error: ErrorBody };

/** A topic the client wants, with its last applied position. */
export interface TopicState {
  readonly topic: Topic;
  /** Last applied `{epoch, seq}`; sent as `since` when resubscribing (UI-003, PRT-020). */
  readonly position: Position | undefined;
  readonly sync: TopicSync;
}

/** What changed since the host last drained it. */
export interface Dirty {
  connection: boolean;
  /** Subscriptions were added, removed or changed state. */
  topics: boolean;
  index: boolean;
  /** Thread ids whose {@link ThreadView} changed or went away. */
  threads: Set<string>;
  /** Workspace ids whose git status changed or went away. */
  workspaces: Set<string>;
  /** Workspace ids whose views state changed or went away. */
  views: Set<string>;
  /** Job ids whose state changed or went away. */
  jobs: Set<string>;
  /** Terminal ids whose state changed or went away. */
  terminals: Set<string>;
  /** Workspace ids whose declared jobs changed or went away. */
  workspaceJobs: Set<string>;
}

/** The data of every topic kind but `index`, by topic id. */
interface Keyed {
  thread: ThreadView;
  workspace: readonly RepoStatus[];
  views: ViewsState;
  job: JobState;
  terminal: TerminalState;
  jobs: WorkspaceJobs;
}

type KeyedKind = keyof Keyed;

/** The {@link Dirty} set of each keyed topic kind. */
const DIRTY = {
  thread: "threads",
  workspace: "workspaces",
  views: "views",
  job: "jobs",
  terminal: "terminals",
  jobs: "workspaceJobs",
} as const satisfies Record<KeyedKind, keyof Dirty>;

function emptyDirty(): Dirty {
  return {
    connection: false,
    topics: false,
    index: false,
    threads: new Set(),
    workspaces: new Set(),
    views: new Set(),
    jobs: new Set(),
    terminals: new Set(),
    workspaceJobs: new Set(),
  };
}

/** Whether nothing changed. */
export function isDirtyEmpty(dirty: Dirty): boolean {
  return (
    !dirty.connection && !dirty.topics && !dirty.index && Object.values(DIRTY).every((key) => dirty[key].size === 0)
  );
}

/** Result of {@link Model.applyEvent}: applied; not for a live subscription or a duplicate; or a gap in the stream
 * (PRT-022), after which the topic must be resubscribed (its position was dropped, so the daemon sends a snapshot). */
export type Applied = "applied" | "ignored" | "gap";

/** A subscribe result the model cannot use. */
export type SubscribeError =
  | { kind: "not_tracked"; topic: Topic }
  /** The replay does not continue the known position: resubscribe without `since`. */
  | { kind: "replay_mismatch"; topic: Topic; epoch: string; fromSeq: number }
  /** The snapshot does not decode as the topic's state; the topic is marked failed. */
  | { kind: "snapshot"; topic: Topic; message: string };

/** Result of {@link Model.applySubscribed}. */
export type Subscribed = { ok: true; mode: StartMode } | { ok: false; error: SubscribeError };

/** Read access to the model, as the client exposes it. */
export interface ModelView {
  readonly connection: ConnectionStatus;
  /** The `index` topic's state (empty until its snapshot arrives). */
  readonly index: Index;
  /** Subscribed threads that received their snapshot. */
  readonly threads: ReadonlyMap<string, ThreadView>;
  thread(id: string): ThreadView | undefined;
  /** Git status of each repository of subscribed workspaces. */
  readonly workspaces: ReadonlyMap<string, readonly RepoStatus[]>;
  /** Views state of subscribed `views/<workspaceId>` topics (PRT-040), by workspace id. */
  readonly views: ReadonlyMap<string, ViewsState>;
  /** State of subscribed `job/<id>` topics (PRT-041), by job id. */
  readonly jobs: ReadonlyMap<string, JobState>;
  /** State of subscribed `terminal/<id>` topics (TRM-100), by terminal id. */
  readonly terminals: ReadonlyMap<string, TerminalState>;
  /** State of subscribed `jobs/<workspaceId>` topics (PRT-200), by workspace id. */
  readonly workspaceJobs: ReadonlyMap<string, WorkspaceJobs>;
  /** Calls `listener` with each `run_started` / `run_ended` event of a `jobs/<workspaceId>` topic as it is applied
   * live (never for a snapshot), whether or not anything renders in between; returns its removal. */
  onRunEvent(listener: (workspaceId: string, event: RunEvent) => void): () => void;

  /** Subscribed topics in subscription order. */
  readonly topics: readonly TopicState[];
  topic(topic: Topic): TopicState | undefined;
  /** Increases with every visible change. */
  readonly revision: number;
}

const EMPTY_INDEX: Index = {
  workspaces: new Map(),
  threads: new Map(),
  theme: NO_THEME,
  settings: NO_SETTINGS,
  init: null,
  models: null,
  login: null,
};

function byId<T extends { id: string }>(items: readonly T[]): Map<string, T> {
  return new Map([...items].sort((a, b) => compareIds(a.id, b.id)).map((item) => [item.id, item]));
}

function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** `map` with `value` under `key`, keeping id order. */
function upsert<T>(map: ReadonlyMap<string, T>, key: string, value: T): Map<string, T> {
  const next = new Map(map);
  next.set(key, value);
  if (map.has(key)) {
    return next;
  }
  let last: string | undefined;
  for (const existing of map.keys()) {
    last = existing;
  }
  // Ids are ULIDs: a new one nearly always sorts last, and then insertion order is already id order.
  return last === undefined || compareIds(last, key) < 0
    ? next
    : new Map([...next].sort(([a], [b]) => compareIds(a, b)));
}

function without<T>(map: ReadonlyMap<string, T>, key: string): ReadonlyMap<string, T> {
  if (!map.has(key)) {
    return map;
  }
  const next = new Map(map);
  next.delete(key);
  return next;
}

function indexFromSnapshot(snapshot: IndexSnapshot): Index {
  return {
    workspaces: byId(snapshot.workspaces),
    threads: byId(snapshot.threads),
    theme: snapshot.theme,
    settings: snapshot.settings,
    init: snapshot.init ?? null,
    models: snapshot.models ?? null,
    login: snapshot.login ?? null,
  };
}

function applyIndexEvent(index: Index, event: IndexEvent): Index {
  switch (event.kind) {
    case "workspace_upserted":
      return { ...index, workspaces: upsert(index.workspaces, event.workspace.id, event.workspace) };
    case "workspace_removed":
      return { ...index, workspaces: without(index.workspaces, event.workspaceId) };
    case "thread_upserted":
      return { ...index, threads: upsert(index.threads, event.thread.id, event.thread) };
    case "thread_removed":
      return { ...index, threads: without(index.threads, event.threadId) };
    case "theme_changed":
      return { ...index, theme: event.theme };
    case "settings_changed":
      return { ...index, settings: event.settings };
    case "init_changed":
      return { ...index, init: event.init ?? null };
    case "models_changed":
      return { ...index, models: event.models };
    case "login_changed":
      return { ...index, login: event.login ?? null };
  }
}

function applyViewsEvent(views: ViewsState, event: ViewsEvent): ViewsState {
  switch (event.kind) {
    case "views_manifest":
      return { ...views, manifest: event.manifest ?? null, error: event.error ?? null };
    case "views_widgets":
      return { ...views, widgets: event.widgets };
    case "views_keymap":
      return { ...views, keymap: event.keymap, keymapError: event.error ?? null };
    case "views_init":
      return { ...views, init: event.init ?? null };
    case "widget_stored": {
      const keys = { ...views.stored[event.instance] };
      if (event.value === null) {
        delete keys[event.key];
      } else {
        keys[event.key] = event.value;
      }
      const stored = { ...views.stored, [event.instance]: keys };
      if (Object.keys(keys).length === 0) {
        delete stored[event.instance];
      }
      return { ...views, stored };
    }
  }
}

/** Text a job topic keeps, like the daemon's state (DMN-080): older lines are dropped and counted in `dropped`. */
export const JOB_TEXT_LIMIT = 1024 * 1024;

/** Raw PTY bytes a terminal topic keeps, like the daemon's state (TRM-103). */
export const TERMINAL_BYTES_LIMIT = 1024 * 1024;
/** Maximum chunks retained, bounding allocation overhead from one-byte PTY writes. */
export const TERMINAL_CHUNKS_LIMIT = 4096;

function base64Bytes(data: string): number {
  const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
  return Math.floor((data.length * 3) / 4) - padding;
}

/** `lines` with `line` appended, the oldest dropped while the text exceeds {@link JOB_TEXT_LIMIT}. */
function appendJobLine(job: JobState, line: JobState["lines"][number]): JobState {
  const lines = [...job.lines, line];
  let size = lines.reduce((total, kept) => total + kept.text.length, 0);
  let cut = 0;
  while (size > JOB_TEXT_LIMIT && cut < lines.length - 1) {
    size -= lines[cut]?.text.length ?? 0;
    cut += 1;
  }
  return cut === 0 ? { ...job, lines } : { ...job, lines: lines.slice(cut), dropped: job.dropped + cut };
}

function applyJobEvent(job: JobState, event: JobEvent): JobState {
  switch (event.kind) {
    case "job_output":
      return appendJobLine(job, { stream: event.stream, text: event.text });
    case "job_exited":
      return {
        ...job,
        job: {
          ...job.job,
          status: { state: "exited", code: event.code ?? null, signal: event.signal ?? null, endedAt: event.endedAt },
        },
      };
  }
}

function applyTerminalEvent(terminal: TerminalState, event: TerminalEvent): TerminalState {
  switch (event.kind) {
    case "output": {
      const scrollback = [...terminal.scrollback, event.data];
      let size = scrollback.reduce((total, chunk) => total + base64Bytes(chunk), 0);
      let cut = 0;
      let dropped = terminal.dropped;
      while (
        (size > TERMINAL_BYTES_LIMIT || scrollback.length - cut > TERMINAL_CHUNKS_LIMIT) &&
        cut < scrollback.length - 1
      ) {
        const bytes = base64Bytes(scrollback[cut] ?? "");
        size -= bytes;
        dropped += bytes;
        cut += 1;
      }
      return { ...terminal, scrollback: cut === 0 ? scrollback : scrollback.slice(cut), dropped };
    }
    case "exited":
      return {
        ...terminal,
        terminal: {
          ...terminal.terminal,
          status: { state: "exited", code: event.code ?? null, endedAt: event.endedAt },
        },
      };
  }
}

/** `jobs` with `job` in place of the job of its name, or inserted in name order. */
function upsertJob(jobs: readonly JobView[], job: JobView): JobView[] {
  const at = jobs.findIndex((candidate) => candidate.name === job.name);
  if (at >= 0) {
    return jobs.with(at, job);
  }
  const before = jobs.findIndex((candidate) => candidate.name > job.name);
  return before < 0 ? [...jobs, job] : [...jobs.slice(0, before), job, ...jobs.slice(before)];
}

/** Applies one event with the shared reducer and records the interactions it closed. */
function applyToView(view: ThreadView, event: ThreadEvent): ThreadView {
  const pending = view.state.interactions;
  const state = reduce(view.state, event);
  const left = pending.filter(
    (request) => !state.interactions.some((still) => still === request || jsonEqual(still, request)),
  );
  if (left.length === 0) {
    return { state, closed: view.closed };
  }
  const resolution = event.kind === "interaction_resolved" ? event.resolution : undefined;
  return { state, closed: [...view.closed, ...left.map((request) => ({ request, resolution }))].slice(-CLOSED_KEPT) };
}

/** Decodes the snapshot state of each keyed topic kind; `previous` is the topic's data before it. */
const SNAPSHOTS: { [K in KeyedKind]: (state: unknown, previous: Keyed[K] | undefined) => Decoded<Keyed[K]> } = {
  thread: (state) => {
    const thread = decodeThreadState(state);
    return thread.ok ? { ok: true, value: { state: thread.value, closed: [] } } : thread;
  },
  workspace: decodeRepoStatusList,
  views: decodeViewsState,
  job: decodeJobState,
  terminal: decodeTerminalState,
  jobs: (state, previous) => {
    const jobs = decodeJobsState(state);
    return jobs.ok ? { ok: true, value: { state: jobs.value, runs: previous?.runs ?? [] } } : jobs;
  },
};

/** The client's state. Mutated only by the session (and by tests); hosts read it through {@link ModelView}. */
export class Model implements ModelView {
  #connection: ConnectionStatus = { kind: "connecting" };
  #index: Index = EMPTY_INDEX;
  readonly #data: { [K in KeyedKind]: ReadonlyMap<string, Keyed[K]> } = {
    thread: new Map(),
    workspace: new Map(),
    views: new Map(),
    job: new Map(),
    terminal: new Map(),
    jobs: new Map(),
  };
  #runSeq = 0;
  readonly #runListeners = new Set<(workspaceId: string, event: RunEvent) => void>();
  #topics: readonly TopicState[] = [];
  #dirty: Dirty = emptyDirty();
  #revision = 0;

  get connection(): ConnectionStatus {
    return this.#connection;
  }

  get index(): Index {
    return this.#index;
  }

  get threads(): ReadonlyMap<string, ThreadView> {
    return this.#data.thread;
  }

  thread(id: string): ThreadView | undefined {
    return this.#data.thread.get(id);
  }

  get workspaces(): ReadonlyMap<string, readonly RepoStatus[]> {
    return this.#data.workspace;
  }

  get views(): ReadonlyMap<string, ViewsState> {
    return this.#data.views;
  }

  get jobs(): ReadonlyMap<string, JobState> {
    return this.#data.job;
  }

  get terminals(): ReadonlyMap<string, TerminalState> {
    return this.#data.terminal;
  }

  get workspaceJobs(): ReadonlyMap<string, WorkspaceJobs> {
    return this.#data.jobs;
  }

  onRunEvent(listener: (workspaceId: string, event: RunEvent) => void): () => void {
    return listen(this.#runListeners, listener);
  }

  get topics(): readonly TopicState[] {
    return this.#topics;
  }

  topic(topic: Topic): TopicState | undefined {
    return this.#topics.find((state) => state.topic === topic);
  }

  get revision(): number {
    return this.#revision;
  }

  /** What changed since the previous call. */
  takeDirty(): Dirty {
    const dirty = this.#dirty;
    this.#dirty = emptyDirty();
    return dirty;
  }

  setConnection(status: ConnectionStatus): void {
    if (!jsonEqual(this.#connection, status)) {
      this.#connection = status;
      this.#dirty.connection = true;
      this.#revision += 1;
    }
  }

  #topicsChanged(): void {
    this.#dirty.topics = true;
    this.#revision += 1;
  }

  /** Replaces the state of tracked `topic`; `false` when it is not tracked. */
  #updateTopic(topic: Topic, update: (state: TopicState) => TopicState): boolean {
    const at = this.#topics.findIndex((state) => state.topic === topic);
    const state = this.#topics[at];
    if (state === undefined) {
      return false;
    }
    this.#topics = this.#topics.with(at, update(state));
    return true;
  }

  /** Adds `topic` to the wanted subscriptions. Returns whether a subscribe request is due: the topic is new, or its
   * previous subscription failed. */
  track(topic: Topic): boolean {
    const state = this.topic(topic);
    if (state !== undefined) {
      return state.sync.kind === "failed";
    }
    this.#topics = [...this.#topics, { topic, position: undefined, sync: { kind: "subscribing" } }];
    this.#topicsChanged();
    return true;
  }

  /** Drops `topic` and its data. Returns whether it was subscribed. */
  untrack(topic: Topic): boolean {
    if (this.topic(topic) === undefined) {
      return false;
    }
    this.#topics = this.#topics.filter((state) => state.topic !== topic);
    const parsed = parseTopic(topic);
    if (parsed?.kind === "index") {
      this.#index = EMPTY_INDEX;
      this.#dirty.index = true;
    } else if (parsed !== undefined) {
      this.#set(parsed.kind, parsed.id, undefined);
    }
    this.#topicsChanged();
    return true;
  }

  /** The params to (re)subscribe `topic` from its last position; marks it subscribing. `undefined` when not
   * tracked. */
  subscribeParams(topic: Topic): SubscribeParams | undefined {
    const state = this.topic(topic);
    if (state === undefined) {
      return undefined;
    }
    if (state.sync.kind !== "subscribing") {
      this.#updateTopic(topic, (state) => ({ ...state, sync: { kind: "subscribing" } }));
      this.#topicsChanged();
    }
    return state.position === undefined ? { topic } : { topic, since: state.position };
  }

  /** `resync_required` (PRT-023), or a stream the client cannot continue: the next subscription asks for a snapshot.
   * The data stays until the snapshot replaces it. Returns whether the topic is tracked. */
  resync(topic: Topic): boolean {
    if (!this.#updateTopic(topic, (state) => ({ ...state, position: undefined, sync: { kind: "subscribing" } }))) {
      return false;
    }
    this.#topicsChanged();
    return true;
  }

  /** The connection dropped: live topics wait for their resubscription; positions are kept for `since`. */
  connectionLost(): void {
    if (!this.#topics.some((state) => state.sync.kind === "live")) {
      return;
    }
    this.#topics = this.#topics.map((state) =>
      state.sync.kind === "live" ? { ...state, sync: { kind: "subscribing" } } : state,
    );
    this.#topicsChanged();
  }

  /** The daemon refused to subscribe `topic`. */
  failTopic(topic: Topic, error: ErrorBody): void {
    if (this.#updateTopic(topic, (state) => ({ ...state, sync: { kind: "failed", error } }))) {
      this.#topicsChanged();
    }
  }

  /** Applies the answer to a subscribe request (PRT-020): a replay continues from the known position, a snapshot
   * replaces the topic's state and position. */
  applySubscribed(topic: Topic, result: SubscribeResult): Subscribed {
    const state = this.topic(topic);
    if (state === undefined) {
      return { ok: false, error: { kind: "not_tracked", topic } };
    }
    if (result.mode === "replay") {
      const { position } = state;
      if (position !== undefined && position.epoch === result.epoch && result.fromSeq === position.seq + 1) {
        this.#updateTopic(topic, (state) => ({ ...state, sync: { kind: "live", mode: "replay" } }));
        this.#topicsChanged();
        return { ok: true, mode: "replay" };
      }
      this.resync(topic);
      return { ok: false, error: { kind: "replay_mismatch", topic, epoch: result.epoch, fromSeq: result.fromSeq } };
    }
    const message = this.#replace(topic, result.state);
    if (message !== undefined) {
      this.failTopic(topic, { code: "internal", message });
      return { ok: false, error: { kind: "snapshot", topic, message } };
    }
    const position = { epoch: result.epoch, seq: result.seq };
    this.#updateTopic(topic, (state) => ({ ...state, position, sync: { kind: "live", mode: "snapshot" } }));
    this.#topicsChanged();
    return { ok: true, mode: "snapshot" };
  }

  /** Replaces a topic's data with a snapshot's `state`; the error message when it does not decode. */
  #replace(topic: Topic, state: unknown): string | undefined {
    const parsed = parseTopic(topic);
    if (parsed === undefined) {
      return `invalid topic ${topic}`;
    }
    if (parsed.kind !== "index") {
      return this.#replaceKeyed(parsed.kind, parsed.id, state);
    }
    const snapshot = decodeIndexSnapshot(state);
    if (!snapshot.ok) {
      return snapshot.error;
    }
    this.#index = indexFromSnapshot(snapshot.value);
    this.#dirty.index = true;
    return undefined;
  }

  #replaceKeyed<K extends KeyedKind>(kind: K, id: string, state: unknown): string | undefined {
    const decoded = SNAPSHOTS[kind](state, this.#data[kind].get(id));
    if (!decoded.ok) {
      return decoded.error;
    }
    this.#set(kind, id, decoded.value);
    return undefined;
  }

  /** Sets the data of keyed topic `kind`/`id`, or drops it for `undefined`. */
  #set<K extends KeyedKind>(kind: K, id: string, value: Keyed[K] | undefined): void {
    // Narrowed to `K`, the table's entry of `kind` takes a map of `Keyed[K]`.
    const data: { [P in K]: ReadonlyMap<string, Keyed[P]> } = this.#data;
    data[kind] = value === undefined ? without(data[kind], id) : upsert(data[kind], id, value);
    this.#dirty[DIRTY[kind]].add(id);
  }

  /** Applies a live event to the data of `parsed` when it is a topic of `kind` with data; the topic's id when it
   * did. */
  #edit<K extends KeyedKind>(
    parsed: ParsedTopic | undefined,
    kind: K,
    apply: (value: Keyed[K]) => Keyed[K],
  ): string | undefined {
    if (parsed === undefined || parsed.kind === "index" || parsed.kind !== kind) {
      return undefined;
    }
    const value = this.#data[kind].get(parsed.id);
    if (value === undefined) {
      return undefined;
    }
    this.#set(kind, parsed.id, apply(value));
    return parsed.id;
  }

  /** Applies one event frame if it continues its topic's stream (PRT-022): the next `seq` of the same epoch, or `seq`
   * 1 of a new epoch (a new omp process; the daemon's state carries over). Duplicates are ignored; a gap drops the
   * position and asks for a resubscription. */
  applyEvent(frame: EventFrame): Applied {
    const state = this.topic(frame.topic);
    if (state === undefined || state.sync.kind !== "live") {
      return "ignored";
    }
    const { position } = state;
    if (position?.epoch === frame.epoch && frame.seq <= position.seq) {
      return "ignored";
    }
    const next = position === undefined ? undefined : position.epoch === frame.epoch ? position.seq + 1 : 1;
    if (frame.seq !== next) {
      this.resync(frame.topic);
      return "gap";
    }
    this.#updateTopic(frame.topic, (state) => ({ ...state, position: { epoch: frame.epoch, seq: frame.seq } }));
    const parsed = parseTopic(frame.topic);
    const { event } = frame;
    // Decoding ties the event kind to the topic kind.
    switch (event.kind) {
      case "workspace_upserted":
      case "workspace_removed":
      case "thread_upserted":
      case "thread_removed":
      case "theme_changed":
      case "settings_changed":
      case "init_changed":
      case "models_changed":
      case "login_changed":
        if (parsed?.kind === "index") {
          this.#index = applyIndexEvent(this.#index, event);
          this.#dirty.index = true;
        }
        break;
      case "status":
        if (parsed?.kind === "workspace") {
          this.#set("workspace", parsed.id, event.repos);
        }
        break;
      case "views_manifest":
      case "views_widgets":
      case "views_keymap":
      case "views_init":
      case "widget_stored":
        this.#edit(parsed, "views", (views) => applyViewsEvent(views, event));
        break;
      case "job_output":
      case "job_exited":
        this.#edit(parsed, "job", (job) => applyJobEvent(job, event));
        break;
      case "output":
      case "exited":
        this.#edit(parsed, "terminal", (terminal) => applyTerminalEvent(terminal, event));
        break;
      case "jobs_set":
        this.#edit(parsed, "jobs", (jobs) => ({ ...jobs, state: { jobs: event.jobs, error: event.error ?? null } }));
        break;
      case "job_updated":
        this.#edit(parsed, "jobs", (jobs) => ({
          ...jobs,
          state: { ...jobs.state, jobs: upsertJob(jobs.state.jobs, event.job) },
        }));
        break;
      case "run_started":
      case "run_ended": {
        const applied: RunEvent = { seq: this.#runSeq + 1, kind: event.kind, run: event.run };
        const id = this.#edit(parsed, "jobs", (jobs) => ({
          ...jobs,
          runs: [...jobs.runs, applied].slice(-RUN_EVENTS_KEPT),
        }));
        if (id !== undefined) {
          this.#runSeq = applied.seq;
          // Synchronously, so a window that renders no frame for a while (a background tab) misses none.
          for (const listener of [...this.#runListeners]) {
            listener(id, applied);
          }
        }
        break;
      }
      default:
        if (parsed?.kind === "thread") {
          const view = this.#data.thread.get(parsed.id) ?? { state: emptyThreadState(), closed: [] };
          this.#set("thread", parsed.id, applyToView(view, event));
        }
    }
    this.#revision += 1;
    return "applied";
  }
}
