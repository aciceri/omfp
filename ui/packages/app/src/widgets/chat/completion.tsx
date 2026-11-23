/**
 * The composer's suggestion menu (UI-060, UI-732), shared by the thread's composer and the draft thread's: `/` commands
 * and `@` workspace files in one non-modal `Menu` under React Aria's `Autocomplete` (S-11), Tab completing the
 * highlighted (else the first) one, Enter the highlighted one, Esc closing it until the next keystroke.
 */

import { Menu, MenuItem, Popover } from "omfp/kit";
import {
  completeFile,
  type FileMention,
  fileMention,
  fileSuggestions,
  type Suggestion,
  strings,
  useFindFile,
} from "omfp/sdk";
import { type KeyboardEvent, type RefObject, type SyntheticEvent, useCallback, useState } from "react";
import { useClock } from "./state.ts";

const text = strings.chat.composer;

/** How long typing must pause before a `@` query goes to `workspaces.find_file` (UI-732). */
export const FILE_QUERY_DELAY_MS = 120;

/** One entry of the menu: a `/` command or a workspace-relative file path. */
type Completion =
  | { readonly kind: "command"; readonly id: string; readonly suggestion: Suggestion }
  | { readonly kind: "file"; readonly id: string; readonly path: string };

/** The draft's text and the caret within it, as last measured on the text area. */
interface Cursor {
  readonly text: string;
  readonly caret: number;
}

/**
 * The workspace-relative paths `workspaces.find_file` finds for `query` (UI-732), asked once typing has paused
 * {@link FILE_QUERY_DELAY_MS}: until the answer comes, the last one narrowed to the paths still matching shows.
 */
function useFileSuggestions(
  workspaceId: string | undefined,
  threadId: string | undefined,
  query: string | undefined,
): string[] {
  const clock = useClock();
  const delay = useCallback(
    (ask: () => void) => {
      const stop = clock.every(FILE_QUERY_DELAY_MS, () => {
        stop();
        ask();
      });
      return stop;
    },
    [clock],
  );
  const found = useFindFile(workspaceId, threadId, query, delay);
  if (query === undefined || found === undefined || found.workspaceId !== workspaceId) {
    return [];
  }
  return fileSuggestions(query, { query: found.query, paths: found.result?.candidates.map((file) => file.path) ?? [] });
}

/**
 * The suggestion menu of a composer whose text is `value` (UI-060, UI-732): the `/` commands `commands` when there
 * are any, else the files of workspace `workspaceId` the `@` mention at the caret asks for. A picked command goes to
 * `onCommand`, a picked file replaces its mention through `onChange`; Esc closes the menu until the next keystroke.
 */
export function useCompletions({
  inputRef,
  value,
  commands,
  workspaceId,
  threadId,
  onChange,
  onCommand,
}: {
  inputRef: RefObject<HTMLTextAreaElement | null>;
  value: string;
  commands: readonly Suggestion[];
  workspaceId: string | undefined;
  threadId: string | undefined;
  onChange: (text: string) => void;
  onCommand?: (suggestion: Suggestion) => void;
}) {
  /** Esc closed the suggestions; typing opens them again. */
  const [dismissed, setDismissed] = useState(false);
  /** The text and caret the last edit left: a `@` before the caret asks for files, until the text changes otherwise
   * (UI-732). */
  const [cursor, setCursor] = useState<Cursor>();
  const mention = commands.length > 0 || cursor?.text !== value ? undefined : fileMention(value, cursor.caret);
  const files = useFileSuggestions(workspaceId, threadId, mention?.query);
  const completions: Completion[] =
    commands.length > 0
      ? commands.map((suggestion) => ({ kind: "command", id: `/${suggestion.name}`, suggestion }))
      : files.map((path) => ({ kind: "file", id: `@${path}`, path }));
  const open = completions.length > 0 && !dismissed;
  /** Measures the text and caret again, keeping the same object while they are unchanged. */
  const measure = () => {
    const input = inputRef.current;
    if (input !== null) {
      const next = { text: input.value, caret: input.selectionEnd };
      setCursor((previous) => (previous?.text === next.text && previous.caret === next.caret ? previous : next));
    }
  };
  const pick = (completion: Completion) => {
    if (completion.kind === "command") {
      onCommand?.(completion.suggestion);
    } else if (mention !== undefined) {
      setCursor(insertFile(inputRef.current, value, mention, completion.path, onChange));
    }
    setDismissed(false);
  };
  return {
    open,
    /** After an edit: the caret measured again, the menu open again. */
    typed: () => {
      measure();
      setDismissed(false);
    },
    /** The form's `onSelect`: a selection change in the text area moves the caret. */
    onSelect: (event: SyntheticEvent) => {
      if (event.target === inputRef.current) {
        measure();
      }
    },
    /** Whether the open menu took the key ({@link completionKey}). */
    onKeyDown: (event: KeyboardEvent<HTMLElement>, input: HTMLTextAreaElement) =>
      open && completionKey(event, input, completions, pick, () => setDismissed(true)),
    /** The menu, inside the composer's `Autocomplete`. */
    menu: (
      <CompletionMenu
        inputRef={inputRef}
        completions={completions}
        open={open}
        onOpenChange={(isOpen) => setDismissed(!isOpen)}
        pick={pick}
      />
    ),
  };
}

