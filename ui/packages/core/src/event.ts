/**
 * Inputs of the thread reducer (port of `omfp-core/src/event.rs`): everything that changes a thread's visible state,
 * in the order the daemon applies them (ARC-006, DMN-040). The same sequence is replayed by clients and recorded as
 * fixtures (TST-012).
 */

import { bool, type Decoder, fail, i32, int, oneOf, optional, record, str } from "./decode.ts";
import { isObject, type JsonValue } from "./json.ts";
import type { AgentMessage, RpcNotification } from "./wire.ts";

/** Lifecycle of the omp process behind a thread (02-daemon, "Process states"). */
export type ProcessState =
  | { state: "stopped" }
  | { state: "starting" }
  | { state: "running" }
  /** Exited unexpectedly or failed to start (DMN-035). `stderr_tail` (last lines of stderr) is left out when empty. */
  | { state: "failed"; message: string; exit_code?: number; stderr_tail?: string };

/** The omp session a thread is bound to. */
export interface SessionInfo {
  session_id: string;
  session_file?: string;
  /** `true` when an existing session was resumed rather than created. */
  resumed: boolean;
}

/** Who answered an interactive request (DMN-051): a connected client, by its `system.hello` name. */
export interface Resolution {
  by: "client";
  client: string;
}

/**
 * Who started a live voice session (DMN-100): the daemon's number of the connection (the `connection` of its
 * `system.hello` result, PRT-051) and that connection's `system.hello` client name.
 */
export interface LiveOwner {
  client: string;
  connection: number;
}

/**
 * A file the agent asked to open with the `omfp_open_file` host tool (DMN-060): the window on connection `connection`
 * (the `connection` of its `system.hello` result, PRT-051), the one the user last used on the thread, opens `path` (as
 * the agent wrote it) at `line`, as a click on the path does. `id` tells one request from the next.
 */
export interface OpenFileRequest {
  id: string;
  connection: number;
  path: string;
  line: number | null;
}

/**
 * A notification the agent sends the user with the `omfp_notify` host tool (DMN-125): the window on connection
 * `connection` (PRT-051), the one the user last used on the thread, shows `title` and `body` as a notice about the
 * thread (UI-215). `id` tells one request from the next.
 */
export interface NotifyRequest {
  id: string;
  connection: number;
  title: string;
  body: string;
}

/** One input of the thread reducer, discriminated by `kind`. */
export type ThreadEvent =
  /** A frame from omp that is not a command response. */
  | { kind: "omp"; frame: RpcNotification }
  /** Transcript of the session right after the process started (DMN-032). Replaces the transcript. */
  | { kind: "history"; session: SessionInfo; messages: AgentMessage[] }
  /** Process lifecycle change. */
  | { kind: "process"; state: ProcessState }
  /** The daemon forwarded an answer to the interactive request with omp id `id` (DMN-051). */
  | { kind: "interaction_resolved"; id: string; resolution: Resolution }
  /**
   * The daemon opened a live voice session for `owner` before forwarding `live_connect` (DMN-100), or closed one omp
   * never started (`null`, DMN-101). omp's `live_*` frames arrive as `omp` events.
   */
  | { kind: "live_session"; owner: LiveOwner | null }
  /** The daemon asks one window to open a file for the agent (DMN-060). */
  | ({ kind: "open_file" } & OpenFileRequest)
  /** The daemon asks one window to notify the user for the agent (DMN-125). */
  | ({ kind: "notify" } & NotifyRequest)
  /** A frame omp-rpc could not decode, or a response nobody waited for (DMN-042). */
  | { kind: "unknown"; raw: JsonValue };

const failed = record<{ message: string; exit_code?: number; stderr_tail?: string }>({
  message: str,
  exit_code: optional(i32),
  stderr_tail: optional(str),
});

export const processState: Decoder<ProcessState> = (value) => {
  const state = isObject(value) ? value.state : undefined;
  switch (state) {
    case "stopped":
    case "starting":
    case "running":
      return { state };
    case "failed": {
      const { stderr_tail, ...rest } = failed(value);
      return stderr_tail === undefined || stderr_tail === "" ? { state, ...rest } : { state, ...rest, stderr_tail };
    }
    default:
      return fail();
  }
};

export const sessionInfo = record<SessionInfo>({ session_id: str, session_file: optional(str), resumed: bool });

export const resolution = record<Resolution>({ by: oneOf(["client"]), client: str });

/** A `u64`: a connection number, a line. */
const u64: Decoder<number> = (value) => {
  const decoded = int(value);
  return decoded >= 0 ? decoded : fail();
};

export const liveOwner = record<LiveOwner>({ client: str, connection: u64 });

const openFileFields = record<{ id: string; connection: number; path: string; line?: number }>({
  id: str,
  connection: u64,
  path: str,
  line: optional(u64),
});

/** An `open_file` event's request; a missing `line` is `null`, as the Rust state serializes it. */
export const openFileRequest: Decoder<OpenFileRequest> = (value) => {
  const { id, connection, path, line } = openFileFields(value);
  return { id, connection, path, line: line ?? null };
};

/** A `notify` event's request (PRT-065). */
export const notifyRequest = record<NotifyRequest>({ id: str, connection: u64, title: str, body: str });
