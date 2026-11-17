/**
 * Border boxes with their title on the top border and labelled rules (UI-187), tool cards (UI-163), approval and ask
 * cards with their resolved line (UI-164).
 */
import { type HTMLAttributes, type ReactNode, type Ref, useId } from "react";
import { Button as AriaButton } from "react-aria-components";
import { cx } from "./class.ts";
import { Icon } from "./icons.tsx";
import { StateMark } from "./marks.tsx";

/** The heading level of a border box's title or a rule's label. */
export type BoxHeadingLevel = 2 | 3 | 4 | 5 | 6;

/** The line of a box's top border after its title, up to the end part or the right corner. */
function BorderLine() {
  return <span className="omfp-bbox__line" aria-hidden="true" />;
}

export interface BorderBoxProps extends Omit<HTMLAttributes<HTMLElement>, "title"> {
  /** The title, set on the top border like a fieldset legend. */
  title: ReactNode;
  /** A heading of this level holds the title; a plain span without one. */
  level?: BoxHeadingLevel;
  /** The title element's id, e.g. for `aria-labelledby`. */
  titleId?: string;
  /** Metadata at the right end of the top border (`─ 1/2 ⏱ 0:42 ─╮`). */
  end?: ReactNode;
  /** Class of the body under the title. */
  bodyClassName?: string;
  /** The box's `<section>` (React 19 passes it as a prop). */
  ref?: Ref<HTMLElement> | undefined;
  children?: ReactNode;
}

/**
 * A box whose title sits on its top border (UI-187): `╭─ title ──────╮`, the line never under the text; the body below
 * has the box's border on its sides and bottom.
 */
export function BorderBox({
  title,
  level,
  titleId,
  end,
  bodyClassName,
  className,
  children,
  ...props
}: BorderBoxProps) {
  const Title = level === undefined ? "span" : (`h${level}` as const);
  return (
    <section {...props} className={cx("omfp-bbox", className)}>
      <div className="omfp-bbox__head">
        <Title id={titleId} className="omfp-bbox__label">
          {title}
        </Title>
        <BorderLine />
        {end !== undefined && <span className="omfp-bbox__end">{end}</span>}
      </div>
      <div className={cx("omfp-bbox__body", bodyClassName)}>{children}</div>
    </section>
  );
}

export interface BoxRuleProps extends Omit<HTMLAttributes<HTMLHeadingElement>, "children"> {
  /** The section's name, on the rule (`├─ Output ──┤`). */
  label: ReactNode;
  level?: BoxHeadingLevel;
}

/** A labelled rule across a border box's body, opening its next section (UI-187): `├─ Output ──────┤`. */
export function BoxRule({ label, level = 3, className, ...props }: BoxRuleProps) {
  const Heading = `h${level}` as const;
  return (
    <Heading {...props} className={cx("omfp-bbox__rule", className)}>
      <span className="omfp-bbox__rule-label">{label}</span>
    </Heading>
  );
}

/** Where a tool call is. */
export type ToolStatus = "running" | "done" | "failed";

export interface ToolCardProps extends Omit<HTMLAttributes<HTMLElement>, "title"> {
  status: ToolStatus;
  /** The tool's name (`bash`, `edit`, …). */
  name: ReactNode;
  /** The main argument (a command, a path), cut with an ellipsis. */
  arg?: ReactNode;
  /** Right-aligned metadata: duration, exit code, line counts. */
  meta?: ReactNode;
  /** Whether the body shows; the head toggles it when there is a body. */
  expanded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
  /** `data-testid` of the head (`tool-head` by default). */
  headTestId?: string;
  /** Class names of the head (forced states in the gallery). */
  headClassName?: string;
  /** The body: a tool renderer (output, diff, todo list). Without one the head is not clickable. */
  children?: ReactNode;
}

/**
 * A tool call (UI-163): a head with the status mark (`●` ok or error, omp's status spinner while running, UI-216), the
 * name, the main argument and metadata; the head toggles the body (no chevron). While the body shows, the head sits on
 * the card's top border (UI-187) and the body's sections are split by `BoxRule`s; collapsed or without a body the card
 * is one row. Running cards shimmer and breathe at motion `full`.
 */
export function ToolCard({
  status,
  name,
  arg,
  meta,
  expanded = false,
  onExpandedChange,
  headTestId = "tool-head",
  headClassName,
  className,
  children,
  ...props
}: ToolCardProps) {
  const bodyId = useId();
  const hasBody = children !== undefined && children !== null && children !== false;
  const boxed = hasBody && expanded;
  const head = (
    <>
      <span className={cx("omfp-tool__label", boxed && "omfp-bbox__label")}>
        <StateMark state={status} spinner="status" />
        <span className="omfp-tool__name">{name}</span>
        {arg !== undefined && <span className="omfp-tool__arg">{arg}</span>}
        {meta !== undefined && <span className="omfp-tool__meta">{meta}</span>}
      </span>
      {boxed && <BorderLine />}
    </>
  );
  const headClass = cx("omfp-tool__head", boxed && "omfp-bbox__head", headClassName);
  return (
    <section {...props} className={cx("omfp-tool", boxed && "omfp-bbox", className)} data-status={status}>
      {hasBody ? (
        // The head is the card's heading (UI-187): the body's `BoxRule`s are its sub-headings.
        <h2 className="omfp-tool__heading">
          <AriaButton
            className={headClass}
            aria-expanded={expanded}
            aria-controls={bodyId}
            onPress={() => onExpandedChange?.(!expanded)}
            data-testid={headTestId}
          >
            {head}
          </AriaButton>
        </h2>
      ) : (
        <div className={headClass} data-testid={headTestId}>
          {head}
        </div>
      )}
      {hasBody && (
        <div id={bodyId} className={cx("omfp-tool__body", boxed && "omfp-bbox__body")} hidden={!expanded}>
          {expanded && children}
        </div>
      )}
    </section>
  );
}

