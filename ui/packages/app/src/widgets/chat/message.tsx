/** One transcript row: a message with its blocks (text, thinking, images, tool cards, records) or a notice row. */

import {
  Badge,
  Button,
  Code,
  Disclosure,
  ExitCode,
  Icon,
  Markdown,
  Message,
  StreamCaret,
  Thinking,
  ToolCard,
  type ToolStatus,
} from "omfp/kit";
import type { MessageBlock as Block, MessageView, NoticeRow, MessageRole as Role, SystemRow } from "omfp/sdk";
import {
  type ToolCard as Card,
  expandedByDefault,
  formatDuration,
  lastOutputLine,
  loopbackCallback,
  markdownChunks,
  OAuthCallbackField,
  strings,
  useOpenFile,
  useUrlActions,
} from "omfp/sdk";
import { memo, type ReactNode, useCallback } from "react";
import { useImageViewer } from "./image-viewer.tsx";
import { useExpanded, useObserved, useObservedElapsed } from "./state.ts";
import { fileTarget, ToolBody, ToolImages, ToolSubject } from "./tool-cards.tsx";

const text = strings.chat;

/** One message of thread `threadId`: name line, blocks, error (UI-171). One that `continues` its turn has no name line
 * (UI-470). `cardKey` prefixes the keys of its collapsible blocks. While it streams, a block caret follows the last
 * character (UI-147). `actions` act on the message (fork from here, UI-660). `openTurn`: the message is in the turn
 * its thread still works on, whose cards stay open (UI-922). */
export function MessageRow({
  threadId,
  cardKey,
  view,
  streaming,
  continues,
  openTurn = false,
  actions,
}: {
  threadId: string;
  cardKey: string;
  view: MessageView;
  streaming: boolean;
  continues: boolean;
  openTurn?: boolean;
  actions?: ReactNode;
}) {
  const name = (
    <>
      {view.label}
      {view.steering && (
        <span className="omfp-message__tag" data-testid="steering-mark">
          {text.steeringMark}
        </span>
      )}
    </>
  );
  const blocks = view.blocks.map((block, index) => (
    <BlockView
      // Blocks only grow at the end while streaming, so the index is a stable key.
      // biome-ignore lint/suspicious/noArrayIndexKey: see above
      key={index}
      threadId={threadId}
      rowKey={cardKey}
      cardKey={blockCardKey(cardKey, block, index)}
      role={view.role}
      block={block}
      streaming={streaming}
      last={index === view.blocks.length - 1}
      openTurn={openTurn}
    />
  ));
  const error = view.error === undefined ? undefined : <span data-testid="message-error">{view.error}</span>;
  const caret = streaming && view.blocks.at(-1)?.type !== "text" ? <StreamCaret /> : null;
  const common = {
    className: "omfp-message",
    "data-testid": "message",
    "data-role": view.role,
    "data-streaming": streaming,
    "data-continues": continues,
    "aria-busy": streaming,
  };
  if (view.role === "record") {
    return (
      <article {...common}>
        <div className="omfp-msg__name">{name}</div>
        {blocks}
        {error !== undefined && <p className="omfp-msg__error">{error}</p>}
      </article>
    );
  }
  return (
    <Message
      {...common}
      author={view.role}
      name={name}
      continues={continues}
      error={error}
      {...(actions === undefined ? {} : { actions })}
    >
      {blocks}
      {caret}
    </Message>
  );
}

/** The key of the collapsible block at `index` of the message `cardKey`: a thinking block's own key when it has one
 * (UI-694), a tool card's by its call, a read group's by its first call (UI-765), else its position. */
export function blockCardKey(cardKey: string, block: Block, index: number): string {
  switch (block.type) {
    case "thinking":
      return block.key ?? `${cardKey}/${index}`;
    case "tool":
      return toolCardKey(cardKey, block.card.toolCallId, `${cardKey}/${index}`);
    case "readGroup":
      return `${cardKey}/reads/${block.cards[0]?.toolCallId ?? index}`;
    default:
      return `${cardKey}/${index}`;
  }
}

