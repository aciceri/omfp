/**
 * The protocol session (03-protocol): `system.hello` with the token (PRT-001, PRT-002), request/response correlation
 * by id (PRT-010), subscriptions resumed from their `{epoch, seq}` (PRT-020..024), transient frames handed to their
 * listeners outside the model (PRT-054), the daemon's window requests handed to their listener (PRT-124), and
 * reconnect with exponential backoff capped at 10 s (UI-003). A refused token (`unauthorized`, close 4401) or protocol
 * version (`version_mismatch`, close 4426, PRT-004) stops the client instead of retrying.
 *
 * The session is an event-driven state machine: transport callbacks, timer callbacks and the host's commands each run
 * to completion synchronously; only call results are promises.
 */

import { isObject } from "@omfp/core";
import {
  type ErrorBody,
  type HelloParams,
  type MethodName,
  type ParamsOf,
  PROTOCOL_VERSION,
  parseTopic,
  type Response,
  type ResultOf,
  type Topic,
  type TransientFrame,
  type WindowRequestFrame,
} from "@omfp/protocol";
import { ChangeHub, Changes, listen } from "./changes.ts";
import { decodeHelloResult, decodeServerFrame, decodeSubscribeResult } from "./decode.ts";
import { type ConnectionStatus, type Dirty, Model, type ModelView } from "./model.ts";
import { systemTimer, type Timer } from "./timer.ts";
import { type Closed, type Connector, describeClosed, type Transport, TransportError } from "./transport.ts";

/** Close code of a connection that did not authenticate (PRT-001). */
export const CLOSE_UNAUTHORIZED = 4401;
/** Close code after `version_mismatch` (PRT-004). */
export const CLOSE_VERSION_MISMATCH = 4426;

/** Upper bound of every reconnect delay (UI-003). */
export const BACKOFF_CAP_MS = 10_000;

/** Reconnect delays: attempt `n` waits `initialMs · 2^(n-1)`, at most `maxMs` and never more than
 * {@link BACKOFF_CAP_MS} (UI-003). */
export interface Backoff {
  readonly initialMs: number;
  readonly maxMs: number;
}

/** 500 ms, doubling, capped at 10 s. */
export const DEFAULT_BACKOFF: Backoff = { initialMs: 500, maxMs: BACKOFF_CAP_MS };

/** Delay before reconnect attempt `attempt` (1 for the first retry). */
export function backoffDelay(backoff: Backoff, attempt: number): number {
  // 2^30 already exceeds any cap; the bound keeps `0 · 2^n` finite.
  const factor = 2 ** Math.min(Math.max(attempt - 1, 0), 30);
  return Math.min(backoff.initialMs * factor, backoff.maxMs, BACKOFF_CAP_MS);
}

/** Why a call has no result. */
export type CallFailure =
  /** The daemon answered with an error. */
  | { kind: "daemon"; error: ErrorBody }
  /** The request was sent, but the connection dropped before its response; it may or may not have run. */
  | { kind: "connection_lost" }
  /** The client stopped (closed, unauthenticated or refused version) before the call could be sent. */
  | { kind: "closed" }
  /** `system.hello`, `subscribe`, `unsubscribe` and `watch` belong to the session; use {@link Client.subscribe} and
   * {@link Client.watch}. */
  | { kind: "reserved"; method: string }
  /** The params did not encode or the response did not decode. */
  | { kind: "codec"; message: string };

function describeFailure(failure: CallFailure): string {
  switch (failure.kind) {
    case "daemon":
      return `${failure.error.message} (${failure.error.code})`;
    case "connection_lost":
      return "the connection was lost before the response arrived";
    case "closed":
      return "the client is closed";
    case "reserved":
      return `\`${failure.method}\` is managed by the client session`;
    case "codec":
      return failure.message;
  }
}

/** The rejection of a call. */
export class CallError extends Error {
  readonly failure: CallFailure;

