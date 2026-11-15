/**
 * Subscription topics (mirrors `omfp_protocol::Topic`): on the wire a name, `index` or `<kind>/<id>`.
 */

import type { Topic } from "./generated.ts";

/** A topic name taken apart. */
export type ParsedTopic =
  | { kind: "index" }
  | { kind: "thread" | "workspace" | "views" | "job" | "terminal"; id: string };

/** The `index` topic: workspaces and thread summaries. */
export const INDEX_TOPIC: Topic = "index";

/** Ids are ULIDs today; any non-empty `[A-Za-z0-9_-]` string is accepted. */
const ID = /^[A-Za-z0-9_-]+$/;

/** Parses a topic name; `undefined` unless it is `index` or `<thread|workspace|views|job>/<id>`. */
export function parseTopic(name: string): ParsedTopic | undefined {
  if (name === "index") {
    return { kind: "index" };
  }
  const slash = name.indexOf("/");
  if (slash < 0) {
    return undefined;
  }
  const kind = name.slice(0, slash);
  const id = name.slice(slash + 1);
  if (!ID.test(id)) {
    return undefined;
  }
  switch (kind) {
    case "thread":
    case "workspace":
    case "views":
    case "job":
    case "terminal":
      return { kind, id };
    default:
      return undefined;
  }
}

/** The wire name of a topic. */
export function formatTopic(topic: ParsedTopic): Topic {
  return topic.kind === "index" ? INDEX_TOPIC : `${topic.kind}/${topic.id}`;
}

/** The topic of one thread's events. */
export function threadTopic(threadId: string): Topic {
  return formatTopic({ kind: "thread", id: threadId });
}

/** The topic of one workspace's git status. */
export function workspaceTopic(workspaceId: string): Topic {
  return formatTopic({ kind: "workspace", id: workspaceId });
}

/** The topic of one workspace's views manifest, widget folders and widget state (PRT-040). */
export function viewsTopic(workspaceId: string): Topic {
  return formatTopic({ kind: "views", id: workspaceId });
}

/** The topic of one job's output (PRT-041). */
export function jobTopic(jobId: string): Topic {
  return formatTopic({ kind: "job", id: jobId });
}

/** The topic of one daemon-owned PTY session. */
export function terminalTopic(terminalId: string): Topic {
  return formatTopic({ kind: "terminal", id: terminalId });
}
