/**
 * PDF documents in `omfp:file` (UI-547): the kit's `PdfView` over the file's raw bytes (`useFileUrl`, DMN-250), which
 * pdf.js reads with byte ranges. By extension, whatever the daemon calls the file: an all-ASCII PDF reads as text.
 */

import type { FileContent } from "@omfp/protocol";
import { PdfView } from "omfp/kit";
import { strings, useFileUrl } from "omfp/sdk";
import type { ReactNode } from "react";
import { versionedUrl } from "./viewers.tsx";

const text = strings.shell.file;

/** A PDF document in the viewer. The URL carries the file's hash, so a changed file loads again. */
export function PdfFile({ file }: { file: FileContent }): ReactNode {
  const base = useFileUrl(file.path);
  if (base === undefined) {
    return <p className="omfp-file__notice">{text.loading}</p>;
  }
  return (
    <PdfView
      url={versionedUrl(base, file.hash)}
      labels={text.pdf}
      aria-label={text.pdfLabel(file.path)}
      role="region"
    />
  );
}
