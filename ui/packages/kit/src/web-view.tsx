/**
 * The kit's `WebView` (UI-670..UI-672, UI-675, UI-682, UI-685, UI-687): a web page alone, filling its container — a
 * placeholder the desktop window draws a native web view over where the window defines `__omfpWebViews`, a sandboxed
 * frame elsewhere — with what a widget needs to build a browser around it (where the page went, its loads, reload,
 * back, forward) and to talk with the page (scripts run in it, messages it posts). Also `webFrameUrl`, the check of the
 * URLs a web view may show.
 */

import {
  type HTMLAttributes,
  type ReactNode,
  type Ref,
  useCallback,
  useId,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { cx } from "./class.ts";
import { EmptyState } from "./feedback.tsx";

/**
 * The `sandbox` of a web frame (UI-672): its scripts run, and in its own origin (`allow-same-origin`: cookies, storage,
 * the dev servers and dashboards that need them), its forms submit and its links open new windows (the browser's tab,
 * the desktop's system browser). Never `allow-top-navigation*`: the framed page cannot navigate the window. It never
 * shares the window's origin either — {@link webFrameUrl} refuses the daemon's — or `allow-same-origin` with
 * `allow-scripts` would let it lift its own sandbox and read the daemon's token.
 */
export const WEB_FRAME_SANDBOX =
  "allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox";

/**
 * Whether the window draws web pages itself (UI-682): the desktop window defines `__omfpWebViews` in its page and draws
 * a native view over each `[data-omfp-web-view]` placeholder, where sites that refuse framing show too.
 */
export function hasNativeWebViews(): boolean {
  return "__omfpWebViews" in globalThis;
}

/** Why {@link webFrameUrl} refuses a URL. */
export type WebFrameRefusal = "missing" | "invalid" | "scheme" | "own-origin";

/** A URL a web view may show (`href`, normalized), or why it may not. */
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
 * Checks what a web view may show (UI-671): an absolute `http:` or `https:` URL — any site, `localhost` and
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

/** The words of a web view: what it says of a URL it refuses; the kit's are English. */
export interface WebViewTexts {
  missing: string;
  invalid: string;
  scheme: string;
  ownOrigin: string;
  /** The hint under a refused URL. */
  refusedUrlHint: string;
}

export const WEB_VIEW_TEXTS: WebViewTexts = {
  missing: "No URL to show",
  invalid: "Not a URL",
  scheme: "Only http and https pages can show here",
  ownOrigin: "omfp's own pages cannot show in a frame",
  refusedUrlHint: "Give an absolute http:// or https:// URL, e.g. http://localhost:5173/.",
};

const REFUSAL_TITLE: Readonly<Record<WebFrameRefusal, keyof WebViewTexts>> = {
  missing: "missing",
  invalid: "invalid",
  scheme: "scheme",
  "own-origin": "ownOrigin",
};

/** Where a web view's page went (UI-685): `canGoBack` and `canGoForward` are `null` where the window cannot tell. */
export interface WebViewLocation {
  url: string;
  canGoBack: boolean | null;
  canGoForward: boolean | null;
}

/** What a widget can ask of its web view's page (UI-685). */
export interface WebViewHandle {
  /** Whether the page is a native view of the window (UI-682): `back` and `forward` work only then. */
  readonly native: boolean;
  /** Loads the page again: the page it shows now in a native view, `url` in a frame. */
  reload(): void;
  back(): void;
  forward(): void;
}

export interface WebViewProps extends Omit<HTMLAttributes<HTMLElement>, "title" | "children" | "onLoad"> {
  /** The page: an absolute `http(s)` URL other than the window's own origin (see {@link webFrameUrl}). */
  url: unknown;
  /** The page's accessible name; default: the URL's host. */
  title?: string | undefined;
  /** The page went elsewhere (a link followed, a redirect): native views only, a frame's page cannot be followed. */
  onNavigate?: ((location: WebViewLocation) => void) | undefined;
  /** The page finished loading. */
  onLoad?: (() => void) | undefined;
  /**
   * A message from the page (UI-687): in a native view, what the page passed to `window.omfp.post(data)` (defined
   * only while `onMessage` is given), as JSON; in a frame, what it passed to `parent.postMessage(data, "*")`. It comes
   * from the page: check it before using it.
   */
  onMessage?: ((data: unknown) => void) | undefined;
  /**
   * Scripts run in a native view's page at document start, before its own (UI-687): what a user script does —
   * restyle the page, hide what is in the way, `window.omfp.post` what the widget needs. A change loads the page
   * anew. Frames run none: a page of another origin cannot be scripted.
   */
  scripts?: readonly string[] | undefined;
  texts?: Partial<WebViewTexts>;
  ref?: Ref<WebViewHandle>;
}

/** Hands `command` for the native view of placeholder `id` to the window's script (UI-685). */
function command(id: string, name: "reload" | "back" | "forward"): void {
  const bridge: unknown = Reflect.get(globalThis, "__omfpWebViews");
  if (typeof bridge === "object" && bridge !== null && "command" in bridge && typeof bridge.command === "function") {
    bridge.command(id, name);
  }
}

/** The location of a navigation event the window's script dispatched; `undefined` for anything else. */
function locationOf(event: Event): WebViewLocation | undefined {
  const detail: unknown = event instanceof CustomEvent ? event.detail : undefined;
  if (typeof detail !== "object" || detail === null || !("url" in detail) || typeof detail.url !== "string") {
    return undefined;
  }
  const flag = (key: string): boolean | null => {
    const value: unknown = Reflect.get(detail, key);
    return typeof value === "boolean" ? value : null;
  };
  return { url: detail.url, canGoBack: flag("canGoBack"), canGoForward: flag("canGoForward") };
}

/**
 * A web page filling its container, and nothing else (UI-670, UI-682): a native view of the window where it draws them
 * (the desktop app), so sites that refuse framing show too; elsewhere a sandboxed frame. Widgets build their own
 * browser around it from `onNavigate`, `onLoad` and the handle's `reload`, `back` and `forward` (UI-685), and talk with
 * the page through `scripts` and `onMessage` (UI-687); the kit's `WebFrame` is one.
 */
export function WebView({
  url,
  title,
  onNavigate,
  onLoad,
  onMessage,
  scripts,
  texts,
  ref,
  className,
  ...props
}: WebViewProps): ReactNode {
  const words = { ...WEB_VIEW_TEXTS, ...texts };
  const native = hasNativeWebViews();
  const id = useId();
  const checked = webFrameUrl(url, globalThis.location?.origin ?? "");
  const [reloads, setReloads] = useState(0);
  // The latest callbacks, so that a new function each render neither re-adds the listeners nor misses an event.
  const callbacks = useRef({ onNavigate, onLoad, onMessage });
  useLayoutEffect(() => {
    callbacks.current = { onNavigate, onLoad, onMessage };
  });
  useImperativeHandle(
    ref,
    () => ({
      native,
      reload: () => (native ? command(id, "reload") : setReloads((count) => count + 1)),
      back: () => {
        if (native) {
          command(id, "back");
        }
      },
      forward: () => {
        if (native) {
          command(id, "forward");
        }
      },
    }),
    [native, id],
  );
  // The placeholder's listeners, from its mount to its unmount: the window's script dispatches its view's events there.
  const listen = useCallback((element: HTMLDivElement | null) => {
    if (element === null) {
      return;
    }
    const navigated = (event: Event) => {
      const location = locationOf(event);
      if (location !== undefined) {
        callbacks.current.onNavigate?.(location);
      }
    };
    const loaded = () => callbacks.current.onLoad?.();
    const message = (event: Event) => {
      if (event instanceof CustomEvent && typeof event.detail === "object" && event.detail !== null) {
        callbacks.current.onMessage?.(Reflect.get(event.detail, "data"));
      }
    };
    element.addEventListener("omfp-web-view-navigate", navigated);
    element.addEventListener("omfp-web-view-load", loaded);
    element.addEventListener("omfp-web-view-message", message);
    return () => {
      element.removeEventListener("omfp-web-view-navigate", navigated);
      element.removeEventListener("omfp-web-view-load", loaded);
      element.removeEventListener("omfp-web-view-message", message);
    };
  }, []);
  // A frame's messages: those its own page posted to the window (UI-687).
  const listenFrame = useCallback((frame: HTMLIFrameElement | null) => {
    if (frame === null) {
      return;
    }
    const received = (event: MessageEvent) => {
      if (event.source !== null && event.source === frame.contentWindow) {
        callbacks.current.onMessage?.(event.data);
      }
    };
    globalThis.addEventListener("message", received);
    return () => globalThis.removeEventListener("message", received);
  }, []);
  if (!checked.ok) {
    return (
      <EmptyState
        data-testid="web-view-refused"
        {...props}
        icon="globe"
        title={words[REFUSAL_TITLE[checked.reason]]}
        className={cx("omfp-web-view__refused", className)}
        data-reason={checked.reason}
      >
        {words.refusedUrlHint}
      </EmptyState>
    );
  }
  const name = title ?? new URL(checked.href).host;
  if (native) {
    return (
      <div
        data-testid="web-view-native"
        {...props}
        ref={listen}
        className={cx("omfp-web-view omfp-web-view--native", className)}
        role="document"
        aria-label={name}
        data-omfp-web-view={id}
        data-url={checked.href}
        data-setup={JSON.stringify({ scripts: scripts ?? [], channel: onMessage !== undefined })}
      />
    );
  }
  return (
    <iframe
      data-testid="web-frame-frame"
      {...props}
      key={reloads}
      ref={listenFrame}
      className={cx("omfp-web-view", className)}
      src={checked.href}
      title={name}
      sandbox={WEB_FRAME_SANDBOX}
      referrerPolicy="no-referrer"
      onLoad={() => callbacks.current.onLoad?.()}
    />
  );
}