/**
 * The key of the card of call `toolCallId` in the message `rowKey`, `fallback` for a call without an id: the same
 * whether the card stands alone or a read group shows it (UI-765), so a read keeps its disclosure when it joins one.
 */
function toolCardKey(rowKey: string, toolCallId: string, fallback: string): string {
  return toolCallId === "" ? fallback : `${rowKey}/call/${toolCallId}`;
}

/** One block; renders again only when the block changes (the view model keeps unchanged blocks, UI-191), so a
 * streaming delta renders the growing block only. */
const BlockView = memo(function BlockView({
  threadId,
  rowKey,
  cardKey,
  role,
  block,
  streaming,
  last,
  openTurn,
}: {
  threadId: string;
  /** The message's key, which its tool cards' keys start with. */
  rowKey: string;
  cardKey: string;
  role: Role;
  block: Block;
  streaming: boolean;
  /** The message's last block: a streaming text then trails the caret (UI-147). */
  last: boolean;
  openTurn: boolean;
}) {
  const openFile = useOpenFile();
  // Paths in the assistant's text open the file (UI-116).
  const openPath = useCallback(
    (reference: { path: string; line: number | undefined }) => openFile(reference.path, { line: reference.line }),
    [openFile],
  );
  switch (block.type) {
    case "text":
      if (role !== "assistant") {
        return (
          <div className="omfp-message__plain omfp-prose" data-testid="plain-text">
            {block.text}
          </div>
        );
      }
      return streaming ? (
        <StreamingMarkdown text={block.text} trail={last} onOpenPath={openPath} />
      ) : (
        <Markdown text={block.text} data-testid="markdown" onOpenPath={openPath} />
      );
    case "thinking":
      return <ThinkingBlock cardKey={cardKey} text={block.text} live={streaming && last} />;
    case "image":
      return <MessageImage src={`data:${block.mime};base64,${block.data}`} />;
    case "tool":
      return <ToolCardView threadId={threadId} cardKey={cardKey} card={block.card} openTurn={openTurn} />;
    case "readGroup":
      return <ReadGroupView threadId={threadId} rowKey={rowKey} cardKey={cardKey} group={block} />;
    case "record":
      return <RecordBlock cardKey={cardKey} summary={block.summary} json={block.json} />;
  }
});

/** A message's image (UI-475), a button opening it at full size, zoomable (UI-476). */
function MessageImage({ src }: { src: string }): ReactNode {
  const viewer = useImageViewer();
  return (
    <>
      <Button
        variant="quiet"
        className="omfp-message__open"
        aria-label={text.openAttachedImage}
        onPress={() =>
          viewer.open({ src, alt: text.attachedImage, title: text.attachedImage, testId: "message-image-dialog" })
        }
        data-testid="message-image-open"
      >
        <img className="omfp-message__image" data-testid="message-image" src={src} alt={text.attachedImage} />
      </Button>
      {viewer.dialog}
    </>
  );
}

/** A Markdown chunk, parsed again only when its text changes. */
const MarkdownChunk = memo(Markdown);

/** Streaming Markdown in top-level chunks (UI-191): a delta parses the chunk it grows, not the whole reply. With
 * `trail`, the stream caret follows its last character (UI-147). */
function StreamingMarkdown({
  text: markdown,
  trail,
  onOpenPath,
}: {
  text: string;
  trail: boolean;
  onOpenPath: (reference: { path: string; line: number | undefined }) => void;
}) {
  return (
    <div className={trail ? "omfp-markdown-stream omfp-caret-trail" : "omfp-markdown-stream"} data-testid="markdown">
      {markdownChunks(markdown).map((chunk, index) => (
        // Chunks only change or grow at the end while streaming, so the index is a stable key.
        // biome-ignore lint/suspicious/noArrayIndexKey: see above
        <MarkdownChunk key={index} text={chunk} onOpenPath={onOpenPath} />
      ))}
    </div>
  );
}

