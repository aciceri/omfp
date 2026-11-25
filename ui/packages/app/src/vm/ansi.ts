/**
 * ANSI escape sequences in command output (the bash card, UI-050): SGR codes become styled runs — the 16 basic
 * colors (foreground and background), bold, dim, italic and underline — which the card renders with theme-token
 * classes; 256-color and true-color codes keep the text unstyled; every other escape sequence is dropped.
 */

/** One run of text with its style names (`fg-red`, `bg-bright-blue`, `bold`, …), foreground first. */
export interface AnsiRun {
  readonly text: string;
  readonly style: readonly string[];
}

const COLORS = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

/** CSI sequences (`ESC [ … final`), OSC sequences (`ESC ] … BEL` or `ESC \`) and other two-character escapes. */
// biome-ignore lint/suspicious/noControlCharactersInRegex: escape sequences are what this matches
const ESCAPE = /\u001b(?:\[([0-9;?]*)([@-~])|\][^\u0007\u001b]*(?:\u0007|\u001b\\)|[@-Z\\-_])/g;

interface Style {
  fg: string | undefined;
  bg: string | undefined;
  bold: boolean;
  dim: boolean;
  italic: boolean;
  underline: boolean;
}

const PLAIN: Style = { fg: undefined, bg: undefined, bold: false, dim: false, italic: false, underline: false };

function names(style: Style): string[] {
  return [
    ...(style.fg === undefined ? [] : [`fg-${style.fg}`]),
    ...(style.bg === undefined ? [] : [`bg-${style.bg}`]),
    ...(style.bold ? ["bold"] : []),
    ...(style.dim ? ["dim"] : []),
    ...(style.italic ? ["italic"] : []),
    ...(style.underline ? ["underline"] : []),
  ];
}

/** The style after the SGR parameters `params` (`1;31`). */
function apply(style: Style, params: string): Style {
  const codes = params === "" ? [0] : params.split(";").map((code) => (code === "" ? 0 : Number(code)));
  const next = { ...style };
  for (let index = 0; index < codes.length; index++) {
    const code = codes[index] ?? 0;
    if (code === 0) {
      Object.assign(next, PLAIN);
    } else if (code === 1) {
      next.bold = true;
    } else if (code === 2) {
      next.dim = true;
    } else if (code === 3) {
      next.italic = true;
    } else if (code === 4) {
      next.underline = true;
    } else if (code === 22) {
      next.bold = false;
      next.dim = false;
    } else if (code === 23) {
      next.italic = false;
    } else if (code === 24) {
      next.underline = false;
    } else if (code >= 30 && code <= 37) {
      next.fg = COLORS[code - 30];
    } else if (code >= 90 && code <= 97) {
      next.fg = `bright-${COLORS[code - 90]}`;
    } else if (code >= 40 && code <= 47) {
      next.bg = COLORS[code - 40];
    } else if (code >= 100 && code <= 107) {
      next.bg = `bright-${COLORS[code - 100]}`;
    } else if (code === 39) {
      next.fg = undefined;
    } else if (code === 49) {
      next.bg = undefined;
    } else if (code === 38 || code === 48) {
      // 256 colors (`5;n`) or true color (`2;r;g;b`): skipped, the text stays.
      index += codes[index + 1] === 5 ? 2 : 4;
    }
  }
  return next;
}

/** The styled runs of `text`; adjacent runs with the same style are merged. */
export function ansiRuns(text: string): AnsiRun[] {
  const runs: AnsiRun[] = [];
  let style = PLAIN;
  let start = 0;
  const push = (end: number) => {
    const part = text.slice(start, end);
    if (part === "") {
      return;
    }
    const current = names(style);
    const last = runs.at(-1);
    if (last !== undefined && last.style.join(" ") === current.join(" ")) {
      runs[runs.length - 1] = { text: last.text + part, style: last.style };
    } else {
      runs.push({ text: part, style: current });
    }
  };
  for (const match of text.matchAll(ESCAPE)) {
    push(match.index);
    if (match[2] === "m") {
      style = apply(style, match[1] ?? "");
    }
    start = match.index + match[0].length;
  }
  push(text.length);
  return runs;
}

/**
 * The styled runs of `text` line by line, so a slice of its lines keeps the styles set before it (UI-935); a trailing
 * newline ends the last line rather than starting an empty one.
 */
export function ansiLines(text: string): AnsiRun[][] {
  const lines: AnsiRun[][] = [[]];
  for (const run of ansiRuns(text)) {
    for (const [index, part] of run.text.split("\n").entries()) {
      if (index > 0) {
        lines.push([]);
      }
      if (part !== "") {
        lines.at(-1)?.push({ text: part, style: run.style });
      }
    }
  }
  if (lines.length > 1 && lines.at(-1)?.length === 0) {
    lines.pop();
  }
  return lines;
}
