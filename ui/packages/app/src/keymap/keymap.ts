/**
 * The keymap (UI-095): data, not code. A binding maps a key sequence in a mode to a command id, which runs through the
 * command registry (`sdk/commands.ts`), so bindings can be made configurable later without touching any widget.
 *
 * Keys are written as tokens: a printable key is its character (`j`, `G`, `:`, `/`, `]`), other keys their name
 * (`Space`, `Esc`, `Enter`, `Tab`, `PageDown`, …), prefixed by modifiers in the order `Ctrl-`, `Alt-`, `Meta-` (and
 * `Shift-` only for non-printable keys: the character already says it for printable ones). A sequence is tokens
 * separated by spaces: `g g`, `Space n`.
 */

/** `normal`: focus not in a text field; `insert`: a text field has the focus; `cmdline`: the `:` line is open. */
export type Mode = "normal" | "insert" | "cmdline";

/**
 * Where a binding comes from when it is not a default: the daemon's `[keymap]` entries (`config`, UI-097), the current
 * workspace's `.omfp/keymap.json` (`workspace`, UI-210), or an init file's `bind` (`init:user`, `init:workspace`,
 * UI-262).
 */
export type BindingSource = "config" | "workspace" | "init:user" | "init:workspace";

export interface Binding {
  readonly mode: Mode;
  /** Tokens separated by spaces, e.g. `g g`. */
  readonly keys: string;
  readonly command: string;
  /** Arguments passed to the command. */
  readonly args?: readonly string[];
  /** Where the binding comes from; a default without one. */
  readonly source?: BindingSource;
}

/**
 * The defaults: vim-style navigation, `:` command line, `Space` leader (UI-095). `Space` alone opens the leader menu
 * (UI-324): the flat leader keys stay as they were; the groups `Space h` (help), `Space p` (workspace), `Space f`
 * (files) and `Space v` (views) hold the rest (UI-326).
 */
