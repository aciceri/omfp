/**
 * Protocol client of the omfp daemon (docs/specs/04-ui.md, "State and code structure"), without any UI.
 *
 * - Transport: a {@link Connector} opens a {@link Transport} per connection attempt and reports to a
 *   {@link TransportListener}; {@link webSocketConnector} uses the standard `WebSocket` (browser or Bun), checked by
 *   the transport suite against a real daemon (UI-001, TST-030).
 * - Session: {@link connect} returns a {@link Client} that sends `system.hello`, correlates responses by id, keeps
 *   subscriptions with their `{epoch, seq}`, reconnects with capped exponential backoff and resubscribes (UI-003),
 *   queues calls made while disconnected, and stops on `unauthorized` / `version_mismatch`. Calls are typed by the
 *   protocol's `Methods` table: `client.call("threads.create", { workspace: { new: {} }, title })`.
 * - Model: `client.model` ({@link ModelView}) holds the connection status, the index, per-thread `ThreadState` from
 *   the shared reducer with the interactions it closed, and workspace git status; its objects are immutable. Every
 *   change bumps `revision` and lands in a {@link Dirty} set the host drains with `client.takeDirty()`, and
 *   `client.changes().next()` wakes the host at most once for any number of changes (UI-012).
 * - Clock: reconnect delays go through a {@link Timer} ({@link systemTimer} by default), so tests use virtual time.
 *
 * ```ts
 * const client = connect({ connector: webSocketConnector(url), token, client: "omfp/web" });
 * client.subscribe(INDEX_TOPIC);
 * const changes = client.changes();
 * for (;;) {
 *   await changes.next(); // then render on the next animation frame
 *   const dirty = client.takeDirty();
 *   render(client.model, dirty);
 * }
 * ```
 */

export { Changes } from "./changes.ts";
export {
  type Applied,
  CLOSED_KEPT,
  type ClosedInteraction,
  type ConnectionStatus,
  type Dirty,
  type Index,
  type InteractionRequest,
  isDirtyEmpty,
  JOB_TEXT_LIMIT,
  Model,
  type ModelView,
  type StartMode,
  type Subscribed,
  type SubscribeError,
  TERMINAL_BYTES_LIMIT,
  TERMINAL_CHUNKS_LIMIT,
  type ThreadView,
  type TopicState,
  type TopicSync,
} from "./model.ts";
export {
  BACKOFF_CAP_MS,
  type Backoff,
  backoffDelay,
  CallError,
  type CallFailure,
  type CallMethod,
  CLOSE_UNAUTHORIZED,
  CLOSE_VERSION_MISMATCH,
  Client,
  type ClientOptions,
  connect,
  DEFAULT_BACKOFF,
  type SessionMethod,
} from "./session.ts";
export { systemTimer, type Timer } from "./timer.ts";
export {
  type Closed,
  type Connector,
  describeClosed,
  type Transport,
  TransportError,
  type TransportListener,
} from "./transport.ts";
export { webSocketConnector } from "./websocket.ts";
