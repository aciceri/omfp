/**
 * The viewers `omfp:file` offers besides the source (UI-560): a rendered form of a text file, chosen by the file's
 * extension, shown while the file is read-only, with a toggle back to the source. Also what every viewer of the
 * widget shares: a path's extension, a file's URL pinned to its version and the bar naming the file.
 */

import type { FileContent } from "@omfp/protocol";
import { Chip, Icon } from "omfp/kit";
import { strings } from "omfp/sdk";
import type { ComponentType, ReactNode } from "react";
import { MarkdownViewer } from "./markdown.tsx";
import { CsvViewer, TsvViewer } from "./table.tsx";
import { JsonViewer, YamlViewer } from "./tree.tsx";

/** The extension of the file `path` names, in lower case; `undefined` without one (`notes/.wav`, `a.b/clip`). */
export function extension(path: string): string | undefined {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : undefined;
}

/** The file's raw URL `base` with its hash, so a changed file loads again. */
export function versionedUrl(base: string, hash: string): string {
  return `${base}?v=${encodeURIComponent(hash)}`;
}

/** The bar over a file: its path and repository, then `children` (its actions). */
export function FileBar({ file, children }: { file: FileContent; children?: ReactNode }): ReactNode {
  return (
    <div className="omfp-file__bar" data-testid="file-bar">
      <span className="omfp-file__path">
        <Icon name="file" className="omfp-file__icon" />
        {typeof file.repo === "string" && <Chip>{file.repo}</Chip>}
        <span data-testid="file-path">{file.path}</span>
      </span>
      {children}
    </div>
  );
}

/** Why a viewer cannot show a file (UI-569): the sentence to show, and the line of the source to show instead. */
export interface ViewerFailure {
  readonly message: string;
  readonly line: number;
}

/** What a viewer gets: the file as read and its text, and where to say it cannot show it (the source shows then). */
export interface ViewerProps {
  readonly file: FileContent;
  readonly content: string;
  readonly onError: (failure: ViewerFailure) => void;
}

export interface Viewer {
  /** Its name, on the view as `data-viewer`. */
  readonly name: string;
  /** The toggle's label. */
  readonly label: string;
  readonly View: ComponentType<ViewerProps>;
}

const text = strings.shell.file;
const markdown: Viewer = { name: "markdown", label: text.viewers.markdown, View: MarkdownViewer };

/** The viewers by lower-case extension (a map: an extension such as `constructor` must find nothing). */
const VIEWERS: ReadonlyMap<string, Viewer> = new Map([
  ["md", markdown],
  ["markdown", markdown],
  ["mdx", markdown],
  ["csv", { name: "table", label: text.viewers.table, View: CsvViewer }],
  ["tsv", { name: "table", label: text.viewers.table, View: TsvViewer }],
  ["json", { name: "tree", label: text.viewers.tree, View: JsonViewer }],
  ["jsonc", { name: "tree", label: text.viewers.tree, View: JsonViewer }],
  ["yaml", { name: "tree", label: text.viewers.tree, View: YamlViewer }],
  ["yml", { name: "tree", label: text.viewers.tree, View: YamlViewer }],
]);

/** The viewer of the file at `path`, by its extension (case aside), if it has one. */
export function viewerOf(path: string): Viewer | undefined {
  const found = extension(path);
  return found === undefined ? undefined : VIEWERS.get(found);
}
