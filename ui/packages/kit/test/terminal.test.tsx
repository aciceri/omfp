/** L1: the terminal emulator's clipboard keys (TRM-108) and what reaches the program. */

import { afterEach, describe, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { TerminalEmulator } from "../src/index.ts";
import { type ClipboardTerminal, clipboardKeys, type TerminalClipboard } from "../src/terminal.tsx";

afterEach(cleanup);

/** A fake clipboard holding `text`; `refuse` rejects every call, as a denied permission does. */
function fakeClipboard(text: string, refuse = false) {
  const writes: string[] = [];
  const calls: Promise<unknown>[] = [];
  const answer = <T,>(value: T): Promise<T> => {
    const call = refuse ? Promise.reject(new DOMException("denied", "NotAllowedError")) : Promise.resolve(value);
    calls.push(call);
    return call;
  };
  const clipboard: TerminalClipboard = {
    readText: () => answer(text),
    writeText: (written) => {
      if (!refuse) {
        writes.push(written);
      }
      return answer(undefined);
    },
  };
  /** Every call made so far has settled, and so have the handler's reactions to it, attached before ours. */
  const settled = () => Promise.allSettled(calls);
  return { clipboard, writes, settled };
}

/** A fake emulator with `selection` selected. */
function fakeTerminal(selection: string) {
  const pasted: string[] = [];
  const terminal: ClipboardTerminal = {
    hasSelection: () => selection !== "",
    getSelection: () => selection,
    paste: (text) => pasted.push(text),
  };
  return { terminal, pasted };
}

function key(init: KeyboardEventInit, type = "keydown"): KeyboardEvent {
  return new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init });
}

const SESSION = () => "one";

