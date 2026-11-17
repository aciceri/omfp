/**
 * What the cards of omp's code-intelligence tools (UI-916..UI-919) read from their results: the `lsp` tool's text
 * (omp 18.22.0 puts its answer in the text only; `details` say the action and whether it succeeded), and the grouped
 * file output of `ast_grep` and `ast_edit` (`details.displayContent`). The formats are omp's own
 * (`packages/coding-agent/src/lsp/{tool,utils,diagnostics,edits}.ts`, `packages/tui/src/tools/grouped-file-output.ts`);
 * `fixtures/threads/<omp>/code_intel_tool_calls.jsonl` records the AST tools and an `lsp` call.
 */

import type { DiffLine } from "@omfp/kit";

/** One diagnostic of an `lsp` `diagnostics` answer. */
export interface Diagnostic {
  readonly path: string;
  readonly line: number;
  readonly column: number;
  /** omp's severity: `error`, `warning`, `info` or `hint`. */
  readonly severity: string;
  /** The tool that reported it (`[eslint]`), when the server names one. */
  readonly source: string | undefined;
  readonly code: string | undefined;
  readonly message: string;
}

/** A source line around a location, with its number. */
export interface SourceLine {
  readonly line: number;
  readonly text: string;
}

/** A location of a definition-style or references answer, with the source lines omp read around it. */
export interface CodeLocation {
  readonly path: string;
  readonly line: number;
  readonly column: number;
  readonly context: readonly SourceLine[];
}

/** A symbol of a `symbols` answer: its nesting depth, its name (with the server's detail) and where it is. */
export interface CodeSymbol {
  readonly depth: number;
  readonly name: string;
  readonly path: string | undefined;
  readonly line: number;
}

/** A file a workspace edit changed (or would change, in a preview); `removed` once it was deleted. */
export interface EditedFile {
  readonly path: string;
  /** What happened to it, in omp's words (`2 edits`, `created`, `renamed from src/a.ts`). */
  readonly note: string;
  readonly removed: boolean;
}

/** A code action the server offers, by omp's index. */
export interface CodeActionItem {
  readonly index: number;
  readonly kind: string;
  readonly title: string;
}

/** What an `lsp` answer shows, by the shape of its text. */
export type LspResult =
  | {
      readonly type: "diagnostics";
      readonly diagnostics: readonly Diagnostic[];
      /** omp's status lines: servers that failed, did not report yet, or none found for a file. */
      readonly notes: readonly string[];
      /**
       * The whole check came back clean (`OK`, `no issues`): no diagnostic, and no file left unchecked (a status
       * note: a server that failed, did not report yet, or none found).
       */
      readonly clean: boolean;
    }
  | {
      readonly type: "locations";
      readonly title: string;
      readonly locations: readonly CodeLocation[];
      readonly notes: readonly string[];
    }
  | {
      readonly type: "symbols";
      readonly title: string;
      /** Workspace symbols each name their own file; a file's symbols are all in the file the title names. */
      readonly workspace: boolean;
      readonly symbols: readonly CodeSymbol[];
    }
  | {
      readonly type: "edits";
      readonly title: string;
      /** Whether the edit was applied (a rename preview was not). */
      readonly applied: boolean;
      readonly files: readonly EditedFile[];
      readonly commands: readonly string[];
      readonly notes: readonly string[];
    }
  | { readonly type: "actions"; readonly title: string; readonly actions: readonly CodeActionItem[] }
  | { readonly type: "hover"; readonly markdown: string }
  | { readonly type: "text"; readonly text: string };

/** A body row of omp's grouped file output with the file it belongs to. */
export interface GroupedRow {
  /** The file's path as omp shows it (relative to the agent's cwd, absolute outside it), when a header named one. */
  readonly file: string | undefined;
  readonly text: string;
}

const GROUPED_HEADER = /^(#+)\s+(.*)$/;
/** A file header's note (`(2 replacements)`). */
const HEADER_NOTE = /\s+\([^)]*\)\s*$/;
/** A file header's hashline tag (`lib.rs#72CA`), as omp's store writes it. */
const HEADER_TAG = /#[0-9A-F]{4}$/;

/**
 * The body rows of omp's grouped file output (`formatGroupedFiles`): `#` headers nest folders (`# src/`, `## lib/`)
 * and name files (`### a.ts#TAG (1 replacement)`), the folded folders' chain giving the file's path; blank separator
 * rows are dropped. Rows before any header belong to `scope` (a single-file scope prints no header).
 */