export interface TodoItem {
  readonly text: ReactNode;
  /** omp's status: `pending`, `in_progress`, `completed`, `abandoned`, or another it may add. */
  readonly status: string;
  /** The status in words, for assistive technology (visually hidden). */
  readonly statusLabel?: string;
}

export interface TodoListProps extends Omit<HTMLAttributes<HTMLUListElement>, "children"> {
  items: readonly TodoItem[];
  /** `data-testid` of each item. */
  itemTestId?: string;
}

/**
 * A todo list (UI-163, UI-447): a `●` per task in omp's statuses, completed in ok and struck through, in progress in
 * primary, abandoned muted and struck through.
 */
export function TodoList({ items, itemTestId, className, ...props }: TodoListProps) {
  return (
    <ul {...props} className={cx("omfp-todo", className)}>
      {items.map((item, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: a todo list is shown as given
        <li key={index} className="omfp-todo__item" data-status={item.status} data-testid={itemTestId}>
          <span className="omfp-todo__mark" aria-hidden="true">
            ●
          </span>
          <span className="omfp-todo__text">{item.text}</span>
          {item.statusLabel !== undefined && <span className="omfp-visually-hidden">{item.statusLabel}</span>}
        </li>
      ))}
    </ul>
  );
}

export interface InteractionCardProps extends Omit<HTMLAttributes<HTMLElement>, "title"> {
  /** The question or the approval's title, after the `◆`. */
  title: ReactNode;
  /** Right-aligned metadata before the countdown, e.g. the step `2/3`. */
  meta?: ReactNode;
  /** Seconds left before the request times out; shown with a clock. */
  countdown?: string;
  /** The countdown is about to run out: it turns error. */
  low?: boolean;
  /** The card's `<section>` (React 19 passes it as a prop). */
  ref?: Ref<HTMLElement>;
  children?: ReactNode;
}

/**
 * A pending approval or `ask` question (UI-164): a dashed warn box whose `◆` title sits on its top border (UI-187),
 * the step and countdown at the border's right end.
 */
export function InteractionCard({
  title,
  meta,
  countdown,
  low = false,
  className,
  children,
  ...props
}: InteractionCardProps) {
  return (
    <section {...props} className={cx("omfp-request", "omfp-bbox", className)}>
      <header className="omfp-request__head omfp-bbox__head">
        <span className="omfp-request__title omfp-bbox__label">
          <span className="omfp-request__glyph" aria-hidden="true">
            ◆{" "}
          </span>
          {title}
        </span>
        <BorderLine />
        {(meta !== undefined || countdown !== undefined) && (
          <span className="omfp-request__meta omfp-bbox__end">
            {meta}
            {countdown !== undefined && (
              <span className="omfp-request__countdown" data-low={low || undefined}>
                <Icon name="clock" />
                {countdown}
              </span>
            )}
          </span>
        )}
      </header>
      <div className="omfp-request__body omfp-bbox__body">{children}</div>
    </section>
  );
}

export interface ApprovalCommandProps extends Omit<HTMLAttributes<HTMLPreElement>, "children"> {
  command: string;
}

/** The command an approval asks about, after a muted `$` (UI-164). */
export function ApprovalCommand({ command, className, ...props }: ApprovalCommandProps) {
  return (
    <pre {...props} className={cx("omfp-request__code", className)}>
      <span className="omfp-request__prompt" aria-hidden="true">
        ${" "}
      </span>
      {command}
    </pre>
  );
}

export type InteractionActionsProps = HTMLAttributes<HTMLDivElement>;

/** The buttons row of an approval or ask card. */
export function InteractionActions({ className, ...props }: InteractionActionsProps) {
  return <div {...props} className={cx("omfp-request__actions", className)} />;
}

export interface OptionTextProps {
  label: ReactNode;
  description?: ReactNode;
}

/** An ask option's text: the label in the code family, the description in prose italic (UI-164). */
export function OptionText({ label, description }: OptionTextProps) {
  return (
    <span className="omfp-option-text">
      <span className="omfp-option-text__label">{label}</span>
      {description !== undefined && <span className="omfp-prose omfp-option-text__description">{description}</span>}
    </span>
  );
}

export interface ResolvedLineProps extends Omit<HTMLAttributes<HTMLDivElement>, "children"> {
  /** `denied` shows an error x, anything else an ok check. */
  outcome: "approved" | "denied" | "answered";
  question: ReactNode;
  answer: ReactNode;
}

/** What replaces an answered card (UI-164): a check or x, the question cut with an ellipsis, the answer. */
export function ResolvedLine({ outcome, question, answer, className, ...props }: ResolvedLineProps) {
  return (
    <div {...props} className={cx("omfp-resolved", className)} data-outcome={outcome}>
      <Icon name={outcome === "denied" ? "x" : "check"} className="omfp-resolved__icon" />
      <span className="omfp-resolved__question">{question}</span>
      <span className="omfp-resolved__answer">{answer}</span>
    </div>
  );
}