export const DEFAULT_BINDINGS: readonly Binding[] = [
  { mode: "normal", keys: "j", command: "nav.next" },
  { mode: "normal", keys: "k", command: "nav.prev" },
  { mode: "normal", keys: "g g", command: "nav.top" },
  { mode: "normal", keys: "G", command: "nav.bottom" },
  { mode: "normal", keys: "Ctrl-d", command: "nav.halfDown" },
  { mode: "normal", keys: "Ctrl-u", command: "nav.halfUp" },
  { mode: "normal", keys: "/", command: "nav.search" },
  { mode: "normal", keys: "Space /", command: "threads.pick" },
  // On the focused row or workspace section of the left column (UI-020, UI-022).
  { mode: "normal", keys: "d d", command: "nav.delete" },
  { mode: "normal", keys: "z a", command: "nav.toggleSection" },
  // Move the focused row or section of the left column; with Shift, a thread into the previous or next workspace
  // (UI-624).
  { mode: "normal", keys: "Alt-ArrowUp", command: "threads.moveUp" },
  { mode: "normal", keys: "Alt-ArrowDown", command: "threads.moveDown" },
  { mode: "normal", keys: "Alt-Shift-ArrowUp", command: "threads.moveToPrevWorkspace" },
  { mode: "normal", keys: "Alt-Shift-ArrowDown", command: "threads.moveToNextWorkspace" },
  { mode: "normal", keys: "i", command: "composer.focus" },
  { mode: "normal", keys: ":", command: "cmdline.open" },
  // View cycling stays independent of the fuzzy picker (UI-302).
  { mode: "normal", keys: "] v", command: "view.next" },
  { mode: "normal", keys: "[ v", command: "view.prev" },
  // Every effective binding and `:` command, filterable (UI-198).
  { mode: "normal", keys: "?", command: "help.open" },
  { mode: "normal", keys: "Space a", command: "thread.nextAttention" },
  { mode: "normal", keys: "] a", command: "thread.nextAttention" },
  { mode: "normal", keys: "Space n", command: "thread.new" },
  { mode: "normal", keys: "Space w", command: "workspace.new" },
  { mode: "normal", keys: "Space r", command: "thread.rename" },
  { mode: "normal", keys: "Space x", command: "thread.archive" },
  { mode: "normal", keys: "Space u", command: "thread.unarchive" },
  { mode: "normal", keys: "Space d", command: "thread.delete" },
  { mode: "normal", keys: "Space s", command: "thread.stop" },
  { mode: "normal", keys: "Space S", command: "thread.stopSpawned" },
  { mode: "normal", keys: "Space R", command: "thread.restart" },
  // Focus the column in a wide window; toggle its drawer in a narrow one.
  { mode: "normal", keys: "Space t", command: "threads.toggle" },
  // Hide the wide column, or show it again (UI-985).
  { mode: "normal", keys: "Space b", command: "threads.collapse" },
  { mode: "normal", keys: "Space :", command: "cmdline.open" },
  // The full command line from the leader menu (UI-325); `:` stays.
  { mode: "normal", keys: "Space Space", command: "cmdline.open" },
  // Leader groups (UI-326).
  { mode: "normal", keys: "Space h h", command: "help.open" },
  { mode: "normal", keys: "Space h k", command: "help.describeKey" },
  // The notices and toasts that went by, as Emacs's `C-h e` (UI-522).
  { mode: "normal", keys: "Space h e", command: "help.messages" },
  { mode: "normal", keys: "Space p n", command: "workspace.new" },
  { mode: "normal", keys: "Space p r", command: "workspace.rescan" },
  { mode: "normal", keys: "Space f", command: "file.pick" },
  { mode: "normal", keys: "Space v", command: "view.pick" },
  { mode: "insert", keys: "Esc", command: "focus.leave" },
  // In the composer: clear the draft, then abort; `u` restores the last cleared draft (UI-063). In normal mode `Ctrl-c`
  // interrupts the selected thread (UI-368), unless text is selected: then it copies as usual.
  { mode: "insert", keys: "Ctrl-c", command: "composer.interrupt" },
  { mode: "normal", keys: "Ctrl-c", command: "thread.abort" },
  { mode: "normal", keys: "u", command: "composer.restore" },
  // One chord reaches the next waiting thread from anywhere, even while typing (UI-021).
  { mode: "normal", keys: "Alt-a", command: "thread.nextAttention" },
  { mode: "insert", keys: "Alt-a", command: "thread.nextAttention" },
  // The next and previous thread of the left column, across workspaces, from anywhere (UI-989).
  { mode: "normal", keys: "Alt-j", command: "thread.next" },
  { mode: "insert", keys: "Alt-j", command: "thread.next" },
  { mode: "normal", keys: "Alt-k", command: "thread.prev" },
  { mode: "insert", keys: "Alt-k", command: "thread.prev" },
  { mode: "normal", keys: "Alt-n", command: "thread.new" },
  { mode: "insert", keys: "Alt-n", command: "thread.new" },
  // readline's line editing in the composer (UI-760); `Ctrl-Shift-a` keeps select-all. A browser tab never gets
  // `Ctrl-w`, `Ctrl-n`, `Ctrl-t`: `Alt-Backspace` kills a word there too.
  { mode: "insert", keys: "Ctrl-a", command: "composer.lineStart" },
  { mode: "insert", keys: "Ctrl-e", command: "composer.lineEnd" },
  { mode: "insert", keys: "Ctrl-b", command: "composer.charBack" },
  { mode: "insert", keys: "Ctrl-f", command: "composer.charForward" },
  { mode: "insert", keys: "Alt-b", command: "composer.wordBack" },
  { mode: "insert", keys: "Alt-f", command: "composer.wordForward" },
  { mode: "insert", keys: "Ctrl-p", command: "composer.linePrev" },
  { mode: "insert", keys: "Ctrl-n", command: "composer.lineNext" },
  { mode: "insert", keys: "Ctrl-d", command: "composer.deleteForward" },
  { mode: "insert", keys: "Ctrl-h", command: "composer.deleteBack" },
  { mode: "insert", keys: "Ctrl-k", command: "composer.killLineEnd" },
  { mode: "insert", keys: "Ctrl-u", command: "composer.killLineStart" },
  { mode: "insert", keys: "Ctrl-w", command: "composer.killWordBack" },
  { mode: "insert", keys: "Alt-Backspace", command: "composer.killWordBack" },
  { mode: "insert", keys: "Alt-d", command: "composer.killWordForward" },
  { mode: "insert", keys: "Ctrl-y", command: "composer.yank" },
  { mode: "insert", keys: "Ctrl-t", command: "composer.transpose" },
  { mode: "insert", keys: "Ctrl-Shift-a", command: "composer.selectAll" },
];

/** How long a started sequence waits for its next key. */
export const SEQUENCE_TIMEOUT_MS = 1000;

/** The parts of a `KeyboardEvent` a token is made of. */
export type KeyInput = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "altKey" | "metaKey" | "shiftKey">;

const NAMED: Readonly<Record<string, string>> = { " ": "Space", Escape: "Esc", Esc: "Esc" };
/** Printable ASCII: what Alt leaves of a key outside macOS. */
const ASCII = /^[\x20-\x7e]+$/;
/** The letter or digit of a physical key (`KeyA`, `Digit1`). */
const LETTER_CODE = /^(?:Key([A-Z])|Digit([0-9]))$/;

/**
 * The key of an Alt chord: macOS's Option composes (`Option-a` is `å`, `Option-n` the dead key of `˜`), so a chord whose
 * key is not ASCII takes the letter or digit of its physical key (UI-021); elsewhere Alt leaves the key as typed.
 */
