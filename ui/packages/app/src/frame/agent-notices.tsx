/**
 * Shows the notifications an agent sends the user with its `omfp_notify` tool (DMN-125, UI-215): the daemon names the
 * window the user last used on the thread (or the one that keeps it, DMN-376) by its connection number (PRT-051) in
 * the thread's `notify`; that window hands each request once to `host.notices.notify` as an `info` notice about the
 * thread, so it is a system notification while the window has no focus and a mode-line notice while it has, even on
 * that very thread (the agent asked); opening it (a click, `:notice`) selects the thread. With no window subscribing
 * the thread the daemon sends the user's most recently focused window a `notify` window request instead (DMN-375),
 * shown the same way (UI-936). A notice about a thread the window does not show names the thread (UI-937).
 */

import { type ReactNode, useEffect, useRef } from "react";
import { type OmfpHost, SELECTED_THREAD, useHost, useSnapshot } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";

/** Hands an agent's notification about `threadId` to the window's notices, naming the thread when not shown. */
function notifyAbout(host: OmfpHost, threadId: string, title: string, body: string): void {
  const thread = host.frames.getSnapshot().index.threads.get(threadId);
  const named =
    thread === undefined || host.window.get(SELECTED_THREAD) === threadId
      ? title
      : strings.shell.notices.agentTitle(thread.title, title);
  host.notices.notify({ title: named, body, threadId }, "info", { onThread: true });
}

export function AgentNotices(): ReactNode {
  const host = useHost();
  const connection = useSnapshot((snapshot) =>
    snapshot.connection.kind === "connected" ? snapshot.connection.hello.connection : undefined,
  );
  const threads = useSnapshot((snapshot) => snapshot.threads);
  // A thread followed again, or its snapshot after a resync, carries the request it already showed.
  const shown = useRef(new Set<string>());
  useEffect(() => {
    for (const [threadId, { state }] of threads) {
      const request = state.notify;
      if (request === null || request.connection !== connection || shown.current.has(request.id)) {
        continue;
      }
      shown.current.add(request.id);
      notifyAbout(host, threadId, request.title, request.body);
    }
  }, [host, connection, threads]);
  useEffect(
    () =>
      host.client.onWindowRequest(({ request }) => {
        if (request.kind === "notify") {
          notifyAbout(host, request.threadId, request.title, request.body);
        }
      }),
    [host],
  );
  return null;
}
