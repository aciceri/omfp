/**
 * Context menus (UI-314..UI-319): the items of omfp's own menu on a message, a file or a thread. An item is a command
 * already registered (`command(...)`, `useCommand`), run with the clicked target as its one argument; the built-in
 * defaults come first, then what init files (`omfp.contextMenu`) and widgets (`useContextMenu`) add, in registration
 * order.
 *
 * Targets are found in the page by attribute, the innermost element carrying one winning: `data-omfp-message` (a
 * transcript row: the message's index in its thread's transcript), `data-omfp-path` (a file reference, with
 * `data-omfp-line`, as `:open` reads it, UI-116) and `data-omfp-thread` (a row of the left column: the thread's id). A
 * widget makes its own elements targets with the same attributes.
 */

import { strings } from "../strings/index.ts";
import { type Command, type CommandRegistry, scopesAt } from "./commands.ts";

const text = strings.shell.contextMenu;

/** What a context menu opens on. */
export type ContextTargetKind = "message" | "file" | "thread";

export const CONTEXT_TARGET_KINDS: readonly ContextTargetKind[] = ["message", "file", "thread"];

/** The attribute of a transcript row naming its message: the index in its thread's transcript. */
export const MESSAGE_ATTRIBUTE = "data-omfp-message";
/** The attribute of a thread row naming its thread. */
export const THREAD_ATTRIBUTE = "data-omfp-thread";
/** The attribute of a file reference (`:open` reads it too), and of its line. */
export const PATH_ATTRIBUTE = "data-omfp-path";
export const LINE_ATTRIBUTE = "data-omfp-line";

/** What a context menu opens on, and what its items' commands get. */
export interface ContextTarget {
  readonly kind: ContextTargetKind;
  /**
   * The commands' argument: a message's index in its thread's transcript, a file's path (`<path>:<line>` with a line,
   * as `:open` takes it), a thread's id.
   */
  readonly argument: string;
  /** The element carrying it: the commands run in its scopes. */
  readonly element: Element;
}

/** The target `element` is in, the innermost one; `undefined` outside every target. */
export function contextTargetAt(element: Element | null): ContextTarget | undefined {
  for (let node = element; node !== null; node = node.parentElement) {
    const message = node.getAttribute(MESSAGE_ATTRIBUTE);
    if (message !== null && message !== "") {
      return { kind: "message", argument: message, element: node };
    }
    const path = node.getAttribute(PATH_ATTRIBUTE);
    if (path !== null && path !== "") {
      const line = Number(node.getAttribute(LINE_ATTRIBUTE));
      return { kind: "file", argument: Number.isInteger(line) && line > 0 ? `${path}:${line}` : path, element: node };
    }
    const thread = node.getAttribute(THREAD_ATTRIBUTE);
    if (thread !== null && thread !== "") {
      return { kind: "thread", argument: thread, element: node };
    }
  }
  return undefined;
}

/** One item: a command's id or alias, and the label it shows (the command's title without one). */
export interface ContextMenuItem {
  readonly command: string;
  readonly title?: string;
}

/** What `omfp.contextMenu` and `useContextMenu` add: items on one kind of target. */
export interface ContextMenu {
  readonly on: ContextTargetKind;
  readonly items: readonly (string | ContextMenuItem)[];
}

/** The built-in items (UI-316), before every added one. */
const DEFAULT_ITEMS: Readonly<Record<ContextTargetKind, readonly ContextMenuItem[]>> = {
  message: [
    { command: "thread.fork", title: text.fork },
    { command: "thread.copy", title: text.copyMessage },
  ],
  file: [
    { command: "file.open", title: text.open },
    { command: "file.copyPath", title: text.copyPath },
  ],
  thread: [
    { command: "thread.archive", title: text.archive },
    { command: "thread.unarchive", title: text.unarchive },
    { command: "thread.copy", title: text.copyConversation },
    { command: "thread.export", title: text.export },
  ],
};

/** `menu` checked and its items as objects; throws what is wrong with it. */
function parseMenu(menu: unknown): { on: ContextTargetKind; items: ContextMenuItem[] } {
  const { on, items } = (typeof menu === "object" && menu !== null ? menu : {}) as Record<string, unknown>;
  const kind = CONTEXT_TARGET_KINDS.find((candidate) => candidate === on);
  if (kind === undefined) {
    throw new Error(text.badTarget(on));
  }
  if (!Array.isArray(items)) {
    throw new Error(text.badItem(items));
  }
  return {
    on: kind,
    items: items.map((item: unknown) => {
      if (typeof item === "string" && item !== "") {
        return { command: item };
      }
      const { command, title } = (typeof item === "object" && item !== null ? item : {}) as Record<string, unknown>;
      if (typeof command !== "string" || command === "" || (title !== undefined && typeof title !== "string")) {
        throw new Error(text.badItem(item));
      }
      return title === undefined ? { command } : { command, title };
    }),
  };
}

/** Who added items: the user's init file, the workspace's, or a widget (`undefined`), the order the menu lists them. */
export type ContextMenuOrigin = "user" | "workspace" | undefined;

const ORIGIN_RANK: Readonly<Record<"user" | "workspace", number>> = { user: 0, workspace: 1 };

/** The items init files and widgets added, per target kind (the host holds one per window). */
export class ContextMenus {
  #menus: ReadonlyArray<{ on: ContextTargetKind; items: ContextMenuItem[]; rank: number }> = [];

  /**
   * Adds `menu`'s items, by `origin`; returns their removal. Throws on an invalid menu. A stable sort keeps the
   * registration order within each origin.
   */
  register(menu: ContextMenu, origin?: ContextMenuOrigin): () => void {
    const entry = { ...parseMenu(menu), rank: origin === undefined ? 2 : ORIGIN_RANK[origin] };
    this.#menus = [...this.#menus, entry].sort((a, b) => a.rank - b.rank);
    return () => {
      this.#menus = this.#menus.filter((registered) => registered !== entry);
    };
  }

  /** The items on `kind`: the defaults, then those of the user's init file, the workspace's and the widgets. */
  items(kind: ContextTargetKind): readonly ContextMenuItem[] {
    return [...DEFAULT_ITEMS[kind], ...this.#menus.filter((menu) => menu.on === kind).flatMap((menu) => menu.items)];
  }
}

/** One item of an open menu. */
export interface ContextMenuEntry {
  /** Its position in the menu, as its key. */
  readonly key: string;
  /** The id or alias it runs. */
  readonly command: string;
  readonly title: string;
}

/**
 * The items `target`'s menu shows: each whose command answers in the target's scopes and is enabled for its argument,
 * a command reached by several items once (its first).
 */
export function contextMenuEntries(
  commands: CommandRegistry,
  menus: ContextMenus,
  target: ContextTarget,
): ContextMenuEntry[] {
  const scopes = scopesAt(target.element);
  const shown = new Set<Command>();
  const entries: ContextMenuEntry[] = [];
  for (const item of menus.items(target.kind)) {
    const command = commands.resolve(item.command, scopes, [target.argument]);
    if (command !== undefined && !shown.has(command)) {
      shown.add(command);
      entries.push({ key: String(entries.length), command: item.command, title: item.title ?? command.title });
    }
  }
  return entries;
}

/** Runs `entry` on `target`: its command in the target's scopes, the target as its argument and its element. */
export function runContextEntry(commands: CommandRegistry, entry: ContextMenuEntry, target: ContextTarget): boolean {
  return commands.run(entry.command, [target.argument], scopesAt(target.element), target.element);
}
