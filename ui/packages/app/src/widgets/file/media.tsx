/**
 * Video and audio files in `omfp:file` (UI-541, UI-542): the browser's own player over the file's raw bytes
 * (`useFileUrl`, DMN-250), which seeks with byte ranges; a file the browser cannot play says so.
 */

import type { FileContent } from "@omfp/protocol";
import { EmptyState } from "omfp/kit";
import { strings, useFileUrl } from "omfp/sdk";
import { type ReactNode, useState } from "react";
import { extension, versionedUrl } from "./viewers.tsx";

const text = strings.shell.file;

export type MediaKind = "video" | "audio";

/** Extensions (lower case) of the files shown in a player. */
const MEDIA: Readonly<Record<string, MediaKind>> = {
  mp4: "video",
  m4v: "video",
  webm: "video",
  mov: "video",
  mkv: "video",
  ogv: "video",
  mp3: "audio",
  ogg: "audio",
  oga: "audio",
  opus: "audio",
  wav: "audio",
  flac: "audio",
  m4a: "audio",
  aac: "audio",
};

/** Whether `path` names a video or an audio file, by its extension. */
export function mediaKind(path: string): MediaKind | undefined {
  const found = extension(path);
  return found === undefined ? undefined : MEDIA[found];
}

/**
 * A video or audio file in a player with the browser's controls, sized to the panel. The URL carries the file's hash,
 * so a changed file loads again.
 */
export function MediaFile({ file, kind }: { file: FileContent; kind: MediaKind }): ReactNode {
  const base = useFileUrl(file.path);
  const src = base === undefined ? undefined : versionedUrl(base, file.hash);
  // The source that failed: a new version gets its own try.
  const [failed, setFailed] = useState<string | undefined>(undefined);
  if (src === undefined) {
    return <p className="omfp-file__notice">{text.loading}</p>;
  }
  if (failed === src) {
    return (
      <EmptyState
        icon="file"
        title={text.cannotPlay(kind, text.size(file.size))}
        className="omfp-file__empty"
        data-testid="file-media-error"
      />
    );
  }
  const player = {
    src,
    controls: true,
    preload: "metadata",
    "aria-label": text.playerLabel(kind, file.path),
    "data-testid": "file-player",
    onError: () => setFailed(src),
  } as const;
  return (
    <div className="omfp-file__media" data-testid="file-media" data-kind={kind}>
      {kind === "video" ? <video {...player} /> : <audio {...player} />}
    </div>
  );
}
