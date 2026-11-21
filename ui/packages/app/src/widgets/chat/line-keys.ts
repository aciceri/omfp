/**
 * The composer's readline keys (UI-760): `composer.*` commands of the `composer` scope, each enabled while its own
 * composer's field has the focus, so the thread's and the draft's composers each edit their own text.
 */

import {
  expandPastes,
  LINE_EDIT_COMMANDS,
  type LineEdit,
  type LineEditCommand,
  lineEdit,
  type Paste,
  strings,
  useCommand,
} from "omfp/sdk";
import { type RefObject, useEffect, useRef } from "react";

const titles = strings.chat.composer.lineKeys;

/** The kill buffer: one per window (a module is), never the system clipboard, as readline's. */
let killBuffer = "";

/**
 * Replaces `from..to` with `insert` as typing would: through `insertText` / `delete`, which keep the field's undo stack
 * and fire the `input` event the composer's draft path listens to (pastes, Autocomplete); where the page has no
 * `execCommand`, the value is set and `onChange` told. The word completion's accept inserts through it too (UI-176).
 */
export function replace(
  input: HTMLTextAreaElement,
  edit: Extract<LineEdit, { kind: "edit" }>,
  onChange: (text: string) => void,
) {
  input.setSelectionRange(edit.from, edit.to);
  const native =
    typeof document.execCommand === "function" &&
    (edit.insert === "" ? document.execCommand("delete") : document.execCommand("insertText", false, edit.insert));
  if (native) return;
  // The caret first: the draft path reads it to tell which paste token an edit touched (UI-358).
  const text = `${input.value.slice(0, edit.from)}${edit.insert}${input.value.slice(edit.to)}`;
  input.value = text;
  input.setSelectionRange(edit.caret, edit.caret);
  onChange(text);
}

/** A mounted composer: its field and how its edits reach its draft. */
interface LineKeysTarget {
  readonly input: () => HTMLTextAreaElement | null;
  readonly onChange: (text: string) => void;
  readonly pastes: () => readonly Paste[];
  readonly history: ((direction: "up" | "down") => boolean) | undefined;
}

/** Every mounted composer: the registry keeps one registration per command id and scope, so whichever composer
 * registered last dispatches to the one whose field has the focus. */
const targets = new Set<LineKeysTarget>();

function focusedTarget(): LineKeysTarget | undefined {
  for (const target of targets) {
    const input = target.input();
    if (input !== null && input === document.activeElement) return target;
  }
  return undefined;
}

function run(id: LineEditCommand): void {
  const target = focusedTarget();
  const input = target?.input() ?? null;
  if (target === undefined || input === null || input.readOnly) return;
  const edit = lineEdit(id, { value: input.value, start: input.selectionStart, end: input.selectionEnd }, killBuffer);
  if (edit.kind === "select") {
    input.setSelectionRange(edit.start, edit.end);
  } else if (edit.kind === "boundary") {
    if (target.history?.(edit.direction) !== true) {
      const at = edit.direction === "up" ? 0 : input.value.length;
      input.setSelectionRange(at, at);
    }
  } else {
    // A killed paste token goes with its text, so a yank brings the text back, not an orphaned token (UI-358).
    if (edit.kill !== undefined) killBuffer = expandPastes(edit.kill, target.pastes());
    replace(input, edit, target.onChange);
  }
}

/**
 * Registers the readline commands for the composer whose field is `inputRef`. `history` walks the prompt history at
 * the first / last line (UI-361, UI-362) and answers whether it did; without it (the draft composer) the caret goes to
 * the start / end of the text, as the arrows do.
 */
export function useLineKeys(
  inputRef: RefObject<HTMLTextAreaElement | null>,
  onChange: (text: string) => void,
  pastes: readonly Paste[],
  history?: (direction: "up" | "down") => boolean,
): void {
  const latest = useRef({ onChange, pastes, history });
  latest.current = { onChange, pastes, history };
  useEffect(() => {
    const target: LineKeysTarget = {
      input: () => inputRef.current,
      onChange: (text) => latest.current.onChange(text),
      pastes: () => latest.current.pastes,
      history: (direction) => latest.current.history?.(direction) ?? false,
    };
    targets.add(target);
    return () => {
      targets.delete(target);
    };
  }, [inputRef]);
  for (const id of LINE_EDIT_COMMANDS) {
    // biome-ignore lint/correctness/useHookAtTopLevel: a constant list, the same hooks in the same order every render
    useCommand({
      id,
      title: titles[id],
      scope: "composer",
      enabled: () => focusedTarget() !== undefined,
      run: () => run(id),
    });
  }
}
