/**
 * Specialized tool cards (UI-050, UI-051): what each card shows, read from the argument and result shapes recorded
 * from the pinned omp (`fixtures/threads/<omp>/*tool_call*.jsonl`, `crates/omfp-daemon/tests/contract_tool_cards.rs`).
 * Tools without a recorded shape keep the fallback card (`generic`). omp's discoverable tools run as tool commands
 * inside a `bash` call (`lsp diagnostics src/a.ts`), and ran as `xd://` devices in omp 18.x (a `write` of their JSON
 * arguments to `xd://<tool>`): `deviceCall` turns such a call back into one of the tool (UI-915).
 */

import type { BackgroundJob, JobState } from "@omfp/core";
import type { DiffLine } from "@omfp/kit";
import { languageOfPath } from "@omfp/kit";
import type { Workspace } from "@omfp/protocol";
import { isObject as isRecord } from "../sdk/json.ts";
import { strings } from "../strings/index.ts";
import { type AstMatch, astEditDiffs, astMatches, type EditedFile, type LspResult, lspResult } from "./code-intel.ts";

const text = strings.chat.tool;

/** A task of the `todo` tool. */
export interface TodoTask {
  readonly content: string;
  /** omp's status: `pending`, `in_progress`, `completed`, or another it may add. */
  readonly status: string;
}

/** One subagent of a `task` call. */
export interface SubagentRun {
  readonly name: string;
  /** The agent kind (`task`, `scout`, …). */
  readonly agent: string;
  readonly task: string;
  /**
   * `pending` until omp reports it, then omp's status (`running`, `completed`, `failed`, `aborted`, …); `failed` when the
   * call ended without its result.
   */
  readonly status: string;
  /** What it does now (its current tool's intent), else what it did last. */
  readonly activity: string | undefined;
  /** Tool calls so far. */
  readonly tools: number;
  /** Its final output, else its latest output line. */
  readonly output: string | undefined;
}

