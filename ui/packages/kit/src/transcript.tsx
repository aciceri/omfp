/**
 * The chat's pieces (UI-171, UI-172): the chat header, messages with their name line, thinking blocks, the stream
 * caret (UI-147), the composer box and queued-message chips. Presentational: the app supplies text and behaviour.
 */
import { type HTMLAttributes, type ReactNode, type Ref, useCallback, useId, useRef } from "react";
import { Button as AriaButton } from "react-aria-components";
import { Button, IconButton } from "./button.tsx";
import { cx } from "./class.ts";
import { Icon } from "./icons.tsx";
import { useMarquee } from "./marquee.ts";

export interface ChatHeaderProps extends HTMLAttributes<HTMLElement> {
  /** The bar under the title row: chips, selects. */
  bar?: ReactNode;
  /** The title row: a {@link ChatTitle}, then controls beside it. */
  children?: ReactNode;
  /** The `<header>` (React 19 passes it as a prop). */
  ref?: Ref<HTMLElement>;
}

/** The chat's head over a primary → tertiary → transparent hairline (UI-171, UI-443): a title row, then a bar. */
export function ChatHeader({ bar, className, children, ...props }: ChatHeaderProps) {
  return (
    <header {...props} className={cx("omfp-chat-header", className)}>
      <div className="omfp-chat-header__top">{children}</div>
      {bar !== undefined && <div className="omfp-chat-header__bar">{bar}</div>}
    </header>
  );
}

export interface ChatTitleProps extends HTMLAttributes<HTMLHeadingElement> {
  /** No title yet (a draft, a thread not yet named): italic and muted, no gradient. */
  placeholder?: boolean;
  /** The heading (React 19 passes it as a prop), e.g. to focus it after a rename. */
  ref?: Ref<HTMLHeadingElement>;
}

/** The thread title, the page's heading: `◆` before it, flowing gradient text (UI-171, UI-443). Longer than its room it
 * is cut with an ellipsis; at motion `full` it scrolls to its end and back instead (UI-202). */
export function ChatTitle({ placeholder = false, className, ref, ...props }: ChatTitleProps) {
  const own = useRef<HTMLHeadingElement>(null);
  useMarquee(own);
  const refs = useCallback(
    (node: HTMLHeadingElement | null) => {
      own.current = node;
      if (typeof ref === "function") {
        ref(node);
      } else if (ref !== null && ref !== undefined) {
        ref.current = node;
      }
    },
    [ref],
  );
  return (
    <h1
      {...props}
      ref={refs}
      className={cx("omfp-chat-header__title", !placeholder && "omfp-grad-text", className)}
      data-placeholder={placeholder ? "true" : undefined}
    />
  );
}

export interface MessageProps extends HTMLAttributes<HTMLElement> {
  author: "user" | "assistant";
  /** The name on the name line (after `◆` for the assistant, `▸` for the user). */
  name: ReactNode;
  /**
   * A later message of the same turn (UI-470, UI-471): no name line, a plain block under the turn's first message,
   * which is the turn's one `article`.
   */
  continues?: boolean;
  /** Why the message failed, shown under its body in error. */
  error?: ReactNode;
  /** Buttons acting on the message (e.g. fork from here), at the end of its name line; shown on hover and focus. */
  actions?: ReactNode;
  children?: ReactNode;
}

/**
 * A transcript message (UI-171): name line, then the body (prose, a 2 px tertiary bar for the user); one that
 * `continues` its turn has the body only.
 */
export function Message({
  author,
  name,
  continues = false,
  error,
  actions,
  className,
  children,
  ...props
}: MessageProps) {
  const Element = continues ? "div" : "article";
  return (
    <Element {...props} className={cx("omfp-msg", className)} data-author={author}>
      {!continues && <div className={cx("omfp-msg__name", author === "assistant" && "omfp-grad-text")}>{name}</div>}
      {actions !== undefined && <div className="omfp-msg__actions">{actions}</div>}
      <div className="omfp-msg__body">{children}</div>
      {error !== undefined && (
        <p className="omfp-msg__error" role="alert">
          <Icon name="circle-alert" />
          {error}
        </p>
      )}
    </Element>
  );
}

export type StreamCaretProps = Omit<HTMLAttributes<HTMLSpanElement>, "children">;

/** The block caret after the last streamed character (UI-147): blinks at motion `full`, steady otherwise. */
export function StreamCaret({ className, ...props }: StreamCaretProps) {
  return <span aria-hidden="true" {...props} className={cx("omfp-caret", className)} />;
}

export interface ThinkingProps extends Omit<HTMLAttributes<HTMLDivElement>, "children"> {
  /** The italic label of the head (e.g. "Thinking"). */
  label: ReactNode;
  /** Metadata after the label (duration, length). */
  meta?: ReactNode;
  expanded: boolean;
  onExpandedChange?: (expanded: boolean) => void;
  /** Class names of the head (forced states in the gallery). */
  headClassName?: string;
  /** The thinking text. */
  children?: ReactNode;
}

