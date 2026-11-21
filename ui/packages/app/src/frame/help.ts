/**
 * The help panel's content (UI-198): every effective binding (the defaults with the daemon's `[keymap]` entries, and
 * whatever command a widget registered), grouped by where it applies, then the `:` commands with their arguments.
 * The current workspace's `.omfp/keymap.json` bindings form a group of their own (UI-211). A binding's description is
 * the title of the command it runs in that place (the registered command of the scope the mode reaches), else the
 * built-in description of its id, else the id itself.
 */

import { type Binding, keyLabel, type Mode } from "../keymap/keymap.ts";
import type { Command } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { queueKeys } from "../vm/composer.ts";
import { rawPasteKeys } from "../vm/paste.ts";
import { unique } from "./cmdline.ts";

export type HelpSectionId = "workspace" | "normal" | "insert" | "transcript" | "cmdline" | "editor" | "commands";

export interface HelpRow {
  /** Key chip labels (UI-148), or `:name` for a command. */
  readonly keys: readonly string[];
  /** A command's argument values (`system | light | dark`). */
  readonly args?: string;
  readonly description: string;
  /** The command id the row runs; absent for keys a widget handles itself. */
  readonly command?: string;
}

export interface HelpSection {
  readonly id: HelpSectionId;
  readonly title: string;
  readonly rows: readonly HelpRow[];
  /** A line under the section's rule (the file editor's vim note). */
  readonly note?: string;
}

const commandTitles = strings.shell.commands;
const composer = strings.chat.composer;
const text = strings.shell.help;
const keyStrings = strings.keys;

/** Descriptions of the built-in command ids, for bindings whose command no mounted widget registers. */
const DESCRIPTIONS: Readonly<Record<string, string>> = {
  "nav.next": commandTitles.next,
  "nav.prev": commandTitles.prev,
  "nav.top": commandTitles.top,
  "nav.bottom": commandTitles.bottom,
  "nav.halfDown": commandTitles.halfDown,
  "nav.halfUp": commandTitles.halfUp,
  "nav.search": commandTitles.search,
  "threads.pick": commandTitles.pickThread,
  "nav.delete": commandTitles.deleteFocused,
  "nav.toggleSection": commandTitles.toggleSection,
  "threads.moveUp": commandTitles.moveUp,
  "threads.moveDown": commandTitles.moveDown,
  "threads.moveToPrevWorkspace": commandTitles.moveToPrevWorkspace,
  "threads.moveToNextWorkspace": commandTitles.moveToNextWorkspace,
  "file.open": commandTitles.open,
  "notice.open": strings.shell.notices.open,
  "composer.focus": composer.focus,
  "composer.interrupt": composer.interruptCommand,
  "composer.restore": composer.restoreCommand,
  ...composer.lineKeys,
  "focus.leave": composer.leave,
  "thread.abort": composer.abortCommand,
  "cmdline.open": commandTitles.cmdline,
  "help.open": commandTitles.help,
  "help.describeKey": strings.init.describe.key,
  "help.describeCommand": strings.init.describe.command,
  "help.messages": commandTitles.messages,
  "thread.nextAttention": commandTitles.nextAttention,
  "thread.next": commandTitles.nextThread,
  "thread.prev": commandTitles.prevThread,
  "thread.new": commandTitles.new,
  "workspace.new": commandTitles.newWorkspace,
  "workspace.rescan": commandTitles.rescan,
  "thread.rename": commandTitles.rename,
  "thread.archive": commandTitles.archive,
  "thread.unarchive": commandTitles.unarchive,
  "thread.delete": commandTitles.delete,
  "thread.stop": commandTitles.stop,
  "thread.stopSpawned": commandTitles.stopSpawned,
  "threads.spawnedBy": commandTitles.spawnedBy,
  "thread.restart": commandTitles.restart,
  "thread.copy": commandTitles.copy,
  "thread.export": commandTitles.export,
  "thread.pickModel": commandTitles.pickModel,
  "thread.compact": commandTitles.compact,
  "threads.toggle": commandTitles.toggleThreads,
  "threads.collapse": commandTitles.collapseThreads,
  "window.theme": commandTitles.theme,
  "density.set": commandTitles.density,
  "settings.set": commandTitles.set,
  "view.switch": commandTitles.view,
  "view.next": commandTitles.nextView,
  "view.prev": commandTitles.prevView,
  "view.pick": commandTitles.pickView,
  "file.pick": commandTitles.pickFile,
  "view.reset": commandTitles.resetViews,
  "widget.manage": commandTitles.widget,
};

