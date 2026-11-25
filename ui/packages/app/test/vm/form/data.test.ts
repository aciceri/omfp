import { describe, expect, test } from "bun:test";
import { choiceValue, fieldError, fieldQuestions, textValue } from "../../../src/vm/form/data.ts";
import { type ChoiceField, type FormField, parseHtmlForm, type TextField } from "../../../src/vm/form/parse.ts";
import { checkFormReply } from "../../../src/vm/form/reply.ts";

function only(html: string): FormField {
  const [field] = parseHtmlForm(html).fields;
  if (field === undefined) {
    throw new Error("no field");
  }
  return field;
}

function input(attributes: string): TextField {
  const field = only(`<input name="f" ${attributes}>`);
  if (field.kind !== "text") {
    throw new Error("not a text field");
  }
  return field;
}

function group(html: string): ChoiceField {
  const field = only(html);
  if (field.kind !== "choice") {
    throw new Error("not a choice field");
  }
  return field;
}

const COLORS = group(`<form>
  <label><input type="checkbox" name="c" value="red"> Red</label>
  <label><input type="checkbox" name="c" value="blue"> Blue</label>
  <label><input type="checkbox" name="c" value="gray" disabled> Gray</label>
</form>`);

describe("form field rules", () => {
  // spec: UI-074
  test("required: blank or whitespace-only text and empty choices fail", () => {
    const field = input("required");
    expect([fieldError(field, ""), fieldError(field, "   "), fieldError(field, undefined)]).toEqual([
      "Required",
      "Required",
      "Required",
    ]);
    expect(fieldError(field, "x")).toBeUndefined();
    expect(fieldError(input(""), "")).toBeUndefined();
    const required = { ...COLORS, required: true };
    expect(fieldError(required, [])).toBe("Select at least one");
    expect(fieldError(required, ["red"])).toBeUndefined();
    const radio = group(
      `<form><input type="radio" name="r" value="a" required><input type="radio" name="r" value="b"></form>`,
    );
    expect(fieldError(radio, undefined)).toBe("Required");
  });

  // spec: UI-074
  test("number and range: a number within min and max", () => {
    const field = input(`type="number" min="1" max="10"`);
    expect([fieldError(field, "abc"), fieldError(field, 0), fieldError(field, "11"), fieldError(field, 5)]).toEqual([
      "Enter a number",
      "Must be at least 1",
      "Must be at most 10",
      undefined,
    ]);
    expect(fieldError(input(`type="range" max="3"`), "4")).toBe("Must be at most 3");
  });

  // spec: UI-074
  test("minlength and maxlength count characters", () => {
    const field = input(`minlength="2" maxlength="4"`);
    expect([fieldError(field, "a"), fieldError(field, "abcde"), fieldError(field, "abc")]).toEqual([
      "Needs at least 2 characters",
      "At most 4 characters",
      undefined,
    ]);
  });

  // spec: UI-074
  test("email and url shapes", () => {
    expect(fieldError(input(`type="email"`), "me@example")).toBeUndefined();
    expect(fieldError(input(`type="email"`), "me at example")).toBe("Enter an email address");
    expect(fieldError(input(`type="url"`), "https://example.com/x")).toBeUndefined();
    expect(fieldError(input(`type="url"`), "example.com")).toBe("Enter a full URL");
  });

  // spec: UI-074
  test("pattern is anchored to the whole value; an invalid pattern is ignored", () => {
    const field = input(`pattern="[a-z]+"`);
    expect(fieldError(field, "abc")).toBeUndefined();
    expect(fieldError(field, "abc1")).toBe("Must match [a-z]+");
    expect(fieldError(input(`pattern="a|b"`), "ab")).toBe("Must match a|b");
    expect(fieldError(input(`pattern="(["`), "anything")).toBeUndefined();
    expect(fieldError(input(`pattern="${"x".repeat(50)}"`), "y")).toBe(`Must match ${"x".repeat(39)}…`);
  });
});

