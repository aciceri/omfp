/**
 * The bodies of the specialized tool cards (UI-050): what `toolView` read from a call, rendered per tool. Failed calls
 * show omp's error first (UI-052); code is highlighted when its language is known (UI-090).
 */

import { Badge, BoxRule, Button, Code, Dialog, Diff, PathLink, TodoList } from "omfp/kit";
import {
  ansiRuns,
  type CardImage,
  type OpenFileOptions,
  repoPath,
  type SubagentRun,
  strings,
  type TodoPhase,
  type ToolCard,
  type ToolView,
  useCurrentWorkspaceId,
  useOpenFile,
  useWorkspaces,
} from "omfp/sdk";
import { type ReactNode, useMemo, useState } from "react";

const text = strings.chat.tool;

/**
 * A file path: relative to the workspace repository holding it, the repository's name as a muted prefix (`repo/path`;
 * no chip, the card is a box already). With `open`, clicking the path opens the file (UI-116); in a card's header the
 * path sits inside the collapse toggle, so it opens on a pointer click without toggling, and the keyboard reaches it with
 * `:open` on the focused card (its `data-omfp-path`).
 */
function FilePath({ path, open }: { path: string; open?: OpenFileOptions | undefined }): ReactNode {
  const workspaceId = useCurrentWorkspaceId();
  const workspace = useWorkspaces().get(workspaceId ?? "");
  const openFile = useOpenFile();
  const shown = repoPath(path, workspace);
  return (
    <span className="omfp-tool-card__path" data-testid="tool-path" data-repo={shown.repo}>
      {shown.repo !== undefined && (
        <span className="omfp-tool-card__repo" data-testid="tool-repo">
          {shown.repo}/
        </span>
      )}
      {open === undefined ? (
        <span className="omfp-tool-card__subject">{shown.path}</span>
      ) : (
        // biome-ignore lint/a11y/useKeyWithClickEvents: inside the toggle button; the keyboard opens it with `:open`
        // biome-ignore lint/a11y/noStaticElementInteractions: same
        <span
          className="omfp-tool-card__subject omfp-path-link"
          data-testid="tool-path-open"
          onPointerDown={(event) => event.stopPropagation()}
          onPointerUp={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation();
            openFile(path, open);
          }}
        >
          {shown.path}
        </span>
      )}
    </span>
  );
}

/** Where a card's path opens (UI-116): `read` at its first line, `edit` and `write` against `HEAD`. */
export function fileTarget(view: ToolView): { path: string; open: OpenFileOptions } | undefined {
  switch (view.kind) {
    case "read":
      return view.directory ? undefined : { path: view.path, open: { line: view.line } };
    case "write":
      return { path: view.path, open: { diff: true } };
    case "edit":
      return view.path === undefined ? undefined : { path: view.path, open: { diff: true } };
    default:
      return undefined;
  }
}

/** What the call is about, shown in the card's header: its command, file, query or URL. */
export function ToolSubject({ view }: { view: ToolView }): ReactNode {
  switch (view.kind) {
    case "bash":
      return <Subject>{view.command}</Subject>;
    case "read":
    case "write":
    case "edit":
      return view.path === undefined ? null : <FilePath path={view.path} open={fileTarget(view)?.open} />;
    case "fetch":
      return <Subject>{view.url}</Subject>;
    case "search":
    case "web_search":
      return <Subject>{view.query}</Subject>;
    case "task":
      return <Subject>{view.agents.map((agent) => agent.name).join(", ")}</Subject>;
    default:
      return null;
  }
}

function Subject({ children }: { children: ReactNode }): ReactNode {
  return (
    <span className="omfp-tool-card__subject" data-testid="tool-subject">
      {children}
    </span>
  );
}

/** Command output with its ANSI colors and attributes as theme classes. */
function AnsiOutput({ output }: { output: string }): ReactNode {
  const runs = useMemo(() => ansiRuns(output), [output]);
  return (
    <pre className="omfp-code omfp-code--block omfp-tool-card__output" data-testid="tool-output">
      {runs.map((run, index) =>
        run.style.length === 0 ? (
          run.text
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: runs of one output never move
          <span key={index} className={run.style.map((name) => `omfp-ansi-${name}`).join(" ")}>
            {run.text}
          </span>
        ),
      )}
    </pre>
  );
}

/** The error of a failed call (omp's message). */
function ToolError({ card }: { card: ToolCard }): ReactNode {
  if (card.status !== "failed" || card.result === undefined) {
    return null;
  }
  return (
    <div className="omfp-tool-card__error" data-testid="tool-error">
      {card.result}
    </div>
  );
}