function altChordKey(event: KeyInput): string {
  if (event.key !== "Dead" && ASCII.test(event.key)) {
    return event.key;
  }
  const letter = LETTER_CODE.exec(event.code);
  return letter === null ? event.key : (letter[1] ?? letter[2] ?? event.key);
}

/**
 * Whether a key press is macOS's Option composing text (`å`, the dead key of `ñ`, `õ`): an Alt press without Ctrl or
 * Meta whose key is not ASCII. While typing it types its text instead of running a chord (UI-405).
 */
export function composesText(event: KeyInput): boolean {
  return event.altKey && !event.ctrlKey && !event.metaKey && (event.key === "Dead" || !ASCII.test(event.key));
}

/** The token of a key press; `undefined` for a lone modifier. */
export function keyToken(event: KeyInput): string | undefined {
  const key = event.altKey ? altChordKey(event) : event.key;
  if (key === "Control" || key === "Alt" || key === "Meta" || key === "Shift" || key === "Dead" || key === "") {
    return undefined;
  }
  const name = NAMED[key] ?? key;
  const printable = [...name].length === 1;
  const prefix = `${event.ctrlKey ? "Ctrl-" : ""}${event.altKey ? "Alt-" : ""}${event.metaKey ? "Meta-" : ""}`;
  // Ctrl/Alt chords on letters are written in lower case (`Ctrl-d`), whatever Caps Lock did to the key; Shift counts
  // on Ctrl chords only (`Ctrl-Shift-a`, UI-760), so macOS's Option-Shift letters stay the Alt chord (UI-021).
  const base = printable && prefix !== "" ? name.toLowerCase() : name;
  const shifted = event.shiftKey && (!printable || (event.ctrlKey && base !== base.toUpperCase()));
  return `${prefix}${shifted ? "Shift-" : ""}${base}`;
}

/** What one key press did. */
export type Match =
  /** A binding's sequence is complete. */
  | { readonly kind: "command"; readonly binding: Binding }
  /** The keys so far start at least one binding: wait for the next. */
  | { readonly kind: "pending"; readonly keys: string }
  /** No binding: the key is left to the page. */
  | { readonly kind: "none" };

/**
 * Matches key presses against bindings, one sequence at a time. A sequence whose next key does not arrive within
 * `timeoutMs` (by the injected clock) is dropped, unless it is held ({@link KeymapMatcher.hold}). A complete binding
 * wins over longer ones sharing its prefix, so defaults never bind both `g` and `g g`.
 */
export class KeymapMatcher {
  readonly #bindings: readonly Binding[];
  readonly #now: () => number;
  readonly #timeoutMs: number;
  #pending: { keys: string[]; mode: Mode; at: number; held: boolean } | undefined;

  constructor(bindings: readonly Binding[], now: () => number, timeoutMs = SEQUENCE_TIMEOUT_MS) {
    this.#bindings = bindings;
    this.#now = now;
    this.#timeoutMs = timeoutMs;
  }

  /** The sequence typed so far (for a hint), if any. */
  get pending(): string | undefined {
    return this.#live()?.keys.join(" ");
  }

  feed(token: string, mode: Mode): Match {
    const pending = this.#live();
    const continued = pending !== undefined && pending.mode === mode ? [...pending.keys, token] : undefined;
    const match = continued === undefined ? undefined : this.#match(continued, mode);
    if (match !== undefined) {
      return match;
    }
    // The key does not continue the sequence: it may start a new one.
    return this.#match([token], mode) ?? this.#none();
  }

  reset(): void {
    this.#pending = undefined;
  }

  /** Keeps the sequence typed so far from timing out until it ends: the which-key panel shows it (UI-321). */
  hold(): void {
    if (this.#pending !== undefined) {
      this.#pending = { ...this.#pending, held: true };
    }
  }

  #match(keys: string[], mode: Mode): Match | undefined {
    const sequence = keys.join(" ");
    const candidates = this.#bindings.filter((binding) => binding.mode === mode);
    const exact = candidates.find((binding) => binding.keys === sequence);
    if (exact !== undefined) {
      this.#pending = undefined;
      return { kind: "command", binding: exact };
    }
    if (candidates.some((binding) => binding.keys.startsWith(`${sequence} `))) {
      this.#pending = { keys, mode, at: this.#now(), held: false };
      return { kind: "pending", keys: sequence };
    }
    return undefined;
  }

  #none(): Match {
    this.#pending = undefined;
    return { kind: "none" };
  }

  #live(): { keys: string[]; mode: Mode } | undefined {
    const pending = this.#pending;
    if (pending !== undefined && !pending.held && this.#now() - pending.at > this.#timeoutMs) {
      this.#pending = undefined;
      return undefined;
    }
    return pending;
  }
}

