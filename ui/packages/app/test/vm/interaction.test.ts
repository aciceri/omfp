import { describe, expect, test } from "bun:test";
import type { ClosedInteraction, InteractionRequest } from "@omfp/client";
import type { AskUiRequest, FormUiRequest } from "@omfp/core";
import {
  type AskForm,
  cardKey,
  checkAsk,
  checkForm,
  chooseFormOption,
  chooseFormOther,
  closedRows,
  emptyAskForm,
  emptyFormDraft,
  type FormDraft,
  formatCountdown,
  formData,
  formImageSource,
  formOf,
  isDialog,
  remainingMs,
  setCustom,
  setFormOther,
  setFormText,
  svgImageSource,
  timeoutMs,
  title,
  toggleOption,
} from "../../src/vm/interaction.ts";

const FORM_HTML = `<form>
  <input type="hidden" name="token" value="t1">
  <fieldset><legend>Setup</legend>
    <p>Which database?</p>
    <label><input type="radio" name="db" value="pg" checked> Postgres</label>
    <label><input type="radio" name="db" value="lite"> SQLite</label>
    <p>Which colors?</p>
    <label><input type="checkbox" name="colors" value="red"> Red</label>
    <label><input type="checkbox" name="colors" value="blue"> Blue</label>
    <label><input type="checkbox" name="colors" value="gray" disabled> Gray</label>
    <label><input type="checkbox" name="agree" value="yes"> I agree</label>
    <label>Port <input type="number" name="port" value="8080" min="1" max="65535" required></label>
    <label>Code <input name="code" pattern="[A-Z]{3}"></label>
  </fieldset>
</form>`;

function formRequest(html: string, timeout?: number): InteractionRequest & FormUiRequest {
  return {
    type: "extension_ui_request",
    method: "form",
    id: "f-1",
    html,
    ...(timeout === undefined ? {} : { timeout }),
  };
}

const ASK: AskUiRequest = {
  id: "ask-1",
  timeout: 30000,
  questions: [
    { id: "color", question: "Which color?", multi: false, options: [{ label: "red" }, { label: "blue" }] },
    {
      id: "tools",
      question: "Which tools?",
      multi: true,
      options: [{ label: "bash" }, { label: "read" }, { label: "edit" }],
    },
  ],
};

function confirm(id: string): InteractionRequest {
  return { type: "extension_ui_request", method: "confirm", id, title: "Proceed?", message: "m" };
}

describe("ask form", () => {
  // spec: UI-071
  test("single select keeps one choice and free text replaces it", () => {
    let form: AskForm = emptyAskForm(ASK);
    form = toggleOption(form, ASK, 0, "red");
    form = toggleOption(form, ASK, 0, "blue");
    expect(form[0]?.selected).toEqual(["blue"]);
    form = setCustom(form, ASK, 0, "green");
    expect(form[0]).toEqual({ selected: [], custom: "green" });
    form = toggleOption(form, ASK, 0, "red");
    expect(form[0]).toEqual({ selected: ["red"], custom: "" });
  });

  // spec: UI-071
  test("multi select toggles and combines with free text in option order", () => {
    let form: AskForm = emptyAskForm(ASK);
    form = toggleOption(form, ASK, 0, "red");
    for (const label of ["edit", "bash", "read", "read"]) {
      form = toggleOption(form, ASK, 1, label);
    }
    form = setCustom(form, ASK, 1, "  grep ");
    const checked = checkAsk(form, ASK);
    expect(checked.ok).toBe(true);
    if (checked.ok) {
      expect(checked.answers[1]).toEqual({ id: "tools", selectedOptions: ["bash", "edit"], customInput: "grep" });
    }
  });

  // spec: UI-071
  test("every question needs an answer and omp's rules are enforced", () => {
    let form: AskForm = toggleOption(emptyAskForm(ASK), ASK, 1, "bash");
    expect(checkAsk(form, ASK)).toEqual({ ok: false, error: "Answer \u201cWhich color?\u201d" });
    form = setCustom(form, ASK, 0, "green");
    expect(checkAsk(form, ASK).ok).toBe(true);
    // A form edited into an invalid shape is refused by `validateAskAnswers`.
    const invalid: AskForm = [{ selected: ["red"], custom: "green" }, form[1] ?? { selected: [], custom: "" }];
    const checked = checkAsk(invalid, ASK);
    expect(checked.ok === false && checked.error.includes("single-select")).toBe(true);
  });
});

