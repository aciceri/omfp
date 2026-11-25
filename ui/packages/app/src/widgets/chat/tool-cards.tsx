/**
 * The bodies of the specialized tool cards (UI-050): what `toolView` read from a call, rendered per tool. Failed calls
 * show omp's error first (UI-052); code is highlighted when its language is known (UI-090); the code-intelligence
 * tools' paths open their files at their lines (UI-916..UI-919). While the model writes a call's arguments the card
 * shows them as they are, but nothing acts on them: no path opens and no widget mounts (UI-054, UI-056).
 */

import {
  Badge,
  BoxRule,
  Button,
  Code,
  Diff,
  languageOfPath,
  Markdown,
  PathLink,
  type PathLinkProps,
  TodoList,
  type Tone,
} from "omfp/kit";
import {
  type AstMatch,
  ansiRuns,
  type CardImage,
  CardScope,
  CardWidget,
  type Diagnostic,
  type EditedFile,
  type EditPreviewFile,
  type LspResult,
  type OpenFileOptions,
  repoPath,
  type SubagentRun,
  strings,
  type TodoPhase,
  type ToolCard,
  type ToolView,
  useAnswer,
  useCurrentWorkspaceId,
  useOpenFile,
  useReportError,
  useWorkspaces,
} from "omfp/sdk";
import { Fragment, type ReactNode, useMemo, useState } from "react";
import { useImageViewer } from "./image-viewer.tsx";

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
 * at its line; nowhere while the model still writes the arguments, whose path may be cut short (UI-056).
 */
export function fileTarget(card: ToolCard): { path: string; open: OpenFileOptions } | undefined {
  const { view } = card;
  if (card.status === "composing") {
    return undefined;
  }
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

/** What the call is about, shown in the card's header: its command, file, query, URL or mount. */
export function ToolSubject({ card }: { card: ToolCard }): ReactNode {
  const { view } = card;
  switch (view.kind) {
    case "bash":
      return <Subject>{view.command}</Subject>;
    case "read":
    case "write":
    case "edit":
      return view.path === undefined ? null : <FilePath path={view.path} open={fileTarget(card)?.open} />;
    case "fetch":
      return <Subject>{view.url}</Subject>;
    case "search":
    case "web_search":
      return <Subject>{view.query}</Subject>;
    case "ast_edit":
      return <Subject>{view.pattern}</Subject>;
    case "resolve":
      return <Subject>{view.label ?? view.action}</Subject>;
    case "card":
      return <Subject>{cardTitle(view)}</Subject>;
    case "mount":
      return <Subject>{view.id}</Subject>;
    case "lsp":
      return <LspSubject view={view} target={fileTarget(card)} />;
    case "task":
      return <Subject>{view.agents.map((agent) => agent.name).join(", ")}</Subject>;
    default:
      return null;
  }
}

/** An `lsp` call's subject: its action, then its file (opening at `target`) or `*`, its line and its symbol. */
function LspSubject({
  view,
  target,
}: {
  view: Extract<ToolView, { kind: "lsp" }>;
  target: { path: string; open: OpenFileOptions } | undefined;
}): ReactNode {
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

/**
 * The body of a specialized card; its sections are split by labelled rules on the card's border (UI-187). `threadId`
 * is the thread whose transcript shows the card.
 */
export function ToolBody({ threadId, card }: { threadId: string; card: ToolCard }): ReactNode {
  const { view } = card;
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
    default:
      // Every other card is one section, a failed call's error (omp's message) first (UI-052).
      return (
        <div className="omfp-tool-card__section">
          {card.status === "failed" && card.result !== undefined && (
            <div className="omfp-tool-card__error" data-testid="tool-error">
              {card.result}
            </div>
          )}
          <ToolSection threadId={threadId} card={card} view={view} />
        </div>
      );
  }
}

/** The section of a card other than `bash`'s and the fallback's, after its error. */
function ToolSection({
  threadId,
  card,
  view,
}: {
  threadId: string;
  card: ToolCard;
  view: Exclude<ToolView, { kind: "bash" | "generic" }>;
}): ReactNode {
  const openFile = useOpenFile();
  switch (view.kind) {
    case "read":
      return (
        <>
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
        </>
      );
    case "fetch":
      return (
        <>
          <a className="omfp-tool-card__link" href={view.url} target="_blank" rel="noreferrer" data-testid="tool-link">
            {view.url}
          </a>
          {card.status !== "failed" && view.content !== undefined && (
            <Code code={view.content} block language="markdown" data-testid="tool-content" />
          )}
        </>
      );
    case "edit":
      if (view.diff !== undefined) {
        return <Diff lines={view.diff} language={view.language} data-testid="tool-diff" />;
      }
      if (view.preview.length > 0) {
        return <EditPreview files={view.preview} />;
      }
      return (
        view.input !== undefined && (
          <>
            <h3>{text.edit}</h3>
            <Code code={view.input} block data-testid="tool-input" />
          </>
        )
      );
    case "write":
      return view.content !== undefined && <WriteContent content={view.content} language={view.language} />;
    case "search":
      return (
        <>
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
        </>
      );
    case "ast_edit":
      return (
        <>
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
                  <LineLink path={file.path} line={file.diff[0]?.oldLine ?? file.diff[0]?.newLine}>
                    <FilePath path={file.path} />
                  </LineLink>
                  {file.count !== undefined && <Badge>{file.count}</Badge>}
                </div>
                {file.diff.length > 0 && (
                  <Diff lines={file.diff} language={languageOfPath(file.path)} data-testid="tool-diff" />
                )}
              </li>
            ))}
          </ul>
          {view.truncated && <p className="omfp-tool-card__meta">{text.truncated}</p>}
        </>
      );
    case "resolve":
      return (
        <>
          {view.action === "discard" && <p className="omfp-tool-card__meta">{text.discarded}</p>}
          {view.reason !== undefined && <p className="omfp-tool-card__task">{view.reason}</p>}
          <ChangedFiles files={view.files} />
        </>
      );
    case "lsp":
      return (
        <>
          {card.status !== "failed" && view.result !== undefined && <LspAnswer result={view.result} call={view} />}
          {view.noServer && (
            <p className="omfp-tool-card__meta" data-testid="lsp-environment">
              {text.serverEnvironment}
            </p>
          )}
        </>
      );
    case "todo":
      return <TodoPhases phases={view.phases} />;
    case "web_search":
      return (
        <>
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
        </>
      );
    case "ask":
      return (
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
      );
    case "task":
      return (
        <ul className="omfp-tool-card__agents">
          {view.agents.map((agent) => (
            <Subagent key={agent.name} agent={agent} />
          ))}
        </ul>
      );
    case "card":
      // The widget mounts once the call's arguments are complete; until then its code shows as it is written (UI-056).
      return card.status === "composing" ? (
        view.code !== undefined && <WriteContent content={view.code} language="tsx" />
      ) : (
        <CardSection threadId={threadId} toolCallId={card.toolCallId} view={view} />
      );
    case "mount":
      // The mounted widget's code as it is written (UI-054), else its reference; then what the daemon answered.
      return (
        <>
          {view.code !== undefined ? (
            <WriteContent content={view.code} language="tsx" />
          ) : (
            view.reference !== undefined && <p className="omfp-tool-card__meta">{view.reference}</p>
          )}
          {card.status === "succeeded" && card.result !== undefined && (
            <p className="omfp-tool-card__meta" data-testid="tool-result">
              {card.result}
            </p>
          )}
        </>
      );
  }
}