/**
 * Picks the workspace-relative `path` for `mention` (UI-732): the text area gets the completed text and the caret after
 * the inserted `@path ` first, so React, finding the same value, leaves the caret there. Returns the new text and caret.
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
 * The keys of an open menu, captured before React Aria's `Autocomplete` sees them: Tab completes the highlighted entry
 * (else the first enabled one), Esc closes the menu. Returns whether the menu owns the key: not a chord (Ctrl, Alt or
 * ⌘ held: the queue keys, Alt+Enter, Ctrl-C keep their composer and keymap roles) nor an Enter that sends (Shift held,
 * or no entry highlighted); arrows and a plain Enter on a highlighted entry go on to the `Autocomplete`.
 */
function completionKey(
  event: KeyboardEvent<HTMLElement>,
  input: HTMLTextAreaElement,
  completions: readonly Completion[],
  pick: (completion: Completion) => void,
  dismiss: () => void,
): boolean {
  const active = input.getAttribute("aria-activedescendant");
  if (
    event.ctrlKey ||
    event.altKey ||
    event.metaKey ||
    (event.key === "Enter" && (event.shiftKey || active === null))
  ) {
    return false;
  }
  if (event.key === "Tab" && !event.shiftKey) {
    event.preventDefault();
    event.stopPropagation();
    const id = document.getElementById(active ?? "")?.getAttribute("data-key");
    // A disabled command is never picked (UI-978).
    const pickable = completions.filter((completion) => completion.kind === "file" || !completion.suggestion.disabled);
    const chosen = pickable.find((completion) => completion.id === id) ?? pickable[0];
    if (chosen !== undefined) {
      pick(chosen);
    }
  } else if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    dismiss();
  }
  return true;
}

/** The suggestion menu over the composer's text area; mount it inside the composer's `Autocomplete`. */
function CompletionMenu({
  inputRef,
  completions,
  open,
  onOpenChange,
  pick,
}: {
  inputRef: RefObject<HTMLTextAreaElement | null>;
  completions: readonly Completion[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pick: (completion: Completion) => void;
}) {
  const files = completions[0]?.kind === "file";
  return (
    <Popover
      className="omfp-composer__menu"
      triggerRef={inputRef}
      isOpen={open}
      isNonModal
      placement="top start"
      onOpenChange={onOpenChange}
    >
      <Menu
        aria-label={files ? text.files : text.commands}
        data-testid={files ? "file-suggestions" : "slash-suggestions"}
        items={completions}
        onAction={(key) => {
          const chosen = completions.find((completion) => completion.id === key);
          if (chosen !== undefined) {
            pick(chosen);
          }
        }}
      >
        {(completion) =>
          completion.kind === "command" ? (
            <MenuItem
              id={completion.id}
              textValue={completion.id}
              isDisabled={completion.suggestion.disabled}
              data-testid="slash-suggestion"
            >
              <span className="omfp-composer__command">{completion.id}</span>
              {completion.suggestion.hint !== undefined && (
                <span className="omfp-composer__hint"> {completion.suggestion.hint}</span>
              )}
              {completion.suggestion.description !== undefined && (
                <span className="omfp-composer__description"> {completion.suggestion.description}</span>
              )}
            </MenuItem>
          ) : (
            <MenuItem id={completion.id} textValue={completion.id} data-testid="file-suggestion">
              <span className="omfp-composer__command">{completion.id}</span>
            </MenuItem>
          )
        }
      </Menu>
    </Popover>
  );
}
