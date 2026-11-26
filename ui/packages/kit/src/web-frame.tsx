import { type HTMLAttributes, type ReactNode, useEffect, useId, useState } from "react";
import { Button, IconButton } from "./button.tsx";
import { cx } from "./class.ts";
import { EmptyState } from "./feedback.tsx";
import { Icon } from "./icons.tsx";
import { Toolbar } from "./layout.tsx";

/**
 * The `sandbox` of a web frame (UI-672): its scripts run, and in its own origin (`allow-same-origin`: cookies, storage,
 * the dev servers and dashboards that need them), its forms submit and its links open new windows (the browser's tab,
 * the desktop's system browser). Never `allow-top-navigation*`: the framed page cannot navigate the window. It never
 * shares the window's origin either — {@link webFrameUrl} refuses the daemon's — or `allow-same-origin` with
 * `allow-scripts` would let it lift its own sandbox and read the daemon's token.
 */
export const WEB_FRAME_SANDBOX =
  "allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox";

/** How long a web frame waits for its page's `load` before it says the site may refuse framing (UI-674). */
export const WEB_FRAME_TIMEOUT_MS = 10_000;

/**
 * Whether the window draws web pages itself (UI-682): the Linux desktop window defines `__omfpWebViews` in its page
 * and draws a native view over each `[data-omfp-web-view]` placeholder, where sites that refuse framing show too.
 */
export function hasNativeWebViews(): boolean {
  return "__omfpWebViews" in globalThis;
}

/** Why {@link webFrameUrl} refuses a URL. */
export type WebFrameRefusal = "missing" | "invalid" | "scheme" | "own-origin";

/** A URL a web frame may show (`href`, normalized), or why it may not. */
export type WebFrameUrl = { ok: true; href: string } | { ok: false; reason: WebFrameRefusal };

/** A loopback host name: `localhost` (and `*.localhost`), `127.0.0.0/8`, `[::1]`, `0.0.0.0`. */
function isLoopback(hostname: string): boolean {
  return (
    hostname === "localhost" ||
    hostname.endsWith(".localhost") ||
    /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(hostname) ||
    hostname === "[::1]" ||
    hostname === "0.0.0.0"
  );
}

/** The port `url` reaches: its own, else its scheme's. */
function port(url: URL): string {
  return url.port === "" ? (url.protocol === "https:" ? "443" : "80") : url.port;
}

/**
 * Checks what a web frame may show (UI-671): an absolute `http:` or `https:` URL — any site, `localhost` and
 * `127.0.0.1` included — but never the window's own origin `own` (the daemon's, `location.origin`), nor a loopback
 * alias of it (`localhost:7420` beside `127.0.0.1:7420`). `javascript:`, `data:`, `blob:`, `file:` and relative URLs
 * are refused.
 */
export function webFrameUrl(text: unknown, own: string): WebFrameUrl {
  if (typeof text !== "string" || text.trim() === "") {
    return { ok: false, reason: "missing" };
  }
  let url: URL;
  try {
    url = new URL(text.trim());
  } catch {
    return { ok: false, reason: "invalid" };
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return { ok: false, reason: "scheme" };
  }
  const window = URL.canParse(own) ? new URL(own) : undefined;
  if (
    window !== undefined &&
    (url.origin === window.origin ||
      (port(url) === port(window) && isLoopback(url.hostname) && isLoopback(window.hostname)))
  ) {
    return { ok: false, reason: "own-origin" };
  }
  return { ok: true, href: url.href };
}

/** The words of a web frame; the kit's are English. */
export interface WebFrameTexts {
  /** The toolbar's accessible name. */
  toolbar: string;
  reload: string;
  open: string;
  /** The quiet button that shows {@link WebFrameTexts.refusedHint} on demand. */
  blank: string;
  /** Said when the page has not loaded in time, or on {@link WebFrameTexts.blank}. */
  refusedHint: string;
  dismiss: string;
  /** Titles of a refused URL. */
  missing: string;
  invalid: string;
  scheme: string;
  ownOrigin: string;
  /** The hint under a refused URL. */
  refusedUrlHint: string;
}

export const WEB_FRAME_TEXTS: WebFrameTexts = {
  toolbar: "Web page",
  reload: "Reload",
  open: "Open in browser",
  blank: "Blank?",
  refusedHint:
    "Nothing showing? The site may refuse to be shown in a frame (X-Frame-Options or a CSP frame-ancestors " +
    "header), or nothing answers at this address. Open it in the browser instead.",
  dismiss: "Dismiss",
  missing: "No URL to show",
  invalid: "Not a URL",
  scheme: "Only http and https pages can show here",
  ownOrigin: "omfp's own pages cannot show in a frame",
  refusedUrlHint: "Give an absolute http:// or https:// URL, e.g. http://localhost:5173/.",
};

const REFUSAL_TITLE: Readonly<Record<WebFrameRefusal, keyof WebFrameTexts>> = {
  missing: "missing",
  invalid: "invalid",
  scheme: "scheme",
  "own-origin": "ownOrigin",
};

export interface WebFrameProps extends Omit<HTMLAttributes<HTMLElement>, "title" | "children"> {
  /** The page: an absolute `http(s)` URL other than the window's own origin (see {@link webFrameUrl}). */
  url: unknown;
  /** The frame's accessible title; default: the URL's host. */
  title?: string | undefined;
  /** How long to wait for the page's `load` before the refusal hint (default {@link WEB_FRAME_TIMEOUT_MS}). */
  loadTimeoutMs?: number;
  texts?: Partial<WebFrameTexts>;
}

/**
 * A web page in a sandboxed frame (UI-670..UI-674) under a toolbar: the URL, Reload (the frame loads again), Open in
 * browser (a new window: the browser's tab, the desktop's system browser) and Blank?. Whether a site refuses framing
 * cannot be told from outside the frame, so the hint saying it may shows when the page has not loaded in
 * `loadTimeoutMs`, or on Blank?. In a window that draws web pages itself (UI-682) the frame is a placeholder the
 * window draws the page over, and there is no hint.
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
  const placeholderId = useId();
  // The window's origin: the daemon's for the app.
  const checked = webFrameUrl(url, globalThis.location?.origin ?? "");
  const href = checked.ok ? checked.href : undefined;
  const [reloads, setReloads] = useState(0);
  // The frame now showing: a reload or another URL waits for its page again.
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
    return (
      <EmptyState
        data-testid="web-frame-refused"
        {...props}
        icon="globe"
        title={words[REFUSAL_TITLE[checked.reason]]}
        className={cx("omfp-web-frame__refused", className)}
        data-reason={checked.reason}
      >
        {words.refusedUrlHint}
      </EmptyState>
    );
  }
  const host = new URL(checked.href).host;
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
          onPress={() => setReloads((count) => count + 1)}
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
      {native ? (
        <div
          key={frame}
          className="omfp-web-frame__frame omfp-web-frame__native"
          role="document"
          aria-label={title ?? host}
          data-omfp-web-view={`${placeholderId}-${reloads}`}
          data-url={checked.href}
          data-testid="web-frame-native"
        />
      ) : (
        <iframe
          key={frame}
          className="omfp-web-frame__frame"
          src={checked.href}
          title={title ?? host}
          sandbox={WEB_FRAME_SANDBOX}
          referrerPolicy="no-referrer"
          data-testid="web-frame-frame"
          onLoad={() => {
            setLoadedFrame(frame);
            setHintFrame(undefined);
          }}
        />
      )}
    </section>
  );
}
