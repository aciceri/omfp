/**
 * `omfp:interactions`: the selected thread's pending requests (UI-070) — approval selects, confirms, inputs, editors,
 * `ask` forms (UI-071) and omp's HTML forms (UI-074) with their countdown (UI-073) — and rows for requests another
 * client or omp closed (UI-072). The first control of the first request takes the focus.
 */

import type { ThreadView } from "@omfp/client";
import type { AskUiRequest, ConfirmUiRequest, SelectUiRequest } from "@omfp/core";
import type { InteractionResponse } from "@omfp/protocol";
import {
  Button,
  Checkbox,
  Icon,
  IconButton,
  InteractionCard as KitInteractionCard,
  OptionText,
  Radio,
  RadioGroup,
  ResolvedLine,
  Select,
  TextArea,
  TextField,
  Toolbar,
} from "omfp/kit";
import {
  type AskForm,
  cardKey,
  checkAsk,
  checkForm,
  chooseFormOption,
  chooseFormOther,
  closedRows,
  type InteractionDialog as Dialog,
  emptyAskForm,
  emptyFormDraft,
  type FormBlock,
  type FormChoiceField,
  type FormDraft,
  type FormFieldDraft,
  type FormSpan,
  type FormTextField,
  formatCountdown,
  formFieldQuestions,
  formImageSource,
  formOf,
  type HtmlForm,
  isDialog,
  remainingMs,
  setCustom,
  setFormOther,
  setFormText,
  strings,
  svgImageSource,
  timeoutMs,
  interactionTitle as title,
  toggleOption,
  useAnnounce,
  useCall,
  useWindowState,
} from "omfp/sdk";
import { type FormEvent, Fragment, type ReactNode, useEffect, useRef, useState } from "react";
import { testId, type WidgetProps } from "./props.ts";
import {
  ANSWERED_HERE,
  ASK_FORMS,
  DISMISSED,
  FORM_DRAFTS,
  interactionKey,
  SHOWN_AT,
  useClock,
  useSelectedThread,
} from "./state.ts";

const text = strings.chat.interactions;

export function InteractionsWidget(_props: WidgetProps) {
  const { threadId, view } = useSelectedThread();
  return threadId === undefined || view === undefined ? null : <Interactions threadId={threadId} view={view} />;
}

/** Announces each request when it arrives (UI-092). */
function useRequestAnnouncements(requests: readonly Dialog[]): void {
  const announce = useAnnounce();
  const seen = useRef<ReadonlySet<string>>(new Set(requests.map((request) => request.id)));
  useEffect(() => {
    for (const request of requests) {
      if (!seen.current.has(request.id)) {
        announce(strings.chat.announce.request(title(request)));
      }
    }
    seen.current = new Set(requests.map((request) => request.id));
  }, [announce, requests]);
}

/** The requests and closed rows of one thread. */
export function Interactions({ threadId, view }: { threadId: string; view: ThreadView }) {
  const requests = view.state.interactions.filter(isDialog);
  const [answered] = useWindowState(ANSWERED_HERE);
  const [dismissed, setDismissed] = useWindowState(DISMISSED);
  useRequestAnnouncements(requests);
  const closed = closedRows(
    view.closed,
    (id) => answered.includes(interactionKey(threadId, id)),
    (id) => dismissed.includes(interactionKey(threadId, id)),
  );
  if (requests.length === 0 && closed.length === 0) {
    return null;
  }
  return (
    <section className="omfp-interactions" data-testid="interactions" aria-label={text.label}>
      {requests.map((request, index) => (
        <InteractionCard key={cardKey(request, index)} threadId={threadId} request={request} first={index === 0} />
      ))}
      {closed.map((row) => (
        <div key={`closed-${row.id}`} className="omfp-interaction-closed" data-testid="interaction-closed">
          <ResolvedLine outcome="answered" question={row.title} answer={row.text} />
          <IconButton
            label={text.dismissClosed}
            icon="×"
            data-testid="interaction-closed-dismiss"
            onPress={() => setDismissed((previous) => [...previous, interactionKey(threadId, row.id)])}
          />
        </div>
      ))}
    </section>
  );
}

