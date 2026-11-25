/** L0: the arguments of a tool call parsed from the JSON text the model has written so far (UI-054). */

import { describe, expect, test } from "bun:test";
import { parsePartialJson } from "../../src/vm/partial-json.ts";

describe("partial JSON", () => {
  // spec: UI-054
  test("complete JSON parses as JSON.parse does", () => {
    const text = '{"a": [1, -2.5e3, true, false, null, {"b": "c\\n\\u00e9\\"\\\\"}], "d": {}, "e": []}';
    expect(parsePartialJson(text)).toEqual(JSON.parse(text));
    expect(parsePartialJson(' "x" ')).toBe("x");
  });

  // spec: UI-054
  test("a string being written is the text so far; a cut escape is left out", () => {
    expect(parsePartialJson('{"path": "src/li')).toEqual({ path: "src/li" });
    expect(parsePartialJson('{"content": "line 1\\nline')).toEqual({ content: "line 1\nline" });
    expect(parsePartialJson('{"content": "a\\')).toEqual({ content: "a" });
    expect(parsePartialJson('{"content": "a\\u00')).toEqual({ content: "a" });
    expect(parsePartialJson('{"emoji": "\\ud83d\\ude00')).toEqual({ emoji: "😀" });
  });

  // spec: UI-054
  test("a key without its value, a cut literal and a lone minus are left out; a number so far stays", () => {
    expect(parsePartialJson("")).toBeUndefined();
    expect(parsePartialJson("{")).toEqual({});
    expect(parsePartialJson('{"pa')).toEqual({});
    expect(parsePartialJson('{"path"')).toEqual({});
    expect(parsePartialJson('{"path": ')).toEqual({});
    expect(parsePartialJson('{"wait": tr')).toEqual({});
    expect(parsePartialJson('{"line": -')).toEqual({});
    expect(parsePartialJson('{"line": 12')).toEqual({ line: 12 });
    expect(parsePartialJson('{"limit": 1.')).toEqual({ limit: 1 });
    expect(parsePartialJson('{"wait": true, "line": 3')).toEqual({ wait: true, line: 3 });
  });

  // spec: UI-054
  test("nested containers keep what they hold so far", () => {
    expect(parsePartialJson('{"tasks": [{"name": "a", "task": "do x"}, {"name": "b", "ta')).toEqual({
      tasks: [{ name: "a", task: "do x" }, { name: "b" }],
    });
    expect(parsePartialJson('{"items": ["one", "tw')).toEqual({ items: ["one", "tw"] });
    expect(parsePartialJson('{"items": [')).toEqual({ items: [] });
  });

  // spec: UI-054
  test("parsing stops at a character that cannot continue the JSON, keeping what came before", () => {
    expect(parsePartialJson('{"a": 1, "b": x, "c": 2}')).toEqual({ a: 1 });
    expect(parsePartialJson('{"a": 1 "b": 2}')).toEqual({ a: 1 });
    expect(parsePartialJson("nope")).toBeUndefined();
  });

  // spec: UI-054
  test("a __proto__ key is an own key, as JSON.parse makes it", () => {
    const parsed = parsePartialJson('{"__proto__": {"polluted": true}, "a": 1');
    expect(Object.keys(parsed as object)).toEqual(["__proto__", "a"]);
    expect(({} as Record<string, unknown>).polluted).toBeUndefined();
  });
});
