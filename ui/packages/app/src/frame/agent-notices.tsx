/**
 * Shows the notifications an agent sends the user with its `omfp_notify` tool (DMN-125, UI-215): the daemon names the
 * window the user last used on the thread by its connection number (PRT-051) in the thread's `notify`; that window
 * hands each request once to `host.notices.notify` as an `info` notice about the thread, so it is a system
 * notification while the window has no focus and a mode-line notice while it has, even on that very thread (the
 * agent asked); opening it (a click, `:notice`) selects the thread.
 */

import { type ReactNode, useEffect, useRef } from "react";
import { useHost, useSnapshot } from "../sdk/index.ts";

export function AgentNotices(): ReactNode {
  const { notices } = useHost();
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
      notices.notify({ title: request.title, body: request.body, threadId }, "info", { onThread: true });
    }
  }, [notices, connection, threads]);
  return null;
}