/** What the card of one call shows besides its header. */
export type ToolView =
  | { readonly kind: "generic" }
  | {
      readonly kind: "bash";
      readonly command: string;
      /** The directory given to the call, when not the thread's. */
      readonly cwd: string | undefined;
      /** The output so far while it runs, then the result's text; a background job's own output. */
      readonly output: string | undefined;
      readonly exitCode: number | undefined;
      /** The omp background job the call became (UI-270): its id and state. */
      readonly background: { readonly jobId: string; readonly state: JobState } | undefined;
    }
  | {
      readonly kind: "read";
      readonly path: string;
      /** The selector the path was given with (`50-80`), shown after it in a read group (UI-765). */
      readonly selector: string | undefined;
      /** The selector of the path (`50-80`), else the lines omp returned (`1–9 of 9`). */
      readonly range: string | undefined;
      /** The first line it read (the selector's, else the first line omp returned), where the file opens. */
      readonly line: number | undefined;
      readonly language: string | undefined;
      readonly content: string | undefined;
      readonly directory: boolean;
    }
  | {
      readonly kind: "fetch";
      readonly url: string;
      readonly content: string | undefined;
      /** omp's notes on the fetch (`Failed to fetch URL (HTTP 404)`). */
      readonly notes: readonly string[];
    }
  | {
      readonly kind: "edit";
      readonly path: string | undefined;
      /** The change omp applied (`details.diff`). */
      readonly diff: readonly DiffLine[] | undefined;
      /** The edit as the agent wrote it (shown while it runs and when it failed). */
      readonly input: string | undefined;
      readonly language: string | undefined;
    }
  | {
      readonly kind: "write";
      readonly path: string;
      readonly content: string | undefined;
      readonly language: string | undefined;
    }
  | {
      readonly kind: "search";
      readonly query: string;
      /** Where it searched (`grep`'s path, the folder of a `glob`). */
      readonly scope: string | undefined;
      /** Matching files with their match counts when known, and an `ast_grep`'s matches (UI-916). */
      readonly files: readonly SearchFile[] | undefined;
      /** `2 matches in 1 file`. */
      readonly total: string | undefined;
      readonly truncated: boolean;
    }
  | {
      readonly kind: "ast_edit";
      /** The first rewrite's pattern, ` (+N more)` when there are several. */
      readonly pattern: string;
      /** The files the rewrite changes, each with its changed code as diff lines (UI-917). */
      readonly files: readonly AstEditFile[];
      /** `1 replacement in 1 file`. */
      readonly total: string | undefined;
      /** omp stages an `ast_edit` as a preview until the agent resolves it (the `resolve` command). */
      readonly applied: boolean;
      readonly truncated: boolean;
    }
  | {
      readonly kind: "resolve";
      /** `apply` or `discard`, omp's word for what the agent did with the staged preview. */
      readonly action: string;
      /** What was staged (`AST Edit: 1 replacement in 1 file`). */
      readonly label: string | undefined;
      readonly reason: string | undefined;
      /** The files an applied preview changed (UI-917). */
      readonly files: readonly EditedFile[];
    }
  | {
      readonly kind: "lsp";
      readonly action: string;
      /** The file it asked about (`*` for the workspace), the line and the symbol there. */
      readonly file: string | undefined;
      readonly line: number | undefined;
      readonly symbol: string | undefined;
      /** The answer once it came (UI-918, UI-919). */
      readonly result: LspResult | undefined;
      /** omp found no language server for the file or action: it starts them from the daemon's environment. */
      readonly noServer: boolean;
    }
  | { readonly kind: "todo"; readonly op: string; readonly phases: readonly TodoPhase[] }
  | {
      readonly kind: "web_search";
      readonly query: string;
      readonly links: readonly { readonly title: string; readonly url: string; readonly snippet: string }[];
    }
  | {
      readonly kind: "ask";
      readonly questions: readonly { readonly question: string; readonly answer: string | undefined }[];
    }
  | { readonly kind: "task"; readonly context: string | undefined; readonly agents: readonly SubagentRun[] }
  | {
      readonly kind: "card";
      /** The widget reference of an `omfp_card` call (AGT-096), a bare name as `./widgets/<name>`; `undefined` for a
       * `code` card. */
      readonly reference: string | undefined;
      /** The source of a one-file card (PRT-225). */
      readonly code: string | undefined;
      /** The component's props. */
      readonly props: Readonly<Record<string, unknown>>;
      readonly title: string | undefined;
      readonly wait: boolean;
      readonly answer: CardAnswerView;
    };

/** Where the answer of a card stands (UI-921), what `useAnswer().state` says. */
export type AnswerState = "none" | "waiting" | "submitted" | "cancelled" | "timeout" | "aborted";

/** A card's answer: its state and, once `submitted`, the submitted value. */
export interface CardAnswerView {
  readonly state: AnswerState;
  readonly value: unknown;
}

/** A phase of the `todo` list. */
export interface TodoPhase {
  readonly name: string;
  readonly tasks: readonly TodoTask[];
}

/** A file of a search result. */
export interface SearchFile {
  readonly path: string;
  readonly count: number | undefined;
  /** An `ast_grep`'s matches in the file; none for `grep` and `glob`, whose details list files only. */
  readonly matches: readonly AstMatch[];
}

/** A file an `ast_edit` rewrites: its replacement count and the changed code. */
export interface AstEditFile {
  readonly path: string;
  readonly count: number | undefined;
  readonly diff: readonly DiffLine[];
}

/** An image of a tool result (`{ type: "image", data, mimeType }`): base64 `data` of type `mime`. */
export interface ResultImage {
  readonly mime: string;
  readonly data: string;
}

/** A tool result reduced to what the cards read: its text, its images and its `details`. */
export interface ToolOutput {
  readonly text: string | undefined;
  readonly images: readonly ResultImage[];
  readonly details: Readonly<Record<string, unknown>>;
}

