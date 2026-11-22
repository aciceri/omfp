/**
 * Ported from oh-my-pi v19.1.0 `packages/tui/src/html-form/data.ts`.
 * MIT License, Copyright (c) 2025 Mario Zechner.
 *
 * Answers of a parsed {@link HtmlForm} (UI-074): the values a submission reports, the rules omp checks before
 * accepting them, and the question each field answers (its accessible name in the card). The messages come from
 * `strings.chat.form`; everything that decides the submitted data matches omp's.
 */

import type { FormAnswerData } from "@omfp/core";
import { strings } from "../../strings/index.ts";
import { type ChoiceField, type FormField, type HtmlForm, spansText, type TextField } from "./parse.ts";

/** Submitted value of one control: text, a number for `number`/`range` inputs, or a group's checked values. */
export type HtmlFormValue = FormAnswerData[string];

const text = strings.chat.form;

/** Whether a choice field reports an array (checkbox group, multi-select) rather than one value. */
export function isArrayChoice(field: ChoiceField): boolean {
  return field.multi && field.options.length > 1;
}

/**
 * Value a choice field contributes given its checked option indices and, when the user chose its "Other" answer, that
 * answer's text: the typed text stands in for a radio's value, or joins a checkbox group's array. Undefined when the
 * field contributes nothing; blank Other text counts as no answer.
 */
export function choiceValue(
  field: ChoiceField,
  checked: ReadonlySet<number>,
  other?: string,
): HtmlFormValue | undefined {
  const typed = field.other ? other?.trim() : undefined;
  const values = field.options.flatMap((option, index) =>
    checked.has(index) && !option.disabled ? [option.value] : [],
  );
  if (isArrayChoice(field)) {
    return typed ? [...values, typed] : values;
  }
  return typed || values[0];
}

/** Value a text field contributes: a number for non-empty numeric `number`/`range` inputs, else the text. */
export function textValue(field: TextField, value: string): HtmlFormValue {
  if ((field.type === "number" || field.type === "range") && value.trim() !== "") {
    const number = Number(value);
    if (Number.isFinite(number)) {
      return number;
    }
  }
  return value;
}

/**
 * Why `value` is not an acceptable answer for `field`, or undefined when it is. Enforces `required`, numeric
 * `min`/`max`, `minlength`/`maxlength`, `email` and `url` shapes, and `pattern` (anchored, as HTML does; an invalid
 * pattern is ignored).
 */
export function fieldError(field: FormField, value: HtmlFormValue | undefined): string | undefined {
  if (field.kind === "choice") {
    const empty = value === undefined || (Array.isArray(value) && value.length === 0);
    if (!field.required || !empty) {
      return undefined;
    }
    return isArrayChoice(field) ? text.selectAtLeastOne : text.required;
  }
  const answer = value === undefined ? "" : String(value);
  if (answer.trim() === "") {
    return field.required ? text.required : undefined;
  }
  if (field.type === "number" || field.type === "range") {
    const number = Number(answer);
    if (!Number.isFinite(number)) {
      return text.enterNumber;
    }
    if (field.min !== undefined && number < field.min) {
      return text.atLeast(field.min);
    }
    if (field.max !== undefined && number > field.max) {
      return text.atMost(field.max);
    }
  }
  if (field.minLength !== undefined && answer.length < field.minLength) {
    return text.minLength(field.minLength);
  }
  if (field.maxLength !== undefined && answer.length > field.maxLength) {
    return text.maxLength(field.maxLength);
  }
  if (field.type === "email" && !/^[^\s@]+@[^\s@]+$/.test(answer)) {
    return text.email;
  }
  if (field.type === "url" && !URL.canParse(answer)) {
    return text.url;
  }
  if (field.pattern && compilePattern(field.pattern)?.test(answer) === false) {
    return text.pattern(field.pattern.length > 40 ? `${field.pattern.slice(0, 39)}…` : field.pattern);
  }
  return undefined;
}

/** `^(?:pattern)$` as HTML matches `pattern`, or undefined when the model wrote an invalid one. */
function compilePattern(pattern: string): RegExp | undefined {
  try {
    return new RegExp(`^(?:${pattern})$`, "v");
  } catch {
    return undefined;
  }
}

/**
 * The question each field answers: its own label, else the text block right before it (`<p>Which database?</p>`
 * above a radio group), else its section title, else a lone option's label (a single checkbox), else its name.
 */
export function fieldQuestions(form: HtmlForm): Map<FormField, string> {
  const questions = new Map<FormField, string>();
  for (const section of form.sections) {
    const title = spansText(section.title);
    let caption = "";
    for (const block of section.blocks) {
      if (block.kind === "field") {
        const field = block.field;
        const own = spansText(field.label);
        const lone =
          field.kind === "choice" && field.options.length === 1 ? spansText(field.options[0]?.label ?? []) : "";
        questions.set(field, (own || caption || title || lone || field.name).replace(/\s+/g, " ").trim());
        caption = "";
        continue;
      }
      caption = block.kind === "text" ? spansText(block.spans) : "";
    }
  }
  return questions;
}
