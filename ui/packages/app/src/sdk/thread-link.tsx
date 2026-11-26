/**
 * Links to a thread (UI-045, UI-046): what the kit's `Markdown` renders for `omfp://thread/<id>` when given
 * `threadLink={ThreadLink}` (the chat's replies, any widget). A click selects the thread in this window as its row does
 * (a phone then shows the chat, UI-204); a middle click, or a click with Ctrl, Cmd or Shift, opens a new window on it
 * (`#thread=<id>`, UI-218; the desktop window starts one more `omfp desktop --thread`, UI-047). A link whose text is
 * the bare id shows the thread's current title and state mark; a thread missing from the index is plain text.
 */

import { type MarkdownThreadLinkProps, StateMark } from "@omfp/kit";
import type { MouseEvent, ReactNode } from "react";
import { displayTitle, statusLabel, threadMark, threadStatus } from "../vm/threads.ts";
import { useThreadActions } from "./actions.ts";
import { useThreadSummary } from "./hooks.ts";

/** The address of a new window on `threadId`: `page` (this window's address) with `#thread=<id>` as its fragment. */
export function threadWindowUrl(page: string, threadId: string): string {
  const url = new URL(page);
  url.hash = `thread=${encodeURIComponent(threadId)}`;
  return url.href;
}

/** A link to thread `threadId` (UI-045): pass it to the kit's `Markdown` as `threadLink`. */
export function ThreadLink({ threadId, bare, children }: MarkdownThreadLinkProps): ReactNode {
  const thread = useThreadSummary(threadId);
  const actions = useThreadActions();
  if (thread === undefined) {
    return <span data-testid="thread-link-text">{children}</span>;
  }
  const status = threadStatus(thread);
  const title = displayTitle(thread);
  const follow = (event: MouseEvent) => {
    // The window acts, never the page's own navigation. A middle click, or Ctrl, Cmd or Shift held: a new window.
    event.preventDefault();
    if (event.button === 1 || event.ctrlKey || event.metaKey || event.shiftKey) {
      window.open(threadWindowUrl(window.location.href, threadId), "_blank", "noopener");
    } else {
      actions.select(threadId);
    }
  };
  return (
    <a
      className="omfp-thread-link"
      href={`#thread=${encodeURIComponent(threadId)}`}
      title={title}
      data-testid="thread-link"
      data-thread-id={threadId}
      data-status={status}
      onClick={follow}
      onAuxClick={(event) => {
        if (event.button === 1) {
          follow(event);
        }
      }}
      onMouseDown={(event) => {
        // A middle press would start the browser's autoscroll.
        if (event.button === 1) {
          event.preventDefault();
        }
      }}
    >
      {bare ? (
        <>
          <StateMark state={threadMark(status)} label={statusLabel(status)} />
          {title}
        </>
      ) : (
        children
      )}
    </a>
  );
}
