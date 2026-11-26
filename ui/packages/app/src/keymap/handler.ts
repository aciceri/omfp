/**
 * The one global key handler (UI-095): turns key presses into command runs through {@link KeymapMatcher}, in the mode
 * the focused element puts the window in ({@link modeAt}). It listens on the document in the bubbling phase, so widgets
 * that handle a key themselves (the transcript's PageUp/PageDown/Home/End, React Aria's menus and lists, the vim
 * editor's Escape) stop it first; inside a modal dialog no binding applies; elements with their own Space or Enter
 * (buttons, links, options) keep it; in Threads and View mode `Ctrl-c` with text selected is left to the page, which
 * copies it (UI-368). While a sequence waits for its next key it tells the which-key panel (UI-320): after
 * {@link WHICH_KEY_DELAY_MS}, at once for the leader, and Escape cancels the sequence.
 */

import { type CommandRegistry, scopesAt } from "../sdk/index.ts";
import { type Binding, composesText, KeymapMatcher, keyToken, sharesNormal, type WindowMode } from "./keymap.ts";
import { type KeyTimers, opensAtOnce, type PendingKeys, WHICH_KEY_DELAY_MS } from "./which-key.ts";

/** Marks the `:` line's input: the window is in `cmdline` mode while it has the focus. */
export const CMDLINE_ATTRIBUTE = "data-omfp-cmdline";

/** Marks the left column and the narrow frame's strip: the window is in `threads` mode while the focus is inside. */
export const THREADS_ATTRIBUTE = "data-omfp-threads";

/** The view the window shows (not one kept mounted hidden, UI-390). */
export const SHOWN_VIEW = '.omfp-view[data-testid="view"]';

const NON_TEXT_INPUTS = new Set(["button", "checkbox", "radio", "submit", "reset", "range", "color", "file", "image"]);
const TEXT_ROLES = new Set(["textbox", "searchbox", "combobox", "spinbutton"]);
/** Elements whose own Space behaviour (toggle, select) wins over the leader. */
const SPACE_ROLES = new Set(["checkbox", "radio", "switch", "option", "menuitem", "menuitemcheckbox", "menuitemradio"]);
/** Elements whose own Enter (press, follow, select, toggle) wins over a binding of Enter. */
const ENTER_ROLES = new Set([
  "button",
  "link",
  "tab",
  "row",
  "gridcell",
  "treeitem",
  "option",
  "menuitem",
  "menuitemcheckbox",
  "menuitemradio",
  "checkbox",
  "radio",
  "switch",
]);

/**
 * The mode `element` (the focused one) puts the window in (UI-098): `cmdline` in the `:` line, `insert` in a text
 * field, `threads` with nothing focused or the focus in the left column, `view` anywhere else (the view's widgets).
 */
export function modeAt(element: Element | null): WindowMode {
  if (element === null || element === document.body || element === document.documentElement) {
    return "threads";
  }
  if (element.closest(`[${CMDLINE_ATTRIBUTE}]`) !== null) {
    return "cmdline";
  }
  if (isTextEntry(element)) {
    return "insert";
  }
  return element.closest(`[${THREADS_ATTRIBUTE}]`) === null ? "view" : "threads";
}

function isTextEntry(element: Element): boolean {
  if (element instanceof HTMLInputElement) {
    return !NON_TEXT_INPUTS.has(element.type);
  }
  if (element instanceof HTMLTextAreaElement || element instanceof HTMLSelectElement) {
    return true;
  }
  if (element instanceof HTMLElement && element.isContentEditable) {
    return true;
  }
  return TEXT_ROLES.has(element.getAttribute("role") ?? "");
}

function keepsSpace(element: Element | null): boolean {
  if (element === null) {
    return false;
  }
  if (element instanceof HTMLInputElement && (element.type === "checkbox" || element.type === "radio")) {
    return true;
  }
  return SPACE_ROLES.has(element.getAttribute("role") ?? "");
}

function keepsEnter(element: Element | null): boolean {
  if (element === null) {
    return false;
  }
  if (
    element instanceof HTMLButtonElement ||
    element instanceof HTMLInputElement ||
    element instanceof HTMLSelectElement ||
    element instanceof HTMLTextAreaElement ||
    (element instanceof HTMLAnchorElement && element.hasAttribute("href")) ||
    element.tagName === "SUMMARY"
  ) {
    return true;
  }
  return ENTER_ROLES.has(element.getAttribute("role") ?? "");
}

