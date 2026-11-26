/** Texts of the workspace keymap, `.omfp/keymap.json` (UI-210..212), and of the which-key panel (UI-320..UI-326). */

const MODES = {
  normal: "normal",
  threads: "threads",
  view: "view",
  insert: "insert",
  cmdline: "command line",
} as const;

export const keys = {
  /** The banner while the file is refused (UI-212). */
  invalid: (error: string) =>
    `The workspace keymap .omfp/keymap.json is invalid, its last valid bindings stay: ${error}`,
  /** The help panel's group of the workspace's bindings (UI-211). */
  help: {
    section: "workspace",
    /** A workspace binding of another mode than normal: its description and that mode. */
    inMode: (description: string, mode: keyof typeof MODES) => `${description} (${MODES[mode]})`,
  },
  /** The which-key panel (UI-320): the keys that complete a started sequence. */
  whichKey: {
    /** The panel's accessible name; `keys` is the chip label of the keys typed so far (`spc`, `g`). */
    label: (keys: string) => `Keys after ${keys}`,
    /** What the live region says when the panel shows (UI-323). */
    announce: (keys: string, count: number) => `${keys}: ${count} ${count === 1 ? "key" : "keys"}, Escape cancels`,
    /** The panel's sections, by what their commands act on. */
    groups: {
      threads: "threads",
      views: "views",
      workspace: "workspace",
      files: "files",
      help: "help",
      navigation: "move",
      commands: "command line",
      other: "other",
    },
    /** A key that starts more bindings: their group (`help…`), or how many when they share none. */
    prefix: (group: string) => `${group}…`,
    count: (count: number) => `${count} ${count === 1 ? "binding" : "bindings"}…`,
    /** After a binding whose command does not answer now (no thread selected, no widget registering it). */
    unavailable: "(not available now)",
    cancel: "cancel",
  },
} as const;
