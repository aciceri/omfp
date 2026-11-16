import { describe, expect, test } from "bun:test";
import { type LineEditCommand, lineEdit } from "../../src/vm/line-edit.ts";

/** `text` with `|` as the caret, or `[` `]` around the selection. */
function field(text: string) {
  const caret = text.indexOf("|");
  if (caret !== -1) {
    return { value: text.replace("|", ""), start: caret, end: caret };
  }
  const start = text.indexOf("[");
  const end = text.indexOf("]") - 1;
  return { value: text.replace("[", "").replace("]", ""), start, end };
}

/** The field after `command`, written as {@link field}; a boundary as `up` / `down`. */
function after(command: LineEditCommand, text: string, killed = ""): string {
  const state = field(text);
  const edit = lineEdit(command, state, killed);
  if (edit.kind === "boundary") return edit.direction;
  if (edit.kind === "select") {
    return edit.start === edit.end
      ? `${state.value.slice(0, edit.start)}|${state.value.slice(edit.start)}`
      : `${state.value.slice(0, edit.start)}[${state.value.slice(edit.start, edit.end)}]${state.value.slice(edit.end)}`;
  }
  const value = `${state.value.slice(0, edit.from)}${edit.insert}${state.value.slice(edit.to)}`;
  return `${value.slice(0, edit.caret)}|${value.slice(edit.caret)}`;
}

describe("readline line editing", () => {
  // spec: UI-760
  test("moves: line ends, characters, words, lines keeping the column", () => {
    expect(after("composer.lineStart", "one\ntw|o")).toBe("one\n|two");
    expect(after("composer.lineEnd", "o|ne\ntwo")).toBe("one|\ntwo");
    expect(after("composer.charBack", "ab|c")).toBe("a|bc");
    expect(after("composer.charBack", "|abc")).toBe("|abc");
    expect(after("composer.charForward", "ab|c")).toBe("abc|");
    expect(after("composer.charForward", "abc|")).toBe("abc|");
    expect(after("composer.charBack", "a😀|b")).toBe("a|😀b");
    expect(after("composer.wordBack", "foo bar.  qu|x")).toBe("foo bar.  |qux");
    expect(after("composer.wordBack", "foo bar.  |qux")).toBe("foo |bar.  qux");
    expect(after("composer.wordForward", "f|oo bar")).toBe("foo| bar");
    expect(after("composer.wordForward", "foo| bar")).toBe("foo bar|");
    expect(after("composer.linePrev", "short\nlonger li|ne")).toBe("short|\nlonger line");
    expect(after("composer.linePrev", "one\ntw|o\nthree")).toBe("on|e\ntwo\nthree");
    expect(after("composer.lineNext", "o|ne\ntwo")).toBe("one\nt|wo");
    expect(after("composer.lineNext", "one|\nx")).toBe("one\nx|");
    // The selection collapses towards the move.
    expect(after("composer.charBack", "a[bc]d")).toBe("a|bcd");
    expect(after("composer.charForward", "a[bc]d")).toBe("abc|d");
  });

  // spec: UI-760
  test("Ctrl-p on the first line and Ctrl-n on the last are the prompt history's", () => {
    expect(after("composer.linePrev", "on|e\ntwo")).toBe("up");
    expect(after("composer.lineNext", "one\nt|wo")).toBe("down");
    expect(after("composer.linePrev", "|")).toBe("up");
    // A draft starting with a line break: offset 0 is on the first line.
    expect(after("composer.linePrev", "|\nx")).toBe("up");
    expect(after("composer.lineStart", "|\nx")).toBe("|\nx");
    expect(after("composer.killLineStart", "|\nx")).toBe("|\nx");
    // A vertical move never lands inside a surrogate pair.
    expect(after("composer.lineNext", "a|\n😀x")).toBe("a\n|😀x");
  });

  // spec: UI-760
  test("deletes, kills and yanks; a selection is what goes or is replaced", () => {
    expect(after("composer.deleteForward", "a|bc")).toBe("a|c");
    expect(after("composer.deleteForward", "abc|")).toBe("abc|");
    expect(after("composer.deleteBack", "ab|c")).toBe("a|c");
    expect(after("composer.deleteBack", "|abc")).toBe("|abc");
    expect(after("composer.deleteBack", "a[bc]d")).toBe("a|d");
    expect(after("composer.killLineEnd", "one t|wo\nthree")).toBe("one t|\nthree");
    expect(after("composer.killLineEnd", "one|\ntwo")).toBe("one|two");
    expect(after("composer.killLineStart", "one\ntw|o")).toBe("one\n|o");
    expect(after("composer.killWordBack", "foo bar  |")).toBe("foo |");
    expect(after("composer.killWordForward", "foo| bar baz")).toBe("foo| baz");
    expect(after("composer.killWordBack", "x [selected] y")).toBe("x | y");
    expect(lineEdit("composer.killLineEnd", field("one t|wo\nthree"), "")).toMatchObject({ kill: "wo" });
    expect(lineEdit("composer.killWordBack", field("x [sel] y"), "")).toMatchObject({ kill: "sel" });
    expect(lineEdit("composer.deleteBack", field("ab|c"), "")).not.toHaveProperty("kill");
    expect(after("composer.yank", "a|b", "XY")).toBe("aXY|b");
    expect(after("composer.yank", "a[bc]d", "XY")).toBe("aXY|d");
    expect(after("composer.yank", "a|b", "")).toBe("a|b");
  });

  // spec: UI-760
  test("transpose swaps around the caret, the two before it at a line's end; select all", () => {
    expect(after("composer.transpose", "ab|cd")).toBe("acb|d");
    expect(after("composer.transpose", "abc|")).toBe("acb|");
    expect(after("composer.transpose", "xy|\ncd")).toBe("yx|\ncd");
    expect(after("composer.transpose", "|ab")).toBe("|ab");
    expect(after("composer.transpose", "a\n|b")).toBe("a\n|b");
    expect(after("composer.selectAll", "a|bc")).toBe("[abc]");
  });
});
