/** L1: the phone's commands sheet's content (UI-462, UI-463). */

import { describe, expect, test } from "bun:test";
import { sheetGroups } from "../../src/frame/commands-sheet.ts";
import type { Command } from "../../src/sdk/index.ts";

const run = () => {};

const COMMANDS: Command[] = [
  { id: "thread.new", title: "New thread", aliases: ["new"], run },
  { id: "window.theme", title: "Colour scheme (system, light, dark)", aliases: ["theme"], args: () => ["dark"], run },
  { id: "help.open", title: "Help", aliases: ["help"], run },
  { id: "thread.archive", title: "Archive", aliases: ["archive"], enabled: () => false, run },
  { id: "composer.interrupt", title: "Interrupt", scope: "composer", run },
  // Every `useCommand` command has `args`; without completions it runs at once.
  { id: "demo.ping", title: "Ping", description: "Pings the demo", args: () => [], run },
  { id: "thread.new", title: "New thread (threads)", scope: "threads", run },
  { id: "view.switch", title: "Switch view (:view <id>)", aliases: ["view"], args: () => ["review"], run },
  { id: "view.pick", title: "Pick a view", run },
  { id: "file.open", title: "Open a file (:open <path>[:<line>]), or the focused path", aliases: ["open"], run },
  // What a phone does otherwise: the Threads button, a tap on the composer, the focus moves.
  { id: "threads.toggle", title: "Show or hide the threads column (narrow window)", run },
  { id: "composer.focus", title: "Focus the composer", run },
  { id: "notice.open", title: "Open the newest notice", run },
  { id: "nav.next", title: "Next thread", scope: "threads", run },
];

/** `[group, [id, name, takes arguments]…]` per group. */
function shape(scopes: readonly string[], query = "") {
  return sheetGroups(COMMANDS, scopes, query).map((group) => [
    group.group,
    group.commands.map((command) => [command.id, command.name, command.takesArgs]),
  ]);
}

describe("commands sheet content", () => {
  // spec: UI-462
  test("what the `:` line runs where the focus was, less what a phone does otherwise, in the which-key groups by title", () => {
    expect(shape([])).toEqual([
      ["threads", [["thread.new", "new", false]]],
      [
        "views",
        [
          ["view.pick", "view.pick", false],
          ["view.switch", "view", true],
        ],
      ],
      ["files", [["file.open", "open", false]]],
      ["help", [["help.open", "help", false]]],
      ["commands", [["window.theme", "theme", true]]],
      ["other", [["demo.ping", "demo.ping", false]]],
    ]);
    expect(shape(["composer", "threads"])[0]).toEqual([
      "threads",
      [
        ["composer.interrupt", "composer.interrupt", false],
        ["thread.new", "new", false],
      ],
    ]);
  });

  // spec: UI-463
  test("the search looks in the ids, aliases, whole titles and descriptions, any case", () => {
    expect(shape([], "SCHEME")).toEqual([["commands", [["window.theme", "theme", true]]]]);
    expect(shape([], "pings")).toEqual([["other", [["demo.ping", "demo.ping", false]]]]);
    expect(shape([], "help.op")).toEqual([["help", [["help.open", "help", false]]]]);
    expect(shape([], "<id>")).toEqual([["views", [["view.switch", "view", true]]]]);
    expect(shape([], "  workspace ")).toEqual([["threads", [["thread.new", "new", false]]]]);
    expect(shape([], "nothing like it")).toEqual([]);
  });
});
