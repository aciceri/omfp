/**
 * File paths in text (UI-116): what reads as a path (`src/lib.rs`, `src/lib.rs:12`, `README.md`) and the inline
 * control that opens it. Which file a path names is the daemon's business (it resolves it against the workspace and
 * its repositories, DMN-089); this only tells a path from other inline code.
 */
import type { ReactNode } from "react";
import { languageOfPath } from "./highlight.tsx";

/** A path with the line it points at. */
export interface PathReference {
  readonly path: string;
  readonly line: number | undefined;
}

/** What `onOpen` gets: the reference and how to show it. */
export type OpenPath = (reference: PathReference) => void;

/** Characters of a path as models write them (no spaces, quotes or brackets). */
const PATH = /^(?:~|\.{1,2})?\/?[\w@+.-]+(?:\/[\w@+.-]+)*$/;
/** `path:line` or `path:line:column`. */
const WITH_LINE = /^(.*?):(\d+)(?::\d+)?$/;
/** A URL scheme or a `C:` drive. */
const SCHEME = /^[a-z][a-z\d+.-]*:/i;
/** Extensions of files `languageOfPath` does not highlight but are still files. */
const OTHER_EXTENSIONS = new Set(["txt", "lock", "cfg", "conf", "env", "csv", "toml", "html", "css", "svg"]);
/** Files known by their whole name. */
const FILE_NAMES = new Set(["Makefile", "Dockerfile", "justfile", "Justfile", "LICENSE", "Cargo.lock", "flake.lock"]);

/**
 * The file path `text` names, with its line: text without spaces made of path characters, with a slash or a known
 * extension or file name; never a URL or a folder (trailing slash).
 */
export function pathReference(text: string): PathReference | undefined {
  const trimmed = text.trim();
  const match = WITH_LINE.exec(trimmed);
  const path = match?.[1] ?? trimmed;
  const line = match?.[2] === undefined ? undefined : Number(match[2]);
  if (!PATH.test(path) || SCHEME.test(path)) {
    return undefined;
  }
  return namesFile(path) ? { path, line } : undefined;
}

/** A path with a slash, a known extension or a known file name (not a folder). */
function namesFile(path: string): boolean {
  const name = path.slice(path.lastIndexOf("/") + 1);
  if (name === "" || name.endsWith(".")) {
    return false;
  }
  const dot = name.lastIndexOf(".");
  const known =
    FILE_NAMES.has(name) ||
    (dot > 0 && (languageOfPath(name) !== undefined || OTHER_EXTENSIONS.has(name.slice(dot + 1).toLowerCase())));
  return known || (path.includes("/") && !path.startsWith("//"));
}

/** The path of a Markdown link's target when it is a relative file path (`docs/a.md`, `src/x.rs#L12`). */
export function linkReference(href: string | undefined): PathReference | undefined {
  if (href === undefined || href === "" || href.startsWith("#") || href.startsWith("/") || SCHEME.test(href)) {
    return undefined;
  }
  const [target = "", fragment] = href.split("#", 2);
  const line = /^L(\d+)/.exec(fragment ?? "")?.[1];
  let decoded: string;
  try {
    decoded = decodeURI(target);
  } catch {
    return undefined;
  }
  // A link target may hold spaces (`my%20notes/a.md`), unlike inline code.
  return namesFile(decoded) ? { path: decoded, line: line === undefined ? undefined : Number(line) } : undefined;
}

/**
 * `target`, a path relative to the folder `base` (a Markdown file's folder, `""` for the root), as one path: empty and
 * `.` segments dropped, `..` folding the segment before it (kept when there is none, past the root).
 */
export function resolvePath(base: string, target: string): string {
  const segments: string[] = [];
  for (const segment of [...base.split("/"), ...target.split("/")]) {
    if (segment === "" || segment === ".") {
      continue;
    }
    if (segment === ".." && segments.length > 0 && segments.at(-1) !== "..") {
      segments.pop();
    } else {
      segments.push(segment);
    }
  }
  return segments.join("/");
}

/** The file path of a Markdown image's source when it is relative (`img/a.png`, `../a%20b.png`), without its query or
 * fragment; never a URL, a `data:` URL, an absolute or protocol-relative path. */
export function relativeSource(src: string | undefined): string | undefined {
  if (src === undefined || src.startsWith("/") || src.startsWith("#") || SCHEME.test(src)) {
    return undefined;
  }
  const target = src.split(/[?#]/, 1)[0] ?? "";
  try {
    const decoded = decodeURI(target);
    return decoded === "" ? undefined : decoded;
  } catch {
    return undefined;
  }
}

export interface PathLinkProps {
  readonly reference: PathReference;
  readonly onOpen: OpenPath;
  readonly children: ReactNode;
  readonly diff?: boolean | undefined;
  readonly "data-testid"?: string;
}

/** An inline path button; it carries the path for `:open` (`data-omfp-path`, `data-omfp-line`). */
export function PathLink({ reference, onOpen, children, diff, "data-testid": testId = "path-link" }: PathLinkProps) {
  return (
    <button
      type="button"
      className="omfp-path-link"
      data-testid={testId}
      data-omfp-path={reference.path}
      data-omfp-line={reference.line}
      data-omfp-diff={diff === true ? "" : undefined}
      onClick={() => onOpen(reference)}
    >
      {children}
    </button>
  );
}