/** Answers a request from this window: busy until the daemon accepted it, retryable when it refused. */
function useRespond(threadId: string, interactionId: string): [boolean, (response: InteractionResponse) => void] {
  const call = useCall();
  const key = interactionKey(threadId, interactionId);
  const [answered, setAnswered] = useWindowState(ANSWERED_HERE);
  const respond = (response: InteractionResponse) => {
    setAnswered((previous) => [...previous, key]);
    call("interaction.respond", { threadId, interactionId, response }).catch(() =>
      setAnswered((previous) => previous.filter((answeredKey) => answeredKey !== key)),
    );
  };
  return [answered.includes(key), respond];
}

/** When the request was first shown in this window (kept across remounts for its countdown). */
function useShownAt(key: string): number {
  const clock = useClock();
  const [shown, setShown] = useWindowState(SHOWN_AT);
  const [at] = useState(() => shown[key] ?? clock.now());
  useEffect(() => {
    setShown((previous) => (previous[key] === undefined ? { ...previous, [key]: at } : previous));
  }, [at, key, setShown]);
  return at;
}

interface BodyProps<R> {
  request: R;
  disabled: boolean;
  onRespond: (response: InteractionResponse) => void;
}

function InteractionCard({ threadId, request, first }: { threadId: string; request: Dialog; first: boolean }) {
  const [answered, onRespond] = useRespond(threadId, request.id);
  const shownAt = useShownAt(interactionKey(threadId, request.id));
  const heading = title(request);
  // The first line sits on the card's border (UI-187); the rest (an approval's command) opens the body.
  const [headline = heading, ...details] = heading.split("\n");
  const timeout = timeoutMs(request);
  const body = { disabled: answered, onRespond };
  const cardRef = useRef<HTMLElement>(null);
  // The first request's first control takes the focus (UI-070).
  useEffect(() => {
    if (first) {
      cardRef.current?.querySelector<HTMLElement>("input, textarea, button")?.focus();
    }
  }, [first]);
  return (
    <KitInteractionCard
      ref={cardRef}
      className="omfp-interaction"
      data-testid="interaction"
      data-kind={request.method}
      aria-busy={answered}
      aria-label={heading}
      title={headline}
      meta={timeout === undefined ? undefined : <Countdown timeout={timeout} shownAt={shownAt} />}
    >
      {details.length > 0 && (
        <p className="omfp-interaction__detail" data-testid="interaction-detail">
          {details.join("\n")}
        </p>
      )}
      {request.method === "select" && <SelectBody request={request} {...body} />}
      {request.method === "confirm" && <ConfirmBody request={request} {...body} />}
      {(request.method === "input" || request.method === "editor") && <TextBody request={request} {...body} />}
      {request.method === "ask" && (
        <AskBody formKey={interactionKey(threadId, request.id)} request={request} {...body} />
      )}
      {request.method === "form" && (
        <FormBody formKey={interactionKey(threadId, request.id)} request={request} {...body} />
      )}
    </KitInteractionCard>
  );
}

/** Time left before the request times out, ticking every second (UI-073). */
function Countdown({ timeout, shownAt }: { timeout: number; shownAt: number }) {
  const clock = useClock();
  const [now, setNow] = useState(() => Math.max(clock.now(), shownAt));
  const remaining = remainingMs(timeout, shownAt, now);
  const expired = remaining === 0;
  useEffect(() => (expired ? undefined : clock.every(1000, () => setNow(clock.now()))), [clock, expired]);
  return (
    <span
      className="omfp-request__countdown omfp-countdown"
      role="timer"
      aria-label={text.timeLeft}
      data-testid="countdown"
      data-expired={expired}
      data-low={remaining < LOW_MS || undefined}
    >
      <Icon name="clock" />
      {formatCountdown(remaining)}
    </span>
  );
}