describe("form values", () => {
  // spec: UI-074
  test("choices give option values, skipping disabled ones; Other joins a group or replaces a radio's value", () => {
    expect(choiceValue(COLORS, new Set([1, 0, 2]))).toEqual(["red", "blue"]);
    expect(choiceValue(COLORS, new Set(), " teal ")).toEqual(["teal"]);
    expect(choiceValue(COLORS, new Set([0]), "  ")).toEqual(["red"]);
    const radio = group(`<form><input type="radio" name="r" value="a"><input type="radio" name="r" value="b"></form>`);
    expect(choiceValue(radio, new Set([1]))).toBe("b");
    expect(choiceValue(radio, new Set(), "c")).toBe("c");
    expect(choiceValue(radio, new Set())).toBeUndefined();
    const lone = group(`<form><input type="checkbox" name="agree" value="yes"></form>`);
    expect([choiceValue(lone, new Set([0])), choiceValue(lone, new Set())]).toEqual(["yes", undefined]);
  });

  // spec: UI-074
  test("number and range inputs give numbers; other text stays text", () => {
    expect(textValue(input(`type="number"`), "4.5")).toBe(4.5);
    expect(textValue(input(`type="range"`), "3")).toBe(3);
    expect(textValue(input(`type="number"`), "")).toBe("");
    expect(textValue(input(`type="number"`), "x")).toBe("x");
    expect(textValue(input(`type="text"`), "4")).toBe("4");
  });

  // spec: UI-074
  test("a field's question: its label, else the text before it, else the section title", () => {
    const form = parseHtmlForm(`<form>
      <p>Which   database?</p>
      <label><input type="radio" name="db" value="pg"> Postgres</label>
      <label><input type="radio" name="db" value="lite"> SQLite</label>
      <fieldset><legend>Extras</legend><input type="checkbox" name="x" value="1"><input type="checkbox" name="x" value="2"></fieldset>
      <label>Name <input name="name"></label>
    </form>`);
    expect(form.fields.map((field) => fieldQuestions(form).get(field))).toEqual(["Which database?", "Extras", "Name"]);
  });
});

describe("form reply check (omp's parseFormDialogResponse)", () => {
  const form = parseHtmlForm(`<form>
    <input type="hidden" name="token" value="t1">
    <label><input type="radio" name="db" value="pg"> Postgres</label>
    <label><input type="radio" name="db" value="lite"> SQLite</label>
    <label><input type="checkbox" name="c" value="red"> Red</label>
    <label><input type="checkbox" name="c" value="blue"> Blue</label>
    <label><input type="checkbox" name="c" value="gray" disabled> Gray</label>
    <label>Port <input type="number" name="port" min="1"></label>
  </form>`);

  // spec: UI-074
  test("accepts option values, one Other answer per group, numbers and hidden echoes", () => {
    expect(checkFormReply(form, { token: "t1", db: "mysql", c: ["blue", "teal"], port: 80 })).toEqual({
      ok: true,
      data: { token: "t1", db: "mysql", c: ["blue", "teal"], port: 80 },
    });
    // Hidden inputs keep the form's values; an absent checkbox group is an empty array.
    expect(checkFormReply(form, { token: "forged", port: "8" })).toEqual({
      ok: true,
      data: { token: "t1", c: [], port: 8 },
    });
  });

  // spec: UI-074
  test("refuses unknown names, disabled options, repeated values and wrong shapes; a broken rule passes", () => {
    const refused = (data: Record<string, string | number | string[]>) => {
      const check = checkFormReply(form, data);
      return check.ok ? undefined : [check.name, check.reason];
    };
    expect(refused({ nope: "x" })).toEqual(["nope", "Not a field of this form"]);
    expect(refused({ c: ["gray"] })).toEqual(["c", "\u201cgray\u201d is not available"]);
    expect(refused({ c: ["red", "red"] })).toEqual(["c", "\u201cred\u201d is not an answer this question takes"]);
    expect(refused({ c: ["teal", "lime"] })).toEqual(["c", "\u201clime\u201d is not an answer this question takes"]);
    expect(refused({ c: "red" })).toEqual(["c", "Takes a list of answers"]);
    expect(refused({ db: ["pg"] })).toEqual(["db", "Takes one answer"]);
    // omp 19.2 only flags a rule in its dialogs: an answer below `min` is sent as it is.
    expect(checkFormReply(form, { port: 0 })).toEqual({ ok: true, data: { token: "t1", c: [], port: 0 } });
  });
});
