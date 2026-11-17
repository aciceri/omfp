/**
 * `omfp:composer`: queue chips (UI-062) and the composer (UI-060..064) of the selected thread. Multi-line input with
 * image paste/drop, large pastes as chips (UI-357..UI-360), `/` command and `@` file completion (React Aria
 * `Autocomplete` + `TextArea` + `Menu` in a non-modal `Popover`, S-11, UI-732), the send mode on the send button, Esc
 * leaving the composer (never aborting), `Ctrl-C` clearing the draft (restored by `u`) then aborting without restoring
 * the queue (UI-063), `thread.abort` restoring it (the Abort button, `Ctrl-C` in normal mode, UI-368), and drafts per
 * thread. With no thread selected, the composer of the draft thread (UI-023): its first message creates the thread.
 */

import type { AvailableSlashCommand, ImageContent } from "@omfp/core";
import {
  Autocomplete,
  Button,
  ComposerBar,
  ComposerHint,
  Icon,
  IconButton,
  Kbd,
  Composer as KitComposer,
  StateMark,
  TextArea,
  type TextAreaProps,
  useWidthClass,
} from "omfp/kit";
import {
  activityOf,
  alternative,
  behavior,
  canAbort,
  commandsOf,
  complete,
  completeFile,
  DRAFT_MESSAGE,
  DRAFT_THREAD,
  type Draft,
  type DraftThread,
  draftKey,
  EMPTY_DRAFT,
  errorMessage,
  expandPastes,
  type FileMention,
  FOCUS_COMPOSER,
  formatDuration,
  INTERRUPT_WINDOW_MS,
  imageAttachment,
  interruptAction,
  isBlank,
  isBusy,
  keyAction,
  keyLabel,
  MODEL_CHANGING,
  type Modifiers,
  type OmfpHost,
  queueKeys,
  queueModifier,
  rawPasteKeys,
  restoreCleared,
  restoreDraft,
  restoredTexts,
  restoreFailedPrompt,
  SCOPE_ATTRIBUTE,
  type SendMode,
  type SlashRoute,
  sendMode,
  slashAction,
  slashSuggestions,
  strings,
  useAnnounce,
  useCall,
  useCommand,
  useHost,
  useReportError,
  useThread,
  useThreadActions,
  useThreadSummary,
  useWindowState,
} from "omfp/sdk";
import {
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
  memo,
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { Attachments } from "./attachments.tsx";
import {
  type Completion,
  CompletionMenu,
  type Cursor,
  completionKey,
  completionsOf,
  cursorOf,
  mentionAt,
  useFileSuggestions,
} from "./completion.tsx";
import { usePromptHistory } from "./history.ts";
import { useLineKeys } from "./line-keys.ts";
import { LiveVoicePanel, MicButton } from "./live.tsx";
import { PasteChips, usePastes } from "./pastes.tsx";
import { testId, type WidgetProps } from "./props.ts";
import { QueueChips } from "./queue.tsx";
import { clearedDraftKey, useClock, useObservedElapsed, useSelectedThread } from "./state.ts";
import { useCoarsePointer } from "./touch.ts";

const text = strings.chat.composer;

export function ComposerWidget(_props: WidgetProps) {
  const { threadId, view } = useSelectedThread();
  const [draft] = useWindowState(DRAFT_THREAD);
  if (threadId === undefined && draft !== undefined) {
    return (
      <div className="omfp-composer-area">
        <DraftComposer draft={draft} />
      </div>
    );
  }
  if (threadId === undefined || view === undefined) {
    return null;
  }
  const { state } = view;
  const activity = activityOf(state);
  const activityLabel =
    activity?.kind === "thinking"
      ? (activity.preview ?? text.activity.thinking)
      : activity?.kind === "tool"
        ? (activity.intent ?? text.activity.tool(activity.name))
        : activity?.kind === "working"
          ? text.activity.working
          : undefined;
  return (
    <div className="omfp-composer-area">
      <LiveVoicePanel threadId={threadId} state={state} />
      <QueueChips threadId={threadId} state={state} />
      <Composer
        key={threadId}
        threadId={threadId}
        busy={isBusy(state)}
        abortable={canAbort(state)}
        commands={state.commands}
        {...(activity === undefined || activityLabel === undefined
          ? {}
          : { activityKey: activity.key, activityKind: activity.kind, activityLabel })}
      />
    </div>
  );
}

/**
 * The draft thread's composer (UI-023): Enter (or the button) creates the thread with the text and the pasted or
 * dropped images (UI-060) as first message; they stay when that fails. It takes the focus whenever the draft is
 * opened; Esc leaves it, discarding an empty draft. `@` completes the draft workspace's files (UI-732); with no
 * thread yet there are no `/` commands.
 */
function DraftComposer({ draft }: { draft: DraftThread }) {
  const actions = useThreadActions();
  const [message, setMessage] = useWindowState(DRAFT_MESSAGE);
  const [sending, setSending] = useState(false);
  /** The input has the focus: the keymap is in insert mode (UI-098), drawn as the gradient border (UI-172). */
  const [insert, setInsert] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const host = useHost();
  const reads = draftReads(host);
  const touch = useCoarsePointer();
  const labels = strings.shell.draft;
  // biome-ignore lint/correctness/useExhaustiveDependencies: `draft` is a new object each time the draft is opened
  useEffect(() => {
    inputRef.current?.focus();
  }, [draft]);
  const send = () => {
    if (sending || (isBlank(message) && reads.size === 0)) {
      return;
    }
    setSending(true);
    // Images still being read join the message before it is sent; a draft discarded meanwhile is not sent, nor is
    // the one opened after it (the user did not submit that).
    const watch = watchDraft(host);
    void Promise.all(reads)
      .then(() => (watch.gone() ? false : actions.sendDraft()))
      .then((sent) => {
        watch.stop();
        if (!sent) {
          setSending(false);
        }
      });
  };
  const leave = () => {
    if (isBlank(message)) {
      actions.discardDraft();
    }
    inputRef.current?.blur();
  };
  useFocusCommands(inputRef, leave);
  const pastes = usePastes(message, setMessage, inputRef);
  /** Esc closed the suggestions; typing opens them again. */
  const [dismissed, setDismissed] = useState(false);
  const [cursor, setCursor] = useState<Cursor>();
  const mention = mentionAt(message.text, cursor);
  const completions = completionsOf([], useFileSuggestions(draft.workspaceId, undefined, mention?.query));
  const open = completions.length > 0 && !dismissed;
  const typed = (value: string) => {
    pastes.onChange(value);
    if (inputRef.current !== null) {
      setCursor(cursorOf(inputRef.current));
    }
    setDismissed(false);
  };
  useLineKeys(inputRef, typed, message.pastes);
  const pick = (completion: Completion) => {
    if (completion.kind === "file" && mention !== undefined) {
      setCursor(insertFile(inputRef.current, message.text, mention, completion.path, pastes.onChange));
    }
    setDismissed(false);
  };
  const images = imageInput((files) => {
    // While the thread is created the message is read-only: images pasted or dropped then are ignored.
    if (sending) {
      return;
    }
    // A read belongs to the draft it started in: once that draft is gone (sent, discarded, its workspace removed),
    // its images join no other draft, even one opened before the read ends (a sent draft waited for them).
    const watch = watchDraft(host);
    const read = attachImages(files, (update) => {
      if (!watch.gone()) {
        setMessage(update);
      }
    });
    reads.add(read);
    void read.then(() => {
      watch.stop();
      reads.delete(read);
    });
  }, pastes.onPaste);
  const onKeyDownCapture = (event: KeyboardEvent<HTMLElement>) => {
    if (event.target !== inputRef.current || event.nativeEvent.isComposing) {
      return;
    }
    pastes.onKeyDown(event, modifiersOf(event));
    if (open && completionKey(event, inputRef.current, completions, pick, () => setDismissed(true))) {
      return;
    }
    composerKey(event, inputRef.current, { busy: false, touch, onChange: pastes.onChange, send, leave });
  };
  return (
    <form
      className="omfp-composer"
      data-testid="composer"
      data-draft="true"
      data-touch={touch || undefined}
      data-workspace-id={draft.workspaceId}
      {...{ [SCOPE_ATTRIBUTE]: "composer" }}
      onSelect={(event) => {
        if (event.target === inputRef.current) {
          setCursor(cursorOf(inputRef.current));
        }
      }}
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
      onDragOver={(event) => event.preventDefault()}
      onDrop={images.onDrop}
      onKeyDownCapture={onKeyDownCapture}
    >
      <KitComposer mode={insert ? "insert" : "normal"}>
        {message.images.length > 0 && <Attachments draft={message} setDraft={setMessage} />}
        {message.pastes.length > 0 && <PasteChips draft={message} setDraft={setMessage} inputRef={inputRef} />}
        <Autocomplete
          inputValue={message.text}
          onInputChange={(value) => {
            pastes.onChange(value);
            if (inputRef.current !== null) {
              setCursor(cursorOf(inputRef.current));
            }
            setDismissed(false);
          }}
        >
          <ComposerTextArea
            touch={touch}
            keys="Enter Alt+Enter Escape Tab"
            placeholder={labels.placeholder}
            onFocus={() => setInsert(true)}
            onBlur={() => setInsert(false)}
            isReadOnly={sending}
            inputRef={inputRef}
            onPaste={images.onPaste}
          />
          <CompletionMenu
            inputRef={inputRef}
            completions={completions}
            open={open}
            onOpenChange={(isOpen) => setDismissed(!isOpen)}
            pick={pick}
          />
        </Autocomplete>
        <ComposerBar hints={touch ? undefined : <KeyHints abortable={false} interruptHint={false} />}>
          <Button
            type="submit"
            variant="primary"
            size="sm"
            icon={<Icon name="arrow-up" />}
            kbd={touch ? undefined : keyLabel("Enter")}
            data-testid="send"
            data-mode="draft"
            isDisabled={sending}
          >
            {labels.send}
          </Button>
        </ComposerBar>
      </KitComposer>
    </form>
  );
}

function sendKey(mode: SendMode): string {
  return keyLabel(mode === "follow-up" ? queueKeys() : "Enter");
}

/**
 * The composer's text area takes prose (UI-422): autocorrect, capitals and the spell check on, which React Aria's
 * `Autocomplete` would turn off as for a search field; the on-screen keyboard's Enter key reads "enter" on a touch
 * screen, where it makes a new line (UI-421), else "send".
 */
function proseInput(touch: boolean) {
  return {
    autoCorrect: "on",
    autoCapitalize: "sentences",
    spellCheck: true,
    enterKeyHint: touch ? "enter" : "send",
  } as const;
}

/** The key hints of the composer's bar (UI-148, UI-172); a touch screen has no such keys and shows none (UI-423). The
 * `ctrl+c` hint comes first: when the row is short, the hints at its end give way (UI-480). */
function KeyHints({
  abortable,
  interruptHint,
  canRestore = false,
}: {
  abortable: boolean;
  interruptHint: boolean;
  canRestore?: boolean;
}) {
  return (
    <>
      {abortable &&
        (interruptHint ? (
          <ComposerHint className="omfp-composer__interrupt" role="status" data-testid="interrupt-hint">
            <Kbd>{keyLabel("Ctrl-c")}</Kbd> {text.interruptHint}
          </ComposerHint>
        ) : (
          <ComposerHint>
            <Kbd>{keyLabel("Ctrl-c")}</Kbd> {text.hint.interrupt}
          </ComposerHint>
        ))}
      {canRestore && (
        <ComposerHint data-testid="restore-draft-hint">
          <Kbd>{keyLabel("Esc")}</Kbd> <Kbd>u</Kbd> {text.hint.restore}
        </ComposerHint>
      )}
      <ComposerHint>
        <Kbd>{keyLabel("Alt-Enter")}</Kbd> {text.hint.newline}
      </ComposerHint>
      <ComposerHint>
        <Kbd>{keyLabel("Esc")}</Kbd> {text.hint.leave}
      </ComposerHint>
    </>
  );
}

/**
 * Alt+Enter (UI-060): a line break replaces the selection, the caret after it. The field's value and caret are set
 * first so React, finding the same value, leaves the caret where it is.
 */
function insertLineBreak(input: HTMLTextAreaElement, apply: (text: string) => void) {
  const { selectionStart, selectionEnd, value } = input;
  const text = `${value.slice(0, selectionStart)}\n${value.slice(selectionEnd)}`;
  input.value = text;
  input.setSelectionRange(selectionStart + 1, selectionStart + 1);
  apply(text);
}

/**
 * Picks the workspace-relative `path` for `mention` (UI-732): the text area gets the completed text and the caret after
 * the inserted `@path ` first, as {@link insertLineBreak} does. Returns the new text and caret.
 */
function insertFile(
  input: HTMLTextAreaElement | null,
  text: string,
  mention: FileMention,
  path: string,
  apply: (text: string) => void,
): Cursor {
  const next = completeFile(text, mention, path);
  if (input !== null) {
    input.value = next.text;
    input.setSelectionRange(next.caret, next.caret);
  }
  apply(next.text);
  return next;
}

/**
 * The transcript of the composer's own chat: the first rendered one found walking up from the composer, so a kept
 * tab's hidden transcript or another chat earlier in the page never takes the focus (UI-400).
 */
function nearestTranscript(from: HTMLElement | null): HTMLElement | undefined {
  const selector = `[${SCOPE_ATTRIBUTE}="transcript"]`;
  for (let node = from?.parentElement; node; node = node.parentElement) {
    const found = [...node.querySelectorAll<HTMLElement>(selector)].find((element) => element.checkVisibility());
    if (found !== undefined) return found;
  }
  return undefined;
}

function modifiersOf(event: KeyboardEvent): Modifiers {
  return { shift: event.shiftKey, alt: event.altKey, ctrl: event.ctrlKey, meta: event.metaKey };
}

/**
 * The composer's own keys once its suggestions and prompt history had theirs (UI-060, UI-095): Enter sends (`send`
 * gets the mode), Alt+Enter breaks the line, Esc leaves; other keys stay in the text area, except the chords that do
 * not send, which go on to the keymap.
 */
function composerKey(
  event: KeyboardEvent<HTMLElement>,
  input: HTMLTextAreaElement,
  actions: {
    busy: boolean;
    touch: boolean;
    onChange: (text: string) => void;
    send: (mode: SendMode) => void;
    leave: () => void;
  },
): void {
  const modifiers = modifiersOf(event);
  const action = keyAction(event.key, modifiers, {
    busy: actions.busy,
    hasSuggestions: false,
    repeat: event.repeat,
    touch: actions.touch,
  });
  if ((modifiers.ctrl || modifiers.alt || modifiers.meta) && event.key !== "Enter" && action.type !== "send") {
    return;
  }
  event.stopPropagation();
  if (action.type === "none") {
    return;
  }
  event.preventDefault();
  if (action.type === "send") {
    actions.send(action.mode);
  } else if (action.type === "newline") {
    insertLineBreak(input, actions.onChange);
  } else if (action.type === "leave") {
    actions.leave();
  }
}

/** `composer.focus`, and `focus.leave` while the focus is in the composer (UI-095). */
function useFocusCommands(inputRef: RefObject<HTMLTextAreaElement | null>, leave: () => void): void {
  useCommand({ id: "composer.focus", title: text.focus, run: () => inputRef.current?.focus() });
  useCommand({ id: "focus.leave", title: text.leave, scope: "composer", run: leave });
}

interface ComposerTextAreaProps
  extends Omit<TextAreaProps, "className" | "label" | "hideLabel" | "rows" | "inputProps" | "onPaste"> {
  readonly touch: boolean;
  /** The keys it answers, as `aria-keyshortcuts` writes them; the raw paste keys follow. */
  readonly keys: string;
  readonly onPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
}

/** The text area of a composer, the thread's and the draft's (UI-060, UI-422). */
function ComposerTextArea({ touch, keys, onPaste, ...props }: ComposerTextAreaProps) {
  return (
    <TextArea
      {...props}
      className="omfp-compose__field"
      label={text.label}
      hideLabel
      rows={2}
      inputProps={{
        ...testId("composer-input"),
        ...proseInput(touch),
        "aria-keyshortcuts": `${keys} ${ariaKeys(rawPasteKeys())}`,
        onPaste,
      }}
    />
  );
}

/** `Ctrl-Shift-v` as `aria-keyshortcuts` writes it: `Control+Shift+V`. */
function ariaKeys(keys: string): string {
  return keys
    .split("-")
    .map((part) => (part === "Ctrl" ? "Control" : part.length === 1 ? part.toUpperCase() : part))
    .join("+");
}

/** The image attachments among `files` (UI-060). */
async function attachments(files: readonly File[]): Promise<ImageContent[]> {
  const images = await Promise.all(
    files.map(async (file) => imageAttachment(file.type, new Uint8Array(await file.arrayBuffer()))),
  );
  return images.filter((image) => image !== undefined);
}

/** Per window, the draft thread's pasted or dropped images still being read. */
const DRAFT_READS = new WeakMap<OmfpHost["window"], Set<Promise<void>>>();

/**
 * The draft thread's images still being read in `host`'s window: a send waits for them. They belong to the draft,
 * not to its composer, which a thread switch unmounts while the draft stays (UI-064), so a send from the composer
 * mounted again still waits for them.
 */
function draftReads(host: OmfpHost): Set<Promise<void>> {
  let reads = DRAFT_READS.get(host.window);
  if (reads === undefined) {
    reads = new Set();
    DRAFT_READS.set(host.window, reads);
  }
  return reads;
}

/**
 * Watches the draft thread open now (UI-023): `gone()` turns true once it is discarded (sent, Esc, its workspace
 * removed), whatever draft opens after it; `stop()` ends the watch.
 */
function watchDraft(host: OmfpHost): { gone: () => boolean; stop: () => void } {
  let gone = host.window.get(DRAFT_THREAD) === undefined;
  const stop = host.window.subscribe(DRAFT_THREAD, () => {
    if (host.window.get(DRAFT_THREAD) === undefined) {
      gone = true;
    }
  });
  return { gone: () => gone, stop };
}

/**
 * Reads the image files among `files` and appends them to the draft (UI-060); resolves once they are in (never
 * rejects: an unreadable file attaches nothing).
 */
async function attachImages(
  files: readonly File[],
  setDraft: (update: (previous: Draft) => Draft) => void,
): Promise<void> {
  const images = await attachments(files).catch((): ImageContent[] => []);
  if (images.length > 0) {
    setDraft((previous) => ({ ...previous, images: [...previous.images, ...images] }));
  }
}

/**
 * Image paste and drop of a composer (UI-060): pasted or dropped files go to `attach`; any other paste goes to
 * `onTextPaste`. The thread's composer and the draft thread's (UI-023) share it.
 */
function imageInput(
  attach: (files: readonly File[]) => void,
  onTextPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void,
) {
  return {
    onPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => {
      const files = [...event.clipboardData.files].filter((file) => file.type.startsWith("image/"));
      if (files.length > 0) {
        event.preventDefault();
        attach(files);
        return;
      }
      onTextPaste(event);
    },
    onDrop: (event: DragEvent<HTMLFormElement>) => {
      event.preventDefault();
      attach([...event.dataTransfer.files]);
    },
  };
}

/**
 * The thread's `/` commands: those of the last `available_commands_update` (UI-060), or `thread.commands` asked once
 * while the state has none yet.
 */
function useThreadCommands(
  threadId: string,
  known: readonly AvailableSlashCommand[],
): readonly AvailableSlashCommand[] {
  const call = useCall();
  const [fetched, setFetched] = useState<readonly AvailableSlashCommand[]>([]);
  const missing = known.length === 0;
  useEffect(() => {
    if (!missing) {
      return;
    }
    let live = true;
    call("thread.commands", { threadId }).then(
      (result) => {
        if (live) {
          setFetched(commandsOf(result));
        }
      },
      // Without the list, `/` simply offers nothing until the next `available_commands_update`.
      () => {},
    );
    return () => {
      live = false;
    };
  }, [call, missing, threadId]);
  return missing ? fetched : known;
}

/**
 * The composer of one thread; mount it with `key={threadId}`. It takes the few facts of the thread state it needs,
 * so a thread event (a streamed delta) does not render it again, and a key press renders the input but not the
 * action bar (UI-197).
 */
export const Composer = memo(function Composer({
  threadId,
  busy,
  abortable,
  commands: known,
  activityKey,
  activityKind,
  activityLabel,
}: {
  threadId: string;
  busy: boolean;
  abortable: boolean;
  commands: readonly AvailableSlashCommand[];
  activityKey?: string;
  activityKind?: "thinking" | "tool" | "working";
  activityLabel?: string;
}) {
  const host = useHost();
  const call = useCall();
  const announce = useAnnounce();
  const clock = useClock();
  const [draft, setDraft] = useWindowState(draftKey(threadId));
  const [cleared, setCleared] = useWindowState(clearedDraftKey(threadId));
  /** When `Ctrl-C` last cleared the draft: a second press within {@link INTERRUPT_WINDOW_MS}, with no typing in
   * between, aborts (UI-063). */
  const lastInterrupt = useRef<number | undefined>(undefined);
  /** "Ctrl-C again to abort" shows while a second press would abort. */
  const [interruptHint, setInterruptHint] = useState(false);
  const [queueHeld, setQueueHeld] = useState(false);
  /** Esc closed the suggestions; typing opens them again. */
  const [dismissed, setDismissed] = useState(false);
  /** The input has the focus: the keymap is in insert mode (UI-098), drawn as the gradient border (UI-172). */
  const [insert, setInsert] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const box = useRef<HTMLFormElement>(null);
  const tight = useWidthClass(box, (width, rem) => width > 0 && width < TIGHT_BELOW_REM * rem);
  const touch = useCoarsePointer();
  const [focusRequest, setFocusRequest] = useWindowState(FOCUS_COMPOSER);
  // Asked to take the focus once shown, e.g. after `:notice` (UI-286).
  useEffect(() => {
    if (focusRequest === threadId) {
      inputRef.current?.focus();
      setFocusRequest(undefined);
    }
  }, [focusRequest, threadId, setFocusRequest]);
  const mode = sendMode(busy, queueHeld);
  const other = alternative(mode);
  const commands = useThreadCommands(threadId, known);
  const commandSuggestions = slashSuggestions(draft.text, commands);
  /** The text and caret the last edit left: a `@` before the caret asks for files (UI-732). */
  const [cursor, setCursor] = useState<Cursor>();
  const mention = commandSuggestions.length > 0 ? undefined : mentionAt(draft.text, cursor);
  const files = useFileSuggestions(useThreadSummary(threadId)?.workspaceId, threadId, mention?.query);
  const completions = completionsOf(commandSuggestions, files);
  const open = completions.length > 0 && !dismissed;

  const actions = useThreadActions();
  const reportError = useReportError();
  /** Runs the omfp equivalent of omp's `/name` (UI-976). */
  const routeSlash = (name: string, route: SlashRoute) => {
    switch (route.type) {
      case "command":
        if (!host.commands.run(route.id, route.args)) {
          reportError(text.slash.unavailable(name));
        }
        return;
      case "fork":
        void actions.fork(threadId);
        return;
    }
  };

  const send = (chosen: SendMode) => {
    if (isBlank(draft) || host.window.get(MODEL_CHANGING).includes(threadId)) {
      return;
    }
    const message = expandPastes(draft.text, draft.pastes).trim();
    // omp's TUI-only commands are routed to omfp's equivalents or refused, never sent (UI-976, UI-977).
    const slash = slashAction(message, commands);
    if (slash.type === "refuse") {
      reportError(slash.message);
      return;
    }
    history.end();
    setDraft(EMPTY_DRAFT);
    if (slash.type === "route") {
      routeSlash(slash.name, slash.route);
      return;
    }
    const images = draft.images;
    const queued = behavior(chosen);
    const sent = draft;
    call("thread.prompt", {
      threadId,
      text: message,
      ...(images.length === 0 ? {} : { images: [...images] }),
      ...(queued === undefined ? {} : { behavior: queued }),
    }).then(
      () => {
        if (queued !== undefined) {
          announce(strings.chat.announce.queued);
        }
      },
      (error: unknown) => {
        setDraft((previous) => restoreFailedPrompt(previous, sent));
        // A command omp refused says why (UI-979).
        if (slash.command !== undefined) {
          reportError(text.slash.failed(slash.command, errorMessage(error)));
        }
      },
    );
  };
  const abort = useCallback(() => {
    call("thread.abort", { threadId }).then(
      (result) => {
        const { steering, followUp } = restoredTexts(result);
        setDraft((previous) => ({ ...previous, text: restoreDraft(previous.text, steering, followUp) }));
        announce(strings.chat.announce.aborted);
      },
      () => announce(strings.chat.announce.abortFailed),
    );
  }, [call, threadId, setDraft, announce]);
  // The action bar gets one stable function that sends the latest draft.
  const latestSend = useRef(send);
  latestSend.current = send;
  const sendLatest = useCallback((chosen: SendMode) => latestSend.current(chosen), []);
  const leave = () => {
    const input = inputRef.current;
    input?.blur();
    nearestTranscript(input)?.focus();
  };
  const pick = (completion: Completion) => {
    if (completion.kind === "command") {
      setDraft((previous) => ({ ...previous, text: complete(completion.suggestion) }));
    } else if (mention !== undefined) {
      setCursor(insertFile(inputRef.current, draft.text, mention, completion.path, pastes.onChange));
    }
    setDismissed(false);
  };

  /** `Ctrl-C` (UI-063): clears a non-empty draft, kept for `u`; aborts an empty one (or on a second press within
   * {@link INTERRUPT_WINDOW_MS}) without putting the queued texts back. */
  const interrupt = () => {
    const now = clock.now();
    const current = host.window.get(draftKey(threadId));
    if (interruptAction(current, now, lastInterrupt.current) === "clear") {
      lastInterrupt.current = now;
      setCleared(current);
      setDraft(EMPTY_DRAFT);
      setInterruptHint(true);
      announce(strings.chat.announce.draftCleared);
      return;
    }
    lastInterrupt.current = undefined;
    setInterruptHint(false);
    if (abortable) {
      call("thread.abort", { threadId }).then(
        () => announce(strings.chat.announce.aborted),
        () => announce(strings.chat.announce.abortFailed),
      );
    }
  };
  useEffect(() => {
    if (!interruptHint) {
      return undefined;
    }
    return clock.every(INTERRUPT_WINDOW_MS / 4, () => {
      const since = lastInterrupt.current;
      if (since === undefined || clock.now() - since > INTERRUPT_WINDOW_MS) {
        setInterruptHint(false);
      }
    });
  }, [clock, interruptHint]);

  useFocusCommands(inputRef, leave);
  useCommand({ id: "thread.abort", title: text.abortCommand, enabled: () => abortable, run: abort });
  useCommand({ id: "composer.interrupt", title: text.interruptCommand, scope: "composer", run: interrupt });
  useCommand({
    id: "composer.restore",
    title: text.restoreCommand,
    enabled: () => cleared !== undefined,
    run: () => {
      if (cleared !== undefined) {
        setDraft((previous) => restoreCleared(previous, cleared));
        setCleared(undefined);
      }
    },
  });

  const pastes = usePastes(draft, setDraft, inputRef);
  const history = usePromptHistory(threadId, draft, setDraft, inputRef);
  const typed = (value: string) => {
    pastes.onChange(value);
    if (inputRef.current !== null) {
      setCursor(cursorOf(inputRef.current));
    }
    // Typing ends a history walk; the recalled text stays as the draft (UI-362).
    history.end();
    setDismissed(false);
    // Typing ends a `Ctrl-C` sequence: the next press clears what was typed rather than aborting (UI-063).
    lastInterrupt.current = undefined;
    setInterruptHint(false);
  };
  // `Ctrl-p` / `Ctrl-n` walk the prompt history at the first / last line, unless the suggestions are open (UI-760).
  useLineKeys(inputRef, typed, draft.pastes, (direction) => !open && history.step(direction));
  // Captured before React Aria's Autocomplete: while no suggestion list is open the text area keeps its own keys
  // (arrows move the caret), and the composer's bindings apply. Chords other than the send ones (Ctrl-C, Alt-a, …)
  // go on to the keymap (UI-095).
  const onKeyDownCapture = (event: KeyboardEvent<HTMLElement>) => {
    if (event.target !== inputRef.current || event.nativeEvent.isComposing) {
      return;
    }
    const modifiers = modifiersOf(event);
    pastes.onKeyDown(event, modifiers);
    setQueueHeld(queueModifier(modifiers));
    if (open && completionKey(event, inputRef.current, completions, pick, () => setDismissed(true))) {
      return;
    }
    // Up on the first line, Down on the last line: the prompt history (UI-361, UI-362).
    if (history.onKey(event)) {
      event.stopPropagation();
      return;
    }
    composerKey(event, inputRef.current, { busy, touch, onChange: pastes.onChange, send, leave });
  };
  const { onPaste, onDrop } = imageInput((files) => void attachImages(files, setDraft), pastes.onPaste);

  return (
    <form
      ref={box}
      className="omfp-composer"
      data-testid="composer"
      data-busy={busy}
      data-touch={touch || undefined}
      data-tight={tight || undefined}
      {...{ [SCOPE_ATTRIBUTE]: "composer" }}
      onSelect={(event) => {
        if (event.target === inputRef.current) {
          setCursor(cursorOf(inputRef.current));
        }
      }}
      onSubmit={(event) => {
        event.preventDefault();
        send(mode);
      }}
      onDragOver={(event) => event.preventDefault()}
      onDrop={onDrop}
      onKeyDownCapture={onKeyDownCapture}
      onKeyUpCapture={(event) => setQueueHeld(queueModifier(modifiersOf(event)))}
    >
      {activityKey !== undefined && activityKind !== undefined && activityLabel !== undefined && (
        <ActivityRow
          threadId={threadId}
          activityKey={activityKey}
          kind={activityKind}
          label={activityLabel}
          short={touch || tight}
          onAbort={abort}
        />
      )}
      <KitComposer mode={insert ? "insert" : "normal"}>
        {draft.images.length > 0 && <Attachments draft={draft} setDraft={setDraft} />}
        {draft.pastes.length > 0 && <PasteChips draft={draft} setDraft={setDraft} inputRef={inputRef} />}
        <Autocomplete inputValue={draft.text} onInputChange={typed}>
          <ComposerTextArea
            touch={touch}
            keys="Enter Control+Enter Meta+Enter Shift+Enter Control+Q Alt+Enter Escape Tab"
            placeholder={busy ? (touch ? text.placeholderBusyTouch : text.placeholderBusy) : text.placeholderIdle}
            inputRef={inputRef}
            onFocus={() => setInsert(true)}
            onBlur={() => {
              setQueueHeld(false);
              setInsert(false);
            }}
            onPaste={onPaste}
          />
          <CompletionMenu
            inputRef={inputRef}
            completions={completions}
            open={open}
            onOpenChange={(isOpen) => setDismissed(!isOpen)}
            pick={pick}
          />
        </Autocomplete>
        <ComposerActions
          threadId={threadId}
          abortable={abortable}
          showAbort={activityKey === undefined}
          touch={touch}
          tight={tight}
          interruptHint={interruptHint}
          canRestore={cleared !== undefined}
          mode={mode}
          other={other}
          onAbort={abort}
          onSend={sendLatest}
        />
      </KitComposer>
    </form>
  );
});

/** The current turn's compact, non-announcing status immediately over the text box (UI-693). */
function ActivityRow({
  threadId,
  activityKey,
  kind,
  label,
  short,
  onAbort,
}: {
  threadId: string;
  activityKey: string;
  kind: "thinking" | "tool" | "working";
  label: string;
  short: boolean;
  onAbort: () => void;
}) {
  const elapsed = useObservedElapsed(`activity/${threadId}/${activityKey}`, true) ?? 0;
  const abort = short ? (
    <IconButton
      className="omfp-composer__activity-abort"
      variant="danger"
      label={text.abort}
      icon={<Icon name="square" />}
      data-testid="abort"
      onPress={onAbort}
    />
  ) : (
    <Button
      className="omfp-composer__activity-abort"
      variant="danger"
      size="sm"
      icon={<Icon name="square" />}
      data-testid="abort"
      onPress={onAbort}
    >
      {text.abort}
    </Button>
  );
  return (
    <div className="omfp-composer__activity" data-testid="composer-activity" data-kind={kind}>
      <StateMark state="running" label={text.activity.running} />
      <span className="omfp-composer__activity-duration" data-testid="activity-duration">
        {formatDuration(elapsed)}
      </span>
      <span className="omfp-composer__activity-separator" aria-hidden="true">
        ·
      </span>
      <span className="omfp-composer__activity-label" data-testid="activity-label">
        {label}
      </span>
      {abort}
    </div>
  );
}

/** Narrower than this (in `rem`) the composer is tight: the busy actions with their long labels would not fit on one
 * row (UI-481). */
const TIGHT_BELOW_REM = 34;

/**
 * The hints and buttons under the input; memoized with stable callbacks, so typing does not render them (UI-197). The
 * actions in one order at the bar's end, the primary last (UI-480): the mic, Abort, the other send mode, then the
 * send button. On a touch screen (UI-423) and in a tight composer (UI-481) the mic and Abort are icons and the send
 * buttons have short labels; only a touch screen drops the key hints and chips.
 */
const ComposerActions = memo(function ComposerActions({
  threadId,
  abortable,
  showAbort,
  touch,
  tight,
  interruptHint,
  canRestore,
  mode,
  other,
  onAbort,
  onSend,
}: {
  threadId: string;
  abortable: boolean;
  showAbort: boolean;
  touch: boolean;
  tight: boolean;
  interruptHint: boolean;
  canRestore: boolean;
  mode: SendMode;
  other: SendMode | undefined;
  onAbort: () => void;
  onSend: (mode: SendMode) => void;
}) {
  const [changing] = useWindowState(MODEL_CHANGING);
  const modelPending = changing.includes(threadId);
  const short = touch || tight;
  const labels = short ? text.sendShort : text.send;
  return (
    <ComposerBar
      hints={
        touch ? undefined : <KeyHints abortable={abortable} interruptHint={interruptHint} canRestore={canRestore} />
      }
    >
      <ThreadMicButton threadId={threadId} />
      {showAbort &&
        abortable &&
        (short ? (
          <IconButton
            variant="danger"
            label={text.abort}
            icon={<Icon name="square" />}
            data-testid="abort"
            onPress={onAbort}
          />
        ) : (
          <Button variant="danger" size="sm" icon={<Icon name="square" />} data-testid="abort" onPress={onAbort}>
            {text.abort}
          </Button>
        ))}
      {other !== undefined && (
        <Button
          size="sm"
          kbd={touch ? undefined : sendKey(other)}
          data-testid="send-alternative"
          data-mode={other}
          isDisabled={modelPending}
          onPress={() => onSend(other)}
        >
          {labels[other]}
        </Button>
      )}
      <Button
        type="submit"
        variant="primary"
        size="sm"
        icon={<Icon name="arrow-up" />}
        kbd={touch ? undefined : sendKey(mode)}
        data-testid="send"
        data-mode={mode}
        isDisabled={modelPending}
      >
        {labels[mode]}
      </Button>
    </ComposerBar>
  );
});

/** The mic button follows the thread's state itself, so thread events render it alone. */
function ThreadMicButton({ threadId }: { threadId: string }) {
  const view = useThread(threadId);
  return view === undefined ? null : <MicButton threadId={threadId} state={view.state} />;
}
