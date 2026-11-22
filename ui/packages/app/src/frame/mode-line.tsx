/**
 * The mode line (UI-098): the keymap mode (UI-095) the focused element puts the window in, shown at the bottom left
 * of the view area like vim's, and announced through the live region when a key changed it (never the mouse); the
 * window's views as small tabs when it has several (UI-301).
 */

import {
  type NoticeTone as KitTone,
  ModeLine,
  ModeMessages,
  ModeNotice,
  ModeSegment,
  ModeViews,
  StateMark,
  type VimMode,
} from "@omfp/kit";
import { type ReactNode, useEffect, useState } from "react";
import { modeAt } from "../keymap/handler.ts";
import { type Binding, keyLabel, type Mode } from "../keymap/keymap.ts";
import {
  FOCUS_COMPOSER,
  MESSAGES,
  NOTICES,
  type Notice,
  type NoticeTone,
  RENAMING_THREAD,
  SELECTED_THREAD,
  useAnnounce,
  useCommand,
  useHost,
  useThreadActions,
  useThreadSummary,
  useWindowState,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { displayTitle, statusLabel, threadMark, threadStatus } from "../vm/threads.ts";
import { ThreadMoreButton, ThreadRename } from "../widgets/header/header.tsx";
import { MESSAGES_COMMAND } from "./messages.tsx";

const text = strings.shell.mode;

/** This window's keymap mode, live; announces the changes a key caused. */
export function useMode(): Mode {
  const [mode, setMode] = useState<Mode>(() => modeAt(document.activeElement));
  const announce = useAnnounce();
  useEffect(() => {
    let byKey = false;
    let current = modeAt(document.activeElement);
    const onKey = () => {
      byKey = true;
    };
    const onPointer = () => {
      byKey = false;
    };
    const update = (focused: Element | null) => {
      const next = modeAt(focused);
      if (next === current) {
        return;
      }
      current = next;
      setMode(next);
      if (byKey) {
        announce(text.announce(text.names[next]));
      }
    };
    const onFocusIn = (event: FocusEvent) => update(event.target instanceof Element ? event.target : null);
    // While focus leaves, `relatedTarget` is where it goes (null: nowhere, the body).
    const onFocusOut = (event: FocusEvent) =>
      update(event.relatedTarget instanceof Element ? event.relatedTarget : null);
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onPointer, true);
    document.addEventListener("mousedown", onPointer, true);
    document.addEventListener("focusin", onFocusIn);
    document.addEventListener("focusout", onFocusOut);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onPointer, true);
      document.removeEventListener("mousedown", onPointer, true);
      document.removeEventListener("focusin", onFocusIn);
      document.removeEventListener("focusout", onFocusOut);
    };
  }, [announce]);
  return mode;
}

/** The window's views as the mode line's tabs show them (UI-301). */
export interface ModeLineViews {
  readonly views: readonly { readonly id: string; readonly title: string }[];
  /** The view the window shows. */
  readonly current: string | undefined;
  readonly onSelect: (id: string) => void;
}

/**
 * The window's mode line, the kit's (UI-167, UI-441): `lead` (Show threads while the wide left column is collapsed,
 * UI-986), the mode pill — the keymap's mode, or vim's while a file editor has the focus (`vim`, UI-112) — then the
 * selected thread's title and state, the window's views when it has several (UI-301), and the newest notice at the
 * right end (UI-192) with the messages button (UI-523); in command mode `children` (the `:` line or the file picker)
 * replace the segments, the views and the notice. On a phone (`phone`) it stands at the top in place of the chat
 * header (UI-419): the state is its mark, and the thread's `⋯` ends the line.
 */
export function WindowModeLine({
  mode,
  vim,
  bindings,
  views,
  lead,
  phone = false,
  children,
}: {
  mode: Mode;
  vim?: VimMode | undefined;
  /** The window's bindings, for the key chip of `notice.open`. */
  bindings: readonly Binding[];
  views?: ModeLineViews | undefined;
  lead?: ReactNode;
  phone?: boolean;
  children?: ReactNode;
}): ReactNode {
  const shown = vim === undefined ? mode : vim === "insert" || vim === "replace" ? "insert" : "normal";
  const pill = shown === "cmdline" ? "command" : shown;
  const open = useOpenNotice();
  return (
    <ModeLine
      className="omfp-frame__mode"
      data-testid="mode-bar"
      lead={lead}
      mode={pill}
      modeLabel={vim === undefined ? text.names[mode] : text.vim[vim]}
      pillProps={{ "data-testid": "mode-line", "data-vim": vim }}
      views={
        mode !== "cmdline" &&
        views !== undefined &&
        views.views.length > 1 && <ModeViews label={strings.shell.frame.viewTabs} data-testid="mode-views" {...views} />
      }
      notice={
        <>
          {mode !== "cmdline" && (
            <>
              <NoticeSlot bindings={bindings} open={open} />
              <MessagesButton />
            </>
          )}
          {phone && <ThreadMore shown={mode !== "cmdline"} />}
        </>
      }
    >
      {mode !== "cmdline" && <ThreadSegments phone={phone} />}
      {children}
    </ModeLine>
  );
}