/** Lines of a `write`'s content shown while it is collapsed (UI-053). */
const WRITE_PREVIEW_LINES = 6;

/** A `write`'s content: its first lines, expandable to the whole content (UI-053). */
function WriteContent({ content, language }: { content: string; language: string | undefined }): ReactNode {
  const [expanded, setExpanded] = useState(false);
  const lines = content.split("\n");
  if (lines.at(-1) === "") {
    lines.pop();
  }
  const hidden = lines.length - WRITE_PREVIEW_LINES;
  const shown = expanded || hidden <= 0 ? content : lines.slice(0, WRITE_PREVIEW_LINES).join("\n");
  return (
    <>
      <div className="omfp-tool-card__preview" data-cut={!expanded && hidden > 0}>
        <Code code={shown} block language={language} data-testid="tool-content" />
      </div>
      {hidden > 0 && (
        <Button
          size="sm"
          variant="quiet"
          aria-expanded={expanded}
          data-testid="tool-more"
          onPress={() => setExpanded((open) => !open)}
        >
          {expanded ? text.fewerLines : text.moreLines(hidden)}
        </Button>
      )}
    </>
  );
}

/** The body of a specialized card; its sections are split by labelled rules on the card's border (UI-187). */
export function ToolBody({ card }: { card: ToolCard }): ReactNode {
  const { view } = card;
  const openFile = useOpenFile();
  switch (view.kind) {
    case "bash":
      return (
        <>
          <div className="omfp-tool-card__section">
            {view.cwd !== undefined && <p className="omfp-tool-card__meta">{text.cwd(view.cwd)}</p>}
            <Code code={view.command} block language="bash" data-testid="tool-command" />
          </div>
          <BoxRule label={text.output} />
          <div className="omfp-tool-card__section">
            {view.output === undefined || view.output === "" ? (
              <p className="omfp-tool-card__meta">{text.noOutput}</p>
            ) : (
              <AnsiOutput output={view.output} />
            )}
          </div>
        </>
      );
    case "read":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          {view.range !== undefined && (
            <p className="omfp-tool-card__meta" data-testid="tool-range">
              {view.range}
            </p>
          )}
          {card.status !== "failed" && view.content !== undefined && (
            <Code
              code={view.content}
              block
              language={view.directory ? undefined : view.language}
              data-testid="tool-content"
            />
          )}
        </div>
      );
    case "fetch":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          <a className="omfp-tool-card__link" href={view.url} target="_blank" rel="noreferrer" data-testid="tool-link">
            {view.url}
          </a>
          {card.status !== "failed" && view.content !== undefined && (
            <Code code={view.content} block language="markdown" data-testid="tool-content" />
          )}
        </div>
      );
    case "edit":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          {view.diff !== undefined ? (
            <Diff lines={view.diff} language={view.language} data-testid="tool-diff" />
          ) : (
            view.input !== undefined && (
              <>
                <h3>{text.edit}</h3>
                <Code code={view.input} block data-testid="tool-input" />
              </>
            )
          )}
        </div>
      );
    case "write":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          {view.content !== undefined && <WriteContent content={view.content} language={view.language} />}
        </div>
      );
    case "search":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          <p className="omfp-tool-card__meta">
            {view.scope !== undefined && <span>{text.in(view.scope)}</span>}
            {view.total !== undefined && <span data-testid="tool-total">{view.total}</span>}
          </p>
          {view.files !== undefined &&
            (view.files.length === 0 ? (
              <p className="omfp-tool-card__meta">{text.noMatch}</p>
            ) : (
              <ul className="omfp-tool-card__files">
                {view.files.map((file) => (
                  <li key={file.path} data-testid="tool-file">
                    <PathLink reference={{ path: file.path, line: undefined }} onOpen={(ref) => openFile(ref.path)}>
                      <FilePath path={file.path} />
                    </PathLink>
                    {file.count !== undefined && <Badge>{file.count}</Badge>}
                  </li>
                ))}
              </ul>
            ))}
          {view.truncated && <p className="omfp-tool-card__meta">{text.truncated}</p>}
        </div>
      );
    case "todo":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          <TodoPhases phases={view.phases} />
        </div>
      );
    case "web_search":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          {card.status === "succeeded" && view.links.length === 0 && (
            <p className="omfp-tool-card__meta">{text.noLinks}</p>
          )}
          <ul className="omfp-tool-card__links">
            {view.links.map((link) => (
              <li key={link.url} data-testid="tool-link">
                <a href={link.url} target="_blank" rel="noreferrer">
                  {link.title}
                </a>
                <span className="omfp-tool-card__meta">{link.url}</span>
                {link.snippet !== "" && <span>{link.snippet}</span>}
              </li>
            ))}
          </ul>
        </div>
      );
    case "ask":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          <dl className="omfp-tool-card__answers">
            {view.questions.map((question) => (
              <div key={question.question} data-testid="ask-question">
                <dt>{question.question}</dt>
                {question.answer !== undefined && (
                  <dd data-testid="ask-answer">{question.answer === "" ? text.noAnswer : question.answer}</dd>
                )}
              </div>
            ))}
          </dl>
        </div>
      );
    case "task":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          <ul className="omfp-tool-card__agents">
            {view.agents.map((agent) => (
              <Subagent key={agent.name} agent={agent} />
            ))}
          </ul>
        </div>
      );
    case "generic":
      return (
        <>
          <div className="omfp-tool-card__section">
            <Code code={card.args} block data-testid="tool-args" />
          </div>
          {card.result !== undefined && (
            <>
              <BoxRule label={text.result} />
              <div className="omfp-tool-card__section">
                <Code code={card.result} block data-testid="tool-result" />
              </div>
            </>
          )}
        </>
      );
  }
}

