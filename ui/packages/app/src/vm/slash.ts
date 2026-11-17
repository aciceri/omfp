/**
 * `/` commands over omp's RPC (UI-975..UI-980). omp 18.22.0 announces (`available_commands_update`) only the builtins
 * with a text-mode handler; a builtin with only a TUI handler is not announced and, typed by hand, reaches the model as
 * plain text (omp's RPC dispatch finds no handler and prompts with it). The window therefore routes the commands omfp
 * has an equivalent for to it, refuses the other TUI-only builtins with the reason instead of sending them, and lists
 * both in the suggestions: the routed ones like any command, the refused ones disabled with the reason.
 */

import type { AvailableSlashCommand } from "@omfp/core";
import { strings } from "../strings/index.ts";

const text = strings.chat.composer.slash;

/**
 * omp 18.22.0's builtin commands with a TUI handler only (`handleTui` without `handle` in its builtin registry,
 * `packages/coding-agent/src/slash-commands/builtin-*.ts`), in the registry's order. The contract test
 * `contract_slash_commands` pins the commands omp announces over RPC: when it fails after an omp upgrade, check this
 * list against the new registry.
 */
export const OMP_TUI_COMMANDS: readonly { readonly name: string; readonly aliases: readonly string[] }[] = [
  { name: "settings", aliases: [] },
  { name: "setup", aliases: ["providers"] },
  { name: "plan", aliases: [] },
  { name: "plan-review", aliases: [] },
  { name: "vibe", aliases: [] },
  { name: "goal", aliases: [] },
  { name: "guided-goal", aliases: [] },
  { name: "loop", aliases: [] },
  { name: "queue", aliases: [] },
  { name: "collab", aliases: [] },
  { name: "join", aliases: [] },
  { name: "leave", aliases: [] },
  { name: "copy", aliases: [] },
  { name: "open", aliases: [] },
  { name: "hotkeys", aliases: [] },
  { name: "extensions", aliases: ["status"] },
  { name: "agents", aliases: [] },
  { name: "git", aliases: [] },
  { name: "local", aliases: [] },
  { name: "hub", aliases: ["subagents"] },
  { name: "branch", aliases: ["rewind"] },
  { name: "fork", aliases: [] },
  { name: "tree", aliases: [] },
  { name: "login", aliases: [] },
  { name: "logout", aliases: [] },
  { name: "new", aliases: [] },
  { name: "clear", aliases: [] },
  { name: "delete", aliases: [] },
  { name: "resume", aliases: [] },
  { name: "btw", aliases: [] },
  { name: "tan", aliases: [] },
  { name: "omfg", aliases: [] },
  { name: "cleanse", aliases: [] },
  { name: "debug", aliases: [] },
  { name: "exit", aliases: [] },
  { name: "restart", aliases: [] },
  { name: "skills", aliases: [] },
  { name: "live", aliases: [] },
  { name: "record", aliases: [] },
  { name: "pause", aliases: [] },
  { name: "quit", aliases: ["q"] },
];

/** What omfp does instead of sending a routed command to omp (UI-976). */
export type SlashRoute =
  /** Runs the window command `id` (`auth.login`, `thread.pickModel`, …) with `args`. */
  | { readonly type: "command"; readonly id: string; readonly args: readonly string[] }
  /** Forks the whole conversation into a new thread (`threads.fork`). */
  | { readonly type: "fork" };

/** What sending a draft does. */
export type SlashAction =
  /** `thread.prompt`: not a command, or one omp runs over RPC; `command` names the announced command sent (UI-979). */
  | { readonly type: "send"; readonly command?: string }
  /** An omfp equivalent runs instead; `name` is the command as omp knows it. */
  | { readonly type: "route"; readonly name: string; readonly route: SlashRoute }
  /** A command known not to work over RPC: nothing is sent, `message` says why (UI-977). */
  | { readonly type: "refuse"; readonly message: string };

const command = (id: string, args: readonly string[] = []): SlashRoute => ({ type: "command", id, args });

/** Splits arguments as the `:` command line does: on whitespace. */
const words = (args: string): string[] => args.split(/\s+/).filter((word) => word !== "");

/**
 * The commands omfp routes (UI-976), by omp's name: the route for the arguments typed, `undefined` when omfp has none
 * for them (an announced command then goes to omp, a TUI-only one is refused).
 */