/** The transcript's moves (its `nav.*` commands in the `transcript` scope). */
const TRANSCRIPT: Readonly<Record<string, string>> = {
  "nav.next": strings.chat.nav.next,
  "nav.prev": strings.chat.nav.prev,
  "nav.top": strings.chat.nav.top,
  "nav.bottom": strings.chat.nav.bottom,
  "nav.halfDown": strings.chat.nav.halfDown,
  "nav.halfUp": strings.chat.nav.halfUp,
};

/** The built-in description of `id`, if it has one. */
export function builtinDescription(id: string): string | undefined {
  return DESCRIPTIONS[id];
}

const keys = text.keys;

/** Keys the composer handles itself (UI-060, UI-061, UI-176). */
const COMPOSER_KEYS: readonly HelpRow[] = [
  { keys: [keyLabel("Enter")], description: keys.send },
  {
    keys: [keyLabel(queueKeys()), keyLabel("Shift-Enter"), keyLabel("Ctrl-q")],
    description: keys.queue,
  },
  { keys: [keyLabel("Alt-Enter")], description: keys.newline },
  { keys: [keyLabel("Tab")], description: keys.complete },
  { keys: [keyLabel("ArrowRight")], description: keys.acceptWord },
  { keys: [keyLabel(rawPasteKeys())], description: keys.pasteRaw },
  { keys: [keyLabel("ArrowUp"), keyLabel("ArrowDown")], description: keys.history },
];

/** Keys the transcript handles itself (UI-095). */
const TRANSCRIPT_KEYS: readonly HelpRow[] = [
  { keys: [keyLabel("PageUp"), keyLabel("PageDown")], description: keys.page },
  { keys: [keyLabel("Home"), keyLabel("End")], description: keys.ends },
];

/** Keys of the `:` line itself (UI-168). */
const CMDLINE_KEYS: readonly HelpRow[] = [
  { keys: [keyLabel("Tab")], description: keys.cmdComplete },
  { keys: [keyLabel("ArrowUp"), keyLabel("ArrowDown")], description: keys.cmdMove },
  { keys: [keyLabel("Enter")], description: keys.cmdRun },
  { keys: [keyLabel("Escape")], description: keys.cmdCancel },
];

/** vim's keys the file editor documents (UI-111). */
const EDITOR_KEYS: readonly HelpRow[] = [
  { keys: [keyLabel("Escape")], description: keys.vimEsc },
  { keys: [":w", keyLabel("Ctrl-s")], description: keys.vimSave },
  { keys: [":q"], description: keys.vimQuit },
  { keys: [":q!"], description: keys.vimDrop },
];

/** The title of the first command registered for `id` in one of `scopes` (`undefined`: global), in that order. */
function registered(commands: readonly Command[], id: string, scopes: readonly (string | undefined)[]) {
  for (const scope of scopes) {
    const command = commands.find((candidate) => candidate.id === id && candidate.scope === scope);
    if (command !== undefined) {
      return command.title;
    }
  }
  return commands.find((candidate) => candidate.id === id)?.title;
}

/** One row per command (and arguments) of `bindings` in `mode`, its keys in binding order. */
function bindingRows(
  bindings: readonly Binding[],
  mode: Mode,
  describe: (binding: Binding) => string | undefined,
): HelpRow[] {
  const rows = new Map<string, { keys: string[]; args?: string; description: string; command: string }>();
  for (const binding of bindings) {
    if (binding.mode !== mode) {
      continue;
    }
    const description = describe(binding);
    if (description === undefined) {
      continue;
    }
    const args = (binding.args ?? []).join(" ");
    const key = `${binding.command}\u0000${args}`;
    const row = rows.get(key);
    if (row === undefined) {
      rows.set(key, {
        keys: [keyLabel(binding.keys)],
        ...(args === "" ? {} : { args }),
        description,
        command: binding.command,
      });
    } else {
      row.keys.push(keyLabel(binding.keys));
    }
  }
  return [...rows.values()];
}

