/** A reusable xterm.js emulator surface. Process ownership and transport stay outside this component. */

import { FitAddon } from "@xterm/addon-fit";
import { type ITerminalOptions, Terminal as XTerm } from "@xterm/xterm";
import { type ReactNode, useEffect, useRef } from "react";
import { cx } from "./class.ts";

export interface TerminalEmulatorProps {
  /** Identifies the output stream. A change resets the emulator before replaying `chunks`. */
  readonly sessionKey: string | undefined;
  /** Base64-encoded byte chunks, oldest first. */
  readonly chunks: readonly string[];
  readonly onData: (data: string) => void;
  readonly onTerminalResize?: (cols: number, rows: number) => void;
  readonly cols?: number;
  readonly rows?: number;
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

function defaultTheme(element: HTMLElement): ITerminalOptions["theme"] | undefined {
  const style = getComputedStyle(element);
  const background = style.getPropertyValue("--omfp-terminal-bg").trim();
  const foreground = style.getPropertyValue("--omfp-terminal-fg").trim();
  return background === "" || foreground === "" ? undefined : { background, cursor: foreground, foreground };
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
    const theme = defaultTheme(element);
    const terminal = new XTerm({
      cursorBlink: true,
      scrollback: 2000,
      ...(theme === undefined ? {} : { theme }),
      ...options,
      cols,
      rows,
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(element);
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
    return () => {
      observer?.disconnect();
      input.dispose();
      terminal.dispose();
      emulator.current = null;
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
