/**
 * Files (UI-110..116) as data: the file tabs a window keeps open per workspace, with the widget tabs reveals open next
 * to them (UI-373), and the text a file had before a patch (what `omfp:changes` diffs the current file against,
 * UI-080). Pure functions, unit tested.
 */

import { parsePatch } from "@omfp/kit";

/** An open file tab (UI-115): the path as it was opened, where to put the cursor, and whether it shows the diff. */
export interface FileTab {
  readonly path: string;
  readonly line?: number | undefined;
  readonly diff?: boolean | undefined;
  /** Bumped when the same file is opened again, so the editor jumps to `line` again. */
  readonly seq: number;
}

/**
 * A workspace's opened tabs of the `main` slot in this window: its open files, then the widgets a reveal opened there
 * (UI-373, by reference). Which tab is selected is per thread and view (UI-372, `TAB_CHOICES`).
 */
export interface FileTabs {
  readonly tabs: readonly FileTab[];
  readonly widgets: readonly string[];
}

export const NO_FILE_TABS: FileTabs = { tabs: [], widgets: [] };

/** Every workspace's {@link FileTabs}, by workspace id (kept in `localStorage`, UI-115, UI-373). */
export type OpenFiles = Readonly<Record<string, FileTabs>>;

/** Options of `openFile`. */
export interface OpenFileOptions {
  readonly line?: number | undefined;
  readonly diff?: boolean | undefined;
  /** The workspace whose file `path` is, among its open files; the window's current workspace by default. */
  readonly workspaceId?: string | undefined;
}

/** The tab id of a file in the `main` slot (the slot's own children use their layout path). */
export function fileTabId(path: string): string {
  return `file:${path}`;
}

/** The tab id of a widget a reveal opened in the `main` slot (UI-373). */
export function widgetTabId(reference: string): string {
  return `widget:${reference}`;
}

/**
 * `path` as a file tab keeps it (UI-119): `.` and `..` resolved without following links, repeated and trailing slashes
 * dropped, and an absolute path inside the workspace folder `folder` made relative to it, so every spelling of a
 * workspace file gives one tab. An absolute path outside the folder (UI-700) stays absolute, the folder itself too; a
 * relative path keeps the `..` that leave the folder (the daemon refuses it, DMN-312). Without `folder` (the
 * workspace not known yet) an absolute path stays absolute.
 */
export function workspaceTabPath(folder: string | undefined, path: string): string {
  const absolute = path.startsWith("/");
  const parts: string[] = [];
  for (const part of path.split("/")) {
    if (part === "..") {
      if (parts.length > 0 && parts.at(-1) !== "..") {
        parts.pop();
      } else if (!absolute) {
        parts.push(part);
      }
    } else if (part !== "" && part !== ".") {
      parts.push(part);
    }
  }
  if (!absolute) {
    return parts.length === 0 ? "." : parts.join("/");
  }
  const resolved = `/${parts.join("/")}`;
  if (folder === undefined) {
    return resolved;
  }
  const root = workspaceTabPath(undefined, folder);
  const prefix = root === "/" ? "/" : `${root}/`;
  return resolved.startsWith(prefix) && resolved.length > prefix.length ? resolved.slice(prefix.length) : resolved;
}

/** The tab id `id` with its file's path as {@link workspaceTabPath} gives it for `folder`; another tab's id as is. */
export function workspaceTabId(folder: string | undefined, id: string): string {
  const prefix = fileTabId("");
  return id.startsWith(prefix) ? fileTabId(workspaceTabPath(folder, id.slice(prefix.length))) : id;
}

/**
 * `tabs` with each file's path as {@link workspaceTabPath} gives it for `folder`, two spellings of one file merged into
 * the first one's tab (tabs kept before UI-119); the same object when nothing changes.
 */
export function normalizeTabs(tabs: FileTabs, folder: string | undefined): FileTabs {
  const seen = new Set<string>();
  let changed = false;
  const normalized = tabs.tabs.flatMap((tab) => {
    const path = workspaceTabPath(folder, tab.path);
    if (seen.has(path)) {
      changed = true;
      return [];
    }
    seen.add(path);
    if (path === tab.path) {
      return [tab];
    }
    changed = true;
    return [{ ...tab, path }];
  });
  return changed ? { ...tabs, tabs: normalized } : tabs;
}

/** The ids of the opened tabs in their order: the files, then the widgets. */
export function openedTabIds(tabs: FileTabs): string[] {
  return [...tabs.tabs.map((tab) => fileTabId(tab.path)), ...tabs.widgets.map(widgetTabId)];
}

