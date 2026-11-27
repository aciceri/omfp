/**
 * The kit's `WebFrame` (UI-673, UI-674): a `WebView` under a toolbar — the URL, Reload, Open in browser — with the hint
 * that a site may refuse framing when its frame does not load.
 */

import { type HTMLAttributes, type ReactNode, useEffect, useRef, useState } from "react";
import { Button, IconButton } from "./button.tsx";
import { cx } from "./class.ts";
import { Icon } from "./icons.tsx";
import { Toolbar } from "./layout.tsx";
import {
  hasNativeWebViews,
  WEB_VIEW_TEXTS,
  WebView,
  type WebViewHandle,
  type WebViewTexts,
  webFrameUrl,
} from "./web-view.tsx";

/** How long a web frame waits for its page's `load` before it says the site may refuse framing (UI-674). */
export const WEB_FRAME_TIMEOUT_MS = 10_000;

/** The words of a web frame; the kit's are English. */
export interface WebFrameTexts extends WebViewTexts {
  /** The toolbar's accessible name. */
  toolbar: string;
  reload: string;
  open: string;
  /** The quiet button that shows {@link WebFrameTexts.refusedHint} on demand. */
  blank: string;
  /** Said when the page has not loaded in time, or on {@link WebFrameTexts.blank}. */
  refusedHint: string;
  dismiss: string;
}

export const WEB_FRAME_TEXTS: WebFrameTexts = {
  ...WEB_VIEW_TEXTS,
  toolbar: "Web page",
  reload: "Reload",
  open: "Open in browser",
  blank: "Blank?",
  refusedHint:
    "Nothing showing? The site may refuse to be shown in a frame (X-Frame-Options or a CSP frame-ancestors " +
    "header), or nothing answers at this address. Open it in the browser instead.",
  dismiss: "Dismiss",
};

export interface WebFrameProps extends Omit<HTMLAttributes<HTMLElement>, "title" | "children" | "onLoad"> {
  /** The page: an absolute `http(s)` URL other than the window's own origin (see {@link webFrameUrl}). */
  url: unknown;
  /** The page's accessible title; default: the URL's host. */
  title?: string | undefined;
  /** How long to wait for the page's `load` before the refusal hint (default {@link WEB_FRAME_TIMEOUT_MS}). */
  loadTimeoutMs?: number;
  texts?: Partial<WebFrameTexts>;
}

/**
 * A {@link WebView} under a toolbar (UI-673, UI-674): the URL, Reload, Open in browser (a new window: the browser's
 * tab, the desktop's system browser) and Blank?. Whether a site refuses framing cannot be told from outside the frame,
 * so the hint saying it may shows when the page has not loaded in `loadTimeoutMs`, or on Blank?; a native view of the
 * window (UI-682) is never refused, so it has neither.
 */
export function WebFrame({
  url,
  title,
  loadTimeoutMs = WEB_FRAME_TIMEOUT_MS,
  texts,
  className,
  ...props
}: WebFrameProps): ReactNode {
  const words = { ...WEB_FRAME_TEXTS, ...texts };
  const native = hasNativeWebViews();
  const view = useRef<WebViewHandle>(null);
  // The window's origin: the daemon's for the app.
  const checked = webFrameUrl(url, globalThis.location?.origin ?? "");
  const href = checked.ok ? checked.href : undefined;
  const [reloads, setReloads] = useState(0);
  // The page now showing: a reload or another URL waits for its load again.
  const frame = `${reloads} ${href}`;
  const [loadedFrame, setLoadedFrame] = useState<string>();
  const [hintFrame, setHintFrame] = useState<string>();
  const loaded = loadedFrame === frame;
  useEffect(() => {
    if (href === undefined || loaded || native) {
      return;
    }
    const timer = setTimeout(() => setHintFrame(frame), loadTimeoutMs);
    return () => clearTimeout(timer);
  }, [href, frame, loaded, loadTimeoutMs, native]);
  if (!checked.ok) {
    return <WebView url={url} texts={words} {...props} className={className} />;
  }
  return (
    <section data-testid="web-frame" {...props} className={cx("omfp-web-frame", className)} data-loaded={loaded}>
      <Toolbar aria-label={words.toolbar} className="omfp-web-frame__bar">
        <Icon name="globe" className="omfp-web-frame__icon" />
        <span className="omfp-web-frame__url" title={checked.href} data-testid="web-frame-url">
          {checked.href}
        </span>
        {!native && (
          <Button
            variant="quiet"
            size="sm"
            className="omfp-web-frame__blank"
            onPress={() => setHintFrame(frame)}
            data-testid="web-frame-blank"
          >
            {words.blank}
          </Button>
        )}
        <IconButton
          label={words.reload}
          icon={<Icon name="refresh-cw" />}
          onPress={() => {
            setReloads((count) => count + 1);
            view.current?.reload();
          }}
          data-testid="web-frame-reload"
        />
        <IconButton
          label={words.open}
          icon={<Icon name="external-link" />}
          onPress={() => globalThis.open(checked.href, "_blank", "noopener,noreferrer")}
          data-testid="web-frame-open"
        />
      </Toolbar>
      {hintFrame === frame && (
        <div className="omfp-web-frame__hint" role="status" data-testid="web-frame-hint">
          <p>{words.refusedHint}</p>
          <IconButton label={words.dismiss} icon={<Icon name="x" />} onPress={() => setHintFrame(undefined)} />
        </div>
      )}
      <WebView
        ref={view}
        url={checked.href}
        title={title}
        onLoad={() => {
          setLoadedFrame(frame);
          setHintFrame(undefined);
        }}
      />
    </section>
  );
}
