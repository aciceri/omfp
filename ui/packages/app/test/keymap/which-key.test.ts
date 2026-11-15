/** L1/L4: the which-key model and the key handler's waiting sequences (UI-320..UI-326). */

import { afterEach, describe, expect, test } from "bun:test";
import { createKeyHandler } from "../../src/keymap/handler.ts";
import { type Binding, DEFAULT_BINDINGS, KeymapMatcher, SEQUENCE_TIMEOUT_MS } from "../../src/keymap/keymap.ts";
import {
  commandGroup,
  type PendingKeys,
  WHICH_KEY_DELAY_MS,
  type WhichKeySection,
  whichKeySections,
} from "../../src/keymap/which-key.ts";
import { CommandRegistry } from "../../src/sdk/index.ts";

afterEach(() => {
  document.body.innerHTML = "";
});

const titleOf = (binding: Binding) => ({ title: binding.command, available: true });

/** `[group, key, what]`: the command id, or `prefix:<group | count>`. */
function flat(sections: readonly WhichKeySection[]): string[][] {
  return sections.flatMap((section) =>
    section.entries.map((entry) => [
      section.group,
      entry.key,
      entry.kind === "command" ? entry.binding.command : `prefix:${entry.group ?? entry.count}`,
    ]),
  );
}

describe("which-key model", () => {
  // spec: UI-320
  test("the keys completing a prefix, by group, in binding order; only the prefix's mode", () => {
    const insert: Binding[] = [
      { mode: "insert", keys: "Ctrl-x Ctrl-s", command: "my.save", source: "init:user" },
      { mode: "normal", keys: "Ctrl-x Ctrl-f", command: "file.open" },
    ];
    expect(flat(whichKeySections(insert, { keys: "Ctrl-x", mode: "insert" }, titleOf))).toEqual([
      ["other", "Ctrl-s", "my.save"],
    ]);
  });

  // spec: UI-320, UI-326
  test("a key starting longer bindings is a group: the leader's, theirs in common, else how many", () => {
    const bindings: Binding[] = [
      { mode: "normal", keys: "Space t", command: "threads.toggle" },
      { mode: "normal", keys: "Space o a", command: "my.a", source: "init:user" },
      { mode: "normal", keys: "Space o b", command: "thread.new", source: "init:user" },
      { mode: "normal", keys: "Space T x", command: "thread.stop", source: "config" },
      { mode: "normal", keys: "Space T y", command: "thread.restart", source: "config" },
      // A complete binding wins over longer ones, like the matcher.
      { mode: "normal", keys: "Space t x", command: "my.never", source: "config" },
    ];
    const prefixes = flat(whichKeySections(bindings, { keys: "Space", mode: "normal" }, titleOf)).filter(([, , what]) =>
      what?.startsWith("prefix:"),
    );
    expect(prefixes).toEqual([
      ["threads", "T", "prefix:threads"],
      ["other", "o", "prefix:2"],
    ]);
    const sections = whichKeySections(bindings, { keys: "Space", mode: "normal" }, titleOf);
    const toggle = sections.flatMap((section) => section.entries).find((entry) => entry.key === "t");
    expect(toggle?.kind === "command" && toggle.binding.command).toBe("threads.toggle");
  });

  // spec: UI-324
  test("command groups come from the id's namespace", () => {
    expect(
      ["thread.archive", "threads.toggle", "view.pick", "workspace.rescan", "file.open", "help.describeKey"].map(
        commandGroup,
      ),
    ).toEqual(["threads", "views", "views", "workspace", "files", "help"]);
    expect([commandGroup("nav.top"), commandGroup("cmdline.open"), commandGroup("tests.run")]).toEqual([
      "navigation",
      "commands",
      "other",
    ]);
  });
});

