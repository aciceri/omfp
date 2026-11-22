/**
 * `omfp:chat`: the conversation of the selected thread — the transcript with the notice of a turn a daemon restart
 * interrupted (UI-932) and the pending interactions inline at its bottom (UI-070), then the queue chips and the
 * composer; with no thread selected, the draft thread (UI-023).
 */

import { DRAFT_THREAD, strings, useWindowState } from "omfp/sdk";
import { ComposerWidget } from "./composer.tsx";
import { Interactions } from "./interactions.tsx";
import { InterruptedTurn } from "./interrupted.tsx";
import type { WidgetProps } from "./props.ts";
import { useSelectedThread } from "./state.ts";
import { ThreadPlaceholder, Transcript } from "./transcript.tsx";

export function ChatWidget(_props: WidgetProps) {
  const { threadId, view } = useSelectedThread();
  const [draft] = useWindowState(DRAFT_THREAD);
  if (threadId === undefined && draft !== undefined) {
    return (
      <section className="omfp-chat" data-testid="draft-thread-view" data-workspace-id={draft.workspaceId}>
        <div className="omfp-chat__empty" data-testid="draft-empty">
          {strings.shell.draft.empty}
        </div>
        <ComposerWidget />
      </section>
    );
  }
  if (threadId === undefined || view === undefined) {
    return <ThreadPlaceholder threadId={threadId} />;
  }
  return (
    <section className="omfp-chat" data-testid="thread" data-thread-id={threadId}>
      <Transcript
        key={threadId}
        threadId={threadId}
        state={view.state}
        footer={
          <>
            <InterruptedTurn threadId={threadId} />
            <Interactions threadId={threadId} view={view} />
          </>
        }
      />
      <ComposerWidget />
    </section>
  );
}