/** Below this the countdown turns error (UI-164). */
const LOW_MS = 30_000;

function DismissButton({ disabled, onRespond }: Pick<BodyProps<unknown>, "disabled" | "onRespond">) {
  return (
    <Button
      variant="quiet"
      data-testid="interaction-dismiss"
      isDisabled={disabled}
      onPress={() => onRespond({ cancelled: true })}
    >
      {text.dismiss}
    </Button>
  );
}

/**
 * The options form one toolbar (arrow keys move between them, Tab leaves it for Dismiss, UI-092). Only an approval's
 * first option (Approve) is primary: an `ask`'s options are equal choices omp never ranked (UI-450).
 */
function SelectBody({ request, disabled, onRespond }: BodyProps<SelectUiRequest>) {
  const approval = request.title.startsWith(APPROVAL_TITLE);
  return (
    <div className="omfp-request__actions">
      <Toolbar className="omfp-request__actions" aria-label={request.title} data-testid="interaction-options">
        {request.options.map((option, index) => (
          <Button
            // Options are fixed for the request's lifetime.
            // biome-ignore lint/suspicious/noArrayIndexKey: see above
            key={index}
            data-testid="interaction-option"
            variant={approval && index === 0 ? "primary" : "secondary"}
            isDisabled={disabled}
            onPress={() => onRespond({ value: option })}
          >
            {option}
            {request.optionDetails?.[index]?.description !== undefined && (
              <span className="omfp-interaction__description">{request.optionDetails[index]?.description}</span>
            )}
          </Button>
        ))}
      </Toolbar>
      <DismissButton disabled={disabled} onRespond={onRespond} />
    </div>
  );
}

/** How omp titles a tool approval's select (`Allow tool: bash`, its `approvalHeader`). */
const APPROVAL_TITLE = "Allow tool: ";

function ConfirmBody({ request, disabled, onRespond }: BodyProps<ConfirmUiRequest>) {
  return (
    <>
      <p className="omfp-interaction__message">{request.message}</p>
      <div className="omfp-request__actions">
        <Button
          data-testid="interaction-confirm"
          variant="primary"
          isDisabled={disabled}
          onPress={() => onRespond({ confirmed: true })}
        >
          {text.yes}
        </Button>
        <Button
          variant="danger"
          data-testid="interaction-deny"
          isDisabled={disabled}
          onPress={() => onRespond({ confirmed: false })}
        >
          {text.no}
        </Button>
      </div>
    </>
  );
}

function submitted(event: FormEvent, respond: () => void): void {
  event.preventDefault();
  respond();
}

/** An `input` request's line (a password field when secret) or an `editor` request's text, sent with Submit. */
function TextBody({ request, disabled, onRespond }: BodyProps<Extract<Dialog, { method: "input" | "editor" }>>) {
  const [value, setValue] = useState(request.method === "editor" ? (request.prefill ?? "") : "");
  const field = { label: request.title, hideLabel: true, value, onChange: setValue, isDisabled: disabled };
  return (
    <form className="omfp-interaction__form" onSubmit={(event) => submitted(event, () => onRespond({ value }))}>
      {request.method === "editor" ? (
        <TextArea {...field} rows={8} inputProps={testId("interaction-editor")} />
      ) : (
        <TextField
          {...field}
          {...(request.placeholder === undefined ? {} : { placeholder: request.placeholder })}
          type={request.secret === true ? "password" : "text"}
          inputProps={testId("interaction-input")}
        />
      )}
      <Button type="submit" variant="primary" data-testid="interaction-submit" isDisabled={disabled}>
        {text.submit}
      </Button>
      <DismissButton disabled={disabled} onRespond={onRespond} />
    </form>
  );
}

