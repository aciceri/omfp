/** A reusable xterm.js emulator surface. Process ownership and transport stay outside this component. */

import { FitAddon } from "@xterm/addon-fit";
import { type ITerminalOptions, type ITheme, Terminal as XTerm } from "@xterm/xterm";
import { type ReactNode, useEffect, useRef } from "react";
import { cx } from "./class.ts";
import { cssColorReader } from "./theme-colors.ts";
import { watchTheme } from "./theme-watch.ts";

export interface TerminalEmulatorProps {
  /** Identifies the output stream. A change resets the emulator before replaying `chunks`. */
  readonly sessionKey: string | undefined;
  /** Base64-encoded byte chunks, oldest first. */
  readonly chunks: readonly string[];
  readonly onData: (data: string) => void;
  readonly onTerminalResize?: (cols: number, rows: number) => void;
  readonly cols?: number;
  readonly rows?: number;
  /** xterm.js options; the keys of `theme` override the colors read from the theme's tokens (TRM-111). */
  readonly options?: Readonly<ITerminalOptions>;
  readonly className?: string;
  readonly ariaLabel?: string;
}

function decode(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

/** xterm.js theme keys and the `--omfp-terminal-` tokens they come from (TRM-110). */
const THEME_TOKENS: readonly (readonly [Exclude<keyof ITheme, "extendedAnsi">, string])[] = [
  ["background", "bg"],
  ["foreground", "fg"],
  ["cursor", "cursor"],
  ["cursorAccent", "cursor-accent"],
  ["selectionBackground", "selection"],
  ["black", "black"],
  ["red", "red"],
  ["green", "green"],
  ["yellow", "yellow"],
  ["blue", "blue"],
  ["magenta", "magenta"],
  ["cyan", "cyan"],
  ["white", "white"],
  ["brightBlack", "bright-black"],
  ["brightRed", "bright-red"],
  ["brightGreen", "bright-green"],
  ["brightYellow", "bright-yellow"],
  ["brightBlue", "bright-blue"],
  ["brightMagenta", "bright-magenta"],
  ["brightCyan", "bright-cyan"],
  ["brightWhite", "bright-white"],
];

/** `#rrggbb` as is, an opaque `rgb()` as `#rrggbb`; anything else goes through the reader's canvas. */
function hexColor(color: string): string | undefined {
  if (/^#[0-9a-f]{6}$/i.test(color)) {
    return color;
  }
  const channels = /^rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*1)?\)$/.exec(color)?.slice(1);
  return channels === undefined
    ? undefined
    : `#${channels.map((channel) => Number(channel).toString(16).padStart(2, "0")).join("")}`;
}

/** The terminal colors of the theme as `element` sees them (TRM-110); a token that does not resolve is left out. */
function tokenTheme(element: HTMLElement): ITheme {
  const read = cssColorReader(element.ownerDocument, hexColor, element);
  const theme: ITheme = {};
  for (const [key, token] of THEME_TOKENS) {
    const color = read(`terminal-${token}`);
    if (color !== undefined) {
      theme[key] = color;
    }
  }
  return theme;
}

/** Longest prefix of `next` that is a suffix of `previous`, in linear time. */
function chunkOverlap(previous: readonly string[], next: readonly string[]): number {
  if (next.length === 0) {
    return 0;
  }
  const prefix = new Array<number>(next.length).fill(0);
  for (let index = 1; index < next.length; index += 1) {
    let matched = prefix[index - 1] ?? 0;
    while (matched > 0 && next[index] !== next[matched]) {
      matched = prefix[matched - 1] ?? 0;
    }
    if (next[index] === next[matched]) {
      matched += 1;
    }
    prefix[index] = matched;
  }
  let matched = 0;
  for (let index = 0; index < previous.length; index += 1) {
    const chunk = previous[index];
    while (matched > 0 && chunk !== next[matched]) {
      matched = prefix[matched - 1] ?? 0;
    }
    if (chunk === next[matched]) {
      matched += 1;
    }
    if (matched === next.length && index + 1 < previous.length) {
      matched = prefix[matched - 1] ?? 0;
    }
  }
  return matched;
}

/** The page's clipboard as the terminal uses it. */
export type TerminalClipboard = Pick<Clipboard, "readText" | "writeText">;

/** What the clipboard keys need of the emulator. */
export type ClipboardTerminal = Pick<XTerm, "hasSelection" | "getSelection" | "paste">;

export interface ClipboardKeyOptions {
  /** macOS takes Cmd chords; elsewhere Ctrl+Shift ones. */
  readonly mac: boolean;
  /** The page's clipboard at the time of the key press; `undefined` where it has none (an insecure origin). */
  readonly clipboard: () => TerminalClipboard | undefined;
  /** The output stream the terminal shows now: a paste whose clipboard read ends after it changed is dropped. */
  readonly session: () => unknown;
}

/** Whether the page runs on macOS (`navigator.platform` `MacIntel`, also in the desktop window's WKWebView). */
const MAC = typeof navigator !== "undefined" && navigator.platform.startsWith("Mac");

/** `navigator.clipboard`, which only secure origins have, whatever its type says. */
function pageClipboard(): TerminalClipboard | undefined {
  return typeof navigator === "undefined" ? undefined : (navigator.clipboard as Clipboard | undefined);
}

function clipboardAction(event: KeyboardEvent, mac: boolean): "copy" | "paste" | undefined {
  const chord = mac
    ? event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
    : event.ctrlKey && event.shiftKey && !event.altKey && !event.metaKey;
  if (!chord) {
    return undefined;
  }
  const key = event.key.toLowerCase();
  return key === "c" ? "copy" : key === "v" ? "paste" : undefined;
}

