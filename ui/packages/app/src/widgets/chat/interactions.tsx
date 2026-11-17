/**
 * `omfp:interactions`: the selected thread's pending requests (UI-070) — approval selects, confirms, inputs, editors
 * and `ask` forms (UI-071) with their countdown (UI-073) — and rows for requests another client or omp closed
 * (UI-072). The first control of the first request takes the focus.
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
  TextArea,
  TextField,
  Toolbar,
} from "omfp/kit";
import {
  type AskForm,
  cardKey,
  checkAsk,
  closedRows,
  type InteractionDialog as Dialog,
  emptyAskForm,
  formatCountdown,
  isDialog,
  remainingMs,
  setCustom,
  strings,
  timeoutMs,
  interactionTitle as title,
  toggleOption,
  useAnnounce,
  useCall,
  useWindowState,
} from "omfp/sdk";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { testId, type WidgetProps } from "./props.ts";
import { ANSWERED_HERE, ASK_FORMS, DISMISSED, interactionKey, SHOWN_AT, useClock, useSelectedThread } from "./state.ts";

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
