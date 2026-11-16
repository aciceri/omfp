/** L1/L4: the keymap as data, its sequence matcher and the global key handler (UI-095). */

import { afterEach, describe, expect, test } from "bun:test";
import { CMDLINE_ATTRIBUTE, createKeyHandler, modeAt } from "../../src/keymap/handler.ts";
import {
  type Binding,
  DEFAULT_BINDINGS,
  describeKeys,
  effectiveBindings,
  KeymapMatcher,
  keyLabel,
  keysFor,
  keyToken,
  SEQUENCE_TIMEOUT_MS,
} from "../../src/keymap/keymap.ts";
import { CommandRegistry, SCOPE_ATTRIBUTE } from "../../src/sdk/index.ts";

function key(
  key: string,
  modifiers: Partial<Record<"ctrlKey" | "altKey" | "metaKey" | "shiftKey", boolean>> = {},
  code = "",
) {
  return { key, code, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, ...modifiers };
}

/** A clock the test moves. */
function clock() {
  let now = 0;
  return {
    now: () => now,
    advance: (ms: number) => {
      now += ms;
    },
  };
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("keymap", () => {
  // spec: UI-095, UI-760
  test("key presses become tokens with modifiers; printable keys keep their character", () => {
    expect(keyToken(key("j"))).toBe("j");
    expect(keyToken(key("G", { shiftKey: true }))).toBe("G");
    expect(keyToken(key(":", { shiftKey: true }))).toBe(":");
    expect(keyToken(key("d", { ctrlKey: true }))).toBe("Ctrl-d");
    // Caps Lock leaves a Ctrl chord as it is; Shift makes another one (`Ctrl-Shift-a` selects all, UI-760).
    expect(keyToken(key("D", { ctrlKey: true }))).toBe("Ctrl-d");
    expect(keyToken(key("A", { ctrlKey: true, shiftKey: true }))).toBe("Ctrl-Shift-a");
    expect(keyToken(key("1", { ctrlKey: true, shiftKey: true }))).toBe("Ctrl-1");
    expect(keyToken(key(" "))).toBe("Space");
    expect(keyToken(key("Escape"))).toBe("Esc");
    expect(keyToken(key("Tab", { shiftKey: true }))).toBe("Shift-Tab");
    expect(keyToken(key("a", { altKey: true }))).toBe("Alt-a");
    expect(keyToken(key("Shift", { shiftKey: true }))).toBeUndefined();
  });

  // spec: UI-021
  test("Alt chords match macOS's Option keys, which compose a character, by their physical key", () => {
    expect(keyToken(key("å", { altKey: true }, "KeyA"))).toBe("Alt-a");
    expect(keyToken(key("Dead", { altKey: true }, "KeyN"))).toBe("Alt-n");
    expect(keyToken(key("Å", { altKey: true, shiftKey: true }, "KeyA"))).toBe("Alt-a");
    expect(keyToken(key("¡", { altKey: true }, "Digit1"))).toBe("Alt-1");
    // An ASCII key stays as typed (Linux, other layouts), and a dead key without a letter or digit is no token.
    expect(keyToken(key("q", { altKey: true }, "KeyA"))).toBe("Alt-q");
    expect(keyToken(key("Dead", { altKey: true }, "Backquote"))).toBeUndefined();
    // Without Alt a composed character or a dead key is left as it is.
    expect(keyToken(key("å", {}, "KeyA"))).toBe("å");
    expect(keyToken(key("Dead", {}, "KeyN"))).toBeUndefined();
  });

  // spec: UI-148
  test("key chips label keys as lowercase words, letters as typed, sequences with a space, never symbol glyphs", () => {
    expect(keyLabel("Space n")).toBe("spc n");
    expect(keyLabel("Esc")).toBe("esc");
    expect(keyLabel("Escape")).toBe("esc");
    expect(keyLabel("Enter")).toBe("enter");
    expect(keyLabel("Tab")).toBe("tab");
    expect(keyLabel("Shift-Tab")).toBe("shift+tab");
    expect(keyLabel("Alt-Enter")).toBe("alt+enter");
    expect(keyLabel("Ctrl-c")).toBe("ctrl+c");
    expect(keyLabel("Ctrl-Shift-ArrowUp")).toBe("ctrl+shift+up");
    expect(keyLabel("ArrowDown")).toBe("down");
    expect(keyLabel("PageDown")).toBe("pagedown");
    expect(keyLabel("Meta-k")).toBe("meta+k");
    expect(keyLabel("G")).toBe("G");
    expect(keyLabel("g g")).toBe("g g");
    expect(keyLabel("Space :")).toBe("spc :");
    expect(keyLabel("Ctrl--")).toBe("ctrl+-");
    expect(keyLabel("-")).toBe("-");
  });

  // spec: UI-280
  test("on macOS Alt chords are labelled opt, elsewhere alt", () => {
    expect(keyLabel("Alt-a", true)).toBe("opt+a");
    expect(keyLabel("Ctrl-Alt-Enter", true)).toBe("ctrl+opt+enter");
    expect(keyLabel("Space n", true)).toBe("spc n");
    expect(keyLabel("Alt-a", false)).toBe("alt+a");
    // The test DOM is not macOS: the default is the page's platform.
    expect(keyLabel("Alt-a")).toBe("alt+a");
  });

  // spec: UI-624
  test("Alt-arrows move the focused row or section of the column; with Shift a thread into another workspace", () => {
    expect(keyToken(key("ArrowUp", { altKey: true }, "ArrowUp"))).toBe("Alt-ArrowUp");
    expect(keyToken(key("ArrowDown", { altKey: true, shiftKey: true }, "ArrowDown"))).toBe("Alt-Shift-ArrowDown");
    const moves = ["threads.moveUp", "threads.moveDown", "threads.moveToPrevWorkspace", "threads.moveToNextWorkspace"];
    expect(moves.map((command) => keysFor(DEFAULT_BINDINGS, command, "normal"))).toEqual([
      ["Alt-ArrowUp"],
      ["Alt-ArrowDown"],
      ["Alt-Shift-ArrowUp"],
      ["Alt-Shift-ArrowDown"],
    ]);
  });

  // spec: UI-095, UI-021
  test("sequences complete, wait, or start over on a key that does not continue them", () => {
    const time = clock();
    const matcher = new KeymapMatcher(DEFAULT_BINDINGS, time.now);
    expect(matcher.feed("g", "normal")).toEqual({ kind: "pending", keys: "g" });
    expect(matcher.pending).toBe("g");
    const top = matcher.feed("g", "normal");
    expect(top.kind === "command" && top.binding.command).toBe("nav.top");
    expect(matcher.pending).toBeUndefined();
    expect(matcher.feed("Space", "normal").kind).toBe("pending");
    const next = matcher.feed("a", "normal");
    expect(next.kind === "command" && next.binding.command).toBe("thread.nextAttention");
    matcher.feed("Space", "normal");
    const restart = matcher.feed("j", "normal");
    expect(restart.kind === "command" && restart.binding.command).toBe("nav.next");
    expect(matcher.feed("q", "normal")).toEqual({ kind: "none" });
    expect(matcher.feed("j", "insert")).toEqual({ kind: "none" });
  });

  // spec: UI-095
  test("a sequence whose next key comes too late is dropped", () => {
    const time = clock();
    const matcher = new KeymapMatcher(DEFAULT_BINDINGS, time.now);
    matcher.feed("g", "normal");
    time.advance(SEQUENCE_TIMEOUT_MS + 1);
    expect(matcher.pending).toBeUndefined();
    expect(matcher.feed("g", "normal")).toEqual({ kind: "pending", keys: "g" });
    time.advance(SEQUENCE_TIMEOUT_MS);
    const top = matcher.feed("g", "normal");
    expect(top.kind === "command" && top.binding.command).toBe("nav.top");
  });

  // spec: UI-095
  test("the focused element decides the mode", () => {
    document.body.innerHTML = `
      <input id="text"><input id="box" type="checkbox"><textarea id="area"></textarea>
      <div id="edit" contenteditable="true"></div><button id="button">b</button>
      <input id="cmd" ${CMDLINE_ATTRIBUTE}>`;
    const mode = (id: string) => modeAt(document.getElementById(id));
    expect([mode("text"), mode("box"), mode("area"), mode("button"), mode("cmd")]).toEqual([
      "insert",
      "normal",
      "insert",
      "normal",
      "cmdline",
    ]);
    expect(modeAt(null)).toBe("normal");
  });
});

describe("global key handler", () => {
  function setup(bindings: readonly Binding[] = DEFAULT_BINDINGS) {
    const commands = new CommandRegistry();
    const ran: string[] = [];
    const register = (id: string, scope?: string, enabled?: () => boolean) =>
      commands.register({
        id,
        title: id,
        ...(scope === undefined ? {} : { scope }),
        ...(enabled === undefined ? {} : { enabled }),
        run: (args) => {
          ran.push(`${scope ?? "global"}:${id}${args.length > 0 ? ` ${args.join(" ")}` : ""}`);
        },
      });
    const handler = createKeyHandler({ bindings, commands, now: clock().now });
    const press = (target: Element, name: string, modifiers: Parameters<typeof key>[1] = {}, code = "") => {
      if (target instanceof HTMLElement) {
        target.focus();
      }
      const event = new KeyboardEvent("keydown", { ...key(name, modifiers, code), bubbles: true, cancelable: true });
      handler(event);
      return event.defaultPrevented;
    };
    return { register, ran, press };
  }

  // spec: UI-095
  test("keys run the command of the innermost scope around the focus, else the global one", () => {
    document.body.innerHTML = `<div ${SCOPE_ATTRIBUTE}="threads"><button id="row">t</button></div><p id="out" tabindex="-1"></p>`;
    const { register, ran, press } = setup();
    register("nav.next");
    register("nav.next", "threads");
    const row = document.getElementById("row");
    const out = document.getElementById("out");
    if (row === null || out === null) {
      throw new Error("fixture");
    }
    expect(press(row, "j")).toBe(true);
    expect(press(out, "j")).toBe(true);
    expect(ran).toEqual(["threads:nav.next", "global:nav.next"]);
    expect(press(out, "k")).toBe(false);
  });

  // spec: UI-095, UI-021
  test("single keys do nothing while typing; chords and Esc still work", () => {
    document.body.innerHTML = `<input id="field">`;
    const { register, ran, press } = setup();
    register("nav.next");
    register("thread.nextAttention");
    const field = document.getElementById("field");
    if (field === null) {
      throw new Error("fixture");
    }
    expect(press(field, "j")).toBe(false);
    expect(press(field, "a", { altKey: true })).toBe(true);
    expect(ran).toEqual(["global:thread.nextAttention"]);
    // Without a `focus.leave` command Esc just leaves the field.
    expect(document.activeElement === field).toBe(true);
    expect(press(field, "Escape")).toBe(true);
    expect(document.activeElement === field).toBe(false);
  });

  // spec: UI-021, UI-405
  test("macOS's Option keys run the attention and new-thread chords in normal mode, and type their text while typing", () => {
    document.body.innerHTML = `<input id="field"><p id="out" tabindex="-1"></p>`;
    const { register, ran, press } = setup();
    register("thread.nextAttention");
    register("thread.new");
    const field = document.getElementById("field");
    const out = document.getElementById("out");
    if (field === null || out === null) {
      throw new Error("fixture");
    }
    expect(press(out, "å", { altKey: true }, "KeyA")).toBe(true);
    expect(press(out, "Dead", { altKey: true }, "KeyN")).toBe(true);
    expect(ran).toEqual(["global:thread.nextAttention", "global:thread.new"]);
    // In a field Option composes `å`, `ñ`, `õ`: the keys are the text's, not the chords'.
    expect(press(field, "å", { altKey: true }, "KeyA")).toBe(false);
    expect(press(field, "Dead", { altKey: true }, "KeyN")).toBe(false);
    expect(ran.length).toBe(2);
    // Alt keeps the chord where it leaves the key as typed (Linux, Windows).
    expect(press(field, "a", { altKey: true }, "KeyA")).toBe(true);
    expect(ran.length).toBe(3);
  });

  // spec: UI-063, UI-095, UI-368
  test("Ctrl-c runs the composer's command while typing in the composer, else in normal mode thread.abort", () => {
    document.body.innerHTML = `<form ${SCOPE_ATTRIBUTE}="composer"><textarea id="draft"></textarea><button id="send">s</button></form><input id="filter"><p id="out" tabindex="-1"></p>`;
    const { register, ran, press } = setup();
    register("composer.interrupt", "composer");
    register("thread.abort");
    const element = (id: string) => {
      const found = document.getElementById(id);
      if (found === null) {
        throw new Error("fixture");
      }
      return found;
    };
    expect(press(element("draft"), "c", { ctrlKey: true })).toBe(true);
    expect(press(element("send"), "c", { ctrlKey: true })).toBe(true);
    expect(press(element("filter"), "c", { ctrlKey: true })).toBe(false);
    expect(press(element("out"), "c", { ctrlKey: true })).toBe(true);
    expect(ran).toEqual(["composer:composer.interrupt", "global:thread.abort", "global:thread.abort"]);
  });

  // spec: UI-368
  test("Ctrl-c in normal mode copies a text selection and does nothing while nothing can be aborted", () => {
    document.body.innerHTML = `<p id="out" tabindex="-1">some reply text</p>`;
    const { register, ran, press } = setup();
    let busy = true;
    register("thread.abort", undefined, () => busy);
    const out = document.getElementById("out");
    const selection = document.getSelection();
    if (out === null || selection === null) {
      throw new Error("fixture");
    }
    const range = document.createRange();
    range.selectNodeContents(out);
    // A range left by an earlier test would keep this one from being added.
    selection.removeAllRanges();
    selection.addRange(range);
    // A selection: the browser copies it, nothing runs.
    expect(press(out, "c", { ctrlKey: true })).toBe(false);
    expect(ran).toEqual([]);
    selection.removeAllRanges();
    expect(press(out, "c", { ctrlKey: true })).toBe(true);
    expect(ran).toEqual(["global:thread.abort"]);
    // Idle: the command is disabled and the key is left to the page.
    busy = false;
    expect(press(out, "c", { ctrlKey: true })).toBe(false);
    expect(ran).toEqual(["global:thread.abort"]);
  });

  // spec: UI-095, UI-092
  test("no binding applies inside a modal dialog; checkboxes keep their Space", () => {
    document.body.innerHTML = `<div role="dialog"><button id="in">x</button></div><input id="box" type="checkbox">`;
    const { register, ran, press } = setup();
    register("nav.next");
    register("thread.new");
    const inside = document.getElementById("in");
    const box = document.getElementById("box");
    if (inside === null || box === null) {
      throw new Error("fixture");
    }
    expect(press(inside, "j")).toBe(false);
    expect(press(box, " ")).toBe(false);
    expect(press(box, "n")).toBe(false);
    expect(ran).toEqual([]);
  });

  // spec: UI-095
  test("the leader waits for its next key and prevents the page's own Space", () => {
    document.body.innerHTML = `<p id="out" tabindex="-1"></p>`;
    const { register, ran, press } = setup([
      { mode: "normal", keys: "Space t", command: "window.theme", args: ["dark"] },
    ]);
    register("window.theme");
    const out = document.getElementById("out");
    if (out === null) {
      throw new Error("fixture");
    }
    expect(press(out, " ")).toBe(true);
    expect(press(out, "t")).toBe(true);
    expect(ran).toEqual(["global:window.theme dark"]);
  });
});

describe("init layers", () => {
  // spec: UI-262
  test("the init files' bindings apply after the workspace's, the workspace's file last; each carries its layer", () => {
    const defaults: Binding[] = [
      { mode: "normal", keys: "g h", command: "help.open" },
      { mode: "normal", keys: "j", command: "nav.next" },
    ];
    const merged = effectiveBindings(
      defaults,
      [{ mode: "normal", keys: "j", command: "nav.prev" }],
      [{ mode: "normal", keys: "g h", command: "demo.ping" }],
      {
        user: [
          { mode: "normal", keys: "g h", command: "my.hello", args: ["user"] },
          { mode: "normal", keys: "Space h", command: "my.hello" },
        ],
        workspace: [
          { mode: "normal", keys: "Space h", command: "my.other" },
          { mode: "normal", keys: "j", command: "" },
        ],
      },
    );
    expect(merged).toEqual([
      { mode: "normal", keys: "g h", command: "my.hello", args: ["user"], source: "init:user" },
      { mode: "normal", keys: "Space h", command: "my.other", source: "init:workspace" },
    ]);
  });

  // spec: UI-264
  test("described keys wait while they start a longer binding and answer with every mode's binding", () => {
    const bindings: Binding[] = [
      { mode: "normal", keys: "Ctrl-c", command: "composer.interrupt" },
      { mode: "insert", keys: "Ctrl-c", command: "composer.interrupt", source: "config" },
      { mode: "normal", keys: "Space n", command: "thread.new" },
    ];
    expect(describeKeys(bindings, "Space")).toEqual({ kind: "pending" });
    expect(describeKeys(bindings, "Space n")).toEqual({ kind: "done", keys: "Space n", bindings: bindings.slice(2) });
    expect(describeKeys(bindings, "Ctrl-c")).toEqual({ kind: "done", keys: "Ctrl-c", bindings: bindings.slice(0, 2) });
    expect(describeKeys(bindings, "q")).toEqual({ kind: "done", keys: "q", bindings: [] });
  });
});

describe("former command ids", () => {
  // spec: UI-527
  test("config, workspace and init entries naming a former built-in id bind its current id", () => {
    const merged = effectiveBindings(
      [],
      [{ mode: "normal", keys: "Space t", command: "theme.set", args: ["dark"] }],
      [{ mode: "normal", keys: "Space w", command: "widget", args: ["add"] }],
      { user: [{ mode: "normal", keys: "Space u", command: "theme.set" }], workspace: [] },
    );
    expect(merged.map((binding) => [binding.keys, binding.command, binding.source])).toEqual([
      ["Space t", "window.theme", "config"],
      ["Space w", "widget.manage", "workspace"],
      ["Space u", "window.theme", "init:user"],
    ]);
  });
});