/** The `ask` form: every question takes options and free text; single/multi rules are checked before sending. */
function AskBody({ formKey, request, disabled, onRespond }: BodyProps<AskUiRequest> & { formKey: string }) {
  const [forms, setForms] = useWindowState(ASK_FORMS);
  const form = forms[formKey] ?? emptyAskForm(request);
  const edit = (change: (form: AskForm) => AskForm) =>
    setForms((previous) => ({ ...previous, [formKey]: change(previous[formKey] ?? emptyAskForm(request)) }));
  const checked = checkAsk(form, request);
  return (
    <form
      className="omfp-interaction__form omfp-ask"
      data-testid="ask-form"
      onSubmit={(event) =>
        submitted(event, () => {
          if (checked.ok) {
            onRespond({ answers: checked.answers });
          }
        })
      }
    >
      {request.questions.map((question, index) => {
        const draft = form[index] ?? { selected: [], custom: "" };
        const legend = `${question.header === undefined ? "" : `${question.header}: `}${question.question}`;
        const options = question.options.map((option, position) => {
          const recommended = question.recommended === position;
          const label = (
            <>
              <OptionText label={option.label} description={option.description} />
              {recommended && <span className="omfp-ask__tag">{text.recommended}</span>}
            </>
          );
          return question.multi ? (
            <Checkbox
              key={option.label}
              data-testid="ask-option"
              value={option.label}
              isSelected={draft.selected.includes(option.label)}
              isDisabled={disabled}
              onChange={() => edit((previous) => toggleOption(previous, request, index, option.label))}
            >
              {label}
            </Checkbox>
          ) : (
            <Radio key={option.label} data-testid="ask-option" value={option.label} isDisabled={disabled}>
              {label}
            </Radio>
          );
        });
        return (
          <fieldset
            key={question.id}
            className="omfp-ask__question"
            data-testid="ask-question"
            data-multi={question.multi}
          >
            <legend>
              {legend}
              <span className="omfp-ask__rule">{question.multi ? text.ruleMulti : text.ruleSingle}</span>
            </legend>
            {question.multi ? (
              <div className="omfp-ask__options">{options}</div>
            ) : (
              <RadioGroup
                label={legend}
                hideLabel
                value={draft.selected[0] ?? null}
                isDisabled={disabled}
                onChange={(label) => edit((previous) => toggleOption(previous, request, index, label))}
              >
                {options}
              </RadioGroup>
            )}
            <TextField
              label={text.otherAnswerTo(question.question)}
              hideLabel
              placeholder={text.otherAnswer}
              value={draft.custom}
              isDisabled={disabled}
              onChange={(value) => edit((previous) => setCustom(previous, request, index, value))}
              inputProps={testId("ask-custom")}
            />
          </fieldset>
        );
      })}
      {!checked.ok && (
        <p className="omfp-ask__error" data-testid="ask-error">
          {checked.error}
        </p>
      )}
      <Button type="submit" variant="primary" data-testid="interaction-submit" isDisabled={disabled || !checked.ok}>
        {text.sendAnswers}
      </Button>
      <DismissButton disabled={disabled} onRespond={onRespond} />
    </form>
  );
}

type FormRequest = Extract<Dialog, { method: "form" }>;

/** The element each inline style draws with. */
const SPAN_TAG = { code: "code", strong: "strong", em: "em" } as const;

/** Inline text of a form: `<code>`, `<strong>` and `<em>` runs, `<br>` as line breaks. */
function Spans({ spans }: { spans: readonly FormSpan[] }) {
  return spans.map((span, index) => {
    const Tag = span.style === undefined ? "span" : SPAN_TAG[span.style];
    return (
      // Spans and their lines are fixed for the request's lifetime.
      // biome-ignore lint/suspicious/noArrayIndexKey: see above
      <Tag key={index}>
        {span.text.split("\n").map((line, at) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: see above
          <Fragment key={at}>
            {at > 0 && <br />}
            {line}
          </Fragment>
        ))}
      </Tag>
    );
  });
}

