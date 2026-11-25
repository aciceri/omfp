/**
 * Pending interactions (UI-070..074): titles, countdowns, the `ask` form with its selection rules, omp's HTML `form`
 * with its constraints, and requests that another client or omp closed.
 */

import type { ClosedInteraction, InteractionRequest } from "@omfp/client";
import {
  type AskAnswer,
  type AskUiRequest,
  type FormAnswerData,
  type FormUiRequest,
  validateAskAnswers,
} from "@omfp/core";
import { strings } from "../strings/index.ts";
import { choiceValue, fieldError, type HtmlFormValue, isArrayChoice, textValue } from "./form/data.ts";
import { type FormField, type HtmlForm, parseHtmlForm, spansText } from "./form/parse.ts";
import { checkFormReply } from "./form/reply.ts";

/** The dialog requests: they wait for an answer. */
export type Dialog = Extract<
  InteractionRequest,
  { method: "select" | "confirm" | "input" | "editor" | "ask" | "form" }
>;

const DIALOGS: Readonly<Record<string, true>> = {
  select: true,
  confirm: true,
  input: true,
  editor: true,
  ask: true,
  form: true,
};

export function isDialog(request: InteractionRequest): request is Dialog {
  return DIALOGS[request.method] === true;
}

/**
 * The key of a request's card: its id, so closing an earlier request never remounts the cards after it (typed text and
 * focus stay); the position only for requests that are not dialogs.
 */
export function cardKey(request: InteractionRequest, index: number): string {
  return isDialog(request) ? `id-${request.id}` : `at-${index}`;
}

/** The request's title (the first question for `ask`, the first section title for `form`). */
export function title(request: InteractionRequest): string {
  if (request.method === "ask") {
    const first = request.questions[0];
    return first === undefined ? "" : (first.header ?? first.question);
  }
  if (request.method === "form") {
    const titled = formOf(request).sections.find((section) => section.title.length > 0);
    return titled === undefined ? strings.chat.form.title : spansText(titled.title);
  }
  return isDialog(request) ? request.title : "";
}

/** Timeout announced by the request, in milliseconds (`editor` has none). */
export function timeoutMs(request: InteractionRequest): number | undefined {
  switch (request.method) {
    case "select":
    case "confirm":
    case "input":
    case "ask":
    case "form":
      return request.timeout !== undefined && request.timeout > 0 ? request.timeout : undefined;
    default:
      return undefined;
  }
}

/** Milliseconds left of `timeout` for a request first shown at `shownAt`, never negative (UI-073). */
export function remainingMs(timeout: number, shownAt: number, now: number): number {
  return Math.max(0, timeout - Math.max(0, now - shownAt));
}