const ROUTES: Readonly<Record<string, (args: string) => SlashRoute | undefined>> = {
  login: (args) => command("auth.login", words(args)),
  logout: (args) => command("auth.logout", words(args)),
  model: (args) => (args === "" ? command("thread.pickModel") : undefined),
  // Every form, its modes included, through `:compact` (UI-910, UI-913); the text stays one argument, as typed.
  compact: (args) => command("thread.compact", args === "" ? [] : [args]),
  fork: (args) => (args === "" ? { type: "fork" } : undefined),
  copy: (args) => (args === "" ? command("thread.copy") : undefined),
  export: (args) => (args === "" ? command("thread.export") : undefined),
  new: (args) => (args === "" ? command("thread.new") : undefined),
  hotkeys: (args) => (args === "" ? command("help.open") : undefined),
  resume: (args) => (args === "" ? command("threads.pick") : undefined),
};

/** omp's `/name args` (`parseSlashCommand`): the name ends at the first whitespace or `:`. */
function parseSlash(draft: string): { name: string; args: string } | undefined {
  if (!draft.startsWith("/") || draft.length === 1) {
    return undefined;
  }
  const body = draft.slice(1);
  const end = body.search(/[\s:]/);
  return end === -1 ? { name: body, args: "" } : { name: body.slice(0, end), args: body.slice(end + 1).trim() };
}

/** Why a TUI-only builtin does nothing here, with the omfp way when there is one. */
function reason(name: string): string {
  const hint = text.hints[name];
  return hint === undefined ? text.tuiOnly : `${text.tuiOnly}; ${hint}`;
}

/**
 * What sending `draft` (trimmed, pastes expanded) does, given the thread's announced `commands` (UI-975..UI-977): a
 * command a user, project, skill or extension defines goes to omp whatever its name; a routed builtin runs its omfp
 * equivalent; another TUI-only builtin is refused; everything else is sent.
 */
export function slashAction(draft: string, commands: readonly AvailableSlashCommand[]): SlashAction {
  const parsed = parseSlash(draft);
  if (parsed === undefined) {
    return { type: "send" };
  }
  const typed = parsed.name;
  const announced = commands.find((entry) => entry.name === typed || (entry.aliases?.includes(typed) ?? false));
  if (announced !== undefined && announced.source !== "builtin") {
    return { type: "send", command: announced.name };
  }
  const name =
    announced?.name ?? OMP_TUI_COMMANDS.find((entry) => entry.name === typed || entry.aliases.includes(typed))?.name;
  if (name === undefined) {
    return { type: "send" };
  }
  const route = ROUTES[name]?.(parsed.args);
  if (route !== undefined) {
    return { type: "route", name, route };
  }
  if (announced !== undefined) {
    return { type: "send", command: name };
  }
  return {
    type: "refuse",
    message: ROUTES[name] === undefined ? text.refused(name, reason(name)) : text.noArguments(name),
  };
}

/** One `/` command the composer offers (UI-978). */
export interface SlashEntry {
  readonly name: string;
  readonly aliases: readonly string[];
  readonly description: string | undefined;
  /** The usage hint of the command's argument. */
  readonly hint: string | undefined;
  /** Known not to work over RPC: shown, not picked; `description` says why. */
  readonly disabled: boolean;
}

/**
 * The commands the composer offers (UI-978): the announced ones in omp's order (a routed builtin described by what
 * omfp does), then omp's TUI-only builtins no announced command shadows, the routed ones like any command, the others
 * disabled with the reason.
 */
export function slashEntries(commands: readonly AvailableSlashCommand[]): SlashEntry[] {
  const taken = new Set(commands.flatMap((announced) => [announced.name, ...(announced.aliases ?? [])]));
  const announced = commands.map((announced) => ({
    name: announced.name,
    aliases: announced.aliases ?? [],
    description: (announced.source === "builtin" ? text.routed[announced.name] : undefined) ?? announced.description,
    hint: announced.input?.hint,
    disabled: false,
  }));
  const tuiOnly = OMP_TUI_COMMANDS.filter((tui) => !taken.has(tui.name)).map((tui) => {
    const routed = text.routed[tui.name];
    return {
      name: tui.name,
      aliases: tui.aliases.filter((alias) => !taken.has(alias)),
      description: routed ?? reason(tui.name),
      hint: routed === undefined ? undefined : text.routedHints[tui.name],
      disabled: routed === undefined,
    };
  });
  return [...announced, ...tuiOnly];
}
