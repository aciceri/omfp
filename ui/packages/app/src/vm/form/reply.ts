/**
 * Ported from oh-my-pi v19.2.0 `packages/coding-agent/src/modes/rpc/rpc-mode.ts` (`parseFormDialogResponse` and its
 * helpers). MIT License, Copyright (c) 2025 Mario Zechner.
 *
 * omp checks a `form` answer's `data` against the form it parsed and fails the whole `ask` call on any mismatch, with
 * no round for correcting a field. The card runs the same check before sending (UI-074), so what it sends is accepted.
 * Since omp 19.2 the check covers the answer's shape only: a broken rule (`required`, `pattern`, a range) is sent.
 */

import type { FormAnswerData } from "@omfp/core";
import { strings } from "../../strings/index.ts";
import { choiceValue, type HtmlFormValue, isArrayChoice, textValue } from "./data.ts";
import type { ChoiceField, FormField, HtmlForm, TextField } from "./parse.ts";

/** What omp would accept, or the first field it would refuse and why. */
export type FormReplyCheck =
  | { readonly ok: true; readonly data: FormAnswerData }
  | { readonly ok: false; readonly name: string; readonly reason: string };

const text = strings.chat.form;

/** A mismatch omp would throw on. */
class Refused extends Error {}

/**
 * Value a choice field's submitted entry gives: each string must be one of the field's enabled option values, except
 * one free-text entry when the field offers "Other". Checkbox groups and multi-selects take an array; every other
 * choice takes one string.
 */
function parseFormChoice(field: ChoiceField, value: unknown): HtmlFormValue | undefined {
  // An omitted field means nothing checked: an empty array for array choices.
  if (value === undefined) {
    return choiceValue(field, new Set());
  }
  const array = isArrayChoice(field);
  if (array !== Array.isArray(value)) {
    throw new Refused(array ? text.wantsList : text.wantsOne);
  }
  const entries: readonly unknown[] = Array.isArray(value) ? value : [value];
  // Maps each entry to an option index, keeping at most one unmatched entry as the "Other" text.
  const checked = new Set<number>();
  let other: string | undefined;
  for (const entry of entries) {
    if (typeof entry !== "string") {
      throw new Refused(array ? text.wantsList : text.wantsOne);
    }
    // Prefers an enabled option, so a disabled duplicate of the same value earlier in the group cannot shadow it.
    const enabled = field.options.findIndex((option) => !option.disabled && option.value === entry);
    const index = enabled >= 0 ? enabled : field.options.findIndex((option) => option.value === entry);
    if (index >= 0 && field.options[index]?.disabled === true) {
      throw new Refused(text.unavailable(entry));
    }
    if (index >= 0 && !checked.has(index)) {
      checked.add(index);
    } else if (index < 0 && field.other && other === undefined) {
      other = entry;
    } else {
      throw new Refused(text.unexpected(entry));
    }
  }
  return choiceValue(field, checked, other);
}

/**
 * Keeps valid selections, treating an echoed same-name hidden value as no selection when it cannot be the answer. A
 * hidden value inside an array choice's entries is dropped up front when no option carries it (it would otherwise pose
 * as the "Other" text), else only when keeping it fails.
 */
function parseChoiceOrHiddenFallback(
  field: ChoiceField,
  value: unknown,
  hiddenValue: string | undefined,
): HtmlFormValue | undefined {
  if (hiddenValue === undefined) {
    return parseFormChoice(field, value);
  }
  const echo = Array.isArray(value) ? value.indexOf(hiddenValue) : value === hiddenValue ? 0 : -1;
  if (echo < 0) {
    return parseFormChoice(field, value);
  }
  const withoutEcho = Array.isArray(value) ? value.toSpliced(echo, 1) : undefined;
  if (Array.isArray(value) && !field.options.some((option) => option.value === hiddenValue)) {
    return parseFormChoice(field, withoutEcho);
  }
  try {
    return parseFormChoice(field, value);
  } catch {
    return parseFormChoice(field, withoutEcho);
  }
}

/** Value a text field's entry gives: a string, or a number for `number`/`range` inputs; absent counts as empty. */
function parseFormText(field: TextField, value: unknown): HtmlFormValue {
  if (value === undefined) {
    return textValue(field, "");
  }
  const numeric = field.type === "number" || field.type === "range";
  if (typeof value === "string" || (numeric && typeof value === "number" && Number.isFinite(value))) {
    return textValue(field, String(value));
  }
  throw new Refused(numeric ? text.wantsNumber : text.wantsText);
}

/** The data omp keeps from `submitted` for `form`, or the first field it refuses (`parseFormDialogResponse`). */
export function checkFormReply(form: HtmlForm, submitted: FormAnswerData): FormReplyCheck {
  const byName = new Map<string, FormField[]>();
  for (const field of form.fields) {
    if (field.name) {
      byName.set(field.name, [...(byName.get(field.name) ?? []), field]);
    }
  }
  const hidden = new Map(form.hidden);
  for (const name of Object.keys(submitted)) {
    if (!byName.has(name) && !hidden.has(name)) {
      return { ok: false, name, reason: text.unknownField };
    }
  }
  // One entry answers every field of its name, as in HTML form data: each field that can carry the entry takes it, a
  // same-name field of another shape counts as unanswered. At least one field must carry the entry. The fields' own
  // rules are not checked: omp's form dialogs only flag them.
  const entries: [string, HtmlFormValue][] = [...form.hidden];
  for (const [name, fields] of byName) {
    const entry = Object.hasOwn(submitted, name) ? submitted[name] : undefined;
    const hiddenValue = hidden.get(name);
    let mismatch: Refused | undefined;
    const values: { field: FormField; value: HtmlFormValue | undefined; carried: boolean }[] = [];
    for (const field of fields) {
      try {
        const value =
          field.kind === "choice"
            ? parseChoiceOrHiddenFallback(field, entry, hiddenValue)
            : parseFormText(field, entry);
        values.push({ field, value, carried: true });
      } catch (error) {
        if (!(error instanceof Refused)) {
          throw error;
        }
        // Only a field sharing its name may decline the entry; a lone field's mismatch fails the answer.
        if (fields.length === 1) {
          return { ok: false, name, reason: error.message };
        }
        mismatch ??= error;
        const value = field.kind === "choice" ? parseFormChoice(field, undefined) : parseFormText(field, undefined);
        values.push({ field, value, carried: false });
      }
    }
    if (mismatch !== undefined && !values.some(({ carried }) => carried)) {
      return { ok: false, name, reason: mismatch.message };
    }
    for (const { value, carried } of values) {
      if (carried && value !== undefined) {
        entries.push([name, value]);
      }
    }
  }
  return { ok: true, data: Object.fromEntries(entries) };
}