/** `m:ss`, rounding up so the countdown shows `0:00` only when time is up. */
export function formatCountdown(remaining: number): string {
  const seconds = Math.ceil(Math.max(0, remaining) / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** The answer being built for one `ask` question. */
export interface AnswerDraft {
  readonly selected: readonly string[];
  /** Free text; offered for every question since omp never lists "Other" (UI-071). */
  readonly custom: string;
}

/** The `ask` form: one draft per question, in order. */
export type AskForm = readonly AnswerDraft[];

export function emptyAskForm(request: AskUiRequest): AskForm {
  return request.questions.map(() => ({ selected: [], custom: "" }));
}

function withAnswer(form: AskForm, index: number, answer: AnswerDraft): AskForm {
  return form.map((draft, position) => (position === index ? answer : draft));
}

/**
 * Picks `label` for question `index`: a single-select question takes it instead of any other choice or free text; a
 * multi-select one toggles it.
 */
export function toggleOption(form: AskForm, request: AskUiRequest, index: number, label: string): AskForm {
  const question = request.questions[index];
  const draft = form[index];
  if (question === undefined || draft === undefined) {
    return form;
  }
  if (!question.multi) {
    return withAnswer(form, index, { selected: [label], custom: "" });
  }
  const selected = draft.selected.includes(label)
    ? draft.selected.filter((chosen) => chosen !== label)
    : [...draft.selected, label];
  return withAnswer(form, index, { ...draft, selected });
}

/** Sets the free text of question `index`; on a single-select question non-blank text replaces the choice. */
export function setCustom(form: AskForm, request: AskUiRequest, index: number, text: string): AskForm {
  const question = request.questions[index];
  const draft = form[index];
  if (question === undefined || draft === undefined) {
    return form;
  }
  const selected = !question.multi && text.trim() !== "" ? [] : draft.selected;
  return withAnswer(form, index, { selected, custom: text });
}

/** The answers in question order, selected options in the request's option order. */
function askAnswers(form: AskForm, request: AskUiRequest): AskAnswer[] {
  return request.questions.map((question, index) => {
    const draft = form[index] ?? { selected: [], custom: "" };
    const custom = draft.custom.trim();
    const selectedOptions = question.options
      .filter((option) => draft.selected.includes(option.label))
      .map((option) => option.label);
    return custom === ""
      ? { id: question.id, selectedOptions }
      : { id: question.id, selectedOptions, customInput: custom };
  });
}

/** The answers when every question has one and omp's rules hold (DMN-054), else what is wrong. */
export type AskCheck =
  | { readonly ok: true; readonly answers: AskAnswer[] }
  | { readonly ok: false; readonly error: string };

export function checkAsk(form: AskForm, request: AskUiRequest): AskCheck {
  const answers = askAnswers(form, request);
  const missing = request.questions.find((_, index) => {
    const answer = answers[index];
    return answer !== undefined && answer.selectedOptions.length === 0 && answer.customInput === undefined;
  });
  if (missing !== undefined) {
    return { ok: false, error: strings.chat.interactions.answerRequired(missing.question) };
  }
  const error = validateAskAnswers(request, answers);
  return error === undefined ? { ok: true, answers } : { ok: false, error };
}

const PARSED = new WeakMap<FormUiRequest, HtmlForm>();

/** The parsed HTML of a `form` request (UI-074), parsed once per request. */
export function formOf(request: FormUiRequest): HtmlForm {
  let form = PARSED.get(request);
  if (form === undefined) {
    form = parseHtmlForm(request.html);
    PARSED.set(request, form);
  }
  return form;
}

/** The answer being built for one field of a `form`, by the field's index in `HtmlForm.fields`. */
export type FieldDraft =
  | {
      readonly kind: "choice";
      /** Checked option indices; a radio group or single select holds at most one. */
      readonly checked: readonly number[];
      /** The group's "Other" answer is picked (a radio group then has no option checked). */
      readonly otherChosen: boolean;
      readonly other: string;
    }
  | { readonly kind: "text"; readonly text: string };

/** A `form` being filled. */
export interface FormDraft {
  readonly fields: readonly FieldDraft[];
  /** Submit was pressed while omp would refuse an answer: the refused field shows why from then on. */
  readonly attempted: boolean;
}

/** The form as omp's HTML opens it: preselected options, prefilled text. */
export function emptyFormDraft(form: HtmlForm): FormDraft {
  return {
    attempted: false,
    fields: form.fields.map((field) =>
      field.kind === "text"
        ? { kind: "text", text: field.value }
        : {
            kind: "choice",
            checked: field.options.flatMap((option, index) => (option.checked ? [index] : [])),
            otherChosen: false,
            other: "",
          },
    ),
  };
}

const NO_CHOICE: Extract<FieldDraft, { kind: "choice" }> = {
  kind: "choice",
  checked: [],
  otherChosen: false,
  other: "",
};

function choiceDraft(draft: FormDraft, index: number): Extract<FieldDraft, { kind: "choice" }> {
  const field = draft.fields[index];
  return field?.kind === "choice" ? field : NO_CHOICE;
}

function withField(draft: FormDraft, index: number, field: FieldDraft): FormDraft {
  return { ...draft, fields: draft.fields.map((current, position) => (position === index ? field : current)) };
}

/**
 * Picks option `option` of choice field `index`: a radio group or single select takes it instead of any other answer
 * (its Other text stays, set aside); a checkbox group or multi-select toggles it.
 */
export function chooseFormOption(form: HtmlForm, draft: FormDraft, index: number, option: number): FormDraft {
  const field = form.fields[index];
  if (field?.kind !== "choice" || field.options[option] === undefined) {
    return draft;
  }
  const current = choiceDraft(draft, index);
  if (!field.multi) {
    return withField(draft, index, { ...current, checked: [option], otherChosen: false });
  }
  const checked = current.checked.includes(option)
    ? current.checked.filter((checkedOption) => checkedOption !== option)
    : [...current.checked, option].sort((a, b) => a - b);
  return withField(draft, index, { ...current, checked });
}

/** Picks (or, in a checkbox group, unpicks) the "Other" answer of group `index`; a radio group drops its option. */
export function chooseFormOther(form: HtmlForm, draft: FormDraft, index: number, chosen: boolean): FormDraft {
  const field = form.fields[index];
  if (field?.kind !== "choice" || !field.other) {
    return draft;
  }
  const current = choiceDraft(draft, index);
  return withField(draft, index, {
    ...current,
    checked: field.multi || !chosen ? current.checked : [],
    otherChosen: field.multi ? chosen : true,
  });
}

/** Sets the "Other" text of group `index`; typing an answer picks Other (a radio group drops its option). */
export function setFormOther(form: HtmlForm, draft: FormDraft, index: number, other: string): FormDraft {
  const current = choiceDraft(draft, index);
  const next = withField(draft, index, { ...current, other });
  return other.trim() === "" ? next : chooseFormOther(form, next, index, true);
}

/** Sets the text of text field `index`. */
export function setFormText(draft: FormDraft, index: number, text: string): FormDraft {
  return draft.fields[index]?.kind === "text" ? withField(draft, index, { kind: "text", text }) : draft;
}

/** What field `index` submits: omp's value rules, a checkbox group's Other text never repeating a checked value. */
function fieldValue(field: FormField, draft: FieldDraft | undefined): HtmlFormValue | undefined {
  if (field.kind === "text") {
    return textValue(field, draft?.kind === "text" ? draft.text : field.value);
  }
  const choice = draft?.kind === "choice" ? draft : NO_CHOICE;
  const value = choiceValue(field, new Set(choice.checked), choice.otherChosen ? choice.other : undefined);
  return isArrayChoice(field) && Array.isArray(value) ? [...new Set(value)] : value;
}

/** The submitted data: hidden inputs, then each named field's value (a later field of the same name wins). */
export function formData(form: HtmlForm, draft: FormDraft): FormAnswerData {
  const entries: [string, HtmlFormValue][] = [...form.hidden];
  form.fields.forEach((field, index) => {
    const value = fieldValue(field, draft.fields[index]);
    if (field.name && value !== undefined) {
      entries.push([field.name, value]);
    }
  });
  // `fromEntries` keeps a control named `__proto__` an ordinary key.
  return Object.fromEntries(entries);
}

/**
 * The form's answers as they stand (UI-074). `flags` holds each answered field's broken rule (`pattern`, a range, a
 * length, `email`/`url`), shown as the user types: omp 19.2 sends such an answer anyway, so a flag never blocks, and
 * an unanswered field shows only its `required` mark, as omp sends unanswered questions too. `ok` carries the data
 * unless omp's check of the reply would refuse it (an Other text naming a disabled option, same-name fields of
 * different shapes), then the field it refuses and why.
 */
export type FormCheck = { readonly flags: readonly (string | undefined)[] } & (
  | { readonly ok: true; readonly data: FormAnswerData }
  | { readonly ok: false; readonly refused: { readonly index: number; readonly reason: string } }
);

/** Whether `value` answers its field: a non-blank text or number, at least one picked option. */
function answered(value: HtmlFormValue | undefined): boolean {
  if (value === undefined) {
    return false;
  }
  return Array.isArray(value) ? value.length > 0 : String(value).trim() !== "";
}

export function checkForm(form: HtmlForm, draft: FormDraft): FormCheck {
  const flags = form.fields.map((field, index) => {
    const value = fieldValue(field, draft.fields[index]);
    return answered(value) ? fieldError(field, value) : undefined;
  });
  const data = formData(form, draft);
  const reply = checkFormReply(form, data);
  if (reply.ok) {
    return { ok: true, data, flags };
  }
  const index = Math.max(
    0,
    form.fields.findIndex((field) => field.name === reply.name),
  );
  return { ok: false, flags, refused: { index, reason: reply.reason } };
}

/** The `src` a form's `<img>` may load: http(s) and `data:` URLs only; a local path or `file:` URL shows as text. */
export function formImageSource(src: string): string | undefined {
  return /^(?:https?|data):/i.test(src) ? src : undefined;
}

/** An inline `<svg>` as an image URL, so its markup is drawn by `<img>` and never inserted into the page. */
export function svgImageSource(markup: string): string {
  const standalone = /^<svg(?![^>]*\sxmlns=)/i.test(markup)
    ? markup.replace(/^<svg/i, '<svg xmlns="http://www.w3.org/2000/svg"')
    : markup;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(standalone)}`;
}

/** A closed request shown in place of the dialog (UI-072). */
export interface ClosedRow {
  readonly id: string;
  readonly title: string;
  readonly text: string;
}

/**
 * Rows for requests closed by someone else: another client (with its name) or omp. Requests answered from this window
 * (`answeredHere`) or dismissed need no row.
 */
export function closedRows(
  closed: readonly ClosedInteraction[],
  answeredHere: (id: string) => boolean,
  dismissed: (id: string) => boolean,
): ClosedRow[] {
  return closed.flatMap(({ request, resolution }) => {
    if (!isDialog(request) || answeredHere(request.id) || dismissed(request.id)) {
      return [];
    }
    const text =
      resolution === undefined
        ? strings.chat.interactions.closedByOmp
        : strings.chat.interactions.answeredElsewhere(resolution.client);
    return [{ id: request.id, title: title(request), text }];
  });
}