/** The `:` names of `command`: its aliases, else its id. */
function commandNames(command: Command): string[] {
  const names = command.aliases === undefined || command.aliases.length === 0 ? [command.id] : command.aliases;
  return names.map((name) => `:${name}`);
}

/** Every section of the help, unfiltered. The current workspace's bindings (UI-211) have their own section, first;
 * the others show the defaults with the daemon's `[keymap]` entries. */
export function helpSections(bindings: readonly Binding[], commands: readonly Command[]): HelpSection[] {
  const transcriptScoped = new Set(
    commands.filter((command) => command.scope === "transcript").map((command) => command.id),
  );
  const fallback = (binding: Binding) => builtinDescription(binding.command) ?? binding.command;
  // The command a binding runs in its mode: the registered one of the scope the mode reaches, else the built-in.
  const describeIn: Readonly<Record<Mode, (binding: Binding) => string>> = {
    normal: (binding) => registered(commands, binding.command, [undefined, "threads"]) ?? fallback(binding),
    insert: (binding) => registered(commands, binding.command, ["composer", undefined]) ?? fallback(binding),
    cmdline: (binding) => registered(commands, binding.command, [undefined]) ?? fallback(binding),
  };
  const workspace = bindings.filter((binding) => binding.source === "workspace");
  const shared = bindings.filter((binding) => binding.source !== "workspace");
  const workspaceRows = (["normal", "insert", "cmdline"] as const).flatMap((mode) =>
    bindingRows(workspace, mode, (binding) =>
      mode === "normal" ? describeIn.normal(binding) : keyStrings.help.inMode(describeIn[mode](binding), mode),
    ),
  );
  const sections: HelpSection[] = [
    { id: "workspace", title: keyStrings.help.section, rows: workspaceRows },
    {
      id: "normal",
      title: text.sections.normal,
      rows: bindingRows(shared, "normal", describeIn.normal),
    },
    {
      id: "insert",
      title: text.sections.insert,
      rows: [...bindingRows(shared, "insert", describeIn.insert), ...COMPOSER_KEYS],
    },
    {
      id: "transcript",
      title: text.sections.transcript,
      rows: [
        ...bindingRows(shared, "normal", (binding) =>
          transcriptScoped.has(binding.command)
            ? registered(commands, binding.command, ["transcript"])
            : TRANSCRIPT[binding.command],
        ),
        ...TRANSCRIPT_KEYS,
      ],
    },
    {
      id: "cmdline",
      title: text.sections.cmdline,
      rows: [...bindingRows(shared, "cmdline", describeIn.cmdline), ...CMDLINE_KEYS],
    },
    { id: "editor", title: text.sections.editor, rows: EDITOR_KEYS, note: text.editorNote },
    {
      id: "commands",
      title: text.sections.commands,
      rows: unique(commands).map((command) => {
        const args = command.args?.([]) ?? [];
        return {
          keys: commandNames(command),
          ...(args.length === 0 ? {} : { args: args.join(" | ") }),
          description: command.title,
          command: command.id,
        };
      }),
    },
  ];
  return sections;
}

/** `sections` keeping the rows whose keys, arguments, description or command contain `query` (any case); sections
 * left empty are dropped. */
export function filterHelp(sections: readonly HelpSection[], query: string): HelpSection[] {
  const needle = query.trim().toLowerCase();
  const matches = (row: HelpRow) =>
    needle === "" ||
    [...row.keys, row.args ?? "", row.description, row.command ?? ""].some((part) =>
      part.toLowerCase().includes(needle),
    );
  return sections
    .map((section) => ({ ...section, rows: section.rows.filter(matches) }))
    .filter((section) => section.rows.length > 0);
}