describe("interactions", () => {
  // spec: UI-073
  test("the countdown counts down from when the request was shown", () => {
    const request: InteractionRequest = { type: "extension_ui_request", method: "ask", ...ASK };
    const timeout = timeoutMs(request) ?? 0;
    expect(formatCountdown(remainingMs(timeout, 1_000, 1_000))).toBe("0:30");
    expect(formatCountdown(remainingMs(timeout, 1_000, 2_500))).toBe("0:29");
    expect(formatCountdown(remainingMs(timeout, 1_000, 40_000))).toBe("0:00");
    expect(formatCountdown(remainingMs(90_000, 0, 0))).toBe("1:30");
    expect(timeoutMs({ type: "extension_ui_request", method: "editor", id: "e", title: "Edit" })).toBeUndefined();
    expect(title(request)).toBe("Which color?");
  });

  // spec: UI-072
  test("requests closed elsewhere say who answered unless answered here", () => {
    const closed: ClosedInteraction[] = [
      { request: confirm("a"), resolution: { by: "client", client: "omfp/web" } },
      { request: confirm("b"), resolution: undefined },
      { request: confirm("c"), resolution: { by: "client", client: "omfp/web" } },
    ];
    const rows = closedRows(
      closed,
      (id) => id === "c",
      () => false,
    );
    expect(rows.map((row) => [row.id, row.text])).toEqual([
      ["a", "Answered elsewhere by omfp/web"],
      ["b", "Closed by omp (timed out or cancelled)"],
    ]);
    expect(rows[0]?.title).toBe("Proceed?");
    expect(
      closedRows(
        closed,
        () => false,
        (id) => id !== "b",
      ).length,
    ).toBe(1);
  });

  // spec: UI-070
  test("cards are keyed by request id and by position only without one", () => {
    expect([cardKey(confirm("c-7"), 0), cardKey(confirm("c-7"), 3)]).toEqual(["id-c-7", "id-c-7"]);
    expect(cardKey({ type: "extension_ui_request", method: "notify", id: "n", message: "hi" }, 2)).toBe("at-2");
  });
});

