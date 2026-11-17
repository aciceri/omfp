/**
 * Finishing an OAuth sign-in from a browser on another machine (UI-761): omp waits for the provider's redirect on a
 * loopback listener of the daemon's host, so the browser here ends on a `localhost` page that fails to load. The user
 * pastes that page's address, and the daemon requests it on its own loopback (`oauth.callback`, PRT-212).
 */

import { Button, Icon, TextField } from "@omfp/kit";
import { type FormEvent, type ReactNode, useState } from "react";
import { strings } from "../strings/index.ts";
import { loopbackCallback } from "../vm/opened-url.ts";
import { useUrlActions } from "./actions.ts";

const text = strings.chat.notice;

/**
 * "Paste the address your browser ended on" and Send, for `url`, an authorization URL omp opened (thread
 * `threadId`'s, else the running login's); nothing when its sign-in does not return to a loopback address
 * ({@link loopbackCallback}). A refusal goes to the error list and the field stays; once omp took the address the field
 * goes and `onRelayed` runs.
 */
export function OAuthCallbackField({
  threadId,
  url,
  onRelayed,
}: {
  threadId: string | undefined;
  url: string;
  onRelayed?: () => void;
}): ReactNode {
  const urls = useUrlActions();
  const [value, setValue] = useState("");
  const [sending, setSending] = useState(false);
  const [relayed, setRelayed] = useState(false);
  const callback = loopbackCallback(url);
  if (callback === undefined || relayed) {
    return null;
  }
  const send = (event: FormEvent) => {
    event.preventDefault();
    const pasted = value.trim();
    if (pasted === "" || sending) {
      return;
    }
    setSending(true);
    void urls.relay(threadId, pasted).then((ok) => {
      setSending(false);
      if (ok) {
        setRelayed(true);
        onRelayed?.();
      }
    });
  };
  return (
    <form className="omfp-oauth-callback" data-testid="oauth-callback" data-callback={callback} onSubmit={send}>
      <TextField
        label={text.relayLabel}
        value={value}
        onChange={setValue}
        autoComplete="off"
        mono
        placeholder={text.relayPlaceholder(callback)}
        inputProps={{ "data-testid": "oauth-callback-input" }}
      />
      <Button
        type="submit"
        size="sm"
        icon={<Icon name="arrow-up" />}
        isPending={sending}
        data-testid="oauth-callback-send"
      >
        {text.relayAction}
      </Button>
    </form>
  );
}