/**
 * omp's preview of an `edit` while the model writes it and it runs (UI-055): each file's diff so far, or why it does
 * not apply; with several files, or a moved one, each under its path.
 */
function EditPreview({ files }: { files: readonly EditPreviewFile[] }): ReactNode {
  const named = files.length > 1 || files.some((file) => file.rename !== undefined);
  return files.map((file) => (
    <Fragment key={file.path}>
      {named && (
        <div className="omfp-tool-card__file" data-testid="tool-preview-file">
          <FilePath path={file.path} />
          {file.rename !== undefined && (
            <>
              {" → "}
              <FilePath path={file.rename} />
            </>
          )}
        </div>
      )}
      {file.error !== undefined && (
        <div className="omfp-tool-card__error" data-testid="tool-preview-error">
          {file.error}
        </div>
      )}
      {file.diff !== undefined && <Diff lines={file.diff} language={file.language} data-testid="tool-diff" />}
    </Fragment>
  ));
}

type CardView = Extract<ToolView, { kind: "card" }>;

/** What a card is called (UI-920): its title, else its widget reference, else "card". */
function cardTitle(view: CardView): string {
  return view.title ?? view.reference ?? text.card;
}

/**
 * An `omfp_card` call's widget (UI-920) in the scope `useAnswer` reads (UI-921), and while the call waits, Cancel
 * outside the widget, so a broken widget never blocks the agent (UI-922).
 */
function CardSection({
  threadId,
  toolCallId,
  view,
}: {
  threadId: string;
  toolCallId: string;
  view: CardView;
}): ReactNode {
  const { answer } = view;
  const scope = useMemo(
    () => ({ threadId, toolCallId, state: answer.state, value: answer.value }),
    [threadId, toolCallId, answer],
  );
  return (
    <CardScope value={scope}>
      <CardWidget
        threadId={threadId}
        toolCallId={toolCallId}
        reference={view.reference}
        code={view.code}
        props={view.props}
      />
      {answer.state === "waiting" && <CardCancel title={cardTitle(view)} />}
    </CardScope>
  );
}

/** The Cancel of a waiting card: `card.answer` with `cancel` (UI-922); a refusal goes to the error toasts. */
function CardCancel({ title }: { title: string }): ReactNode {
  const { cancel } = useAnswer();
  const reportError = useReportError();
  return (
    <Button
      size="sm"
      aria-label={text.cancelCardLabel(title)}
      data-testid="card-cancel"
      onPress={() => cancel().catch(reportError)}
    >
      {text.cancelCard}
    </Button>
  );
}

