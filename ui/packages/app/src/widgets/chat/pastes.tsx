/**
 * Large pastes in the composer (UI-357..UI-360): a paste of more than 10 lines or 1000 characters becomes a chip and a
 * `[Paste #N, …]` token in the text; the chip expands to its editable text, × removes it; tokens are atomic; sending
 * expands them. Ctrl+Shift+V (⌘+Shift+V on macOS) pastes raw.
 */

import { Button, Chip, TextArea } from "omfp/kit";
import {
  applyEdit,
  type Draft,
  editPaste,
  insertPaste,
  isLargePaste,
  isRawPaste,
  keyLabel,
  lineCount,
  type Modifiers,
  rawPasteKeys,
  removePaste,
  strings,
  useAnnounce,
} from "omfp/sdk";
import { type ClipboardEvent, type KeyboardEvent, type RefObject, useRef, useState } from "react";
import { testId } from "./props.ts";

const text = strings.chat.composer;

type SetDraft = (update: (previous: Draft) => Draft) => void;

/** Puts `value` into the field with the caret at `caret`, before React renders the same value (which then leaves
 * the caret alone). */
function place(input: HTMLTextAreaElement, value: string, caret: number): void {
  input.value = value;
  input.setSelectionRange(caret, caret);
}

/** The composer's paste handling: give `onKeyDown` every key press of the field (it notes the raw paste keys),
 * `onPaste` its paste events, and route its changes through `onChange`. */
export function usePastes(
  draft: Draft,
  setDraft: SetDraft,
  inputRef: RefObject<HTMLTextAreaElement | null>,
): {
  onKeyDown: (event: KeyboardEvent, modifiers: Modifiers) => void;
  onPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
  onChange: (value: string) => void;
} {
  const announce = useAnnounce();
  // The paste keys were the raw ones: the next paste goes in as it is.
  const raw = useRef(false);
  return {
    onKeyDown: (event, modifiers) => {
      raw.current = isRawPaste(event.key, modifiers);
    },
    onPaste: (event) => {
      const asIs = raw.current;
      raw.current = false;
      const pasted = event.clipboardData.getData("text/plain");
      if (asIs || !isLargePaste(pasted)) {
        return;
      }
      event.preventDefault();
      const input = event.currentTarget;
      const edited = insertPaste(draft, pasted, input.selectionStart, input.selectionEnd);
      place(input, edited.draft.text, edited.caret);
      setDraft(() => edited.draft);
      const paste = edited.draft.pastes.at(-1);
      if (paste !== undefined) {
        announce(text.pasted(paste.id, lineCount(paste.text), keyLabel(rawPasteKeys())));
      }
    },
    onChange: (value) => {
      const input = inputRef.current;
      const edited = applyEdit(draft, value, input?.selectionStart ?? value.length);
      if (input !== null && edited.draft.text !== value) {
        place(input, edited.draft.text, edited.caret);
      }
      setDraft(() => edited.draft);
    },
  };
}

/** The draft's pastes as chips: the label expands the editable text, × removes the paste and its token; `locked`
 * (the message on its way) shows the text read-only and offers no ×. */
export function PasteChips({
  draft,
  setDraft,
  inputRef,
  locked = false,
}: {
  draft: Draft;
  setDraft: SetDraft;
  inputRef: RefObject<HTMLTextAreaElement | null>;
  locked?: boolean;
}) {
  const announce = useAnnounce();
  const [open, setOpen] = useState<number | undefined>(undefined);
  return (
    <ul className="omfp-composer__pastes" aria-label={text.pastes}>
      {draft.pastes.map((paste) => {
        const expanded = open === paste.id;
        const textId = `omfp-paste-${paste.id}`;
        return (
          <li key={paste.id} className="omfp-composer__paste" data-testid="paste-chip" data-paste={paste.id}>
            <Chip
              {...(locked
                ? {}
                : {
                    onRemove: () => {
                      setDraft((previous) => removePaste(previous, paste.id));
                      announce(text.pasteRemoved(paste.id));
                      inputRef.current?.focus();
                    },
                  })}
              removeLabel={text.removePaste(paste.id)}
              removeTestId="paste-remove"
            >
              <Button
                variant="quiet"
                size="sm"
                className="omfp-composer__paste-toggle"
                aria-expanded={expanded}
                {...(expanded ? { "aria-controls": textId } : {})}
                data-testid="paste-toggle"
                onPress={() => setOpen(expanded ? undefined : paste.id)}
              >
                {text.paste(paste.id, lineCount(paste.text), paste.text.length)}
              </Button>
            </Chip>
            {expanded && (
              <TextArea
                id={textId}
                className="omfp-composer__paste-text"
                label={text.pasteText(paste.id)}
                hideLabel
                rows={6}
                value={paste.text}
                isReadOnly={locked}
                onChange={(value) => setDraft((previous) => editPaste(previous, paste.id, value))}
                inputProps={testId("paste-text")}
              />
            )}
          </li>
        );
      })}
    </ul>
  );
}
