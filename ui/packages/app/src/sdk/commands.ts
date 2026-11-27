/**
 * Commands (UI-095): every keyboard action and every `:` command line entry is a command id run through one registry.
 * The keymap maps key sequences to ids; the command line lists the registered commands with completion.
 *
 * Several widgets may register the same id in different **scopes**: View mode's `j` runs `nav.next`, which moves
 * between messages in the `transcript` scope and does what another widget registers in its own. A scope is the
 * `data-omfp-scope` attribute of a widget's root element; {@link CommandRegistry.resolve} prefers the command of the
 * innermost scope around the focused element, then the global one (no scope).
 */

/** One runnable command. */
export interface Command {
  /** Dotted id, e.g. `thread.archive`; the keymap binds to it. */
  readonly id: string;
  /** Shown in the command line's completion list. */
  readonly title: string;
  /** A longer explanation for `:describe-command` (UI-265); the title stands alone without one. */
  readonly description?: string;
  /** The init file that registered it (UI-261), for `:describe-command`; `undefined` for widgets and the frame. */
  readonly init?: "user" | "workspace";
  /**
   * The workspace or user widget that registered it (its reference, UI-351): set by `useCommand` inside a module
   * widget; `undefined` for built-ins, the frame and init files.
   */
  readonly widget?: string;
  /** Names typed after `:` besides the id, e.g. `archive` for `thread.archive`. */
  readonly aliases?: readonly string[];
  /** Widget scope it belongs to; global when absent. */
  readonly scope?: string;
  /**
   * Completions of the argument being typed, given the arguments before it (`:theme ` → `light`, `dark`, `system`;
   * `:widget add ` → the widgets).
   */
  readonly args?: (before: readonly string[]) => readonly string[];
  /**
   * Hidden from the command line and not run when `false`. `args` are those it would run with when known (a context
   * menu item passes its target, UI-315), so a command can offer itself for some targets only.
   */
  readonly enabled?: (args?: readonly string[]) => boolean;
  run(args: readonly string[]): void | Promise<void>;
}

/** The HTML attribute naming a widget's command scope. */
export const SCOPE_ATTRIBUTE = "data-omfp-scope";

/** Scopes around `element`, innermost first. */
export function scopesAt(element: Element | null): string[] {
  const scopes: string[] = [];
  for (let node = element; node !== null; node = node.parentElement) {
    const scope = node.getAttribute(SCOPE_ATTRIBUTE);
    if (scope !== null) {
      scopes.push(scope);
    }
  }
  return scopes;
}

export class CommandRegistry {
  /** Newest registration last; a later registration of the same id and scope shadows the earlier one. */
  #commands: Command[] = [];
  readonly #listeners = new Set<() => void>();

  /** Adds `command`; returns the function that removes it. */
  register(command: Command): () => void {
    this.#commands = [...this.#commands, command];
    this.#changed();
    return () => {
      this.#commands = this.#commands.filter((registered) => registered !== command);
      this.#changed();
    };
  }

  /** Registered commands, newest registration of each (id, scope) only. */
  list(): readonly Command[] {
    const seen = new Set<string>();
    const commands: Command[] = [];
    for (const command of [...this.#commands].reverse()) {
      const key = `${command.scope ?? ""}\u0000${command.id}`;
      if (!seen.has(key)) {
        seen.add(key);
        commands.push(command);
      }
    }
    return commands.reverse();
  }

  /**
   * The enabled command for `id` (or an alias) in the innermost of `scopes`, else the global one; `args`, when given,
   * are what it would run with (its `enabled` sees them).
   */
  resolve(name: string, scopes: readonly string[] = [], args?: readonly string[]): Command | undefined {
    const matching = this.list().filter(
      (command) =>
        (command.enabled?.(args) ?? true) && (command.id === name || (command.aliases?.includes(name) ?? false)),
    );
    for (const scope of scopes) {
      const scoped = matching.find((command) => command.scope === scope);
      if (scoped !== undefined) {
        return scoped;
      }
    }
    return matching.find((command) => command.scope === undefined);
  }

  /** Runs `name` in `scopes`; `false` when no enabled command answers to it. */
  run(name: string, args: readonly string[] = [], scopes: readonly string[] = []): boolean {
    const command = this.resolve(name, scopes, args);
    if (command === undefined) {
      return false;
    }
    void command.run(args);
    return true;
  }

  /** Calls `listener` whenever the set of commands changes. */
  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  #changed(): void {
    for (const listener of [...this.#listeners]) {
      listener();
    }
  }
}