/** A thinking block (UI-171): a `▸`/`▾` head expanding to italic prose with a rule bar. */
export function Thinking({
  label,
  meta,
  expanded,
  onExpandedChange,
  headClassName,
  className,
  children,
  ...props
}: ThinkingProps) {
  const textId = useId();
  return (
    <div {...props} className={cx("omfp-reasoning", className)} data-expanded={expanded}>
      <AriaButton
        className={cx("omfp-reasoning__head", headClassName)}
        aria-expanded={expanded}
        aria-controls={textId}
        onPress={() => onExpandedChange?.(!expanded)}
        data-testid="thinking-head"
      >
        <span className="omfp-reasoning__glyph" aria-hidden="true" />
        <span className="omfp-reasoning__label">{label}</span>
        {meta !== undefined && <span className="omfp-reasoning__meta">{meta}</span>}
      </AriaButton>
      <div id={textId} className="omfp-prose omfp-reasoning__text" hidden={!expanded}>
        {expanded && children}
      </div>
    </div>
  );
}

export interface ComposerProps extends HTMLAttributes<HTMLDivElement> {
  /** `insert` draws the gradient border and the focus glow (UI-172). */
  mode?: "normal" | "insert";
  disabled?: boolean;
  /** The input (give a `<textarea>` the `omfp-compose__input` class), then a {@link ComposerBar}. */
  children?: ReactNode;
}

/** The message box (UI-172). */
export function Composer({ mode = "normal", disabled = false, className, ...props }: ComposerProps) {
  return (
    <div {...props} className={cx("omfp-compose", className)} data-mode={mode} data-disabled={disabled || undefined} />
  );
}

export interface ComposerBarProps extends HTMLAttributes<HTMLDivElement> {
  /** What the bar starts with: attachment chips, key hints ({@link ComposerHint}). They give way to the actions: one
   * that does not fit on the row is not shown at all, so the bar stays one row (UI-480). */
  hints?: ReactNode;
  /** The actions, the primary one last: one group at the bar's end (UI-480). */
  children?: ReactNode;
}

/** The row under the composer's input (UI-172, UI-480): the hints at its start, the actions at its end, the primary
 * one last, so it sits at the composer's bottom-right whatever else shows. */
export function ComposerBar({ hints, className, children, ...props }: ComposerBarProps) {
  return (
    <div {...props} className={cx("omfp-compose__bar", className)}>
      {hints !== undefined && <div className="omfp-compose__hints">{hints}</div>}
      <div className="omfp-compose__actions">{children}</div>
    </div>
  );
}

export type ComposerHintProps = HTMLAttributes<HTMLSpanElement>;

/** A key hint in the composer's bar (key chips with a word). */
export function ComposerHint({ className, ...props }: ComposerHintProps) {
  return <span {...props} className={cx("omfp-compose__hint", className)} />;
}

export type QueueListProps = HTMLAttributes<HTMLUListElement>;

/** The queued messages above the composer (UI-172, UI-445): a wrapping list of {@link QueueChip}s; label it. */
export function QueueList({ className, ...props }: QueueListProps) {
  return <ul {...props} className={cx("omfp-queue", className)} />;
}

export interface QueueChipProps extends Omit<HTMLAttributes<HTMLLIElement>, "children"> {
  kind: "follow-up" | "steer";
  /** The kind's word (`follow-up` / `steer` by default). */
  label?: ReactNode;
  /** The queued text, cut with an ellipsis. */
  children: ReactNode;
  /** An action on it waits for an answer: faded, its actions disabled. */
  busy?: boolean;
  /** Shows the promote action (a follow-up sent as a steer now). */
  onPromote?: () => void;
  promoteLabel?: string;
  onRemove?: () => void;
  removeLabel?: string;
}

/** A queued follow-up or steering message in a {@link QueueList}: a dashed chip (UI-172, UI-445). */
export function QueueChip({
  kind,
  label,
  children,
  busy = false,
  onPromote,
  promoteLabel = "Steer now",
  onRemove,
  removeLabel = "Remove",
  className,
  ...props
}: QueueChipProps) {
  return (
    <li {...props} className={cx("omfp-queue-chip", className)} data-kind={kind} aria-busy={busy}>
      <span className="omfp-queue-chip__kind">{label ?? kind}</span>
      <span className="omfp-queue-chip__text">{children}</span>
      {onPromote !== undefined && (
        <Button size="sm" variant="quiet" data-testid="queue-promote" isDisabled={busy} onPress={onPromote}>
          {promoteLabel}
        </Button>
      )}
      {onRemove !== undefined && (
        <IconButton
          label={removeLabel}
          icon={<Icon name="x" />}
          onPress={onRemove}
          isDisabled={busy}
          data-testid="queue-remove"
        />
      )}
    </li>
  );
}
