/**
 * Texts of the init files (UI-260..UI-263, UI-267) and of `:describe-key` /
 * `:describe-command` (UI-264, UI-265).
 */

const MODES = { normal: "normal", insert: "insert", cmdline: "command line" } as const;

export const init = {
  /** How the banner names each file. */
  files: { user: "the user's init.ts", workspace: "the workspace's .omfp/init.ts" },
  /** The banner of a failed init file (UI-263). */
  banner: (file: string, message: string) => `Init file ${file} failed, the window works without it: ${message}`,
  noDefaultExport: "it has no default export: write `export default function init(omfp) { … }`",
  load: (message: string) => `cannot load it: ${message}`,
  cleanup: (message: string) => `its cleanup failed: ${message}`,
  command: (id: string, message: string) => `its command ${id} failed: ${message}`,
  listener: (event: string, message: string) => `its "${event}" listener failed: ${message}`,
  /** `omfp.views` (UI-267). */
  views: {
    loadOnly: (file: string) => `only "views.json" can be loaded, not ${JSON.stringify(file)}`,
    notViews: "expected views from omfp.views.load(…)",
    transform: (message: string) => `its views transform failed: ${message}`,
    invalid: (error: string) => `invalid view: ${error}`,
    noTabs: (id: string) => `no tabs with "id": ${JSON.stringify(id)} in the layout`,
    emptyLayout: (reference: string) => `removing ${reference} would leave the layout empty`,
    noView: (id: string) => `no view ${JSON.stringify(id)}`,
    lastView: "cannot remove the last view",
  },
  invalid: (error: string) => `invalid layout: ${error}`,
  /** `:describe-key` and `:describe-command` (UI-264, UI-265). */
  describe: {
    key: "Describe a key: what it runs and where it is bound",
    command: "Describe a command: its keys and where they are bound (:describe-command <name>)",
    title: "describe",
    label: "Describe a key or a command",
    close: "close",
    press: "Press the keys to describe (Escape cancels)",
    pending: (keys: string) => `${keys} …`,
    unbound: (keys: string) => `${keys} is not bound in any mode`,
    unknown: (name: string) => `No command answers to ${name}`,
    missingName: "Name the command: :describe-command <name>",
    mode: (mode: keyof typeof MODES) => MODES[mode],
    fields: {
      keys: "keys",
      mode: "mode",
      command: "command",
      args: "arguments",
      title: "title",
      description: "description",
      names: "names",
      scope: "scope",
      defined: "defined by",
      bound: "bound in",
      bindings: "key bindings",
    },
    /** A binding's command that no widget or init file registers right now. */
    notRegistered: "no registered command answers to it now",
    noBinding: "none",
    global: "global",
    /** Where a binding comes from. */
    sources: {
      default: "default keymap",
      config: "[keymap] of the daemon's config file",
      workspace: "the workspace's .omfp/keymap.json",
      "init:user": "the user's init.ts",
      "init:workspace": "the workspace's .omfp/init.ts",
    },
    /** Who registered a command. */
    origins: {
      user: "the user's init.ts",
      workspace: "the workspace's .omfp/init.ts",
      window: "the window or a widget",
    },
  },
} as const;