describe("waiting sequences in the key handler", () => {
  function setup(bindings: readonly Binding[] = DEFAULT_BINDINGS) {
    let now = 0;
    const timers: Array<{ run: () => void; ms: number }> = [];
    const shown: Array<string | undefined> = [];
    const commands = new CommandRegistry();
    const ran: string[] = [];
    for (const id of ["nav.top", "file.open", "cmdline.open", "help.open"]) {
      commands.register({
        id,
        title: id,
        run: () => {
          ran.push(id);
        },
      });
    }
    const handler = createKeyHandler({
      bindings,
      commands,
      now: () => now,
      whichKey: {
        timers: { setTimeout: (run, ms) => timers.push({ run, ms }) },
        show: (pending: PendingKeys | undefined) => shown.push(pending?.keys),
      },
    });
    document.body.innerHTML = `<p id="out" tabindex="-1"></p>`;
    const out = document.getElementById("out");
    out?.focus();
    const press = (key: string) => {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      handler(event);
      return event.defaultPrevented;
    };
    const fire = () => {
      for (const timer of timers.splice(0)) {
        timer.run();
      }
    };
    return {
      press,
      fire,
      timers,
      shown,
      ran,
      handler,
      advance: (ms: number) => {
        now += ms;
      },
    };
  }

  // spec: UI-321
  test("a prefix shows after the delay; the next key in time never shows it", () => {
    const keys = setup([
      { mode: "normal", keys: "q a", command: "nav.top" },
      { mode: "normal", keys: "q b", command: "file.open" },
    ]);
    keys.press("q");
    expect(keys.timers.map((timer) => timer.ms)).toEqual([WHICH_KEY_DELAY_MS]);
    keys.press("a");
    keys.fire();
    expect(keys.shown).toEqual([]);
    expect(keys.ran).toEqual(["nav.top"]);
    keys.press("q");
    keys.fire();
    expect(keys.shown).toEqual(["q"]);
    keys.press("b");
    expect(keys.shown).toEqual(["q", undefined]);
    expect(keys.ran).toEqual(["nav.top", "file.open"]);
  });

  // spec: UI-321
  test("a shown sequence waits past the sequence timeout and updates as it grows", () => {
    const keys = setup();
    keys.press(" ");
    expect(keys.shown).toEqual(["Space"]);
    keys.advance(SEQUENCE_TIMEOUT_MS * 5);
    keys.press("h");
    expect(keys.shown).toEqual(["Space", "Space h"]);
    expect(keys.timers).toEqual([]);
    keys.advance(SEQUENCE_TIMEOUT_MS * 5);
    keys.press("h");
    expect(keys.ran).toEqual(["help.open"]);
    expect(keys.shown).toEqual(["Space", "Space h", undefined]);
  });

  // spec: UI-322
  test("Escape cancels the sequence and does nothing else; cancel() does the same", () => {
    const keys = setup();
    keys.press("g");
    keys.fire();
    expect(keys.press("Escape")).toBe(true);
    expect(keys.shown).toEqual(["g", undefined]);
    // `g` starts again: no `g g`.
    keys.press("g");
    expect(keys.ran).toEqual([]);
    keys.handler.cancel();
    keys.fire();
    expect(keys.shown).toEqual(["g", undefined]);
    keys.press("g");
    expect(keys.ran).toEqual([]);
    // Without a waiting sequence Escape is left to the page.
    keys.handler.cancel();
    expect(keys.press("Escape")).toBe(false);
  });

  // spec: UI-324, UI-325
  test("Space opens the leader menu at once; Space Space runs the command line", () => {
    const keys = setup();
    expect(keys.press(" ")).toBe(true);
    expect(keys.shown).toEqual(["Space"]);
    keys.press(" ");
    expect(keys.ran).toEqual(["cmdline.open"]);
    expect(keys.shown).toEqual(["Space", undefined]);
  });

  // spec: UI-321
  test("a held sequence is the matcher's until it ends", () => {
    let now = 0;
    const matcher = new KeymapMatcher(DEFAULT_BINDINGS, () => now);
    matcher.feed("g", "normal");
    matcher.hold();
    now += SEQUENCE_TIMEOUT_MS * 10;
    expect(matcher.pending).toBe("g");
    matcher.feed("g", "normal");
    matcher.feed("g", "normal");
    now += SEQUENCE_TIMEOUT_MS + 1;
    expect(matcher.pending).toBeUndefined();
  });
});