function str(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function num(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function list(value: unknown): readonly unknown[] {
  return Array.isArray(value) ? value : [];
}

function records(value: unknown): readonly Readonly<Record<string, unknown>>[] {
  return list(value).filter(isRecord);
}

/**
 * The text parts, the image parts and the `details` of a tool result (`{ content: [{ type: "text", text }, { type:
 * "image", data, mimeType }], details }`, UI-472).
 */
export function toolOutput(value: unknown): ToolOutput | undefined {
  if (!isRecord(value)) {
    return undefined;
  }
  const parts = records(value.content);
  const texts = parts.flatMap((part) => (part.type === "text" ? [str(part.text) ?? ""] : []));
  return {
    text: texts.length === 0 ? undefined : texts.join("\n"),
    images: parts.flatMap((part) => {
      const data = str(part.data);
      return part.type === "image" && data !== undefined && data !== ""
        ? [{ mime: str(part.mimeType) ?? "image/png", data }]
        : [];
    }),
    details: isRecord(value.details) ? value.details : {},
  };
}

/**
 * Why a call that omp did not flag `isError` still failed: `web_search` reports engine failures in `details.error`,
 * a `read` of a URL that could not be fetched has `details.method` `failed`, an `lsp` call that failed has
 * `details.success` `false` and says why in its text (UI-918).
 */
export function quietFailure(name: string, output: ToolOutput | undefined): string | undefined {
  if (output === undefined) {
    return undefined;
  }
  const { details } = output;
  if (name === "web_search") {
    return str(details.error);
  }
  if (name === "read" && details.kind === "url" && details.method === "failed") {
    const notes = list(details.notes).flatMap((note) => (typeof note === "string" ? [note] : []));
    return notes.join("\n") || text.fetchFailed;
  }
  if (name === "lsp" && details.success === false) {
    return output.text ?? text.failed;
  }
  return undefined;
}

/** A call of an omp tool run as a tool command (or an 18.x `xd://` device): the tool, its arguments and result. */
export interface DeviceCall {
  readonly name: string;
  /**
   * The validated arguments omp echoes (`details.toolCommands[0].args`, `details.xdev.args`), else the written JSON;
   * `undefined` while partial.
   */
  readonly args: Readonly<Record<string, unknown>> | undefined;
  /** The result with the tool's own details (the dispatch's `inner`). */
  readonly output: ToolOutput | undefined;
  /**
   * The tool command's own exit code (`details.toolCommands[0].exitCode`): non-zero fails the card even when the bash
   * call around it succeeded (`ast_grep … || true`); `undefined` for an `xd://` device.
   */
  readonly exitCode?: number | undefined;
}

/** `xd://<tool>`: a device's URL (`xd://<tool>/<topic>` is a doc topic, read only). */
const DEVICE_URL = /^xd:\/\/([\w.-]+)$/;
/** The `path` of a `write` whose arguments the model is still writing. */
const PARTIAL_PATH = /"path"\s*:\s*"(xd:\/\/[^"]*)"/;

/**
 * The tool call a `bash` call or a `write` to `xd://<tool>` runs (UI-915); `undefined` for any other call.
 * omp 19 runs its discoverable tools (`lsp`, `ast_grep`, `ast_edit`, MCP tools, …) as commands in a `bash` call and
 * lists the dispatches in the result's `details.toolCommands`: a call whose result holds exactly one `execute` dispatch
 * is that tool's (several dispatches, help only or no details yet stay a bash card). omp 18.x mounted them as devices
 * the model wrote JSON arguments to; sessions recorded by it still replay so. `partial` is the argument text the model
 * is writing.
 */
export function deviceCall(
  name: string,
  args: unknown,
  partial: string | undefined,
  output: ToolOutput | undefined,
): DeviceCall | undefined {
  if (name === "bash") {
    return commandCall(output);
  }
  if (name !== "write") {
    return undefined;
  }
  // While the model writes them, the call's arguments are `{}` beside the partial text: the path is in the text.
  const path =
    (isRecord(args) ? str(args.path) : undefined) ??
    (partial === undefined ? undefined : PARTIAL_PATH.exec(partial)?.[1]);
  const device = path === undefined ? undefined : DEVICE_URL.exec(path)?.[1];
  if (device === undefined) {
    return undefined;
  }
  const xdev = isRecord(output?.details.xdev) ? output.details.xdev : undefined;
  const echoed = isRecord(xdev?.args) ? xdev.args : undefined;
  return {
    name: device,
    args: echoed ?? writtenArguments(isRecord(args) ? str(args.content) : undefined),
    output: output === undefined ? undefined : { ...output, details: isRecord(xdev?.inner) ? xdev.inner : {} },
  };
}

