/** `omfp:terminal`: a copyable xterm.js view over the daemon's reusable PTY session (`useTerminal`). */

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

export function TerminalWidget(props: WidgetProps): ReactNode {
  const command = stringProp(props, "command") ?? String.raw`\${SHELL:-sh}`;
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