/**
 * omp's HTML `form` (UI-074): its sections, context and fields drawn with the kit's controls. Submit sends `{ data }`
 * only once every answer passes the form's rules (omp fails the whole call on an invalid reply); before that it marks
 * every invalid field and moves the focus to the first one.
 */
function FormBody({ formKey, request, disabled, onRespond }: BodyProps<FormRequest> & { formKey: string }) {
  const form = formOf(request);
  const [drafts, setDrafts] = useWindowState(FORM_DRAFTS);
  const draft = drafts[formKey] ?? emptyFormDraft(form);
  const edit = (change: (draft: FormDraft) => FormDraft) =>
    setDrafts((previous) => ({ ...previous, [formKey]: change(previous[formKey] ?? emptyFormDraft(form)) }));
  const checked = checkForm(form, draft);
  const errors = draft.attempted && !checked.ok ? checked.errors : [];
  const questions = formFieldQuestions(form);
  const formRef = useRef<HTMLFormElement>(null);
  const submit = () => {
    if (checked.ok) {
      onRespond({ data: checked.data });
      return;
    }
    edit((previous) => ({ ...previous, attempted: true }));
    const first = checked.errors.findIndex((error) => error !== undefined);
    formRef.current
      ?.querySelector<HTMLElement>(`[data-field="${first}"] :is(input, textarea, button):not(:disabled)`)
      ?.focus();
  };
  const fieldProps = (field: FormChoiceField | FormTextField) => {
    const index = form.fields.indexOf(field);
    const question = questions.get(field) ?? field.name;
    return { index, question, draft: draft.fields[index], error: errors[index], disabled, edit };
  };
  return (
    <form
      ref={formRef}
      noValidate
      className="omfp-interaction__form omfp-form"
      data-testid="form-form"
      onSubmit={(event) => submitted(event, submit)}
    >
      {form.sections.map((section, sectionIndex) => (
        <fieldset
          // Sections are fixed for the request's lifetime.
          // biome-ignore lint/suspicious/noArrayIndexKey: see above
          key={sectionIndex}
          className="omfp-form__section"
          data-testid="form-section"
        >
          {section.title.length > 0 && (
            <legend className="omfp-form__legend">
              <Spans spans={section.title} />
            </legend>
          )}
          {section.blocks.map((block, blockIndex) => {
            if (block.kind !== "field") {
              // Blocks are fixed for the request's lifetime.
              // biome-ignore lint/suspicious/noArrayIndexKey: see above
              return <FormContext key={blockIndex} block={block} />;
            }
            const props = fieldProps(block.field);
            return block.field.kind === "text" ? (
              <FormTextInput key={`field-${props.index}`} field={block.field} {...props} />
            ) : (
              <FormChoice key={`field-${props.index}`} form={form} field={block.field} {...props} />
            );
          })}
        </fieldset>
      ))}
      {errors.length > 0 && (
        <p className="omfp-ask__error" data-testid="form-error" role="alert">
          {strings.chat.form.fixErrors}
        </p>
      )}
      <div className="omfp-request__actions">
        <Button type="submit" variant="primary" data-testid="interaction-submit" isDisabled={disabled}>
          {text.submit}
        </Button>
        <DismissButton disabled={disabled} onRespond={onRespond} />
      </div>
    </form>
  );
}