/** `tabs` with `path` open: a new tab at the end, or the open one with the new `line` and `diff`. */
export function openTab(tabs: FileTabs, path: string, options: OpenFileOptions = {}): FileTabs {
  const open = tabs.tabs.find((tab) => tab.path === path);
  const tab: FileTab = { path, line: options.line, diff: options.diff, seq: (open?.seq ?? 0) + 1 };
  return {
    ...tabs,
    tabs: open === undefined ? [...tabs.tabs, tab] : tabs.tabs.map((other) => (other === open ? tab : other)),
  };
}

/** `tabs` with the widget `reference` open (UI-373): a new tab after the others, or the open one. */
export function openWidgetTab(tabs: FileTabs, reference: string): FileTabs {
  return tabs.widgets.includes(reference) ? tabs : { ...tabs, widgets: [...tabs.widgets, reference] };
}

/** `tabs` without the opened tab `id` (a file's or a widget's); the same object when it is not open. */
export function closeOpenedTab(tabs: FileTabs, id: string): FileTabs {
  if (!openedTabIds(tabs).includes(id)) {
    return tabs;
  }
  return {
    tabs: tabs.tabs.filter((tab) => fileTabId(tab.path) !== id),
    widgets: tabs.widgets.filter((reference) => widgetTabId(reference) !== id),
  };
}

/** `tabs` without the file `path` (see {@link closeOpenedTab}). */
export function closeTab(tabs: FileTabs, path: string): FileTabs {
  return closeOpenedTab(tabs, fileTabId(path));
}

/** The tab selected once the opened tab `id` of `tabs`, selected, closes: its neighbour in {@link openedTabIds} order
 * (the previous tab, else the next), else `undefined` (the slot's own first tab). */
export function neighbourTab(tabs: FileTabs, id: string): string | undefined {
  const ids = openedTabIds(tabs);
  const index = ids.indexOf(id);
  return index < 0 ? undefined : (ids[index - 1] ?? ids[index + 1]);
}

/**
 * `tabs` with the tab of `from` (a path that does not exist as given) showing `to` instead (DMN-092), keeping its
 * `line` and `diff`; when `to` is already open, that tab takes them and `from`'s goes. Without a tab of `from`, `to`
 * opens as {@link openTab} does with `options`.
 */
export function retargetTab(tabs: FileTabs, from: string, to: string, options: OpenFileOptions = {}): FileTabs {
  const old = tabs.tabs.find((tab) => tab.path === from);
  if (old === undefined) {
    return openTab(tabs, to, options);
  }
  const open = tabs.tabs.find((tab) => tab.path === to);
  const moved: FileTab = { path: to, line: old.line, diff: old.diff, seq: Math.max(old.seq, open?.seq ?? 0) + 1 };
  return {
    ...tabs,
    tabs: tabs.tabs.flatMap((tab) =>
      tab === old ? (open === undefined ? [moved] : []) : tab === open ? [moved] : [tab],
    ),
  };
}

/** Where the floating file panel sits over the view and whether it is minimised (UI-115), in CSS pixels. */
export interface FloatingPanel {
  /** `undefined` until the user moves or resizes the panel: it then fills the view with a margin. */
  readonly box:
    | { readonly left: number; readonly top: number; readonly width: number; readonly height: number }
    | undefined;
  readonly minimised: boolean;
}

export const FLOATING_PANEL_DEFAULT: FloatingPanel = { box: undefined, minimised: false };

/** The smallest panel a resize leaves. */
export const FLOATING_MIN = { width: 240, height: 120 } as const;

/** `panel` moved by `dx`, `dy` (from `from`, its box when the move started), never above or left of the view. */
export function movePanel(
  panel: FloatingPanel,
  from: NonNullable<FloatingPanel["box"]>,
  dx: number,
  dy: number,
): FloatingPanel {
  return {
    ...panel,
    box: { ...from, left: Math.max(0, Math.round(from.left + dx)), top: Math.max(0, Math.round(from.top + dy)) },
  };
}

/** `panel` resized by `dx`, `dy` from `from` (its box when the resize started), never below {@link FLOATING_MIN}. */
export function resizePanel(
  panel: FloatingPanel,
  from: NonNullable<FloatingPanel["box"]>,
  dx: number,
  dy: number,
): FloatingPanel {
  return {
    ...panel,
    box: {
      ...from,
      width: Math.max(FLOATING_MIN.width, Math.round(from.width + dx)),
      height: Math.max(FLOATING_MIN.height, Math.round(from.height + dy)),
    },
  };
}