/** Text is selected in the page (not in a text field, whose selection the document's does not show). */
function hasSelection(): boolean {
  const selection = document.getSelection();
  return selection !== null && !selection.isCollapsed && selection.toString() !== "";
}

/** Where the handler shows a waiting sequence (UI-320, UI-321). */
export interface WhichKeyOptions {
  /** The waiting sequence once the panel shows it; `undefined` when it ends (a key, Escape, {@link KeyHandler.cancel}). */
  readonly show: (pending: PendingKeys | undefined) => void;
  readonly timers: KeyTimers;
  /** Default {@link WHICH_KEY_DELAY_MS}. */
  readonly delayMs?: number;
}

export interface KeymapOptions {
  readonly bindings: readonly Binding[];
  readonly commands: CommandRegistry;
  /** Clock for sequence timeouts (`performance.now` in the browser, a counter in tests). */
  readonly now: () => number;
  /** The focused element; default `document.activeElement`. */
  readonly focused?: () => Element | null;
  /** The which-key panel; none: sequences wait unseen. */
  readonly whichKey?: WhichKeyOptions;
}

/** The document's `keydown` handler; `cancel` drops a waiting sequence (a click, the window losing the focus). */
export type KeyHandler = ((event: KeyboardEvent) => void) & { readonly cancel: () => void };

/** `keydown` handler for the document; returns it so the frame can add and remove it. */
export function createKeyHandler(options: KeymapOptions): KeyHandler {
  const matcher = new KeymapMatcher(options.bindings, options.now);
  const focused = options.focused ?? (() => document.activeElement);
  const whichKey = options.whichKey;
  // Each sequence change makes a new generation, so a delay set for an older one does nothing.
  let generation = 0;
  let shown = false;
  const hide = () => {
    generation += 1;
    if (shown) {
      shown = false;
      whichKey?.show(undefined);
    }
  };
  const reveal = (pending: PendingKeys) => {
    // The panel lists the next keys: the sequence waits for them as long as it shows.
    matcher.hold();
    shown = true;
    whichKey?.show(pending);
  };
  const wait = (pending: PendingKeys) => {
    generation += 1;
    if (whichKey === undefined) {
      return;
    }
    if (shown || opensAtOnce(pending)) {
      reveal(pending);
      return;
    }
    const current = generation;
    whichKey.timers.setTimeout(() => {
      if (current === generation && matcher.pending === pending.keys) {
        reveal(pending);
      }
    }, whichKey.delayMs ?? WHICH_KEY_DELAY_MS);
  };
  const cancel = () => {
    matcher.reset();
    hide();
  };
  const handle = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.isComposing) {
      return;
    }
    const element = focused();
    const token = keyToken(event);
    const modal = element?.closest('[role="dialog"], [role="alertdialog"]') ?? null;
    if (token === undefined || modal !== null) {
      return;
    }
    if (token === "Space" && matcher.pending === undefined && keepsSpace(element)) {
      return;
    }
    if (token === "Enter" && matcher.pending === undefined && keepsEnter(element)) {
      return;
    }
    if (token === "Esc" && matcher.pending !== undefined) {
      // Escape ends the sequence and nothing else (UI-322).
      cancel();
      event.preventDefault();
      return;
    }
    const mode = modeAt(element);
    if (sharesNormal(mode) && token === "Ctrl-c" && matcher.pending === undefined && hasSelection()) {
      return;
    }
    if (mode === "insert" && composesText(event)) {
      return;
    }
    const match = matcher.feed(token, mode);
    if (match.kind === "pending") {
      event.preventDefault();
      wait({ keys: match.keys, mode, scopes: scopesAt(element) });
      return;
    }
    hide();
    if (match.kind === "none") {
      return;
    }
    const { command, args = [] } = match.binding;
    const ran = options.commands.run(command, args, scopesAt(element));
    if (!ran && command === "focus.leave" && element instanceof HTMLElement) {
      // No widget decides where focus goes: a field of the view leaves for the view (View mode), any other field for
      // nothing (Threads mode), UI-095.
      const inView = element.closest(SHOWN_VIEW) !== null;
      element.blur();
      if (inView) {
        options.commands.run("view.focus");
      }
      event.preventDefault();
      return;
    }
    if (ran) {
      event.preventDefault();
    }
  };
  return Object.assign(handle, { cancel });
}