/** Thinking: live and settled have distinct labels; the disclosure stays collapsed unless the user chose otherwise. */
function ThinkingBlock({ cardKey, text: thinking, live }: { cardKey: string; text: string; live: boolean }) {
  const [expanded, setExpanded] = useExpanded(cardKey, false);
  return (
    <Thinking
      data-testid="thinking"
      data-live={live ? "true" : undefined}
      label={live ? text.thinkingLive : text.thought}
      expanded={expanded}
      onExpandedChange={setExpanded}
    >
      {thinking}
    </Thinking>
  );
}

function RecordBlock({ cardKey, summary, json }: { cardKey: string; summary: string; json: string }) {
  const [expanded, setExpanded] = useExpanded(cardKey, false);
  return (
    <div data-testid="record" data-expanded={expanded}>
      <Disclosure title={summary} expanded={expanded} onExpandedChange={setExpanded} level={2}>
        <Code code={json} block data-testid="record-json" />
      </Disclosure>
    </div>
  );
}

/** The status mark of a call (UI-163): composing, pending and running calls spin. */
function markStatus(status: Card["status"]): ToolStatus {
  switch (status) {
    case "succeeded":
      return "done";
    case "failed":
      return "failed";
    default:
      return "running";
  }
}

/** A tool card of thread `threadId` (UI-050, UI-163): collapsible, failed cards distinct and expanded by default
 * (UI-052); tools with a recorded shape show their specialized body, others the fallback's arguments and result. The
 * result's images show under it, open or not (UI-473). In a read group (`grouped`) a read's head adds its path's
 * selector (UI-765). An `omfp_card` call's card stays open while `openTurn` (UI-922). */
export function ToolCardView({
  threadId,
  cardKey,
  card,
  grouped = false,
  openTurn = false,
}: {
  threadId: string;
  cardKey: string;
  card: Card;
  grouped?: boolean;
  openTurn?: boolean;
}) {
  const [expanded, setExpanded] = useExpanded(cardKey, expandedByDefault(card, openTurn));
  // `:open` on the focused card opens its file (UI-116).
  const target = fileTarget(card.view);
  const exitCode = card.view.kind === "bash" ? card.view.exitCode : undefined;
  const selector = grouped && card.view.kind === "read" ? card.view.selector : undefined;
  return (
    <div
      data-omfp-path={target?.path}
      data-omfp-line={target?.open.line}
      data-omfp-diff={target?.open.diff === true ? "" : undefined}
      data-testid="tool-card"
      data-tool={card.name}
      data-call={card.toolCallId}
      data-view={card.view.kind}
      data-status={card.status}
    >
      <ToolCard
        aria-label={text.tool.label(card.name)}
        status={markStatus(card.status)}
        name={card.name}
        headTestId="toggle"
        expanded={expanded}
        onExpandedChange={setExpanded}
        arg={
          <>
            {selector === undefined ? (
              <ToolSubject view={card.view} />
            ) : (
              <span className="omfp-tool-card__target">
                <ToolSubject view={card.view} />
                <span className="omfp-tool-card__selector" data-testid="tool-selector">
                  :{selector}
                </span>
              </span>
            )}
            <span className="omfp-tool-card__summary" data-testid="tool-summary">
              {card.summary}
            </span>
          </>
        }
        meta={<ToolMeta card={card} exitCode={exitCode} />}
      >
        <ToolBody threadId={threadId} card={card} />
      </ToolCard>
      {card.images.length > 0 && <ToolImages card={card} />}
    </div>
  );
}

/**
 * Consecutive `read` calls as one card (UI-765): `read N files` with the run's state mark (running while any read
 * runs, else failed when any failed), collapsed by default; open, the reads in call order, each its own read card
 * (status mark, path and selector opening the file at its line, UI-116) that opens to its output, under the key it
 * has alone in the message `rowKey`. A failed read and a read's images stay shown under the collapsed group, and its
 * live reads are observed while it is collapsed (UI-695).
 */