/** Reads a persisted {@link FloatingPanel}; `undefined` when it is not one. */
export function decodeFloatingPanel(stored: unknown): FloatingPanel | undefined {
  if (typeof stored !== "object" || stored === null) {
    return undefined;
  }
  const { box, minimised } = stored as { box?: unknown; minimised?: unknown };
  const numbers =
    typeof box === "object" && box !== null
      ? (["left", "top", "width", "height"] as const).map((key) => (box as Record<string, unknown>)[key])
      : [];
  const [left, top, width, height] = numbers;
  const valid =
    typeof left === "number" && typeof top === "number" && typeof width === "number" && typeof height === "number";
  return {
    box: valid ? { left, top, width, height } : undefined,
    minimised: minimised === true,
  };
}

/** Reads a persisted {@link OpenFiles}; `undefined` when it is not one. */
export function decodeOpenFiles(stored: unknown): OpenFiles | undefined {
  if (typeof stored !== "object" || stored === null || Array.isArray(stored)) {
    return undefined;
  }
  const out: Record<string, FileTabs> = {};
  for (const [workspaceId, value] of Object.entries(stored)) {
    if (typeof value !== "object" || value === null) {
      continue;
    }
    const { tabs, widgets } = value as { tabs?: unknown; widgets?: unknown };
    if (!Array.isArray(tabs)) {
      continue;
    }
    const decoded: FileTab[] = tabs.flatMap((tab: unknown) => {
      if (typeof tab !== "object" || tab === null) {
        return [];
      }
      const { path, line, diff } = tab as { path?: unknown; line?: unknown; diff?: unknown };
      return typeof path === "string"
        ? [{ path, line: typeof line === "number" ? line : undefined, diff: diff === true ? true : undefined, seq: 0 }]
        : [];
    });
    // Tabs stored before widget tabs existed have no `widgets`.
    const references = Array.isArray(widgets)
      ? widgets.filter((reference: unknown): reference is string => typeof reference === "string")
      : [];
    out[workspaceId] = { tabs: decoded, widgets: [...new Set(references)] };
  }
  return out;
}

/** The last part of a path, the tab's title. */
export function fileName(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("/") + 1) || path;
}

/** A repository's file as a path of the workspace: under the repository's entry, or as is for the folder itself. */
export function workspaceFilePath(repo: { readonly name: string; readonly isRoot: boolean }, path: string): string {
  return repo.isRoot ? path : `${repo.name}/${path}`;
}

/** The lines of a text and whether it ends with a line break. */
function lines(text: string): { lines: string[]; newline: boolean } {
  if (text === "") {
    return { lines: [], newline: false };
  }
  const split = text.split("\n");
  const newline = split.at(-1) === "";
  if (newline) {
    split.pop();
  }
  return { lines: split, newline };
}

const HUNK = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@/;

/**
 * The text a file had before `patch` (one file's `git diff`), given its text now: the patch applied in reverse.
 * `undefined` when the patch does not fit `current` (a context or added line differs, the file changed since) or has
 * no hunk for a change (a binary file). An empty patch means no change.
 */
export function originalFromPatch(current: string, patch: string): string | undefined {
  if (patch.trim() === "") {
    return current;
  }
  const now = lines(current);
  const out: string[] = [];
  // Next line of `current` (1-based) not copied yet.
  let next = 1;
  let hunks = 0;
  // `\ No newline at end of file` after an old line (deleted or context) or after an added one.
  let oldWithoutNewline = false;
  let newWithoutNewline = false;
  let previous: string | undefined;
  const copyUpTo = (last: number): boolean => {
    if (last > now.lines.length) {
      return false;
    }
    for (; next <= last; next++) {
      out.push(now.lines[next - 1] ?? "");
    }
    return true;
  };
  for (const line of parsePatch(patch)) {
    switch (line.kind) {
      case "hunk": {
        hunks++;
        const match = HUNK.exec(line.text);
        const start = Number(match?.[1] ?? 0);
        // `+N,0`: the deleted lines were after line N.
        if (!copyUpTo(match?.[2] === "0" ? start : start - 1)) {
          return undefined;
        }
        break;
      }
      case "context":
      case "add": {
        if (line.newLine !== next || now.lines[next - 1] !== line.text) {
          return undefined;
        }
        if (line.kind === "context") {
          out.push(line.text);
        }
        next++;
        break;
      }
      case "del":
        out.push(line.text);
        break;
      case "note":
        if (previous === "add") {
          newWithoutNewline = true;
        } else {
          oldWithoutNewline = true;
        }
        break;
      default:
        break;
    }
    previous = line.kind;
  }
  if (hunks === 0 || !copyUpTo(now.lines.length)) {
    return undefined;
  }
  // An empty file now (deleted) says nothing about the old one's last line break: only a marker does.
  const newline = !oldWithoutNewline && (newWithoutNewline || now.newline || now.lines.length === 0);
  return out.length === 0 ? "" : `${out.join("\n")}${newline ? "\n" : ""}`;
}
