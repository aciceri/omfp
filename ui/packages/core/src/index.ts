/**
 * `@omfp/core`: the thread model and pure reducer over omp events, a port of the Rust `omfp-core` crate. No I/O, no
 * runtime dependencies; the state is the JSON the Rust `ThreadState` serializes to.
 */

export { validateAskAnswers } from "./ask.ts";
export type {
  LiveOwner,
  NotifyRequest,
  OpenFileRequest,
  ProcessState,
  Resolution,
  SessionInfo,
  ThreadEvent,
} from "./event.ts";
export type { JsonObject, JsonValue } from "./json.ts";
export { liveErrorMessage } from "./live.ts";
export { dialogId, reduce, replay } from "./reducer.ts";
export {
  type BackgroundJob,
  emptyThreadState,
  type JobState,
  type LiveLine,
  type LiveSession,
  type Notice,
  type NoticeFrame,
  type Presentation,
  type QueueState,
  type StreamingMessage,
  type Subagent,
  type ThreadState,
  type ToolRun,
  type ToolStatus,
  type TranscriptEntry,
  type Widget,
} from "./state.ts";
export type * from "./wire.ts";

/**
 * The omp release whose RPC protocol this build speaks; equals `omfp_core::OMP_VERSION`. The golden-state test replays
 * the fixtures recorded for this version.
 */
export const OMP_VERSION = "18.22.0";