/** The tool a `bash` result's single `execute` dispatch (`details.toolCommands`) ran. */
function commandCall(output: ToolOutput | undefined): DeviceCall | undefined {
  const dispatches = output === undefined ? [] : list(output.details.toolCommands);
  const dispatch = dispatches.length === 1 ? dispatches[0] : undefined;
  if (output === undefined || !isRecord(dispatch) || dispatch.mode !== "execute") {
    return undefined;
  }
  const tool = str(dispatch.tool);
  if (tool === undefined) {
    return undefined;
  }
  return {
    name: tool,
    args: isRecord(dispatch.args) ? dispatch.args : {},
    output: {
      ...output,
      text: output.text?.replace(EXIT_LINE, ""),
      details: isRecord(dispatch.inner) ? dispatch.inner : {},
    },
    exitCode: num(dispatch.exitCode),
  };
}

/** The status line omp's `bash` appends to the output of a failed command: the bash call's, not the tool's. */
const EXIT_LINE = /\n*Command exited with code -?\d+$/;

/** A device write's content as its JSON arguments object, if it is one. */
function writtenArguments(content: string | undefined): Readonly<Record<string, unknown>> | undefined {
  if (content === undefined) {
    return undefined;
  }
  try {
    const parsed: unknown = JSON.parse(content);
    return isRecord(parsed) ? parsed : undefined;
  } catch {
    return undefined;
  }
}

/** `path:50-80` (omp's selector syntax) split into the path and its selector. */
function splitSelector(path: string): { path: string; selector: string | undefined } {
  const match = /^(.*?):((?:raw:)?\d+(?:[-+]\d*)?(?:,\d+(?:[-+]\d*)?)*(?::raw)?)$/.exec(path);
  return match === null ? { path, selector: undefined } : { path: match[1] ?? path, selector: match[2] };
}

/** omp's edit diff (` 2|ctx`, `-4|old`, `+4|new`) as diff lines; other lines (`...`) become separators. */
export function ompDiffLines(diff: string): DiffLine[] {
  if (diff === "") {
    return [];
  }
  return diff.split("\n").map((line): DiffLine => {
    const match = /^([ +-])\s*(\d+)\|(.*)$/.exec(line);
    if (match === null) {
      return { kind: "hunk", text: line };
    }
    const number = Number(match[2]);
    const body = match[3] ?? "";
    switch (match[1]) {
      case "+":
        return { kind: "add", text: body, newLine: number };
      case "-":
        return { kind: "del", text: body, oldLine: number };
      default:
        return { kind: "context", text: body, newLine: number };
    }
  });
}

