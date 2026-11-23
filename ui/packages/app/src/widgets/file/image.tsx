/**
 * Images in `omfp:file` (UI-548..UI-555): the kit's `ImageView`, zoomed and panned, from the bytes the daemon sent or,
 * over its 8 MiB cap, from the file's raw bytes (`useFileUrl`, DMN-250); an image that differs from `HEAD` (its
 * `headHash`, DMN-255) compares with it side by side or under a slider.
 */

import type { FileContent } from "@omfp/protocol";
import { Button, type ImageCompare, ImageView } from "omfp/kit";
import { strings, useFileUrl } from "omfp/sdk";
import { type ReactNode, useState } from "react";
import { FileBar, versionedUrl } from "./viewers.tsx";

const text = strings.shell.file;

/** An image in the viewer; `diff` starts it compared with `HEAD` when it differs. */
export function ImageFile({ file, diff }: { file: FileContent; diff: boolean }): ReactNode {
  const base = useFileUrl(file.path);
  const [compare, setCompare] = useState(diff);
  const [layout, setLayout] = useState<ImageCompare>("split");
  const src =
    typeof file.content === "string"
      ? `data:${file.mime ?? "image/png"};base64,${file.content}`
      : base === undefined
        ? undefined
        : versionedUrl(base, file.hash);
  if (src === undefined) {
    return <p className="omfp-file__notice">{text.loading}</p>;
  }
  const head =
    base !== undefined && typeof file.headHash === "string" && file.headHash !== file.hash
      ? `${base}?rev=HEAD&v=${encodeURIComponent(file.headHash)}`
      : undefined;
  return (
    <>
      <FileBar file={file}>
        {head !== undefined && (
          <span className="omfp-file__actions">
            <Button
              size="sm"
              variant="quiet"
              aria-pressed={compare}
              onPress={() => setCompare((on) => !on)}
              data-testid="file-diff"
            >
              {text.diff}
            </Button>
            {compare && (
              <Button
                size="sm"
                variant="quiet"
                onPress={() => setLayout((current) => (current === "split" ? "slider" : "split"))}
                data-testid="file-layout"
              >
                {layout === "split" ? text.slider : text.split}
              </Button>
            )}
          </span>
        )}
      </FileBar>
      <ImageView
        src={src}
        alt={file.path}
        labels={text.image}
        before={head === undefined ? undefined : { src: head, alt: text.headImage(file.path) }}
        compare={compare ? layout : undefined}
        aria-label={text.imageLabel(file.path)}
        role="region"
        data-testid="file-image"
      />
    </>
  );
}