export function groupedRows(text: string, scope: string | undefined): GroupedRow[] {
  const folders: string[] = [];
  let file = scope;
  const rows: GroupedRow[] = [];
  for (const line of text.split("\n")) {
    const header = GROUPED_HEADER.exec(line);
    if (header === null) {
      if (line.trim() !== "") {
        rows.push({ file, text: line });
      }
      continue;
    }
    const depth = (header[1] ?? "").length;
    const rest = (header[2] ?? "").trimEnd();
    folders.length = Math.min(folders.length, depth - 1);
    if (rest.endsWith("/")) {
      folders.push(rest);
      file = undefined;
      continue;
    }
    const name = rest.replace(HEADER_NOTE, "").replace(HEADER_TAG, "");
    file = folders.join("") + name;
  }
  return rows;
}

/** A code-frame row of search output: `*12│text` (a match starts), ` 13│text`; `:` or `|` in the model's text. */
const CODE_FRAME = /^\s*(\*?)(\d+)(?:│|[:|])(.*)$/;
/** An `ast_grep` match's metavariable captures, after its code. */
const CAPTURES = /^\s*meta: (.*)$/;

/** One `ast_grep` match: where it starts, its code and its metavariable captures. */
export interface AstMatch {
  readonly line: number;
  readonly code: string;
  /** `NAME=add, RET=u64`. */
  readonly captures: string | undefined;
}

/** The matches of an `ast_grep` result's grouped output by file, in omp's order; rows of `scope` when it has none. */
export function astMatches(output: string, scope: string | undefined): Map<string, AstMatch[]> {
  const files = new Map<string, AstMatch[]>();
  let current: { line: number; lines: string[]; captures: string | undefined } | undefined;
  let owner: string | undefined;
  const flush = () => {
    if (current !== undefined && owner !== undefined) {
      const matches = files.get(owner) ?? [];
      matches.push({ line: current.line, code: current.lines.join("\n"), captures: current.captures });
      files.set(owner, matches);
    }
    current = undefined;
  };
  for (const row of groupedRows(output, scope)) {
    if (row.file !== owner) {
      flush();
      owner = row.file;
    }
    const frame = CODE_FRAME.exec(row.text);
    if (frame !== null) {
      if (frame[1] === "*" || current === undefined) {
        flush();
        current = { line: Number(frame[2]), lines: [], captures: undefined };
      }
      current.lines.push(frame[3] ?? "");
      continue;
    }
    const captures = CAPTURES.exec(row.text);
    if (captures !== null && current !== undefined) {
      current.captures = captures[1];
    }
  }
  flush();
  return files;
}

/** An `ast_edit` change row of the display output: `-4│before`, `+4│after`. */
const CHANGE_ROW = /^\s*([+-])(\d+)│(.*)$/;

/** The changes of an `ast_edit` result's display output as diff lines by file; rows of `scope` when it has none. */
export function astEditDiffs(display: string, scope: string | undefined): Map<string, DiffLine[]> {
  const files = new Map<string, DiffLine[]>();
  for (const row of groupedRows(display, scope)) {
    const change = CHANGE_ROW.exec(row.text);
    if (change === null || row.file === undefined) {
      continue;
    }
    const line = Number(change[2]);
    const text = change[3] ?? "";
    const lines = files.get(row.file) ?? [];
    lines.push(change[1] === "-" ? { kind: "del", text, oldLine: line } : { kind: "add", text, newLine: line });
    files.set(row.file, lines);
  }
  return files;
}

/** `src/a.ts:12:5 [error] [eslint] message (code)` (`formatDiagnostic`); grouped rows lack the path. */
const DIAGNOSTIC = /^(.+?):(\d+):(\d+)\s+\[(\w+)\]\s+(?:\[([^\]]+)\]\s+)?(.+?)(?:\s+\(([^)]+)\))?$/;
const DIAGNOSTIC_IN_FILE = /^\d+:\d+\s+\[/;
/** The answer's counts (`2 error(s), 1 warning(s):`), repeated by the rows. */
const DIAGNOSTICS_SUMMARY = /^\d+ \w+\(s\)(?:, \d+ \w+\(s\))*:?$/;
/** A file's status row (`✘ a.ts: 2 error(s)`, `✓ a.ts: no issues`): its counts repeat the rows. */
const FILE_SUMMARY = /: (?:no issues|\d+ \w+\(s\)(?:, \d+ \w+\(s\))*)$/;
/** The leading status glyph of omp's status rows (`✘`, `⚠`, a Nerd Font glyph, ASCII `[-]`), theme-dependent. */
const STATUS_GLYPH = /^(?:\[[^\]\s]*\]|[^\p{L}\p{N}\s./~\\]+)\s+/u;

