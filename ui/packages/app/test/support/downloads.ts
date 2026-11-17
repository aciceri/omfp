/** Captures the files the app lets the browser download (an object URL clicked as a `download` link, UI-791). */

export interface Downloads {
  /** Each download's file name and text, in order. */
  readonly files: { readonly name: string; readonly text: Promise<string> }[];
  /** Puts `URL.createObjectURL`, `URL.revokeObjectURL` and anchor clicks back. */
  restore(): void;
}

export function captureDownloads(): Downloads {
  const files: { name: string; text: Promise<string> }[] = [];
  const blobs = new Map<string, Blob>();
  const { createObjectURL, revokeObjectURL } = URL;
  const click = HTMLAnchorElement.prototype.click;
  URL.createObjectURL = (blob: Blob) => {
    const url = `blob:test/${blobs.size}`;
    blobs.set(url, blob);
    return url;
  };
  URL.revokeObjectURL = () => {};
  HTMLAnchorElement.prototype.click = function (this: HTMLAnchorElement) {
    const blob = blobs.get(this.getAttribute("href") ?? "");
    files.push({ name: this.download, text: blob === undefined ? Promise.resolve("") : blob.text() });
  };
  return {
    files,
    restore: () => {
      URL.createObjectURL = createObjectURL;
      URL.revokeObjectURL = revokeObjectURL;
      HTMLAnchorElement.prototype.click = click;
    },
  };
}