/** A todo tool's phases (UI-163): the kit's todo list per phase, named when there are several. */
function TodoPhases({ phases }: { phases: readonly TodoPhase[] }): ReactNode {
  return phases.map((phase) => (
    <section key={phase.name} aria-label={phase.name}>
      {phases.length > 1 && <h3>{phase.name}</h3>}
      <TodoList
        className="omfp-tool-card__todo"
        itemTestId="todo-task"
        items={phase.tasks.map((task) => ({
          text: task.content,
          status: task.status,
          statusLabel: text.todoStatus[task.status] ?? task.status,
        }))}
      />
    </section>
  ));
}

function Subagent({ agent }: { agent: SubagentRun }): ReactNode {
  return (
    <li className="omfp-tool-card__agent" data-testid="subagent" data-status={agent.status}>
      <div className="omfp-tool-card__agent-head">
        <strong>{agent.name}</strong>
        {agent.agent !== "" && <span className="omfp-tool-card__meta">{agent.agent}</span>}
        <Badge
          tone={agent.status === "completed" ? "success" : agent.status === "running" ? "primary" : "default"}
          data-testid="subagent-status"
        >
          {agent.status}
        </Badge>
        {agent.tools > 0 && <span className="omfp-tool-card__meta">{text.tools(agent.tools)}</span>}
      </div>
      <p className="omfp-tool-card__task">{agent.task}</p>
      {agent.activity !== undefined && (
        <p className="omfp-tool-card__meta" data-testid="subagent-activity">
          {agent.activity}
        </p>
      )}
      {agent.output !== undefined && (
        <p className="omfp-tool-card__agent-output" data-testid="subagent-output">
          {agent.output}
        </p>
      )}
    </li>
  );
}

/** The `src` of an image of a result: its base64 data as a data URL. */
function imageSource(image: CardImage): string {
  return `data:${image.mime};base64,${image.data}`;
}

/**
 * The images of a call's result (UI-473, UI-474): thumbnails in a row that wraps, shown whether the card is open or
 * not; a click (or Enter, Space) opens one at full size in a dialog that Escape closes.
 */
export function ToolImages({ card }: { card: ToolCard }): ReactNode {
  const [opened, setOpened] = useState<CardImage | undefined>(undefined);
  return (
    <>
      <ul className="omfp-tool-images" aria-label={text.resultImages(card.name)} data-testid="tool-images">
        {card.images.map((image, index) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: a result's images never change once it arrived
          <li key={index} className="omfp-tool-images__item">
            <Button
              variant="quiet"
              className="omfp-tool-images__thumb"
              aria-label={text.openImage(image.alt)}
              onPress={() => setOpened(image)}
              data-testid="tool-image"
            >
              <img className="omfp-tool-images__img" src={imageSource(image)} alt={image.alt} decoding="async" />
            </Button>
          </li>
        ))}
      </ul>
      <Dialog
        isOpen={opened !== undefined}
        onOpenChange={(open) => {
          if (!open) {
            setOpened(undefined);
          }
        }}
        isDismissable
        title={opened?.alt ?? ""}
        className="omfp-image-dialog"
        data-testid="tool-image-dialog"
      >
        {opened !== undefined && (
          <img
            className="omfp-image-dialog__image"
            src={imageSource(opened)}
            alt={opened.alt}
            data-testid="tool-image-full"
          />
        )}
      </Dialog>
    </>
  );
}