function diagnostic(text: string): Diagnostic | undefined {
  const match = DIAGNOSTIC.exec(text);
  if (match === null) {
    return undefined;
  }
  return {
    path: match[1] ?? "",
    line: Number(match[2]),
    column: Number(match[3]),
    severity: (match[4] ?? "").toLowerCase(),
    source: match[5],
    code: match[7],
    message: match[6] ?? "",
  };
}

/**
 * A `diagnostics` answer: one file's (`2 error(s):` then its rows, or `OK`), several files' (a status row per file,
 * then its rows), or the workspace's (`Workspace diagnostics (…):` and the checker's own output).
 */
function diagnosticsResult(text: string): LspResult {
  const diagnostics: Diagnostic[] = [];
  const notes: string[] = [];
  let clean = false;
  for (const row of groupedRows(text, undefined)) {
    const line = row.text.trim();
    if (line === "OK") {
      clean = true;
      continue;
    }
    if (DIAGNOSTICS_SUMMARY.test(line)) {
      continue;
    }
    if (FILE_SUMMARY.test(line)) {
      clean ||= line.endsWith(": no issues");
      continue;
    }
    const found = diagnostic(row.file !== undefined && DIAGNOSTIC_IN_FILE.test(line) ? `${row.file}:${line}` : line);
    if (found === undefined) {
      notes.push(line.replace(STATUS_GLYPH, ""));
    } else {
      diagnostics.push(found);
    }
  }
  // A note is a file the check did not cover (or a cut glob): the answer is incomplete, never "No issues".
  return { type: "diagnostics", diagnostics, notes, clean: clean && diagnostics.length === 0 && notes.length === 0 };
}

const LOCATIONS_HEADER = /^Found \d+ (?:definition|type definition|implementation|reference)\(s\):$/;
/** `  file:line:col`, then its `    N: source` context rows (`formatLocationWithContext`). */
const LOCATION = /^ {2}(.+):(\d+):(\d+)$/;
const LOCATION_CONTEXT = /^ {4}(\d+): ?(.*)$/;

function locationsResult(text: string): LspResult | undefined {
  const [title = "", ...lines] = text.split("\n");
  if (!LOCATIONS_HEADER.test(title)) {
    return undefined;
  }
  const locations: { path: string; line: number; column: number; context: SourceLine[] }[] = [];
  const notes: string[] = [];
  for (const line of lines) {
    const context = LOCATION_CONTEXT.exec(line);
    const last = locations.at(-1);
    if (context !== null && last !== undefined) {
      last.context.push({ line: Number(context[1]), text: context[2] ?? "" });
      continue;
    }
    const location = LOCATION.exec(line);
    if (location === null) {
      if (line.trim() !== "") {
        notes.push(line.trim());
      }
      continue;
    }
    locations.push({ path: location[1] ?? "", line: Number(location[2]), column: Number(location[3]), context: [] });
  }
  return { type: "locations", title: title.replace(/:$/, ""), locations, notes };
}

/** `Symbols in src/a.ts:` then `  <icon> name detail @ line 12` rows, two spaces per level (`formatDocumentSymbol`). */
const DOCUMENT_SYMBOLS = /^Symbols in (.+):$/;
const DOCUMENT_SYMBOL = /^( *)\S+ (.+) @ line (\d+)$/;
/** `Found 3 symbol(s) matching "q":` then `  <icon> name (container) @ file:line:col` (`formatSymbolInformation`). */
const WORKSPACE_SYMBOLS = /^Found \d+ symbol\(s\) matching ".*":$/;
const WORKSPACE_SYMBOL = /^ {2}\S+ (.+) @ (.+):(\d+):\d+$/;

function symbolsResult(text: string): LspResult | undefined {
  const [title = "", ...lines] = text.split("\n");
  const document = DOCUMENT_SYMBOLS.exec(title);
  if (document === null && !WORKSPACE_SYMBOLS.test(title)) {
    return undefined;
  }
  const symbols = lines.flatMap((line): CodeSymbol[] => {
    if (document === null) {
      const symbol = WORKSPACE_SYMBOL.exec(line);
      return symbol === null ? [] : [{ depth: 0, name: symbol[1] ?? "", path: symbol[2], line: Number(symbol[3]) }];
    }
    const symbol = DOCUMENT_SYMBOL.exec(line);
    const depth = Math.floor((symbol?.[1] ?? "").length / 2);
    return symbol === null ? [] : [{ depth, name: symbol[2] ?? "", path: document[1], line: Number(symbol[3]) }];
  });
  return { type: "symbols", title: title.replace(/:$/, ""), workspace: document === null, symbols };
}

