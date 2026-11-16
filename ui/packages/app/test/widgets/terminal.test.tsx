/** L4: `omfp:terminal` (TRM-105, TRM-107): the command its PTY runs, from its `command` and `shell` props. */

import { afterEach, describe, expect, test } from "bun:test";
import type { JsonValue } from "@omfp/core";
import { cleanup } from "@testing-library/react";
import { type WidgetInfoScope, WidgetScope } from "../../src/sdk/index.ts";
import { BUILTIN_WIDGETS } from "../../src/view/registry.ts";
import { DEFAULT_TERMINAL_COMMAND, TerminalWidget, terminalCommand } from "../../src/widgets/terminal/index.tsx";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

const WS = "01W";
const SCOPE: WidgetInfoScope = { instance: "terminal", reference: "omfp:terminal", workspaceId: WS };

/** The params of the one `terminals.start` call made by an `omfp:terminal` mounted with `props`. */
function started(props: { [name: string]: JsonValue }): unknown {
  const host = testHost();
  renderWithHost(
    <WidgetScope value={SCOPE}>
      <TerminalWidget {...props} />
    </WidgetScope>,
    host,
  );
  host.client.views(WS);
  host.flush();
  const starts = host.client.callsTo("terminals.start");
  expect(starts.length).toBe(1);
  return starts[0]?.params;
}

/** The `terminals.start` params of a default-sized terminal running `command`. */
function start(command: string): { [name: string]: JsonValue } {
  return { workspaceId: WS, command, cols: 80, rows: 24 };
}

/** What `sh -c <command>` prints when it reads `script` on its standard input, under `env`. */
function runShell(command: string, script: string, env: Record<string, string>): string {
  const run = Bun.spawnSync(["sh", "-c", command], {
    stdin: new TextEncoder().encode(script),
    env: { PATH: process.env.PATH ?? "/usr/bin:/bin", ...env },
  });
  return run.stdout.toString();
}

describe("omfp:terminal", () => {
  // spec: TRM-105, TRM-107
  test("without props starts the daemon's $SHELL, or sh when it is empty or unset", () => {
    expect(BUILTIN_WIDGETS.get("omfp:terminal")).toBe(TerminalWidget);
    expect(started({})).toEqual(start(DEFAULT_TERMINAL_COMMAND));
    expect(DEFAULT_TERMINAL_COMMAND).toBe(`exec "\${SHELL:-sh}"`);
    // The daemon runs `sh -c <command>` (TRM-100): the default really starts a shell reading its input. `SHELL` is
    // always set here, as bash (a common `sh`) fills an unset one in from the user's login shell.
    const sh = Bun.which("sh") ?? "/bin/sh";
    expect(runShell(DEFAULT_TERMINAL_COMMAND, "echo shell:$0:$((6 * 7))\n", { SHELL: sh })).toBe(`shell:${sh}:42\n`);
    expect(runShell(DEFAULT_TERMINAL_COMMAND, "echo shell:$0:$((6 * 7))\n", { SHELL: "" })).toBe("shell:sh:42\n");
  });

  // spec: TRM-107
  test("the shell prop picks the program and arguments, each passed literally", () => {
    expect(started({ shell: "zsh" })).toEqual(start("exec 'zsh'"));
    expect(started({ shell: " bash  -l " })).toEqual(start("exec 'bash' '-l'"));
    expect(terminalCommand({ shell: "/run/current-system/sw/bin/fish" })).toBe(
      "exec '/run/current-system/sw/bin/fish'",
    );
    expect(terminalCommand({ shell: "it's $HOME;x" })).toBe(String.raw`exec 'it'\''s' '$HOME;x'`);
    expect(runShell(terminalCommand({ shell: "sh -s" }), "echo args:$#:$0\n", {})).toBe("args:0:sh\n");
    for (const shell of ["", "   ", 3]) {
      expect(terminalCommand({ shell })).toBe(DEFAULT_TERMINAL_COMMAND);
    }
  });

  // spec: TRM-107
  test("an explicit command runs unchanged and wins over the shell prop", () => {
    expect(started({ command: "htop" })).toEqual(start("htop"));
    expect(started({ command: "lazygit", shell: "zsh", cwd: "repo" })).toEqual({ ...start("lazygit"), cwd: "repo" });
    expect(terminalCommand({ command: "", shell: "zsh" })).toBe("exec 'zsh'");
  });
});
