/**
 * Frames on the WebSocket (mirrors `omfp_protocol::envelope`): client requests, and daemon responses, events,
 * `resync_required`, transient frames and window requests. Hand-written: their serde forms are custom, so they have
 * no JSON Schema.
 */

import type { ThreadEvent } from "@omfp/core";
import type {
  ErrorBody,
  IndexEvent,
  JobEvent,
  MethodName,
  ParamsOf,
  TerminalEvent,
  Topic,
  TransientEvent,
  ViewsEvent,
  WindowRequest,
  WorkspaceEvent,
} from "./generated.ts";

/** A client request (PRT-010); `id` is chosen by the client, unique per connection, and echoed by the response. */
export type Request<M extends MethodName = MethodName> = {
  [K in M]: { type: "request"; id: string; method: K; params: ParamsOf<K> };
}[M];

/** A successful response: `result` is the method's result (`ResultOf<M>` of the request's method). */
export type SuccessResponse = { type: "response"; id: string; ok: true; result: unknown };

/** A failed response (PRT-011). */
export type FailureResponse = { type: "response"; id: string; ok: false; error: ErrorBody };

/** The answer to a request. */
export type Response = SuccessResponse | FailureResponse;

/** The `event` of an event frame; its kind follows from the topic. */
export type TopicEvent = IndexEvent | ThreadEvent | WorkspaceEvent | ViewsEvent | JobEvent | TerminalEvent;

/** One event of a topic (PRT-022): `seq` is strictly increasing, without gaps, within an `epoch`. */
export type EventFrame = { type: "event"; topic: Topic; epoch: string; seq: number; event: TopicEvent };

/** The client's queue overflowed: the topic was dropped and must be resubscribed (PRT-023). */
export type ResyncRequired = { type: "resync_required"; topic: Topic };

/**
 * A momentary value of a topic outside its event stream (PRT-054): no `seq`, not replayed, not part of the topic's
 * state, dropped when the client falls behind. A thread's `live_levels` (DMN-110).
 */
export type TransientFrame = { type: "transient"; topic: Topic; event: TransientEvent };

/**
 * A request of the daemon to this window (PRT-124): run a command or render a widget out of sight; the window answers
 * with `windows.answer { requestId: id, answer }`.
 */
export type WindowRequestFrame = { type: "window_request"; id: string; request: WindowRequest };

/** A frame from the daemon. */
export type ServerFrame = Response | EventFrame | ResyncRequired | TransientFrame | WindowRequestFrame;

/** A request frame for `method`. */
export function request<M extends MethodName>(id: string, method: M, params: ParamsOf<M>): Request<M> {
  // The mapped type collapses to this object for a single method; TypeScript cannot see it through the generic.
  return { type: "request", id, method, params } as Request<M>;
}