/** The rows of an applied or previewed workspace edit (`applyWorkspaceEdit`, `formatWorkspaceEdit`). */
const EDIT_ROWS: readonly { readonly pattern: RegExp; readonly file: (match: RegExpExecArray) => EditedFile }[] = [
  {
    // `Applied 2 edit(s) to src/a.ts`; `rename_file`'s `  rust-analyzer: applied 2 edit(s) to src/a.ts`.
    pattern: /^\s*(?:[^:]+: )?[Aa]pplied (\d+) edit\(s\) to (.+)$/,
    file: (match) => ({ path: match[2] ?? "", note: `${match[1]} edits`, removed: false }),
  },
  {
    pattern: /^\s*(?:Created|CREATE:) (.+)$/,
    file: (match) => ({ path: match[1] ?? "", note: "created", removed: false }),
  },
  {
    pattern: /^\s*(?:Deleted|DELETE:) (.+)$/,
    file: (match) => ({ path: match[1] ?? "", note: "deleted", removed: true }),
  },
  {
    // `Renamed a → b` (applied, `rename_file`); a preview's `RENAME: a <cursor> b`, the cursor being the theme's
    // `nav.cursor` (`❯`, the Nerd Font chevron or `>`). Split at the separator so paths keep their spaces.
    pattern: /^\s*(?:Renamed (.+?) → |RENAME: (.+?) (?:❯|\u{f054}|>) )(.+)$/u,
    file: (match) => ({ path: match[3] ?? "", note: `renamed from ${match[1] ?? match[2]}`, removed: false }),
  },
  {
    // A preview's `src/a.ts: 2 edits`.
    pattern: /^\s+(.+): (\d+) edits?$/,
    file: (match) => ({ path: match[1] ?? "", note: `${match[2]} edits`, removed: false }),
  },
];

/** A `rename`, `rename_file` or applied `code_actions` answer: the files it changed and the commands it ran. */
function editsResult(text: string): LspResult | undefined {
  const [title = "", ...lines] = text.split("\n");
  const files: EditedFile[] = [];
  const commands: string[] = [];
  const notes: string[] = [];
  let inCommands = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed === "" || trimmed === "Workspace edit:") {
      inCommands = false;
      continue;
    }
    if (trimmed === "Executed command(s):") {
      inCommands = true;
      continue;
    }
    if (inCommands) {
      commands.push(trimmed);
      continue;
    }
    const row = EDIT_ROWS.map(({ pattern, file }) => {
      const match = pattern.exec(line);
      return match === null ? undefined : file(match);
    }).find((file) => file !== undefined);
    if (row === undefined) {
      notes.push(trimmed);
    } else {
      files.push(row);
    }
  }
  if (files.length === 0 && commands.length === 0) {
    return undefined;
  }
  return {
    type: "edits",
    title: title.replace(/:$/, ""),
    applied: !title.startsWith("Rename preview"),
    files,
    commands,
    notes,
  };
}

/** `  0: [quickfix] Add missing import (preferred)` (`formatCodeAction`). */
const CODE_ACTION = /^\s+(\d+): \[([^\]]+)\] (.+)$/;

function actionsResult(text: string): LspResult | undefined {
  const [title = "", ...lines] = text.split("\n");
  const actions = lines.flatMap((line) => {
    const action = CODE_ACTION.exec(line);
    return action === null ? [] : [{ index: Number(action[1]), kind: action[2] ?? "", title: action[3] ?? "" }];
  });
  return actions.length === 0 ? undefined : { type: "actions", title: title.replace(/:$/, ""), actions };
}

/** The answer of an `lsp` call of `action`, read from its text. */
export function lspResult(action: string, text: string): LspResult {
  const plain: LspResult = { type: "text", text };
  switch (action) {
    case "diagnostics":
      return diagnosticsResult(text);
    case "definition":
    case "type_definition":
    case "implementation":
    case "references":
      return locationsResult(text) ?? plain;
    case "symbols":
      return symbolsResult(text) ?? plain;
    case "rename":
    case "rename_file":
      return editsResult(text) ?? plain;
    case "code_actions":
      return (text.startsWith('Applied "') ? editsResult(text) : actionsResult(text)) ?? plain;
    case "hover":
      return text === "No hover information" ? plain : { type: "hover", markdown: text };
    default:
      return plain;
  }
}