describe("html form", () => {
  const request = formRequest(FORM_HTML, 60_000);
  const form = formOf(request);
  /** Field indices: db, colors, agree, port, code. */
  const [DB, COLORS, AGREE, PORT, CODE] = [0, 1, 2, 3, 4];

  // spec: UI-074, UI-073
  test("a form request is a dialog titled by its first section, with its timeout", () => {
    expect(isDialog(request)).toBe(true);
    expect(title(request)).toBe("Setup");
    expect(timeoutMs(request)).toBe(60_000);
    expect(cardKey(request, 0)).toBe("id-f-1");
    const untitled = formRequest(`<form><input name="a"></form>`);
    expect([title(untitled), timeoutMs(untitled)]).toEqual(["Form", undefined]);
    expect(formOf(request)).toBe(form);
  });

  // spec: UI-074
  test("an untouched form submits its defaults: hidden inputs, preselected options, prefilled text as numbers", () => {
    const draft = emptyFormDraft(form);
    expect(formData(form, draft)).toEqual({ token: "t1", db: "pg", colors: [], port: 8080, code: "" });
    expect(checkForm(form, draft)).toEqual({
      ok: true,
      data: { token: "t1", db: "pg", colors: [], port: 8080, code: "" },
    });
  });

  // spec: UI-074
  test("radios take one option; checkboxes toggle; a lone checkbox gives its value", () => {
    let draft: FormDraft = emptyFormDraft(form);
    draft = chooseFormOption(form, draft, DB, 1);
    draft = chooseFormOption(form, draft, COLORS, 1);
    draft = chooseFormOption(form, draft, COLORS, 0);
    draft = chooseFormOption(form, draft, COLORS, 1);
    draft = chooseFormOption(form, draft, COLORS, 0);
    draft = chooseFormOption(form, draft, COLORS, 1);
    draft = chooseFormOption(form, draft, AGREE, 0);
    draft = setFormText(draft, PORT, "443");
    draft = setFormText(draft, CODE, "ABC");
    expect(formData(form, draft)).toEqual({
      token: "t1",
      db: "lite",
      colors: ["blue"],
      agree: "yes",
      port: 443,
      code: "ABC",
    });
    // Out-of-range indices and the wrong kind of field leave the draft alone.
    expect(chooseFormOption(form, draft, PORT, 0)).toBe(draft);
    expect(chooseFormOption(form, draft, DB, 9)).toBe(draft);
    expect(setFormText(draft, DB, "x")).toBe(draft);
    expect(chooseFormOther(form, draft, AGREE, true)).toBe(draft);
  });

  // spec: UI-074
  test("Other: typing picks it (a radio drops its option), picking an option sets it aside, a group adds it", () => {
    let draft = setFormOther(form, emptyFormDraft(form), DB, "mysql");
    expect(formData(form, draft).db).toBe("mysql");
    draft = chooseFormOption(form, draft, DB, 0);
    expect(formData(form, draft).db).toBe("pg");
    draft = chooseFormOther(form, draft, DB, true);
    expect(formData(form, draft).db).toBe("mysql");
    // Blank Other text is no answer.
    draft = setFormOther(form, draft, DB, "  ");
    expect(formData(form, draft).db).toBeUndefined();

    draft = chooseFormOption(form, draft, COLORS, 0);
    draft = setFormOther(form, draft, COLORS, " teal ");
    expect(formData(form, draft).colors).toEqual(["red", "teal"]);
    draft = chooseFormOther(form, draft, COLORS, false);
    expect(formData(form, draft).colors).toEqual(["red"]);
    // An Other text repeating a checked option is sent once, as omp refuses duplicates.
    draft = setFormOther(form, draft, COLORS, "red");
    expect(formData(form, draft).colors).toEqual(["red"]);
  });

  // spec: UI-074
  test("each invalid field gets its error; omp's own refusals land on their field", () => {
    let draft = setFormText(emptyFormDraft(form), PORT, " ");
    draft = setFormText(draft, CODE, "abc");
    expect(checkForm(form, draft)).toEqual({
      ok: false,
      errors: [undefined, undefined, undefined, "Required", "Must match [A-Z]{3}"],
    });
    draft = setFormText(draft, PORT, "70000");
    draft = setFormText(draft, CODE, "");
    expect(checkForm(form, draft)).toEqual({
      ok: false,
      errors: [undefined, undefined, undefined, "Must be at most 65535", undefined],
    });
    // An Other text naming a disabled option passes the field rules but omp would refuse it.
    draft = setFormText(draft, PORT, "1");
    draft = setFormOther(form, draft, COLORS, "gray");
    expect(checkForm(form, draft)).toEqual({
      ok: false,
      errors: [undefined, "\u201cgray\u201d is not available", undefined, undefined, undefined],
    });
  });

  // spec: UI-074
  test("pictures load only from http(s) and data: URLs; an inline svg becomes a standalone image URL", () => {
    expect(formImageSource("https://example.com/a.png")).toBe("https://example.com/a.png");
    expect(formImageSource("data:image/png;base64,AA==")).toBe("data:image/png;base64,AA==");
    expect([
      formImageSource("file:///tmp/a.png"),
      formImageSource("/tmp/a.png"),
      formImageSource("javascript:x"),
    ]).toEqual([undefined, undefined, undefined]);
    expect(decodeURIComponent(svgImageSource(`<svg viewBox="0 0 1 1"></svg>`))).toBe(
      `data:image/svg+xml;charset=utf-8,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"></svg>`,
    );
    const own = `<svg xmlns="http://www.w3.org/2000/svg"></svg>`;
    expect(decodeURIComponent(svgImageSource(own))).toBe(`data:image/svg+xml;charset=utf-8,${own}`);
  });
});
