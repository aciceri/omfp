/**
 * L4: `crates/omfp-daemon/src/widgets/keymap-defaults.json`, which the daemon serves to agents as `omfp://keymap`
 * (UI-305), is the default keymap and the commands a window showing the default view with a thread registers. Run
 * with `OMFP_UPDATE_KEYMAP=1` to write it again after changing a binding or a command.
 */

import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { act, cleanup } from "@testing-library/react";
import { Frame } from "../../src/frame/frame.tsx";
import { DEFAULT_BINDINGS } from "../../src/keymap/keymap.ts";
import { SELECTED_THREAD } from "../../src/sdk/index.ts";
import { BUILTIN_WIDGETS } from "../../src/view/registry.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

const FILE = join(import.meta.dir, "../../../../../crates/omfp-daemon/src/widgets/keymap-defaults.json");

/** The keymap and commands as the daemon's file holds them. */
function exported(): string {
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: new ManualTimers(),
    noticeTimers: new ManualTimers(),
  });
  base.client.answers.set("widgets.report", () => ({}));
  const test = renderWithHost(<Frame registry={BUILTIN_WIDGETS} now={() => 0} />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "checkout", "w1")] });
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  test.client.thread("t1", []);
  test.client.views("w1", {});
  test.flush();
  const commands = [...test.host.commands.list()]
    .sort((a, b) => (a.id === b.id ? (a.scope ?? "").localeCompare(b.scope ?? "") : a.id.localeCompare(b.id)))
    .map((command) => ({
      id: command.id,
      title: command.title,
      aliases: command.aliases ?? [],
      ...(command.description === undefined ? {} : { description: command.description }),
      ...(command.scope === undefined ? {} : { scope: command.scope }),
    }));
  const bindings = DEFAULT_BINDINGS.map(({ mode, keys, command, args }) => ({
    mode,
    keys,
    command,
    ...(args === undefined ? {} : { args }),
  }));
  return `${JSON.stringify({ bindings, commands }, null, 2)}\n`;
}

describe("the keymap agents read", () => {
  // spec: UI-305
  test("keymap-defaults.json is the default keymap and the commands of a window", () => {
    const current = exported();
    if (process.env.OMFP_UPDATE_KEYMAP === "1") {
      writeFileSync(FILE, current);
    }
    expect(readFileSync(FILE, "utf8")).toBe(current);
    // Every default binding runs one of the listed commands.
    const parsed = JSON.parse(current) as { bindings: { command: string }[]; commands: { id: string }[] };
    const ids = new Set(parsed.commands.map((command) => command.id));
    expect(parsed.bindings.filter((binding) => !ids.has(binding.command)).map((binding) => binding.command)).toEqual(
      [],
    );
  });

  // spec: UI-525, UI-526, UI-529
  test("every built-in command id is a dotted `<area>.<verb>`; the short names stay aliases", () => {
    const { commands } = JSON.parse(exported()) as { commands: { id: string; aliases: string[] }[] };
    expect(commands.filter((command) => !/^[a-z]+\.[A-Za-z]+$/.test(command.id)).map((command) => command.id)).toEqual(
      [],
    );
    const byId = new Map(commands.map((command) => [command.id, command.aliases]));
    expect([byId.get("window.theme"), byId.get("widget.manage"), byId.has("theme.set")]).toEqual([
      ["theme"],
      ["widget"],
      false,
    ]);
  });
});