  constructor(failure: CallFailure) {
    super(describeFailure(failure));
    this.name = "CallError";
    this.failure = failure;
  }
}

/** Methods the session sends itself. */
export type SessionMethod = "system.hello" | "subscribe" | "unsubscribe" | "watch";

/** Methods a host may call. */
export type CallMethod = Exclude<MethodName, SessionMethod>;

const SESSION_METHODS: Record<SessionMethod, true> = {
  "system.hello": true,
  subscribe: true,
  unsubscribe: true,
  watch: true,
};

/** How a client connects, what it says in `system.hello`, and how it retries. */
export interface ClientOptions {
  /** Opens every connection attempt, e.g. `webSocketConnector("ws://127.0.0.1:7420/api/ws")`. */
  readonly connector: Connector;
  /** The daemon's token; sent in `system.hello`, never in the URL (PRT-002). */
  readonly token: string;
  /** Client name recorded as provenance (PRT-033), e.g. `omfp/web`. */
  readonly client: string;
  /** Default: {@link DEFAULT_BACKOFF}. */
  readonly backoff?: Backoff;
  /** Default: {@link systemTimer}. */
  readonly timer?: Timer;
}

interface Call {
  readonly method: MethodName;
  readonly params: unknown;
  readonly resolve: (result: unknown) => void;
  readonly reject: (error: CallError) => void;
}

/** What a sent request waits for. */
type Pending =
  | { kind: "hello" }
  | { kind: "call"; call: Call }
  | { kind: "subscribe"; topic: Topic }
  | { kind: "unsubscribe" }
  | { kind: "watch" };

/** How a connection ended: retry after a backoff (`ready` when it had completed its hello), or stop for good. */
type Outcome = { kind: "lost"; error: string; ready: boolean } | { kind: "stop"; status: ConnectionStatus };

/** One connection attempt. */
class Link {
  transport: Transport | undefined;
  nextId = 1;
  /** `system.hello` succeeded. */
  ready = false;
  readonly pending = new Map<string, Pending>();
  /** Latest subscribe request per topic; older answers are stale. */
  readonly subscribing = new Map<Topic, string>();
}

/** The outcome of a failed `system.hello`. */
function refused(error: ErrorBody): Outcome {
  switch (error.code) {
    case "unauthorized":
      return { kind: "stop", status: { kind: "unauthenticated" } };
    case "version_mismatch": {
      const supported = isObject(error.details) ? error.details.supported : undefined;
      const versions =
        Array.isArray(supported) && supported.every((version) => Number.isSafeInteger(version) && version >= 0)
          ? supported.map(Number)
          : [];
      return { kind: "stop", status: { kind: "version_mismatch", supported: versions, message: error.message } };
    }
    default:
      return { kind: "lost", error: error.message, ready: false };
  }
}

function onClosed(link: Link, closed: Closed): Outcome {
  switch (closed.code) {
    case CLOSE_UNAUTHORIZED:
      return { kind: "stop", status: { kind: "unauthenticated" } };
    case CLOSE_VERSION_MISMATCH:
      return { kind: "stop", status: { kind: "version_mismatch", supported: [], message: closed.reason } };
    default:
      return { kind: "lost", error: describeClosed(closed), ready: link.ready };
  }
}

function transportError(error: unknown): TransportError {
  return error instanceof TransportError ? error : new TransportError(String(error));
}

/** A running client: one connection at a time, re-established until the client stops. */
export class Client {
  readonly #connector: Connector;
  readonly #timer: Timer;
  readonly #hello: HelloParams;
  readonly #backoff: Backoff;
  readonly #model = new Model();
  readonly #hub = new ChangeHub(() => this.#model.revision);
  readonly #done: Promise<void>;
  #finish: () => void = () => {};
  /** Calls waiting for a ready connection; never sent twice. */
  #queued: Call[] = [];
  #link: Link | undefined;
  #attempt = 0;
  #cancelRetry: (() => void) | undefined;
  #stopped = false;
  readonly #transient = new Set<(frame: TransientFrame) => void>();
  readonly #windowRequests = new Set<(frame: WindowRequestFrame) => void>();
  /** Followed thread topics kept without watching their thread (PRT-222), for every (re)subscription. */
  readonly #unwatched = new Set<Topic>();