export function ReadGroupView({
  threadId,
  rowKey,
  cardKey,
  group,
}: {
  threadId: string;
  rowKey: string;
  cardKey: string;
  group: Extract<Block, { type: "readGroup" }>;
}) {
  const [expanded, setExpanded] = useExpanded(cardKey, false);
  const count = group.cards.length;
  const readKey = (card: Card, index: number) => toolCardKey(rowKey, card.toolCallId, `${cardKey}/${index}`);
  const failed = group.cards.flatMap((card, index) => (card.status === "failed" ? [{ card, index }] : []));
  const images = group.cards.filter((card) => card.status !== "failed" && card.images.length > 0);
  // A read's card counts its elapsed time from when it was first observed (`ToolMeta`'s key), even if not shown yet.
  useObserved(
    group.cards.flatMap((card) =>
      card.status === "composing" || card.status === "pending" || card.status === "running"
        ? [`tool/${card.toolCallId}`]
        : [],
    ),
  );
  return (
    <div className="omfp-read-group" data-testid="read-group" data-status={group.status} data-count={count}>
      <ToolCard
        aria-label={text.tool.readGroup(count)}
        status={markStatus(group.status)}
        name="read"
        headTestId="read-group-toggle"
        expanded={expanded}
        onExpandedChange={setExpanded}
        arg={
          <span className="omfp-tool-card__summary" data-testid="read-group-count">
            {text.tool.readGroupFiles(count)}
          </span>
        }
        meta={<ReadGroupMeta cardKey={cardKey} group={group} />}
      >
        <ul className="omfp-read-group__reads" data-testid="read-group-reads">
          {group.cards.map((card, index) => (
            <li key={card.toolCallId}>
              <ToolCardView threadId={threadId} cardKey={readKey(card, index)} card={card} grouped />
            </li>
          ))}
        </ul>
      </ToolCard>
      {!expanded && (failed.length > 0 || images.length > 0) && (
        <div className="omfp-read-group__shown" data-testid="read-group-shown">
          {failed.map(({ card, index }) => (
            <ToolCardView
              key={card.toolCallId}
              threadId={threadId}
              cardKey={readKey(card, index)}
              card={card}
              grouped
            />
          ))}
          {images.map((card) => (
            <ToolImages key={card.toolCallId} card={card} />
          ))}
        </div>
      )}
    </div>
  );
}

/** A read group's head metadata: its state for assistive technology and its live or settled duration (UI-695). */
function ReadGroupMeta({
  cardKey,
  group,
}: {
  cardKey: string;
  group: Extract<Block, { type: "readGroup" }>;
}): ReactNode {
  const observed = useObservedElapsed(`tool/${cardKey}`, group.status === "running");
  const settled = group.cards.map((card) => card.durationMs ?? 0);
  const duration =
    group.status === "running" || group.cards.every((card) => card.durationMs === undefined)
      ? observed
      : Math.max(...settled);
  return (
    <>
      <span className="omfp-visually-hidden" data-testid="read-group-status">
        {text.tool.status[group.status]}
      </span>
      {duration !== undefined && <span data-testid="read-group-duration">{formatDuration(duration)}</span>}
    </>
  );
}

/** Right-aligned metadata of a card's head: status, background job, live or settled duration, and exit code. */
function ToolMeta({ card, exitCode }: { card: Card; exitCode: number | undefined }): ReactNode {
  const job = card.view.kind === "bash" ? card.view.background : undefined;
  const active = card.status === "composing" || card.status === "pending" || card.status === "running";
  const observedDuration = useObservedElapsed(`tool/${card.toolCallId}`, active);
  const duration = card.durationMs ?? observedDuration;
  const lastLine =
    job?.state === "running" && card.view.kind === "bash" ? lastOutputLine(card.view.output ?? "") : undefined;
  return (
    <>
      <span className="omfp-visually-hidden" data-testid="tool-status">
        {text.tool.status[card.status]}
      </span>
      {lastLine !== undefined && lastLine !== "" && (
        <span className="omfp-tool-card__last-line" data-testid="tool-last-line" title={lastLine}>
          {lastLine}
        </span>
      )}
      {job !== undefined && (
        <Badge
          tone={job.state === "completed" ? "success" : job.state === "running" ? "primary" : "default"}
          data-testid="tool-background"
          data-state={job.state}
          title={text.tool.backgroundCommand(job.jobId)}
        >
          {text.tool.background[job.state]}
        </Badge>
      )}
      {duration !== undefined && <span data-testid="tool-duration">{formatDuration(duration)}</span>}
      {exitCode !== undefined && <ExitCode code={exitCode} data-testid="tool-exit" />}
    </>
  );
}