/** The keys bound to `command` in `mode`, for hints (`Space n`). */
export function keysFor(bindings: readonly Binding[], command: string, mode: Mode = "normal"): string[] {
  return bindings
    .filter((binding) => binding.command === command && binding.mode === mode)
    .map((binding) => binding.keys);
}

const MODIFIER = /^(Ctrl|Alt|Meta|Shift)-(?=.)/;
/** Named keys whose chip is a shorter word than their lowercased name (`ArrowUp` → `up`). */
const CHIP_NAMES: Readonly<Record<string, string>> = {
  Space: "spc",
  Escape: "esc",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
};

/** Whether the page runs on macOS (`navigator.platform` `MacIntel`, also in the desktop window's WKWebView). */
export const MAC = typeof navigator !== "undefined" && navigator.platform.startsWith("Mac");

/**
 * The key chip label of a key sequence (UI-148): lowercase words (`spc`, `esc`, `enter`, `alt+enter`, `ctrl+c`),
 * printable keys as typed (`G`, `:`), tokens separated by a space (`spc n`, `d d`); never symbol glyphs. On macOS
 * (`mac`) Alt is the Option key: `opt+a` (UI-280).
 */
export function keyLabel(keys: string, mac = MAC): string {
  return normalizeKeys(keys)
    .split(" ")
    .map((token) => {
      const modifiers: string[] = [];
      let rest = token;
      for (let match = MODIFIER.exec(rest); match !== null; match = MODIFIER.exec(rest)) {
        const modifier = (match[1] ?? "").toLowerCase();
        modifiers.push(mac && modifier === "alt" ? "opt" : modifier);
        rest = rest.slice(match[0].length);
      }
      const base = CHIP_NAMES[rest] ?? ([...rest].length === 1 ? rest : rest.toLowerCase());
      return [...modifiers, base].join("+");
    })
    .join(" ");
}

/** One `[keymap]` entry of the daemon's settings (UI-097), or of a workspace's `.omfp/keymap.json` with its `args`
 * (UI-210): `command: ""` removes the binding of `keys` in `mode`. */
export interface KeymapOverride {
  readonly mode: Mode;
  readonly keys: string;
  readonly command: string;
  readonly args?: readonly string[];
}

/** `keys` with single spaces between tokens, as bindings are written. */
export function normalizeKeys(keys: string): string {
  return keys.trim().split(/\s+/).join(" ");
}

/**
 * The defaults with `overrides` applied one by one (UI-097): an entry replaces the binding with the same mode and
 * keys (keeping its place), `""` removes it, any other entry is added after the defaults. Later entries win. The
 * bindings an entry makes carry its `args` and `source`.
 */
export function mergeBindings(
  defaults: readonly Binding[],
  overrides: readonly KeymapOverride[],
  source?: Binding["source"],
): readonly Binding[] {
  if (overrides.length === 0) return defaults;
  let bindings: Binding[] = [...defaults];
  for (const override of overrides) {
    const keys = normalizeKeys(override.keys);
    const same = (binding: Binding) => binding.mode === override.mode && binding.keys === keys;
    if (override.command === "") {
      bindings = bindings.filter((binding) => !same(binding));
      continue;
    }
    const replacement: Binding = {
      mode: override.mode,
      keys,
      command: override.command,
      ...(override.args === undefined || override.args.length === 0 ? {} : { args: override.args }),
      ...(source === undefined ? {} : { source }),
    };
    const index = bindings.findIndex(same);
    if (index < 0) {
      bindings.push(replacement);
    } else {
      bindings[index] = replacement;
    }
  }
  return bindings;
}

/** The bindings the init files make (UI-262), applied after the workspace's: the user's, then the workspace's. */
export interface InitBindings {
  readonly user: readonly KeymapOverride[];
  readonly workspace: readonly KeymapOverride[];
}

export const NO_INIT_BINDINGS: InitBindings = { user: [], workspace: [] };

/** What a key sequence typed for `:describe-key` is (UI-264). */
export type Described =
  /** The keys so far start at least one binding: wait for the next. */
  | { readonly kind: "pending" }
  /** The bindings of exactly these keys, in every mode (none: not bound). */
  | { readonly kind: "done"; readonly keys: string; readonly bindings: readonly Binding[] };

/**
 * The bindings of `keys` (tokens separated by spaces) in any mode: complete as soon as a binding has exactly these
 * keys or none starts with them, like {@link KeymapMatcher} where a complete binding wins over longer ones.
 */
export function describeKeys(bindings: readonly Binding[], keys: string): Described {
  const exact = bindings.filter((binding) => binding.keys === keys);
  if (exact.length === 0 && bindings.some((binding) => binding.keys.startsWith(`${keys} `))) {
    return { kind: "pending" };
  }
  return { kind: "done", keys, bindings: exact };
}