describe("clipboardKeys", () => {
  // spec: TRM-108
  test("Ctrl+Shift+C copies the selection and Ctrl+Shift+V pastes the clipboard through paste", async () => {
    const { clipboard, writes, settled } = fakeClipboard("echo pasted\n");
    const { terminal, pasted } = fakeTerminal("selected text");
    const handle = clipboardKeys(terminal, { mac: false, clipboard: () => clipboard, session: SESSION });
    const copy = key({ key: "C", ctrlKey: true, shiftKey: true });
    const paste = key({ key: "V", ctrlKey: true, shiftKey: true });
    expect([handle(copy), copy.defaultPrevented]).toEqual([false, true]);
    expect([handle(paste), paste.defaultPrevented]).toEqual([false, true]);
    await settled();
    expect(writes).toEqual(["selected text"]);
    expect(pasted).toEqual(["echo pasted\n"]);
  });

  // spec: TRM-108
  test("on macOS Cmd+C copies and Cmd+V is left to the platform's paste event", async () => {
    const { clipboard, writes, settled } = fakeClipboard("from another app");
    const { terminal, pasted } = fakeTerminal("selected text");
    const handle = clipboardKeys(terminal, { mac: true, clipboard: () => clipboard, session: SESSION });
    const copy = key({ key: "c", metaKey: true });
    const paste = key({ key: "v", metaKey: true });
    expect([handle(copy), copy.defaultPrevented]).toEqual([false, true]);
    expect([handle(paste), paste.defaultPrevented]).toEqual([false, false]);
    // The Ctrl+Shift chords are no clipboard keys there.
    expect(handle(key({ key: "C", ctrlKey: true, shiftKey: true }))).toBe(true);
    await settled();
    expect(writes).toEqual(["selected text"]);
    expect(pasted).toEqual([]);
  });

  // spec: TRM-108
  test("plain Ctrl+C and Ctrl+V and other chords go on to the program", () => {
    const { clipboard, writes } = fakeClipboard("text");
    const { terminal } = fakeTerminal("selected text");
    for (const mac of [false, true]) {
      const handle = clipboardKeys(terminal, { mac, clipboard: () => clipboard, session: SESSION });
      for (const init of [
        { key: "c", ctrlKey: true },
        { key: "v", ctrlKey: true },
        { key: "C", ctrlKey: true, shiftKey: true, altKey: true },
        { key: "c", metaKey: true, shiftKey: true },
        { key: "A", ctrlKey: true, shiftKey: true },
        { key: "c" },
      ]) {
        const event = key(init);
        expect([mac, init, handle(event), event.defaultPrevented]).toEqual([mac, init, true, false]);
      }
    }
    expect(writes).toEqual([]);
  });

  // spec: TRM-108
  test("a clipboard key's keypress and keyup stay out of xterm.js; only keydown acts", () => {
    const read = mock(() => Promise.resolve("text"));
    const write = mock((_: string) => Promise.resolve());
    const handle = clipboardKeys(fakeTerminal("selected text").terminal, {
      mac: false,
      clipboard: () => ({ readText: read, writeText: write }),
      session: SESSION,
    });
    expect(handle(key({ key: "C", ctrlKey: true, shiftKey: true }, "keyup"))).toBe(false);
    expect(handle(key({ key: "V", ctrlKey: true, shiftKey: true }, "keypress"))).toBe(false);
    expect([read.mock.calls.length, write.mock.calls.length]).toEqual([0, 0]);
  });

  // spec: TRM-108
  test("without a selection Ctrl+Shift+C copies nothing; a refused clipboard throws nothing", async () => {
    const empty = fakeClipboard("text");
    const handleEmpty = clipboardKeys(fakeTerminal("").terminal, {
      mac: false,
      clipboard: () => empty.clipboard,
      session: SESSION,
    });
    expect(handleEmpty(key({ key: "C", ctrlKey: true, shiftKey: true }))).toBe(false);
    const refused = fakeClipboard("text", true);
    const { terminal, pasted } = fakeTerminal("selected text");
    const handle = clipboardKeys(terminal, { mac: false, clipboard: () => refused.clipboard, session: SESSION });
    expect(handle(key({ key: "C", ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(handle(key({ key: "V", ctrlKey: true, shiftKey: true }))).toBe(false);
    const outcomes = await refused.settled();
    expect(outcomes.map((outcome) => outcome.status)).toEqual(["rejected", "rejected"]);
    await empty.settled();
    expect([empty.writes, refused.writes, pasted]).toEqual([[], [], []]);
  });

  // spec: TRM-108
  test("without a clipboard API copy falls back to the copy command and paste to the platform's own", () => {
    // happy-dom has no `execCommand`.
    const execCommand = mock((_: string) => true);
    Object.defineProperty(document, "execCommand", { configurable: true, value: execCommand });
    try {
      const { terminal, pasted } = fakeTerminal("selected text");
      const handle = clipboardKeys(terminal, { mac: false, clipboard: () => undefined, session: SESSION });
      const copy = key({ key: "C", ctrlKey: true, shiftKey: true });
      const paste = key({ key: "V", ctrlKey: true, shiftKey: true });
      expect([handle(copy), copy.defaultPrevented]).toEqual([false, true]);
      expect([handle(paste), paste.defaultPrevented]).toEqual([false, false]);
      expect(execCommand.mock.calls).toEqual([["copy"]]);
      expect(pasted).toEqual([]);
    } finally {
      Reflect.deleteProperty(document, "execCommand");
    }
  });
});

describe("TerminalEmulator", () => {
  function textarea(container: HTMLElement): HTMLTextAreaElement {
    const found = container.querySelector(".xterm-helper-textarea");
    if (!(found instanceof HTMLTextAreaElement)) {
      throw new Error("no xterm.js textarea");
    }
    return found;
  }

  /** Runs `body` with `clipboard` as the page's `navigator.clipboard`. */
  async function withClipboard(clipboard: TerminalClipboard, body: () => Promise<void>): Promise<void> {
    const original = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: clipboard });
    try {
      await body();
    } finally {
      if (original === undefined) {
        Reflect.deleteProperty(navigator, "clipboard");
      } else {
        Object.defineProperty(navigator, "clipboard", original);
      }
    }
  }

  const PASTE_KEY = { key: "V", code: "KeyV", keyCode: 86, ctrlKey: true, shiftKey: true };

  // spec: TRM-108
  test("Ctrl+Shift+V pastes the clipboard as input, Ctrl+C reaches the program, the window sees handled keys prevented", async () => {
    const seen: [string, boolean][] = [];
    const listener = (event: KeyboardEvent) => seen.push([event.key, event.defaultPrevented]);
    document.addEventListener("keydown", listener);
    try {
      await withClipboard(fakeClipboard("echo one\necho two").clipboard, async () => {
        const onData = mock((_: string) => undefined);
        const { container } = render(<TerminalEmulator sessionKey="one" chunks={[]} onData={onData} />);
        const input = textarea(container);
        fireEvent.keyDown(input, PASTE_KEY);
        await waitFor(() => expect(onData.mock.calls).toEqual([["echo one\recho two"]]));
        fireEvent.keyDown(input, { key: "c", code: "KeyC", keyCode: 67, ctrlKey: true });
        expect(onData.mock.calls).toEqual([["echo one\recho two"], ["\x03"]]);
        // xterm.js stops the keys it sends; the clipboard key reaches the window's keymap already prevented.
        expect(seen).toEqual([["V", true]]);
      });
    } finally {
      document.removeEventListener("keydown", listener);
    }
  });

  // spec: TRM-108
  test("a paste whose clipboard read ends after the session changed is dropped", async () => {
    const read = Promise.withResolvers<string>();
    const clipboard: TerminalClipboard = { readText: () => read.promise, writeText: () => Promise.resolve() };
    await withClipboard(clipboard, async () => {
      const onData = mock((_: string) => undefined);
      const { container, rerender } = render(<TerminalEmulator sessionKey="one" chunks={[]} onData={onData} />);
      fireEvent.keyDown(textarea(container), PASTE_KEY);
      rerender(<TerminalEmulator sessionKey="two" chunks={[]} onData={onData} />);
      read.resolve("rm -r build\n");
      // The handler's reaction was attached first, so it has run once this one does.
      await read.promise;
      expect(onData.mock.calls).toEqual([]);
    });
  });
});