const KIT_TONES: Readonly<Record<NoticeTone, KitTone>> = {
  attention: "attention",
  failed: "error",
  finished: "ok",
  info: "info",
};

/**
 * Opens the newest notice (`notice.open`, `:notice`, or a click on it): selects its thread, if any, with the
 * focus in its composer (UI-286), and drops it; visiting the thread clears the thread's other notices.
 */
function useOpenNotice(): () => void {
  const { notices, window } = useHost();
  const actions = useThreadActions();
  const open = () => {
    const newest: Notice | undefined = window.get(NOTICES).at(-1);
    if (newest === undefined) {
      return;
    }
    notices.dismiss(newest.id);
    if (newest.threadId !== undefined) {
      actions.select(newest.threadId);
      window.set(FOCUS_COMPOSER, newest.threadId);
    }
  };
  useCommand({
    id: "notice.open",
    title: strings.shell.notices.open,
    aliases: ["notice"],
    enabled: () => window.get(NOTICES).length > 0,
    run: open,
  });
  return open;
}

/** The newest notice and how many more wait behind it. */
function NoticeSlot({ bindings, open }: { bindings: readonly Binding[]; open: () => void }): ReactNode {
  const [shown] = useWindowState(NOTICES);
  const newest = shown.at(-1);
  if (newest === undefined) {
    return null;
  }
  const key = bindings.find((binding) => binding.mode === "normal" && binding.command === "notice.open")?.keys;
  return (
    <ModeNotice
      key={newest.id}
      data-testid="mode-notice"
      data-thread={newest.threadId}
      tone={KIT_TONES[newest.tone]}
      title={newest.title}
      body={newest.body}
      more={shown.length - 1}
      {...(key === undefined ? {} : { kbd: keyLabel(key) })}
      onPress={open}
    />
  );
}

/** Opens the window's messages (UI-523), once there are some: the way back to notices gone from the mode line. */
function MessagesButton(): ReactNode {
  const { commands } = useHost();
  const [messages] = useWindowState(MESSAGES);
  if (messages.length === 0) {
    return null;
  }
  return (
    <ModeMessages
      label={strings.shell.messages.open}
      data-testid="mode-messages"
      onPress={() => commands.run(MESSAGES_COMMAND)}
    />
  );
}

/** The selected thread's title and state: on a phone its mark, a spinner while it works, with the word as its name,
 * and its rename editor in their place while it is renamed (UI-419). */
function ThreadSegments({ phone }: { phone: boolean }): ReactNode {
  const [selected] = useWindowState(SELECTED_THREAD);
  const [renaming] = useWindowState(RENAMING_THREAD);
  const summary = useThreadSummary(selected);
  if (summary === undefined) {
    return null;
  }
  if (phone && renaming === summary.id) {
    return <ThreadRename summary={summary} />;
  }
  const status = threadStatus(summary);
  return (
    <>
      <ModeSegment kind="title" data-testid="mode-thread-title">
        {displayTitle(summary)}
      </ModeSegment>
      <ModeSegment data-testid="mode-thread-state" data-status={status}>
        {phone ? <StateMark state={threadMark(status)} label={statusLabel(status)} /> : statusLabel(status)}
      </ModeSegment>
    </>
  );
}

/** The selected thread's `⋯` (UI-419), keyed by the thread so its popover and session follow the selection; mounted
 * in command mode too, its button hidden (`shown`), so `:model` stays registered. */
function ThreadMore({ shown }: { shown: boolean }): ReactNode {
  const [selected] = useWindowState(SELECTED_THREAD);
  const summary = useThreadSummary(selected);
  return summary === undefined ? null : <ThreadMoreButton key={summary.id} summary={summary} shown={shown} />;
}