/** A context block of a form: text and headings, lists, preformatted text, tables, pictures. */
function FormContext({ block }: { block: Exclude<FormBlock, { kind: "field" }> }) {
  switch (block.kind) {
    case "text":
      return block.heading ? (
        <h4 className="omfp-form__heading" data-testid="form-text">
          <Spans spans={block.spans} />
        </h4>
      ) : (
        <p className="omfp-form__text" data-testid="form-text">
          <Spans spans={block.spans} />
        </p>
      );
    case "list": {
      const items = block.items.map((item, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: list items are fixed for the request's lifetime
        <li key={index}>
          <Spans spans={item} />
        </li>
      ));
      return block.ordered ? (
        <ol className="omfp-form__list" data-testid="form-list">
          {items}
        </ol>
      ) : (
        <ul className="omfp-form__list" data-testid="form-list">
          {items}
        </ul>
      );
    }
    case "pre":
      return (
        <pre className="omfp-form__pre" data-testid="form-pre">
          {block.text}
        </pre>
      );
    case "table":
      return <FormTable rows={block.rows} headerRows={block.headerRows} />;
    case "image": {
      const source = formImageSource(block.src);
      return source === undefined ? (
        <p className="omfp-form__text" data-testid="form-image-text">
          {strings.chat.form.image(block.alt || block.src)}
        </p>
      ) : (
        <img className="omfp-form__image" data-testid="form-image" src={source} alt={block.alt} />
      );
    }
    case "svg":
      return (
        <img className="omfp-form__image" data-testid="form-image" src={svgImageSource(block.markup)} alt={block.alt} />
      );
  }
}

function FormTable({ rows, headerRows }: { rows: readonly FormSpan[][][]; headerRows: number }) {
  const row = (cells: readonly FormSpan[][], header: boolean, index: number) => (
    <tr key={index}>
      {cells.map((cell, at) =>
        header ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: cells are fixed for the request's lifetime
          <th key={at}>
            <Spans spans={cell} />
          </th>
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: cells are fixed for the request's lifetime
          <td key={at}>
            <Spans spans={cell} />
          </td>
        ),
      )}
    </tr>
  );
  return (
    <table className="omfp-form__table" data-testid="form-table">
      {headerRows > 0 && <thead>{rows.slice(0, headerRows).map((cells, index) => row(cells, true, index))}</thead>}
      <tbody>{rows.slice(headerRows).map((cells, index) => row(cells, false, index))}</tbody>
    </table>
  );
}

interface FormFieldProps<F> {
  field: F;
  index: number;
  /** The question the field answers: its accessible name. */
  question: string;
  draft: FormFieldDraft | undefined;
  error: string | undefined;
  disabled: boolean;
  edit: (change: (draft: FormDraft) => FormDraft) => void;
}

function FieldError({ error }: { error: string | undefined }) {
  return error === undefined ? null : (
    <p className="omfp-form__error" data-testid="form-field-error">
      <Icon name="circle-alert" />
      {error}
    </p>
  );
}

/** Input types the browser's own text input keeps as text; any other (`number`, `date`, …) is a plain text input. */
const TEXT_TYPES: Readonly<Record<string, true>> = { email: true, password: true, search: true, tel: true, url: true };

function FormTextInput({ field, index, question, draft, error, disabled, edit }: FormFieldProps<FormTextField>) {
  const common = {
    label: question,
    value: draft?.kind === "text" ? draft.text : field.value,
    onChange: (value: string) => edit((previous) => setFormText(previous, index, value)),
    isDisabled: disabled,
    isInvalid: error !== undefined,
    isRequired: field.required,
    ...(field.placeholder === "" ? {} : { placeholder: field.placeholder }),
  };
  return (
    <div className="omfp-form__field" data-testid="form-field" data-field={index} data-name={field.name}>
      {field.multiline ? (
        <TextArea {...common} rows={field.rows} inputProps={testId("form-input")} />
      ) : (
        <TextField
          {...common}
          type={TEXT_TYPES[field.type] === true ? field.type : "text"}
          inputProps={{
            ...testId("form-input"),
            ...(field.type === "number" || field.type === "range" ? { inputMode: "decimal" as const } : {}),
          }}
        />
      )}
      <FieldError error={error} />
    </div>
  );
}

/** `RadioGroup` value of a radio group's added "Other" answer (option values are their indices). */
const OTHER = "other";

/**
 * A choice field: a single `<select>` as the kit's dropdown, a radio group as radios, a checkbox group, a lone
 * checkbox or a multi-select as checkboxes; groups offering "Other" add that choice and its text field.
 */
