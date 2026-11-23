/** The Markdown preview of `omfp:file` (UI-560..562): the kit's renderer, links and images relative to the file. */

import { Markdown, type MarkdownImageProps } from "omfp/kit";
import { strings, useFileUrl, useOpenFile } from "omfp/sdk";
import { type ReactNode, useCallback } from "react";
import type { ViewerProps } from "./viewers.tsx";

const text = strings.shell.file;

/** An image's alt text in place of the image. */
function ImageAlt({ alt }: Pick<MarkdownImageProps, "alt">): ReactNode {
  return (
    <span className="omfp-file__image-alt" data-testid="file-image-alt">
      {alt}
    </span>
  );
}

/** The image of a relative source: loaded from its file's URL, its alt text while there is none (UI-562). */
export function FileImage({ path, alt, title }: MarkdownImageProps): ReactNode {
  const url = useFileUrl(path);
  return url === undefined ? <ImageAlt alt={alt} /> : <img src={url} alt={alt} title={title} />;
}

/** The folder of a workspace path (`""` at the root). */
function folderOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

export function MarkdownViewer({ file, content }: ViewerProps): ReactNode {
  const openFile = useOpenFile();
  const openPath = useCallback(
    (reference: { path: string; line: number | undefined }) => openFile(reference.path, { line: reference.line }),
    [openFile],
  );
  return (
    <Markdown
      text={content}
      onOpenPath={openPath}
      linkBase={folderOf(file.path)}
      image={FileImage}
      aria-label={text.previewLabel(file.path)}
    />
  );
}
