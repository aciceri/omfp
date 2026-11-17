/**
 * The composer's readline keys (UI-760): pure edits of a text field's value and selection. Each command gives either
 * a new selection (a move), a replacement of one range with the caret after it (an edit, possibly killing the text it
 * removes), or, for `linePrev` / `lineNext` on the first / last line, the boundary the prompt history takes over.
 */

/** The field's text and selection (`start <= end`). */
export interface FieldState {
  readonly value: string;
  readonly start: number;
  readonly end: number;
}

export type LineEdit =
  /** Select `start..end` (a caret when equal). */
  | { readonly kind: "select"; readonly start: number; readonly end: number }
  /** Replace `from..to` with `insert`, the caret at `caret`; `kill` goes into the kill buffer. */
  | {
      readonly kind: "edit";
      readonly from: number;
      readonly to: number;
      readonly insert: string;
      readonly caret: number;
      readonly kill?: string;
    }
  /** `linePrev` on the first line, `lineNext` on the last one: the prompt history's turn (UI-361, UI-362). */
  | { readonly kind: "boundary"; readonly direction: "up" | "down" };

/** The command ids, in the order of the default bindings. */
export const LINE_EDIT_COMMANDS = [
  "composer.lineStart",
  "composer.lineEnd",
  "composer.charBack",
  "composer.charForward",
  "composer.wordBack",
  "composer.wordForward",
  "composer.linePrev",
  "composer.lineNext",
  "composer.deleteForward",
  "composer.deleteBack",
  "composer.killLineEnd",
  "composer.killLineStart",
  "composer.killWordBack",
  "composer.killWordForward",
  "composer.yank",
  "composer.transpose",
  "composer.selectAll",
] as const;

export type LineEditCommand = (typeof LINE_EDIT_COMMANDS)[number];

const WORD = /[\p{L}\p{N}_]/u;

function isWord(value: string, index: number): boolean {
  return WORD.test(value.charAt(index));
}

function lineStart(value: string, at: number): number {
  // `lastIndexOf` searches from 0 for a negative position, so offset 0 is answered here.
  return at <= 0 ? 0 : value.lastIndexOf("\n", at - 1) + 1;
}

function lineEnd(value: string, at: number): number {
  const next = value.indexOf("\n", at);
  return next === -1 ? value.length : next;
}

/** One character (a whole surrogate pair) before `at`. */
function charBefore(value: string, at: number): number {
  if (at <= 0) return 0;
  const low = value.charCodeAt(at - 1);
  return at >= 2 && low >= 0xdc00 && low <= 0xdfff ? at - 2 : at - 1;
}

/** One character (a whole surrogate pair) after `at`. */
function charAfter(value: string, at: number): number {
  if (at >= value.length) return value.length;
  const high = value.charCodeAt(at);
  return high >= 0xd800 && high <= 0xdbff && at + 1 < value.length ? at + 2 : at + 1;
}

/** The start of the word before `at` (Emacs's `backward-word`): skips what is not a word, then the word. */
function wordBefore(value: string, at: number): number {
  let index = at;
  while (index > 0 && !isWord(value, index - 1)) index -= 1;
  while (index > 0 && isWord(value, index - 1)) index -= 1;
  return index;
}

/** The end of the word after `at` (Emacs's `forward-word`). */
function wordAfter(value: string, at: number): number {
  let index = at;
  while (index < value.length && !isWord(value, index)) index += 1;
  while (index < value.length && isWord(value, index)) index += 1;
  return index;
}

const caret = (at: number): LineEdit => ({ kind: "select", start: at, end: at });

/** Removes `from..to`, into the kill buffer when `killing`; a selection is removed instead when there is one. */
function remove(state: FieldState, from: number, to: number, killing = false): LineEdit {
  const [start, end] = state.start === state.end ? [Math.min(from, to), Math.max(from, to)] : [state.start, state.end];
  if (start === end) return caret(start);
  const edit: LineEdit = { kind: "edit", from: start, to: end, insert: "", caret: start };
  return killing ? { ...edit, kill: state.value.slice(start, end) } : edit;
}

/** The same column (clamped) on the line starting at `start`, never between the halves of a surrogate pair. */
function column(value: string, start: number, wanted: number): number {
  const at = Math.min(start + wanted, lineEnd(value, start));
  const code = value.charCodeAt(at);
  return at > start && code >= 0xdc00 && code <= 0xdfff ? at - 1 : at;
}

/** What `command` does to `state`; `killed` is the kill buffer, which `yank` inserts. */
export function lineEdit(command: LineEditCommand, state: FieldState, killed: string): LineEdit {
  const { value, start, end } = state;
  const selected = start !== end;
  switch (command) {
    case "composer.lineStart":
      return caret(lineStart(value, start));
    case "composer.lineEnd":
      return caret(lineEnd(value, end));
    case "composer.charBack":
      return caret(selected ? start : charBefore(value, start));
    case "composer.charForward":
      return caret(selected ? end : charAfter(value, end));
    case "composer.wordBack":
      return caret(wordBefore(value, start));
    case "composer.wordForward":
      return caret(wordAfter(value, end));
    case "composer.linePrev": {
      const here = lineStart(value, start);
      if (here === 0) return { kind: "boundary", direction: "up" };
      return caret(column(value, lineStart(value, here - 1), start - here));
    }
    case "composer.lineNext": {
      const next = value.indexOf("\n", end);
      if (next === -1) return { kind: "boundary", direction: "down" };
      return caret(column(value, next + 1, end - lineStart(value, end)));
    }
    case "composer.deleteForward":
      return remove(state, start, charAfter(value, start));
    case "composer.deleteBack":
      return remove(state, charBefore(value, start), start);
    case "composer.killLineEnd": {
      const stop = lineEnd(value, start);
      // At the end of a line the line break goes, as in Emacs.
      return remove(state, start, stop === start ? charAfter(value, start) : stop, true);
    }
    case "composer.killLineStart":
      return remove(state, lineStart(value, start), start, true);
    case "composer.killWordBack":
      return remove(state, wordBefore(value, start), start, true);
    case "composer.killWordForward":
      return remove(state, start, wordAfter(value, start), true);
    case "composer.yank":
      if (killed === "" && !selected) return caret(start);
      return { kind: "edit", from: start, to: end, insert: killed, caret: start + killed.length };
    case "composer.transpose": {
      if (selected || start === 0 || value.length < 2) return { kind: "select", start, end };
      // At the end of a line the two characters before the caret swap; elsewhere those around it, the caret after.
      const atEnd = start === value.length || value.charAt(start) === "\n";
      const right = atEnd ? charBefore(value, start) : start;
      const left = charBefore(value, right);
      const stop = atEnd ? start : charAfter(value, start);
      if (left === right || value.charAt(left) === "\n") return { kind: "select", start, end };
      const swapped = value.slice(right, stop) + value.slice(left, right);
      return { kind: "edit", from: left, to: stop, insert: swapped, caret: stop };
    }
    case "composer.selectAll":
      return { kind: "select", start: 0, end: value.length };
  }
}
