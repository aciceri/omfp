/**
 * The composer's suggestion menu (UI-060, UI-732), shared by the thread's composer and the draft thread's: `/` commands
 * and `@` workspace files in one non-modal `Menu` under React Aria's `Autocomplete` (S-11), Tab completing the
 * highlighted (else the first) one, Enter the highlighted one, Esc closing it until the next keystroke.
 */

import { Menu, MenuItem, Popover } from "omfp/kit";
import { type FileMention, fileMention, fileSuggestions, type Suggestion, strings, useHost } from "omfp/sdk";
import { type KeyboardEvent, type RefObject, useEffect, useState } from "react";
import { useClock } from "./state.ts";

const text = strings.chat.composer;

/** How long typing must pause before a `@` query goes to `workspaces.find_file` (UI-732). */
export const FILE_QUERY_DELAY_MS = 120;

/** One entry of the menu: a `/` command or a workspace-relative file path. */
export type Completion =
  | { readonly kind: "command"; readonly id: string; readonly suggestion: Suggestion }
  | { readonly kind: "file"; readonly id: string; readonly path: string };

/** The draft's text and the caret within it, as last measured on the text area. */
export interface Cursor {
  readonly text: string;
  readonly caret: number;
}

/** A state update to the text area's text and caret, for {@link mentionAt}; the same object while they are unchanged. */
export function cursorOf(input: HTMLTextAreaElement): (previous: Cursor | undefined) => Cursor {
  const next = { text: input.value, caret: input.selectionEnd };
  return (previous) => (previous?.text === next.text && previous.caret === next.caret ? previous : next);
}

/** The `@` mention at the caret, measured on the current `text` only: after a programmatic change it is gone. */
export function mentionAt(text: string, cursor: Cursor | undefined): FileMention | undefined {
  return cursor?.text === text ? fileMention(text, cursor.caret) : undefined;
}

/**
 * The workspace-relative paths `workspaces.find_file { fuzzy: true }` (DMN-092, UI-731) finds for `query` (UI-732),
 * asked once typing has paused {@link FILE_QUERY_DELAY_MS}. An answer to an older query is dropped; until the new one
 * comes, the last answer narrowed to the paths still matching shows.
 */
export function useFileSuggestions(
  workspaceId: string | undefined,
  threadId: string | undefined,
  query: string | undefined,
): string[] {
  const host = useHost();
  const clock = useClock();
  const [answer, setAnswer] = useState<{ workspaceId: string; query: string; paths: readonly string[] }>();
  useEffect(() => {
    if (workspaceId === undefined || query === undefined) {
      return undefined;
    }
    let live = true;
    const stop = clock.every(FILE_QUERY_DELAY_MS, () => {
      stop();
      host.client
        .call("workspaces.find_file", {
          workspaceId,
          query,
          fuzzy: true,
          ...(threadId === undefined ? {} : { threadId }),
        })
        .then(
          (result) => {
            if (live) {
              setAnswer({ workspaceId, query, paths: result.candidates.map((candidate) => candidate.path) });
            }
          },
          () => {
            if (live) {
              setAnswer({ workspaceId, query, paths: [] });
            }
          },
        );
    });
    return () => {
      live = false;
      stop();
    };
  }, [host, clock, workspaceId, threadId, query]);
  if (query === undefined || answer === undefined || answer.workspaceId !== workspaceId) {
    return [];
  }
  return fileSuggestions(query, answer);
}

/** The menu entries: the `/` commands when the draft is one, else the `@` files. */
export function completionsOf(commands: readonly Suggestion[], files: readonly string[]): Completion[] {
  return commands.length > 0
    ? commands.map((suggestion) => ({ kind: "command", id: `/${suggestion.name}`, suggestion }))
    : files.map((path) => ({ kind: "file", id: `@${path}`, path }));
}

/**
 * The keys of an open menu, captured before React Aria's `Autocomplete` sees them: Tab completes the highlighted entry
 * (else the first enabled one), Esc closes the menu. Returns whether the menu owns the key: not a chord (Ctrl, Alt or
 * ⌘ held: the queue keys, Alt+Enter, Ctrl-C keep their composer and keymap roles) nor an Enter that sends (Shift held,
 * or no entry highlighted); arrows and a plain Enter on a highlighted entry go on to the `Autocomplete`.
 */
export function completionKey(
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
export function CompletionMenu({
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
