/**
 * The notice of a turn a daemon restart interrupted (UI-932, UI-933): at the bottom of the transcript while the
 * thread's summary carries `interruptedAt`, with Resume (`thread.resume`, as `:resume`); the notice goes once the
 * daemon clears the mark (a resume, or any message sent to the thread).
 */

import { Button, Icon } from "omfp/kit";
import { interruptedTurn, strings, useThreadActions, useThreadSummary } from "omfp/sdk";
import { type ReactNode, useState } from "react";

const text = strings.chat.interrupted;

export function InterruptedTurn({ threadId }: { threadId: string }): ReactNode {
  const turn = interruptedTurn(useThreadSummary(threadId));
  const actions = useThreadActions();
  const [pending, setPending] = useState(false);
  if (turn === undefined) {
    return null;
  }
  return (
    <div className="omfp-notice omfp-interrupted" role="status" data-testid="interrupted-turn" data-level="warning">
      <Icon name="circle-alert" />
      <span className="omfp-interrupted__text">{turn.text}</span>
      <Button
        size="sm"
        variant="primary"
        icon={<Icon name="play" />}
        isPending={pending}
        aria-description={text.resumeHint}
        data-testid="resume-turn"
        onPress={() => {
          setPending(true);
          void actions.resume(threadId).finally(() => setPending(false));
        }}
      >
        {text.resume}
      </Button>
    </div>
  );
}