/**
 * An `ast_grep` file's matches (UI-916): each match's first line opens the file there, its code is highlighted in the
 * file's language, its metavariable captures under it.
 */
function Matches({ path, matches }: { path: string; matches: readonly AstMatch[] }): ReactNode {
  const language = languageOfPath(path);
  return (
    <ol className="omfp-tool-card__matches">
      {matches.map((match, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: a result's matches never move
        <li key={index} data-testid="tool-match">
          <LineLink path={path} line={match.line} data-testid="tool-match-line">
            {match.line}
          </LineLink>
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

/** A link opening `path` at `line` (UI-116). */
function LineLink({
  path,
  line,
  ...link
}: { path: string; line: number | undefined } & Pick<PathLinkProps, "children" | "data-testid">): ReactNode {
  const openFile = useOpenFile();
  return <PathLink reference={{ path, line }} onOpen={(ref) => openFile(ref.path, { line: ref.line })} {...link} />;
}

/** `path:line:col`, opening the file at the line (UI-116). */
function Location({ path, line, column }: { path: string; line: number; column?: number | undefined }): ReactNode {
  return (
    <LineLink path={path} line={line} data-testid="lsp-location">
      <FilePath path={path} />
      <span className="omfp-tool-card__selector">
        :{line}
        {column !== undefined && `:${column}`}
      </span>
    </LineLink>
  );
}

/** Files of a long list of changed files shown until it is expanded (UI-919). */
const FILES_SHOWN = 8;

/**
 * Files a workspace edit or an applied rewrite changed, each opening against `HEAD` (UI-917, UI-919); with `preview`,
 * the files an edit that was not applied would change, each opening as it is (the call's file at the call's line). A
 * long list shows its first files, expandable to all of them.
 */
function ChangedFiles({
  files,
  preview,
}: {
  files: readonly EditedFile[];
  preview?: { readonly file: string | undefined; readonly line: number | undefined } | undefined;
}): ReactNode {
  const openFile = useOpenFile();
  const [expanded, setExpanded] = useState(false);
  if (files.length === 0) {
    return null;
  }
  const hidden = files.length - FILES_SHOWN;
  return (
    <>
      <ul className="omfp-tool-card__files">
        {(expanded || hidden <= 0 ? files : files.slice(0, FILES_SHOWN)).map((file) => (
          <li key={file.path} data-testid="tool-file">
            <div className="omfp-tool-card__file">
              {file.removed ? (
                <FilePath path={file.path} />
              ) : (
                <PathLink
                  reference={{ path: file.path, line: file.path === preview?.file ? preview.line : undefined }}
                  diff={preview === undefined}
                  onOpen={(ref) => openFile(ref.path, preview === undefined ? { diff: true } : { line: ref.line })}
                >
                  <FilePath path={file.path} />
                </PathLink>
              )}
              <span className="omfp-tool-card__meta">{file.note}</span>
            </div>
          </li>
        ))}
      </ul>
      {hidden > 0 && (
        <Button
          size="sm"
          variant="quiet"
          aria-expanded={expanded}
          data-testid="tool-more"
          onPress={() => setExpanded((open) => !open)}
        >
          {expanded ? text.fewerFiles : text.moreFiles(hidden)}
        </Button>
      )}
    </>
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
 * symbols at their lines, the files an edit changed (a symbol rename's old and new names first), the code actions
 * offered, hover documentation as Markdown, else omp's text.
 */
function LspAnswer({ result, call }: { result: LspResult; call: Extract<ToolView, { kind: "lsp" }> }): ReactNode {
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
                  <LineLink path={symbol.path} line={symbol.line} data-testid="lsp-symbol-line">
                    {result.workspace && <FilePath path={symbol.path} />}:{symbol.line}
                  </LineLink>
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
            {result.total !== undefined && <span data-testid="lsp-edit-total">{result.total}</span>}
            {!result.applied && <span>{text.notApplied}</span>}
          </p>
          {result.rename !== undefined && (
            <p className="omfp-tool-card__meta">
              <span data-testid="lsp-rename">
                {result.rename.from === undefined ? (
                  `${text.renamedTo} `
                ) : (
                  <>
                    <Code code={result.rename.from} /> →{" "}
                  </>
                )}
                <Code code={result.rename.to} />
              </span>
            </p>
          )}
          <ChangedFiles
            files={result.files}
            preview={result.applied ? undefined : { file: call.file, line: call.line }}
          />
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
 * not; a click (or Enter, Space) opens one at full size in a dialog, zoomed and panned like a file's image, that Escape
 * closes.
 */
export function ToolImages({ card }: { card: ToolCard }): ReactNode {
  const viewer = useImageViewer();
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
              onPress={() =>
                viewer.open({ src: imageSource(image), alt: image.alt, title: image.alt, testId: "tool-image-dialog" })
              }
              data-testid="tool-image"
            >
              <img className="omfp-tool-images__img" src={imageSource(image)} alt={image.alt} decoding="async" />
            </Button>
          </li>
        ))}
      </ul>
      {viewer.dialog}
    </>
  );
}
