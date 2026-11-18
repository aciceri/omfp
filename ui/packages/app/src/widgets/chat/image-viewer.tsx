/**
 * The chat's full view of an image (UI-474, UI-476): one dialog per transcript, held outside its virtualized rows, so it
 * stays open while the row it came from scrolls away and unmounts (the transcript follows a running turn), and its keys
 * and wheel never reach the transcript's own handlers. A message's image or a result's thumbnail asks it to open; a row
 * rendered outside a transcript shows its own.
 */

import { ImageDialog } from "omfp/kit";
import { strings } from "omfp/sdk";
import {
  createContext,
  type ReactNode,
  type RefObject,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";

/** An image to show, with the dialog's heading and test id. */
export interface ViewedImage {
  readonly src: string;
  readonly alt: string;
  readonly title: string;
  readonly testId: string;
}

const OpenImage = createContext<((image: ViewedImage) => void) | undefined>(undefined);

function ViewerDialog({ image, onClose }: { image: ViewedImage | undefined; onClose: () => void }): ReactNode {
  return (
    <ImageDialog
      image={image}
      title={image?.title ?? ""}
      labels={strings.shell.file.image}
      onClose={onClose}
      data-testid={image?.testId}
    />
  );
}

/**
 * The transcript's image dialog, which the images inside `children` open. Closing it gives the focus back to the image
 * that opened it, or to `fallback` (the transcript) when that image's row is gone.
 */
export function ImageViewer({
  children,
  fallback,
}: {
  children: ReactNode;
  fallback: RefObject<HTMLElement | null>;
}): ReactNode {
  const [image, setImage] = useState<ViewedImage | undefined>(undefined);
  const opener = useRef<Element | null>(null);
  const closed = useRef(false);
  const open = useCallback((next: ViewedImage) => {
    opener.current = document.activeElement;
    setImage(next);
  }, []);
  useEffect(() => {
    if (image !== undefined || !closed.current) {
      return;
    }
    closed.current = false;
    // React Aria gives the focus back to a connected opener only.
    if (opener.current?.isConnected !== true) {
      fallback.current?.focus({ preventScroll: true });
    }
  }, [image, fallback]);
  return (
    <OpenImage.Provider value={open}>
      {children}
      <ViewerDialog
        image={image}
        onClose={() => {
          closed.current = true;
          setImage(undefined);
        }}
      />
    </OpenImage.Provider>
  );
}

/**
 * Opens an image in the transcript's dialog; outside a transcript, in `dialog`, which the caller renders (`null` inside
 * one).
 */
export function useImageViewer(): { open: (image: ViewedImage) => void; dialog: ReactNode } {
  const shared = useContext(OpenImage);
  const [image, setImage] = useState<ViewedImage | undefined>(undefined);
  if (shared !== undefined) {
    return { open: shared, dialog: null };
  }
  return { open: setImage, dialog: <ViewerDialog image={image} onClose={() => setImage(undefined)} /> };
}
