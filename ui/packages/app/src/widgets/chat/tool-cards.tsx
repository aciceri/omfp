/**
 * The bodies of the specialized tool cards (UI-050): what `toolView` read from a call, rendered per tool. Failed calls
 * show omp's error first (UI-052); code is highlighted when its language is known (UI-090); the code-intelligence
 * tools' paths open their files at their lines (UI-916..UI-919).
 */

import {
  Badge,
  BoxRule,
  Button,
  Code,
  Dialog,
  Diff,
  languageOfPath,
  Markdown,
  PathLink,
  TodoList,
  type Tone,
} from "omfp/kit";
import {
  type AstMatch,
  ansiRuns,
  type CardImage,
  type Diagnostic,
  type EditedFile,
  type LspResult,
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

/** A path a call names that is one file: not the workspace (`*`) nor a glob. */
function oneFile(path: string | undefined): path is string {
  return path !== undefined && path !== "*" && !/[*?[{]/.test(path);
}

/**
 * Where a card's path opens (UI-116): `read` at its first line, `edit` and `write` against `HEAD`, an `lsp` call's file
 * at its line.
 */
export function fileTarget(view: ToolView): { path: string; open: OpenFileOptions } | undefined {
  switch (view.kind) {
    case "read":
      return view.directory ? undefined : { path: view.path, open: { line: view.line } };
    case "write":
      return { path: view.path, open: { diff: true } };
    case "edit":
      return view.path === undefined ? undefined : { path: view.path, open: { diff: true } };
    case "lsp":
      return oneFile(view.file) ? { path: view.file, open: { line: view.line } } : undefined;
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
    case "ast_edit":
      return <Subject>{view.pattern}</Subject>;
    case "resolve":
      return <Subject>{view.label ?? view.action}</Subject>;
    case "lsp":
      return <LspSubject view={view} />;
    case "task":
      return <Subject>{view.agents.map((agent) => agent.name).join(", ")}</Subject>;
    default:
      return null;
  }
}

/** An `lsp` call's subject: its action, then its file (opening at its line) or `*`, its line and its symbol. */
function LspSubject({ view }: { view: Extract<ToolView, { kind: "lsp" }> }): ReactNode {
  const target = fileTarget(view);
  return (
    <span className="omfp-tool-card__target">
      <Subject>{view.action}</Subject>
      {view.file !== undefined && (
        <>
          &nbsp;
          {target === undefined ? <Subject>{view.file}</Subject> : <FilePath path={target.path} open={target.open} />}
          {view.line !== undefined && <span className="omfp-tool-card__selector">:{view.line}</span>}
        </>
      )}
      {view.symbol !== undefined && <span className="omfp-tool-card__selector">&nbsp;({view.symbol})</span>}
    </span>
  );
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
                    <div className="omfp-tool-card__file">
                      <PathLink reference={{ path: file.path, line: undefined }} onOpen={(ref) => openFile(ref.path)}>
                        <FilePath path={file.path} />
                      </PathLink>
                      {file.count !== undefined && <Badge>{file.count}</Badge>}
                    </div>
                    {file.matches.length > 0 && <Matches path={file.path} matches={file.matches} />}
                  </li>
                ))}
              </ul>
            ))}
          {view.truncated && <p className="omfp-tool-card__meta">{text.truncated}</p>}
        </div>
      );
    case "ast_edit":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          <p className="omfp-tool-card__meta">
            {view.total !== undefined && <span data-testid="tool-total">{view.total}</span>}
            {card.status === "succeeded" && !view.applied && <span data-testid="tool-preview">{text.preview}</span>}
          </p>
          {card.status === "succeeded" && view.files.length === 0 && (
            <p className="omfp-tool-card__meta">{text.noMatch}</p>
          )}
          <ul className="omfp-tool-card__files">
            {view.files.map((file) => (
              <li key={file.path} data-testid="tool-file">
                <div className="omfp-tool-card__file">
                  <PathLink
                    reference={{ path: file.path, line: file.diff[0]?.oldLine ?? file.diff[0]?.newLine }}
                    onOpen={(ref) => openFile(ref.path, { line: ref.line })}
                  >
                    <FilePath path={file.path} />
                  </PathLink>
                  {file.count !== undefined && <Badge>{file.count}</Badge>}
                </div>
                {file.diff.length > 0 && (
                  <Diff lines={file.diff} language={languageOfPath(file.path)} data-testid="tool-diff" />
                )}
              </li>
            ))}
          </ul>
          {view.truncated && <p className="omfp-tool-card__meta">{text.truncated}</p>}
        </div>
      );
    case "resolve":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          {view.action === "discard" && <p className="omfp-tool-card__meta">{text.discarded}</p>}
          {view.reason !== undefined && <p className="omfp-tool-card__task">{view.reason}</p>}
          <ChangedFiles files={view.files} />
        </div>
      );
    case "lsp":
      return (
        <div className="omfp-tool-card__section">
          <ToolError card={card} />
          {card.status !== "failed" && view.result !== undefined && <LspAnswer result={view.result} />}
          {view.noServer && (
            <p className="omfp-tool-card__meta" data-testid="lsp-environment">
              {text.serverEnvironment}
            </p>
          )}
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

/**
 * An `ast_grep` file's matches (UI-916): each match's first line opens the file there, its code is highlighted in the
 * file's language, its metavariable captures under it.
 */
function Matches({ path, matches }: { path: string; matches: readonly AstMatch[] }): ReactNode {
  const openFile = useOpenFile();
  const language = languageOfPath(path);
  return (
    <ol className="omfp-tool-card__matches">
      {matches.map((match, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: a result's matches never move
        <li key={index} data-testid="tool-match">
          <PathLink
            reference={{ path, line: match.line }}
            onOpen={(ref) => openFile(ref.path, { line: ref.line })}
            data-testid="tool-match-line"
          >
            {match.line}
          </PathLink>
          <Code code={match.code} block language={language} data-testid="tool-match-code" />
          {match.captures !== undefined && (
            <p className="omfp-tool-card__meta omfp-tool-card__captures" data-testid="tool-match-captures">
              {match.captures}
            </p>
          )}
        </li>
      ))}
    </ol>
  );
}

/** `path:line:col`, opening the file at the line (UI-116). */
function Location({ path, line, column }: { path: string; line: number; column?: number | undefined }): ReactNode {
  const openFile = useOpenFile();
  return (
    <PathLink
      reference={{ path, line }}
      onOpen={(ref) => openFile(ref.path, { line: ref.line })}
      data-testid="lsp-location"
    >
      <FilePath path={path} />
      <span className="omfp-tool-card__selector">
        :{line}
        {column !== undefined && `:${column}`}
      </span>
    </PathLink>
  );
}

/** Files a workspace edit or an applied rewrite changed, each opening against `HEAD` (UI-917, UI-919). */
function ChangedFiles({ files }: { files: readonly EditedFile[] }): ReactNode {
  const openFile = useOpenFile();
  if (files.length === 0) {
    return null;
  }
  return (
    <ul className="omfp-tool-card__files">
      {files.map((file) => (
        <li key={file.path} data-testid="tool-file">
          <div className="omfp-tool-card__file">
            {file.removed ? (
              <FilePath path={file.path} />
            ) : (
              <PathLink
                reference={{ path: file.path, line: undefined }}
                diff
                onOpen={(ref) => openFile(ref.path, { diff: true })}
              >
                <FilePath path={file.path} />
              </PathLink>
            )}
            <span className="omfp-tool-card__meta">{file.note}</span>
          </div>
        </li>
      ))}
    </ul>
  );
}

/** omp's status lines of an `lsp` answer. */
function Notes({ notes }: { notes: readonly string[] }): ReactNode {
  return notes.map((note, index) => (
    // biome-ignore lint/suspicious/noArrayIndexKey: an answer's notes never move
    <p key={index} className="omfp-tool-card__meta" data-testid="lsp-note">
      {note}
    </p>
  ));
}

/** The badge tone of a diagnostic's severity. */
const SEVERITY_TONE: Readonly<Record<string, Tone>> = {
  error: "error",
  warning: "warning",
  info: "primary",
  hint: "default",
};

function DiagnosticRow({ diagnostic }: { diagnostic: Diagnostic }): ReactNode {
  const facts = [diagnostic.source, diagnostic.code].filter((fact) => fact !== undefined);
  return (
    <li data-testid="lsp-diagnostic" data-severity={diagnostic.severity}>
      <Badge tone={SEVERITY_TONE[diagnostic.severity] ?? "default"}>{diagnostic.severity}</Badge>
      <Location path={diagnostic.path} line={diagnostic.line} column={diagnostic.column} />
      <span className="omfp-tool-card__message">{diagnostic.message}</span>
      {facts.length > 0 && <span className="omfp-tool-card__meta">{facts.join(" · ")}</span>}
    </li>
  );
}

/**
 * An `lsp` answer (UI-918, UI-919): diagnostics with their severity and location, locations with their source,
 * symbols at their lines, the files an edit changed, the code actions offered, hover documentation as Markdown, else
 * omp's text.
 */
function LspAnswer({ result }: { result: LspResult }): ReactNode {
  const openFile = useOpenFile();
  switch (result.type) {
    case "diagnostics":
      return (
        <>
          {result.diagnostics.length > 0 && (
            <ul className="omfp-tool-card__diagnostics">
              {result.diagnostics.map((diagnostic, index) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: an answer's diagnostics never move
                <DiagnosticRow key={index} diagnostic={diagnostic} />
              ))}
            </ul>
          )}
          {result.clean && (
            <p className="omfp-tool-card__meta" data-testid="lsp-clean">
              {text.noIssues}
            </p>
          )}
          <Notes notes={result.notes} />
        </>
      );
    case "locations":
      return (
        <>
          <p className="omfp-tool-card__meta" data-testid="tool-total">
            {result.title}
          </p>
          <ul className="omfp-tool-card__locations">
            {result.locations.map((location) => (
              <li key={`${location.path}:${location.line}:${location.column}`} data-testid="lsp-location-item">
                <Location path={location.path} line={location.line} column={location.column} />
                {location.context.length > 0 && (
                  <Code
                    code={location.context.map((line) => line.text).join("\n")}
                    block
                    language={languageOfPath(location.path)}
                    data-testid="lsp-context"
                  />
                )}
              </li>
            ))}
          </ul>
          <Notes notes={result.notes} />
        </>
      );
    case "symbols":
      return (
        <>
          <p className="omfp-tool-card__meta" data-testid="tool-total">
            {result.title}
          </p>
          <ul className="omfp-tool-card__symbols">
            {result.symbols.map((symbol, index) => (
              <li
                // biome-ignore lint/suspicious/noArrayIndexKey: an answer's symbols never move
                key={index}
                data-testid="lsp-symbol"
                style={{ paddingInlineStart: `${symbol.depth}rem` }}
              >
                <span className="omfp-tool-card__symbol">{symbol.name}</span>
                {symbol.path !== undefined && (
                  <PathLink
                    reference={{ path: symbol.path, line: symbol.line }}
                    onOpen={(ref) => openFile(ref.path, { line: ref.line })}
                    data-testid="lsp-symbol-line"
                  >
                    {result.workspace && <FilePath path={symbol.path} />}:{symbol.line}
                  </PathLink>
                )}
              </li>
            ))}
          </ul>
        </>
      );
    case "edits":
      return (
        <>
          <p className="omfp-tool-card__meta">
            <span data-testid="tool-total">{result.title}</span>
            {!result.applied && <span>{text.notApplied}</span>}
          </p>
          <ChangedFiles files={result.files} />
          {result.commands.length > 0 && (
            <>
              <h3>{text.commands}</h3>
              <ul className="omfp-tool-card__files">
                {result.commands.map((command) => (
                  <li key={command} data-testid="lsp-command">
                    <Code code={command} />
                  </li>
                ))}
              </ul>
            </>
          )}
          <Notes notes={result.notes} />
        </>
      );
    case "actions":
      return (
        <>
          <p className="omfp-tool-card__meta" data-testid="tool-total">
            {result.title}
          </p>
          <ol className="omfp-tool-card__actions">
            {result.actions.map((action) => (
              <li key={action.index} data-testid="lsp-action">
                <span className="omfp-tool-card__meta">{action.index}</span>
                <Badge>{action.kind}</Badge>
                <span>{action.title}</span>
              </li>
            ))}
          </ol>
        </>
      );
    case "hover":
      return <Markdown text={result.markdown} data-testid="lsp-hover" />;
    case "text":
      return <Code code={result.text} block data-testid="tool-result" />;
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
