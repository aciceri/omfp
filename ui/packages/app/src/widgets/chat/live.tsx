/**
 * Live voice in the chat (UI-180..UI-186): the composer's mic button and the live panel above the composer — the
 * spectrum, the latest transcript line per role and the status line (`● listening · spc mute · esc end`), whose keys
 * work only while the panel has the focus, and its Mute and End buttons for touch screens without keys. This window's
 * session lives in the SDK's `LiveClient` (`useLive`).
 */

import type { ThreadState } from "@omfp/core";
import { Button, Icon, IconButton, type LiveHint, LivePanel, Spectrum } from "omfp/kit";
import {
  draftMicButton,
  keyLabel,
  livePanelView,
  type MicButton as MicButtonView,
  micButton,
  strings,
  useCommand,
  useLive,
  useSnapshot,
  useThreadActions,
} from "omfp/sdk";
import { type KeyboardEvent, useEffect, useId, useRef } from "react";

const text = strings.chat.live;

/** This window's connection number (`hello.connection`, PRT-051): who owns a session. */
function useConnectionNumber(): number | undefined {
  return useSnapshot((snapshot) =>
    snapshot.connection.kind === "connected" ? snapshot.connection.hello.connection : undefined,
  );
}

/** The live panel of thread `threadId`; nothing while no session runs and no failure shows. */
export function LiveVoicePanel({ threadId, state }: { threadId: string; state: ThreadState }) {
  const [live, snapshot] = useLive();
  const view = livePanelView(threadId, state, snapshot.local, useConnectionNumber(), snapshot.dismissed[threadId]);
  const panelRef = useRef<HTMLElement>(null);
  const own = view.kind === "own";
  // The panel takes the focus when this window's session starts, so Space and Esc reach it (UI-182).
  useEffect(() => {
    if (own) {
      panelRef.current?.focus();
    }
  }, [own]);
  if (view.kind === "none") {
    return null;
  }
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget || event.ctrlKey || event.altKey || event.metaKey) {
      return;
    }
    let handled = true;
    if (view.kind === "own" && event.key === " ") {
      live.toggleMute();
    } else if (view.kind === "own" && event.key === "Escape") {
      live.end();
    } else if (view.kind === "error" && event.key === "Escape") {
      live.dismiss(threadId, view.message);
    } else {
      handled = false;
    }
    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
  const common = {
    ref: panelRef,
    tabIndex: 0,
    "aria-label": text.panel,
    title: text.panel,
    "data-testid": "live-panel",
    "data-view": view.kind,
    onKeyDown,
  };
  if (view.kind === "error") {
    return (
      <LivePanel
        {...common}
        status="error"
        statusLabel={text.status.error}
        message={view.message}
        hints={[{ keys: keyLabel("Escape"), label: text.hint.dismiss }]}
        actions={
          <IconButton
            label={text.dismiss}
            icon={<Icon name="x" />}
            data-testid="live-dismiss"
            onPress={() => live.dismiss(threadId, view.message)}
          />
        }
      />
    );
  }
  if (view.kind === "elsewhere") {
    return (
      <LivePanel
        {...common}
        status={view.status}
        statusLabel={text.status[view.status]}
        lines={view.lines}
        note={view.behind ? `${text.elsewhere(view.client)} · ${text.behind}` : text.elsewhere(view.client)}
        actions={
          <Button
            variant="danger"
            size="sm"
            icon={<Icon name="square" />}
            data-testid="live-end"
            onPress={() => live.endThread(threadId)}
          >
            {text.endButton}
          </Button>
        }
      />
    );
  }
  const hints: LiveHint[] = [
    { keys: keyLabel("Space"), label: view.muted ? text.hint.unmute : text.hint.mute },
    { keys: keyLabel("Escape"), label: text.hint.end },
  ];
  return (
    <LivePanel
      {...common}
      status={view.status}
      statusLabel={text.status[view.status]}
      spectrum={<Spectrum source={live.source()} status={view.status} label={text.level} onLevels={live.noteLevels} />}
      lines={view.lines}
      note={view.behind ? text.behind : undefined}
      hints={hints}
      data-muted={view.muted}
      actions={
        <>
          <IconButton
            label={text.mute}
            icon={<Icon name={view.muted ? "mic-off" : "mic"} />}
            aria-pressed={view.muted}
            // Until omp answered there is nothing to mute (`toggleMute` waits for a running session).
            isDisabled={snapshot.local.kind !== "running"}
            data-testid="live-mute"
            onPress={() => live.toggleMute()}
          />
          <Button
            variant="danger"
            size="sm"
            icon={<Icon name="square" />}
            data-testid="live-end"
            onPress={() => live.end()}
          >
            {text.endButton}
          </Button>
        </>
      }
    />
  );
}

/**
 * The composer's mic button (UI-180): starts a live session, ends the running one, or says why it cannot start. It
 * registers `:live` (`live.toggle`, UI-108), which does the same from the keyboard, the Commands sheet and `/live`.
 */
export function MicButton({ threadId, state }: { threadId: string; state: ThreadState }) {
  const [live, snapshot] = useLive();
  return (
    <MicIconButton
      button={micButton(threadId, state, snapshot.local, live.support())}
      onStart={() => void live.start(threadId)}
      onEnd={() => live.endThread(threadId)}
    />
  );
}

/**
 * The draft thread's mic button (UI-023, UI-180): makes the thread and starts its live session at once; pressed while
 * that runs, it gives up. `disabled` while the draft's message is being sent.
 */
export function DraftMicButton({ disabled }: { disabled: boolean }) {
  const [live, snapshot] = useLive();
  const actions = useThreadActions();
  return (
    <MicIconButton
      button={draftMicButton(snapshot.local, live.support())}
      disabled={disabled}
      onStart={() => void actions.startDraftLive()}
      onEnd={() => live.end()}
    />
  );
}

/**
 * The mic icon button of a {@link MicButtonView}; an unavailable one says why in its accessible description. It
 * registers `:live` (`live.toggle`, UI-108), doing what pressing it does, so on the draft thread `:live` starts the
 * draft's mic (UI-023).
 */
function MicIconButton({
  button,
  disabled = false,
  onStart,
  onEnd,
}: {
  button: MicButtonView;
  disabled?: boolean;
  onStart: () => void;
  onEnd: () => void;
}) {
  const reasonId = useId();
  const toggle = () => {
    if (button.kind === "end") {
      onEnd();
    } else if (!disabled) {
      onStart();
    }
  };
  useCommand({ id: "live.toggle", title: text.toggle, aliases: ["live"], run: toggle });
  if (button.kind === "end") {
    return (
      <IconButton
        label={text.end}
        icon={<Icon name="mic" />}
        aria-pressed
        data-testid="live-mic"
        data-live="end"
        onPress={onEnd}
      />
    );
  }
  const reason = button.kind === "unavailable" ? text[button.reason] : undefined;
  return (
    <>
      <IconButton
        label={text.start}
        icon={<Icon name="mic" />}
        aria-pressed={false}
        {...(reason === undefined ? {} : { "aria-describedby": reasonId, "data-unavailable": true })}
        isDisabled={disabled}
        data-testid="live-mic"
        data-live={button.kind}
        onPress={onStart}
      />
      {reason !== undefined && (
        <span id={reasonId} className="omfp-visually-hidden">
          {reason}
        </span>
      )}
    </>
  );
}
