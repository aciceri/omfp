/**
 * `omfp:terminal`: a copyable xterm.js view over the daemon's reusable PTY session (`useTerminal`).
 * It runs `command` when given; otherwise it execs the `shell` prop or the daemon's `$SHELL`, `sh` when unset (TRM-107).
 */

import { TerminalEmulator } from "omfp/kit";
import { useTerminal, type WidgetProps } from "omfp/sdk";
import { type ReactNode, useRef } from "react";

function stringProp(props: WidgetProps, name: string): string | undefined {
  const value = props[name];
  return typeof value === "string" ? value : undefined;
}

function numberProp(props: WidgetProps, name: string, fallback: number): number {
  const value = props[name];
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? Math.floor(value) : fallback;
}

/** The daemon environment's login shell, `sh` when `$SHELL` is unset or empty; `sh -c` expands it on the daemon. */
export const DEFAULT_TERMINAL_COMMAND = `exec "\${SHELL:-sh}"`;

/** One shell word, single-quoted so `sh -c` passes it literally. */
function quoteWord(word: string): string {
  return `'${word.replaceAll("'", String.raw`'\''`)}'`;
}

/**
 * The command a terminal runs: a non-empty `command` prop as given, else `exec` of the `shell` prop's
 * whitespace-separated words (a program and its arguments, each passed literally), else the daemon's `$SHELL`.
 */
export function terminalCommand(props: WidgetProps): string {
  const command = stringProp(props, "command");
  if (command !== undefined && command !== "") {
    return command;
  }
  const words =
    stringProp(props, "shell")
      ?.split(/\s+/)
      .filter((word) => word !== "") ?? [];
  return words.length === 0 ? DEFAULT_TERMINAL_COMMAND : `exec ${words.map(quoteWord).join(" ")}`;
}

export function TerminalWidget(props: WidgetProps): ReactNode {
  const command = terminalCommand(props);
  const cwd = stringProp(props, "cwd");
  const key = stringProp(props, "key");
  const cols = numberProp(props, "cols", 80);
  const rows = numberProp(props, "rows", 24);
  const session = useTerminal(command, {
    ...(cwd === undefined ? {} : { cwd }),
    ...(key === undefined ? {} : { key }),
    cols,
    rows,
  });
  const sessionRef = useRef(session);
  sessionRef.current = session;

  return (
    <TerminalEmulator
      sessionKey={session.id}
      chunks={session.state?.scrollback ?? []}
      cols={cols}
      rows={rows}
      onData={(data) => sessionRef.current.input(data)}
      onTerminalResize={(nextCols, nextRows) => sessionRef.current.resize(nextCols, nextRows)}
      className="omfp-terminal"
      ariaLabel={command}
    />
  );
}