/** An inline notice row (UI-043); unknown frames expand to their JSON. A command's output and an `open_url` request
 * are cards (UI-345) of thread `threadId`. */
export function NoticeRowView({ threadId, cardKey, row }: { threadId: string; cardKey: string; row: NoticeRow }) {
  const [expanded, setExpanded] = useExpanded(cardKey, false);
  if (row.kind === "command" || row.kind === "open-url") {
    return <CommandOutputCard threadId={threadId} row={row} />;
  }
  return (
    <div className="omfp-notice" data-testid="notice" data-kind={row.kind} data-level={row.level}>
      <span>{row.text}</span>
      {row.json !== undefined && (
        <Disclosure title={text.details} expanded={expanded} onExpandedChange={setExpanded} level={2}>
          <Code code={row.json} block data-testid="notice-json" />
        </Disclosure>
      )}
    </div>
  );
}

/**
 * The output of a local slash command (`/dirs`, `/mcp reauth …`) or omp's `open_url` request as a compact card
 * (UI-345): the text as omp wrote it, then each URL omp opened on the daemon's host, selectable with Open and Copy,
 * where an OAuth sign-in returns to (UI-347) and, when it returns to omp's loopback listener, the field its address is
 * pasted into (UI-761).
 */
function CommandOutputCard({ threadId, row }: { threadId: string; row: NoticeRow }): ReactNode {
  const urls = useUrlActions();
  const relayable = row.urls.some((url) => loopbackCallback(url) !== undefined);
  return (
    <section
      className="omfp-notice omfp-command"
      data-testid="command-output"
      data-kind={row.kind}
      data-level={row.level}
      aria-label={text.notice.commandOutput}
    >
      <pre className="omfp-command__text" data-testid="command-output-text">
        {row.text}
      </pre>
      {row.urls.map((url) => (
        <div key={url} className="omfp-command__url" data-testid="opened-url" data-url={url}>
          <span>{text.notice.openedUrl}</span>
          <code className="omfp-command__url-text">{url}</code>
          <Button
            size="sm"
            variant="quiet"
            icon={<Icon name="link-2" />}
            aria-label={text.notice.openUrlLabel(url)}
            onPress={() => urls.open(url)}
            data-testid="opened-url-open"
          >
            {text.notice.openUrlAction}
          </Button>
          <Button
            size="sm"
            variant="quiet"
            icon={<Icon name="copy" />}
            aria-label={text.notice.copyUrlLabel(url)}
            onPress={() => urls.copy(url)}
            data-testid="opened-url-copy"
          >
            {text.notice.copyUrlAction}
          </Button>
          <OAuthCallbackField threadId={threadId} url={url} />
        </div>
      ))}
      {row.urls.length > 0 && (
        <p data-testid="loopback-note">{relayable ? text.notice.relayNote : text.notice.loopbackNote}</p>
      )}
    </section>
  );
}

/** omp's own bookkeeping (UI-043): one muted `system` row, expandable to the whole text when there is more. */
export function SystemRowView({ cardKey, row }: { cardKey: string; row: SystemRow }) {
  const [expanded, setExpanded] = useExpanded(cardKey, false);
  const summary = (
    <span className="omfp-system__summary" data-testid="system-summary" title={row.summary}>
      {row.summary}
    </span>
  );
  return (
    <div className="omfp-system" data-testid="system-record" data-expanded={expanded}>
      <span className="omfp-system__label" data-testid="system-label">
        {text.system}
      </span>
      {row.text === undefined ? (
        summary
      ) : (
        <Disclosure title={summary} expanded={expanded} onExpandedChange={setExpanded} level={2}>
          <div className="omfp-system__text" data-testid="system-text">
            {row.text}
          </div>
        </Disclosure>
      )}
    </div>
  );
}