function FormChoice({
  form,
  field,
  index,
  question,
  draft,
  error,
  disabled,
  edit,
}: FormFieldProps<FormChoiceField> & { form: HtmlForm }) {
  const choice: Extract<FormFieldDraft, { kind: "choice" }> =
    draft?.kind === "choice" ? draft : { kind: "choice", checked: [], otherChosen: false, other: "" };
  const choose = (option: number) => edit((previous) => chooseFormOption(form, previous, index, option));
  let control: ReactNode;
  if (field.select && !field.multi) {
    control = (
      <Select
        label={question}
        items={field.options.map((option, at) => ({
          id: String(at),
          label: option.label.map((span) => span.text).join(""),
          isDisabled: option.disabled,
        }))}
        selectedKey={choice.checked[0] === undefined ? null : String(choice.checked[0])}
        onSelectionChange={(key) => (key === null ? undefined : choose(Number(key)))}
        isDisabled={disabled}
        data-testid="form-select"
      />
    );
  } else if (field.multi) {
    control = (
      // A multi-select's own label is its visible question, as a single select's is; a checkbox group's question is
      // the text around it, so the group only names itself to assistive technology.
      <fieldset
        className="omfp-ask__options omfp-form__group"
        {...(field.select && field.label.length > 0 ? {} : { "aria-label": question })}
      >
        {field.select && field.label.length > 0 && (
          <legend className="omfp-form__legend" data-testid="form-field-label">
            <Spans spans={field.label} />
          </legend>
        )}
        {field.options.map((option, at) => (
          <Checkbox
            // Options are fixed for the request's lifetime.
            // biome-ignore lint/suspicious/noArrayIndexKey: see above
            key={at}
            data-testid="form-option"
            data-value={option.value}
            value={String(at)}
            isSelected={choice.checked.includes(at)}
            isDisabled={disabled || option.disabled}
            isInvalid={error !== undefined}
            onChange={() => choose(at)}
          >
            <Spans spans={option.label} />
          </Checkbox>
        ))}
        {field.other && (
          <Checkbox
            data-testid="form-other-choice"
            value={OTHER}
            isSelected={choice.otherChosen}
            isDisabled={disabled}
            onChange={(selected) => edit((previous) => chooseFormOther(form, previous, index, selected))}
          >
            {strings.chat.form.other}
          </Checkbox>
        )}
      </fieldset>
    );
  } else {
    control = (
      <RadioGroup
        label={question}
        hideLabel
        value={choice.otherChosen ? OTHER : choice.checked[0] === undefined ? null : String(choice.checked[0])}
        isDisabled={disabled}
        isInvalid={error !== undefined}
        onChange={(value) =>
          edit((previous) =>
            value === OTHER
              ? chooseFormOther(form, previous, index, true)
              : chooseFormOption(form, previous, index, Number(value)),
          )
        }
      >
        {field.options.map((option, at) => (
          <Radio
            // Options are fixed for the request's lifetime.
            // biome-ignore lint/suspicious/noArrayIndexKey: see above
            key={at}
            data-testid="form-option"
            data-value={option.value}
            value={String(at)}
            isDisabled={option.disabled}
          >
            <Spans spans={option.label} />
          </Radio>
        ))}
        {field.other && (
          <Radio data-testid="form-other-choice" value={OTHER}>
            {strings.chat.form.other}
          </Radio>
        )}
      </RadioGroup>
    );
  }
  return (
    <div
      className="omfp-form__field"
      data-testid="form-field"
      data-field={index}
      data-name={field.name}
      data-multi={field.multi}
    >
      {control}
      {field.other && (
        <TextField
          label={strings.chat.form.otherAnswerTo(question)}
          hideLabel
          placeholder={strings.chat.form.otherPlaceholder}
          value={choice.other}
          isDisabled={disabled}
          onChange={(value) => edit((previous) => setFormOther(form, previous, index, value))}
          inputProps={testId("form-other")}
        />
      )}
      <FieldError error={error} />
    </div>
  );
}