/**
 * xterm.js's custom key handler for the clipboard keys (TRM-108): Ctrl+Shift+C / Cmd+C copies the selection and
 * Ctrl+Shift+V pastes the clipboard through `paste`, bracketed when the program asked for it. macOS's Cmd+V, and
 * Ctrl+Shift+V without a clipboard API, are left to the platform's own paste, whose `paste` event xterm.js pastes the
 * same way: WKWebView and Safari would confirm every `readText` of another app's text. A handled key is prevented, so
 * the window keymap leaves it alone; any other key, plain Ctrl+C and Ctrl+V included, goes on to the program. A paste
 * whose clipboard read (maybe behind a confirmation) ends after the session changed is dropped: it was meant for the
 * previous program.
 */
export function clipboardKeys(
  terminal: ClipboardTerminal,
  { mac, clipboard, session }: ClipboardKeyOptions,
): (event: KeyboardEvent) => boolean {
  return (event) => {
    const action = clipboardAction(event, mac);
    if (action === undefined) {
      return true;
    }
    if (event.type !== "keydown") {
      return false;
    }
    const target = clipboard();
    if (action === "paste") {
      if (!mac && target !== undefined) {
        event.preventDefault();
        const asked = session();
        target.readText().then(
          (text) => {
            if (session() === asked) {
              terminal.paste(text);
            }
          },
          () => undefined,
        );
      }
      return false;
    }
    event.preventDefault();
    if (!terminal.hasSelection()) {
      return false;
    }
    if (target === undefined) {
      // xterm.js answers the `copy` event with its selection.
      document.execCommand("copy");
    } else {
      target.writeText(terminal.getSelection()).catch(() => undefined);
    }
    return false;
  };
}

/** An xterm.js terminal that replays byte chunks and forwards input and measured PTY dimensions. */
export function TerminalEmulator({
  sessionKey,
  chunks,
  onData,
  onTerminalResize,
  cols = 80,
  rows = 24,
  options,
  className,
  ariaLabel = "Terminal",
}: TerminalEmulatorProps): ReactNode {
  const host = useRef<HTMLDivElement>(null);
  const emulator = useRef<XTerm | null>(null);
  const callbacks = useRef({ onData, onTerminalResize });
  const rendered = useRef<{ sessionKey: string | undefined; chunks: readonly string[] }>({
    sessionKey: undefined,
    chunks: [],
  });
  const stream = useRef({ sessionKey, chunks });
  stream.current = { sessionKey, chunks };
  callbacks.current = { onData, onTerminalResize };

  useEffect(() => {
    const element = host.current;
    if (element === null) {
      return undefined;
    }
    // The theme's colors under the widget's own (TRM-111); a theme change repaints the emulator in place.
    const themed = (): ITheme => ({ ...tokenTheme(element), ...options?.theme });
    const theme = themed();
    let applied = JSON.stringify(theme);
    const terminal = new XTerm({
      cursorBlink: true,
      scrollback: 2000,
      ...options,
      theme,
      cols,
      rows,
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.attachCustomKeyEventHandler(
      clipboardKeys(terminal, { mac: MAC, clipboard: pageClipboard, session: () => stream.current.sessionKey }),
    );
    terminal.open(element);
    // xterm.js 5.5's viewport syncs its scroll area in a zero-delay timer that `open` starts and `dispose` does not
    // cancel (xtermjs/xterm.js#5011): disposed before it runs, the terminal throws from it. A timer started right
    // after it, from the same task, runs after it (TRM-112).
    const opened = new Promise<void>((resolve) => setTimeout(resolve));
    emulator.current = terminal;
    const currentStream = stream.current;
    for (const chunk of currentStream.chunks) {
      terminal.write(decode(chunk));
    }
    rendered.current = currentStream;
    const input = terminal.onData((data) => callbacks.current.onData(data));
    const resize = () => {
      fit.fit();
      callbacks.current.onTerminalResize?.(terminal.cols, terminal.rows);
    };
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(resize);
    observer?.observe(element);
    resize();
    const unwatch = watchTheme(element.ownerDocument, () => {
      const next = themed();
      const key = JSON.stringify(next);
      if (key !== applied) {
        applied = key;
        terminal.options.theme = next;
      }
    });
    return () => {
      unwatch();
      observer?.disconnect();
      input.dispose();
      emulator.current = null;
      terminal.element?.remove();
      void opened.then(() => terminal.dispose());
    };
  }, [cols, options, rows]);

  useEffect(() => {
    const terminal = emulator.current;
    if (terminal === null) {
      return;
    }
    const current = rendered.current;
    const overlap = current.sessionKey === sessionKey ? chunkOverlap(current.chunks, chunks) : 0;
    if (
      current.sessionKey !== sessionKey ||
      (current.chunks.length > 0 && chunks.length > 0 && overlap === 0) ||
      (current.chunks.length > 0 && chunks.length === 0)
    ) {
      terminal.reset();
      for (const chunk of chunks) {
        terminal.write(decode(chunk));
      }
    } else {
      for (const chunk of chunks.slice(overlap)) {
        terminal.write(decode(chunk));
      }
    }
    rendered.current = { sessionKey, chunks };
  }, [chunks, sessionKey]);

  return <section ref={host} className={cx("omfp-terminal-emulator", className)} aria-label={ariaLabel} />;
}