  /** Starts connecting at once; see {@link connect}. */
  constructor(options: ClientOptions) {
    this.#connector = options.connector;
    this.#timer = options.timer ?? systemTimer;
    this.#backoff = options.backoff ?? DEFAULT_BACKOFF;
    this.#hello = { token: options.token, protocol: PROTOCOL_VERSION, client: options.client };
    this.#done = new Promise((resolve) => {
      this.#finish = resolve;
    });
    this.#open();
  }

  /** The client's state. Its objects are replaced, never mutated, so references can be compared between reads. */
  get model(): ModelView {
    return this.#model;
  }

  /** Settles once the client stopped for good: closed by the host, or the daemon refused the token or protocol. */
  get done(): Promise<void> {
    return this.#done;
  }

  /** What changed since the previous call (the model's dirty set, UI-012). */
  takeDirty(): Dirty {
    return this.#model.takeDirty();
  }

  /** Wakes the host after model changes, coalesced (UI-012). */
  changes(): Changes {
    return new Changes(this.#hub);
  }

  /**
   * Sends `method` now, or once connected, and resolves to its result; rejects with a {@link CallError}. A call made
   * while disconnected waits for the next connection; one in flight when the connection drops fails with
   * `connection_lost`. The result is the daemon's, typed by the protocol's method table but not validated.
   */
  call<M extends CallMethod>(method: M, params: ParamsOf<M>): Promise<ResultOf<M>> {
    if (Object.hasOwn(SESSION_METHODS, method)) {
      return Promise.reject(new CallError({ kind: "reserved", method }));
    }
    if (this.#stopped) {
      return Promise.reject(new CallError({ kind: "closed" }));
    }
    return new Promise<ResultOf<M>>((resolve, reject) => {
      // The daemon's result for `method` is `ResultOf<M>` (PRT-010).
      const call: Call = { method, params, resolve: resolve as (result: unknown) => void, reject };
      this.#command((link) => {
        if (link === undefined) {
          this.#queued.push(call);
        } else {
          this.#send(link, method, params, { kind: "call", call });
        }
      });
    });
  }

  /**
   * Calls `listener` with every transient frame (PRT-054) of a followed topic until the returned function is called.
   * Transient frames never touch the model: no revision, no dirty set, no replay after a reconnect.
   */
  onTransient(listener: (frame: TransientFrame) => void): () => void {
    return listen(this.#transient, listener);
  }

  /**
   * Calls `listener` with every request of the daemon to this window (PRT-124) until the returned function is called;
   * the listener answers it with `windows.answer`. Without a listener the client answers that it takes no requests.
   */
  onWindowRequest(listener: (frame: WindowRequestFrame) => void): () => void {
    return listen(this.#windowRequests, listener);
  }

  /** Follows `topic`: subscribed now if connected, and resubscribed from its last position after every reconnect. */
  subscribe(topic: Topic): void {
    this.#command((link) => {
      if (this.#model.track(topic) && link !== undefined) {
        this.#subscribe(link, topic);
      }
    });
  }

  /** Stops following `topic` and drops its data from the model. */
  unsubscribe(topic: Topic): void {
    this.#command((link) => {
      link?.subscribing.delete(topic);
      this.#unwatched.delete(topic);
      if (this.#model.untrack(topic) && link !== undefined) {
        this.#send(link, "unsubscribe", { topic }, { kind: "unsubscribe" });
      }
    });
  }

  /**
   * Whether followed `topic`, a `thread/<id>`, watches its thread (PRT-222): `false` keeps it followed only for a
   * quick return, so it keeps no omp alive (DMN-034). Changed on the live subscription with `watch` (PRT-223, no new
   * snapshot) and kept for every resubscription; a no-op for a topic not followed or already so.
   */
  watch(topic: Topic, watch: boolean): void {
    this.#command((link) => {
      const state = this.#model.topic(topic);
      if (state === undefined || this.#unwatched.has(topic) !== watch) {
        return;
      }
      if (watch) {
        this.#unwatched.delete(topic);
      } else {
        this.#unwatched.add(topic);
      }
      // A failed subscription has nothing to change; its retry subscribes with the flag.
      if (link !== undefined && state.sync.kind !== "failed") {
        this.#send(link, "watch", { topic, watch }, { kind: "watch" });
      }
    });
  }

  /** Closes the connection and stops reconnecting; pending calls fail. */
  close(): void {
    if (this.#stopped) {
      return;
    }
    const link = this.#link;
    if (link === undefined) {
      this.#run(() => this.#stop({ kind: "closed" }));
    } else {
      this.#run(() => this.#end(link, { kind: "stop", status: { kind: "closed" } }));
    }
  }

  /** Runs one step; notifies change watchers when the model's revision moved. */
  #run(step: () => void): void {
    const before = this.#model.revision;
    try {
      step();
    } finally {
      if (this.#model.revision !== before) {
        this.#hub.notify();
      }
    }
  }

  /** Runs a step of `link` unless it is no longer the current connection; a transport error or an outcome ends it. */
  #onLink(link: Link, step: () => Outcome | undefined): void {
    if (link !== this.#link) {
      return;
    }
    this.#run(() => {
      let outcome: Outcome | undefined;
      try {
        outcome = step();
      } catch (error) {
        if (!(error instanceof TransportError)) {
          throw error;
        }
        outcome = { kind: "lost", error: error.message, ready: link.ready };
      }
      if (outcome !== undefined) {
        this.#end(link, outcome);
      }
    });
  }

  /** Runs a host command with the ready connection, or `undefined` while there is none (handled offline). */
  #command(step: (link: Link | undefined) => void): void {
    if (this.#stopped) {
      return;
    }
    const link = this.#link;
    if (link?.ready) {
      this.#onLink(link, () => {
        step(link);
        return undefined;
      });
    } else {
      this.#run(() => step(undefined));
    }
  }

  #open(): void {
    const link = new Link();
    this.#link = link;
    let transport: Transport;
    try {
      transport = this.#connector({
        opened: () => this.#onLink(link, () => this.#onOpened(link)),
        received: (text) => this.#onLink(link, () => this.#onText(link, text)),
        closed: (closed) => this.#onLink(link, () => onClosed(link, closed)),
        failed: (error) => this.#onLink(link, () => ({ kind: "lost", error, ready: false })),
      });
    } catch (error) {
      this.#onLink(link, () => ({ kind: "lost", error: transportError(error).message, ready: false }));
      return;
    }
    if (this.#link === link) {
      link.transport = transport;
    } else {
      transport.close();
    }
  }

  /** Ends `link`: in-flight calls fail, then the client retries after a backoff or stops. */
  #end(link: Link, outcome: Outcome): void {
    if (link !== this.#link) {
      return;
    }
    this.#link = undefined;
    link.transport?.close();
    for (const pending of link.pending.values()) {
      if (pending.kind === "call") {
        pending.call.reject(new CallError({ kind: "connection_lost" }));
      }
    }
    link.pending.clear();
    if (outcome.kind === "stop") {
      this.#stop(outcome.status);
      return;
    }
    if (outcome.ready) {
      this.#attempt = 0;
    }
    this.#attempt += 1;
    const retryInMs = backoffDelay(this.#backoff, this.#attempt);
    this.#model.connectionLost();
    this.#model.setConnection({ kind: "reconnecting", attempt: this.#attempt, retryInMs, error: outcome.error });
    this.#cancelRetry = this.#timer.schedule(retryInMs, () => {
      this.#cancelRetry = undefined;
      if (!this.#stopped) {
        this.#run(() => this.#open());
      }
    });
  }

  #stop(status: ConnectionStatus): void {
    if (this.#stopped) {
      return;
    }
    this.#stopped = true;
    this.#cancelRetry?.();
    this.#cancelRetry = undefined;
    this.#model.connectionLost();
    this.#model.setConnection(status);
    const queued = this.#queued;
    this.#queued = [];
    for (const call of queued) {
      call.reject(new CallError({ kind: "closed" }));
    }
    this.#finish();
  }

  /**
   * Sends a request; its response is routed by `pending`. Returns the request id. Throws a {@link TransportError}
   * when the frame was not delivered: a call then goes back to the queue for the next connection.
   */
  #send(link: Link, method: MethodName, params: unknown, pending: Pending): string {
    const id = `c${link.nextId}`;
    link.nextId += 1;
    let text: string;
    try {
      text = JSON.stringify({ type: "request", id, method, params });
    } catch (error) {
      if (pending.kind === "call") {
        pending.call.reject(new CallError({ kind: "codec", message: String(error) }));
      }
      return id;
    }
    link.pending.set(id, pending);
    try {
      if (link.transport === undefined) {
        throw new TransportError("not connected");
      }
      link.transport.send(text);
    } catch (error) {
      link.pending.delete(id);
      if (pending.kind === "call") {
        this.#queued.push(pending.call);
      }
      throw transportError(error);
    }
    return id;
  }

  #onOpened(link: Link): undefined {
    this.#send(link, "system.hello", this.#hello, { kind: "hello" });
  }

  /** (Re)subscribes `topic` from its last position, unwatched when it is only kept (PRT-222). */
  #subscribe(link: Link, topic: Topic): void {
    const params = this.#model.subscribeParams(topic);
    if (params !== undefined) {
      const sent = this.#unwatched.has(topic) ? { ...params, watch: false } : params;
      link.subscribing.set(topic, this.#send(link, "subscribe", sent, { kind: "subscribe", topic }));
    }
  }

  /** Asks for a snapshot of `topic` (PRT-023), unless a subscribe request is already in flight: its answer starts a
   * stream the daemon still delivers. */
  #resync(link: Link, topic: Topic): void {
    if (!link.subscribing.has(topic) && this.#model.resync(topic)) {
      this.#subscribe(link, topic);
    }
  }

  /** Handles one daemon frame. */
  #onText(link: Link, text: string): Outcome | undefined {
    let value: unknown;
    try {
      value = JSON.parse(text);
    } catch {
      // Not JSON: nothing to answer or resync.
      return undefined;
    }
    const decoded = decodeServerFrame(value);
    if (!decoded.ok) {
      this.#onMalformed(link, value, decoded.error);
      return undefined;
    }
    const frame = decoded.value;
    switch (frame.type) {
      case "response":
        return this.#onResponse(link, frame);
      case "event":
        if (this.#model.applyEvent(frame) === "gap") {
          this.#subscribe(link, frame.topic);
        }
        return undefined;
      case "resync_required":
        this.#resync(link, frame.topic);
        return undefined;
      case "transient":
        if (this.#model.topic(frame.topic) !== undefined) {
          for (const listener of [...this.#transient]) {
            listener(frame);
          }
        }
        return undefined;
      case "window_request":
        this.#onWindowRequest(link, frame);
        return undefined;
    }
  }

  /** Hands a window request to the listeners (PRT-124); without one, answers that this client takes none. */
  #onWindowRequest(link: Link, frame: WindowRequestFrame): void {
    if (this.#windowRequests.size === 0) {
      const params = {
        requestId: frame.id,
        answer: {
          kind: "not_run" as const,
          reason: "failed" as const,
          message: "this client takes no window requests",
        },
      };
      const call: Call = { method: "windows.answer", params, resolve: () => {}, reject: () => {} };
      this.#send(link, "windows.answer", params, { kind: "call", call });
      return;
    }
    for (const listener of [...this.#windowRequests]) {
      listener(frame);
    }
  }

  /** A frame that does not decode: the request it answers fails (a bad hello drops the connection), and an event of
   * a followed topic resyncs that topic, since the stream can no longer be continued. */
  #onMalformed(link: Link, value: unknown, error: string): void {
    if (!isObject(value)) {
      return;
    }
    if (value.type === "response" && typeof value.id === "string") {
      const pending = link.pending.get(value.id);
      link.pending.delete(value.id);
      switch (pending?.kind) {
        case "hello":
          throw new TransportError(`invalid hello answer: ${error}`);
        case "call":
          pending.call.reject(new CallError({ kind: "codec", message: `invalid response: ${error}` }));
          break;
        case "subscribe":
          if (link.subscribing.get(pending.topic) === value.id) {
            link.subscribing.delete(pending.topic);
            this.#model.failTopic(pending.topic, { code: "internal", message: `invalid subscribe answer: ${error}` });
          }
          break;
        case "unsubscribe":
        case "watch":
        case undefined:
          break;
      }
    } else if (value.type === "event" && typeof value.topic === "string" && parseTopic(value.topic) !== undefined) {
      this.#resync(link, value.topic);
    }
  }

  #onResponse(link: Link, response: Response): Outcome | undefined {
    const pending = link.pending.get(response.id);
    if (pending === undefined) {
      return undefined;
    }
    link.pending.delete(response.id);
    switch (pending.kind) {
      case "hello":
        return this.#onHello(link, response);
      case "call":
        if (response.ok) {
          pending.call.resolve(response.result);
        } else {
          pending.call.reject(new CallError({ kind: "daemon", error: response.error }));
        }
        return undefined;
      case "subscribe":
        if (link.subscribing.get(pending.topic) === response.id) {
          link.subscribing.delete(pending.topic);
          this.#onSubscribed(link, pending.topic, response);
        }
        return undefined;
      case "unsubscribe":
      case "watch":
        return undefined;
    }
  }

  #onSubscribed(link: Link, topic: Topic, response: Response): void {
    if (!response.ok) {
      this.#model.failTopic(topic, response.error);
      return;
    }
    const result = decodeSubscribeResult(response.result);
    if (!result.ok) {
      this.#model.failTopic(topic, { code: "internal", message: `invalid subscribe result: ${result.error}` });
      return;
    }
    const applied = this.#model.applySubscribed(topic, result.value);
    if (!applied.ok && applied.error.kind === "replay_mismatch") {
      this.#subscribe(link, topic);
    }
  }

  #onHello(link: Link, response: Response): Outcome | undefined {
    if (!response.ok) {
      return refused(response.error);
    }
    const hello = decodeHelloResult(response.result);
    if (!hello.ok) {
      throw new TransportError(`invalid hello result: ${hello.error}`);
    }
    link.ready = true;
    this.#model.setConnection({ kind: "connected", hello: hello.value });
    for (const { topic } of this.#model.topics) {
      this.#subscribe(link, topic);
    }
    const queued = this.#queued;
    this.#queued = [];
    for (const [at, call] of queued.entries()) {
      try {
        this.#send(link, call.method, call.params, { kind: "call", call });
      } catch (error) {
        // The failed call went back to the queue; the rest follows it.
        this.#queued.push(...queued.slice(at + 1));
        throw error;
      }
    }
    return undefined;
  }
}

/** Creates a client and starts connecting through `options.connector`; it reconnects until {@link Client.close}, or
 * until the daemon refuses the token or protocol version. */
export function connect(options: ClientOptions): Client {
  return new Client(options);
}