/** The file a hashline `edit` input targets: its first `[path#TAG]` header. */
function editPath(input: string | undefined): string | undefined {
  const match = input === undefined ? null : /^\[(.+?)#[0-9A-Za-z]{4}\]/m.exec(input);
  return match?.[1];
}

/** The questions of an `ask` form in order: each `<fieldset>`'s `<legend>`, and each `<label>` outside a fieldset. */
function askQuestions(html: string): string[] {
  const questions = [
    ...html.matchAll(/<fieldset\b[^>]*>([\s\S]*?)<\/fieldset>|<label\b[^>]*>([\s\S]*?)<\/label>/gi),
  ].map((match) => {
    const legend = match[1] === undefined ? undefined : /<legend[^>]*>([\s\S]*?)<\/legend>/i.exec(match[1])?.[1];
    return (legend ?? match[2] ?? "").replace(/<[^>]*>/g, "").trim();
  });
  return questions.filter((question) => question !== "");
}

/**
 * The arguments a specialized card shows outside its header's argument summary: its subject (command, path, query,
 * pattern) and the text of its body (a write's content, an edit's input, the questions of an ask's form).
 */
const SHOWN_ARGUMENTS: Readonly<Record<string, readonly string[]>> = {
  bash: ["command"],
  read: ["path"],
  edit: ["path", "input"],
  write: ["path", "content"],
  grep: ["pattern"],
  glob: ["path"],
  ast_grep: ["pat"],
  ast_edit: ["ops"],
  lsp: ["action", "file", "line", "symbol"],
  web_search: ["query"],
  ask: ["html"],
  omfp_card: ["widget", "code", "props", "title"],
};

/** The arguments of a call of tool `name` its card shows elsewhere than the header's summary; `undefined` if none. */
export function shownArguments(name: string): readonly string[] | undefined {
  return Object.hasOwn(SHOWN_ARGUMENTS, name) ? SHOWN_ARGUMENTS[name] : undefined;
}

function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`;
}

/**
 * The specialized view of a call of tool `name`. `args` are its arguments (or `undefined` while the model writes them),
 * `output` the result once it ended or the latest partial result while it runs, `succeeded` whether it ended without
 * an error (a `bash` that did so without an exit code in its details exited 0; an aborted one has none), `job` the
 * background job the call became (a `bash` then shows the job's output and exit code, a completed job exited 0),
 * `ended` whether the call ended (succeeded or failed).
 */
export function toolView(
  name: string,
  args: unknown,
  output: ToolOutput | undefined,
  succeeded: boolean,
  job: BackgroundJob | null = null,
  ended = false,
): ToolView {
  const input = isRecord(args) ? args : {};
  const details = output?.details ?? {};
  switch (name) {
    case "bash":
      return {
        kind: "bash",
        command: str(input.command) ?? "",
        cwd: str(input.cwd),
        ...(job === null
          ? {
              output: output?.text,
              exitCode: num(details.exitCode) ?? (succeeded && output !== undefined ? 0 : undefined),
              background: undefined,
            }
          : {
              output: job.output ?? undefined,
              exitCode: job.exit_code ?? (job.state === "completed" ? 0 : undefined),
              background: { jobId: job.job_id, state: job.state },
            }),
      };
    case "read": {
      const raw = str(input.path) ?? "";
      if (/^https?:\/\//i.test(raw) || details.kind === "url") {
        return {
          kind: "fetch",
          url: str(details.finalUrl) ?? str(details.url) ?? raw,
          content: output?.text,
          notes: list(details.notes).flatMap((note) => (typeof note === "string" ? [note] : [])),
        };
      }
      const { path, selector } = splitSelector(raw);
      const shown = isRecord(details.displayContent) ? details.displayContent : undefined;
      const numbers = list(shown?.lineNumbers).flatMap((line) => (typeof line === "number" ? [line] : []));
      const total = num(details.totalLines);
      const first = numbers[0];
      const last = numbers.at(-1);
      return {
        kind: "read",
        path,
        selector,
        range:
          selector ??
          (first !== undefined && last !== undefined && total !== undefined
            ? text.lines(first, last, total)
            : undefined),
        line: selector === undefined ? first : Number(/^(?:raw:)?(\d+)/.exec(selector)?.[1] ?? first),
        language: languageOfPath(path),
        content: str(shown?.text) ?? output?.text,
        directory: details.isDirectory === true,
      };
    }
    case "edit": {
      const edit = str(input.input);
      const path = str(input.path) ?? editPath(edit);
      const diff = str(details.diff);
      return {
        kind: "edit",
        path,
        diff: diff === undefined || diff === "" ? undefined : ompDiffLines(diff),
        input: edit,
        language: path === undefined ? undefined : languageOfPath(path),
      };
    }
    case "write": {
      const path = str(input.path) ?? "";
      return { kind: "write", path, content: str(input.content), language: languageOfPath(path) };
    }
    case "grep":
    case "glob":
    case "ast_grep": {
      const fileMatches = records(details.fileMatches);
      const single = fileMatches.length === 1 ? str(fileMatches[0]?.path) : undefined;
      const found =
        name === "ast_grep"
          ? astMatches(str(details.displayContent) ?? output?.text ?? "", single)
          : new Map<string, AstMatch[]>();
      const files =
        fileMatches.length > 0
          ? fileMatches.map((file) => {
              const path = str(file.path) ?? "";
              return { path, count: num(file.count), matches: found.get(path) ?? [] };
            })
          : Array.isArray(details.files)
            ? list(details.files).flatMap((file) =>
                typeof file === "string" ? [{ path: file, count: undefined, matches: [] }] : [],
              )
            : undefined;
      const matches = num(details.matchCount);
      const fileCount = num(details.fileCount) ?? files?.length;
      return {
        kind: "search",
        query: str(name === "grep" ? input.pattern : name === "ast_grep" ? input.pat : input.path) ?? "",
        scope: name === "glob" ? str(details.scopePath) : str(input.path),
        files,
        total:
          fileCount === undefined
            ? undefined
            : matches === undefined
              ? plural(fileCount, "file", "files")
              : `${plural(matches, "match", "matches")} in ${plural(fileCount, "file", "files")}`,
        truncated: details.truncated === true || details.limitReached === true,
      };
    }
    case "ast_edit": {
      const ops = records(input.ops);
      const first = str(ops[0]?.pat) ?? "";
      const replacements = records(details.fileReplacements);
      const single = replacements.length === 1 ? str(replacements[0]?.path) : undefined;
      const diffs = astEditDiffs(str(details.displayContent) ?? "", single);
      const total = num(details.totalReplacements);
      const files = replacements.map((file) => {
        const path = str(file.path) ?? "";
        return { path, count: num(file.count), diff: diffs.get(path) ?? [] };
      });
      const touched = num(details.filesTouched) ?? files.length;
      return {
        kind: "ast_edit",
        pattern: ops.length > 1 ? text.morePatterns(first, ops.length - 1) : first,
        files,
        total:
          total === undefined
            ? undefined
            : `${plural(total, "replacement", "replacements")} in ${plural(touched, "file", "files")}`,
        applied: details.applied === true,
        truncated: details.limitReached === true,
      };
    }
    case "resolve": {
      const action = str(details.action) ?? "";
      const source = isRecord(details.sourceResultDetails) ? details.sourceResultDetails : {};
      return {
        kind: "resolve",
        action,
        label: str(details.label),
        reason: str(details.reason) ?? str(input.reason),
        files:
          action === "apply"
            ? records(source.fileReplacements).map((file) => ({
                path: str(file.path) ?? "",
                note: plural(num(file.count) ?? 0, "replacement", "replacements"),
                edits: undefined,
                removed: false,
              }))
            : [],
      };
    }
    case "lsp": {
      const action = str(input.action) ?? str(details.action) ?? "";
      const answer = output?.text;
      const symbol = str(input.symbol);
      return {
        kind: "lsp",
        action,
        file: str(input.file),
        line: num(input.line),
        symbol,
        result: answer === undefined ? undefined : lspResult(action, answer, { symbol, newName: str(input.new_name) }),
        // omp 19.1: "No language server handles .txt files", "… found for this file", "… servers configured".
        noServer: answer?.includes("No language server") === true,
      };
    }
    case "todo":
      return {
        kind: "todo",
        op: str(input.op) ?? "",
        phases: records(details.phases).map((phase) => ({
          name: str(phase.name) ?? "",
          tasks: records(phase.tasks).map((task) => ({
            content: str(task.content) ?? "",
            status: str(task.status) ?? "pending",
          })),
        })),
      };
    case "web_search": {
      const response = isRecord(details.response) ? details.response : {};
      return {
        kind: "web_search",
        query: str(input.query) ?? "",
        links: records(response.sources).map((source) => ({
          title: str(source.title) ?? str(source.url) ?? "",
          url: str(source.url) ?? "",
          snippet: str(source.snippet) ?? "",
        })),
      };
    }
    case "ask": {
      const answers = records(details.answers);
      if (answers.length > 0) {
        return {
          kind: "ask",
          questions: answers.map((answer) => ({
            question: str(answer.question) ?? str(answer.name) ?? "",
            answer: str(answer.answer),
          })),
        };
      }
      return {
        kind: "ask",
        questions: askQuestions(str(input.html) ?? "").map((question) => ({ question, answer: undefined })),
      };
    }
    case "task":
      return { kind: "task", context: str(input.context), agents: subagents(input, details, ended) };
    case "omfp_card": {
      const widget = str(input.widget);
      const wait = input.wait === true;
      return {
        kind: "card",
        // A bare name means the workspace's `./widgets/<name>` (AGT-096).
        reference:
          widget === undefined || widget.includes(":") || widget.startsWith("./") ? widget : `./widgets/${widget}`,
        code: str(input.code),
        props: isRecord(input.props) ? input.props : {},
        title: str(input.title),
        wait,
        answer: cardAnswer(wait, ended, succeeded, output?.text),
      };
    }
    default:
      return { kind: "generic" };
  }
}

const NO_ANSWER: CardAnswerView = { state: "none", value: undefined };

/**
 * The answer of an `omfp_card` call (UI-921): `waiting` while a `wait` call runs, `aborted` once it ended in an error,
 * else what its JSON text says (`{"status":"submitted","value":…}`, `cancelled`, `timeout`; `shown` is `none`).
 */
function cardAnswer(wait: boolean, ended: boolean, succeeded: boolean, answer: string | undefined): CardAnswerView {
  if (!ended) {
    return wait ? { state: "waiting", value: undefined } : NO_ANSWER;
  }
  if (!succeeded) {
    return { state: "aborted", value: undefined };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(answer ?? "");
  } catch {
    return NO_ANSWER;
  }
  const record: Readonly<Record<string, unknown>> = isRecord(parsed) ? parsed : {};
  switch (record.status) {
    case "submitted":
      return { state: "submitted", value: record.value };
    case "cancelled":
      return { state: "cancelled", value: undefined };
    case "timeout":
      return { state: "timeout", value: undefined };
    default:
      return NO_ANSWER;
  }
}

/**
 * The subagents of a `task` call: what it asked for, with omp's progress while it runs and the results once done. One
 * without a result once the call `ended` never ran to the end: `failed`, not `pending` (UI-406).
 */
function subagents(
  input: Readonly<Record<string, unknown>>,
  details: Readonly<Record<string, unknown>>,
  ended: boolean,
): SubagentRun[] {
  const progress = records(details.progress);
  const results = records(details.results);
  return records(input.tasks).map((task, index) => {
    const name = str(task.name) ?? String(index + 1);
    const result = results.find((entry) => entry.id === name);
    const live = progress.find((entry) => entry.id === name);
    const status = result !== undefined ? resultStatus(result) : ended ? "failed" : (str(live?.status) ?? "pending");
    const recentOutput = list(live?.recentOutput).flatMap((line) => (typeof line === "string" ? [line] : []));
    return {
      name,
      agent: str(task.agent) ?? "",
      task: str(task.task) ?? "",
      status,
      activity: str(live?.currentToolIntent) ?? str(result?.lastIntent) ?? str(live?.lastIntent),
      tools: num(live?.toolCount) ?? list(live?.recentTools).length,
      output: str(result?.output) ?? recentOutput.at(-1),
    };
  });
}

function resultStatus(result: Readonly<Record<string, unknown>>): string {
  if (result.aborted === true) {
    return "aborted";
  }
  const exitCode = num(result.exitCode);
  return exitCode === undefined || exitCode === 0 ? "completed" : "failed";
}

/** A path as the card shows it: in a repository of the workspace (its chip) or as given. */
export interface RepoPath {
  readonly repo: string | undefined;
  readonly path: string;
}

/**
 * `path` (absolute, or relative to the workspace folder, the agent's cwd) relative to the repository of `workspace`
 * holding it, with that repository's name; as given when no repository holds it.
 */
export function repoPath(path: string, workspace: Pick<Workspace, "path" | "repos"> | undefined): RepoPath {
  if (workspace === undefined) {
    return { repo: undefined, path };
  }
  const folder = workspace.path.replace(/\/+$/, "");
  const inside = path.startsWith(`${folder}/`)
    ? path.slice(folder.length + 1)
    : path.startsWith("/")
      ? undefined
      : path;
  if (inside === undefined) {
    return { repo: undefined, path };
  }
  const relative = inside.replace(/^\.\//, "");
  const [first, ...rest] = relative.split("/");
  const repo = workspace.repos.find((candidate) => !candidate.isRoot && candidate.name === first);
  if (repo !== undefined && rest.length > 0) {
    return { repo: repo.name, path: rest.join("/") };
  }
  const root = workspace.repos.find((candidate) => candidate.isRoot);
  return root === undefined ? { repo: undefined, path: relative } : { repo: root.name, path: relative };
}
