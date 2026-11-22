/** L4: `:copy` and `:export` of the selected thread's conversation (UI-790, UI-791) over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import { threadTopic } from "@omfp/protocol";
import { act, cleanup, waitFor } from "@testing-library/react";
import { GlobalCommands } from "../../src/frame/commands.tsx";
import { DEFAULT_BINDINGS } from "../../src/keymap/keymap.ts";
import { ERRORS, SELECTED_THREAD, SUCCESSES } from "../../src/sdk/index.ts";
import { captureDownloads } from "../support/downloads.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

const HISTORY: ThreadEvent[] = [
  {
    kind: "history",
    session: { session_id: "s", resumed: true },
    messages: [
      { role: "user", content: "Why is the sky blue?", timestamp: 1 },
      { role: "assistant", content: [{ type: "text", text: "Rayleigh scattering." }], timestamp: 2 },
    ],
  } as ThreadEvent,
];

const MARKDOWN = "# Sky: “colour”\n\n## You\n\nWhy is the sky blue?\n\n## Assistant\n\nRayleigh scattering.\n";

function setup() {
  const base = testHost();
  base.host.window.set(SELECTED_THREAD, "t1");
  const test = renderWithHost(
    <GlobalCommands viewIds={[]} currentView={undefined} widgets={[]} addWidget={() => Promise.resolve(false)} />,
    base,
  );
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "Sky: “colour”", "w1")] });
  test.flush();
  return test;
}

describe(":copy and :export", () => {
  // spec: UI-790
  test(":copy starts the clipboard write at once, then fills it once a thread no widget shows arrives", async () => {
    const test = setup();
    const written: ClipboardItem[][] = [];
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        write: (items: ClipboardItem[]) => Promise.resolve(void written.push(items)),
        writeText: () => Promise.reject(new Error("writeText after the gesture")),
      },
    });
    try {
      act(() => {
        expect(test.host.commands.run("copy")).toBe(true);
      });
      // Still in the user's gesture (Safari drops the clipboard after it): the item's text comes later.
      expect(written.length).toBe(1);
      expect(test.client.subscribed).toContain(threadTopic("t1"));
      test.client.thread("t1", HISTORY);
      test.flush();
      await waitFor(() =>
        expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual(["Copied the conversation"]),
      );
      expect(await (await written[0]?.[0]?.getType("text/plain"))?.text()).toBe(MARKDOWN);
    } finally {
      Reflect.deleteProperty(navigator, "clipboard");
    }
  });

  // spec: UI-790
  test("without ClipboardItem, :copy writes the text once the thread arrives", async () => {
    const test = setup();
    const { ClipboardItem: item } = globalThis;
    Reflect.deleteProperty(globalThis, "ClipboardItem");
    try {
      act(() => {
        test.host.commands.run("copy");
      });
      test.client.thread("t1", HISTORY);
      test.flush();
      await waitFor(() =>
        expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual(["Copied the conversation"]),
      );
      expect(await navigator.clipboard.readText()).toBe(MARKDOWN);
    } finally {
      globalThis.ClipboardItem = item;
    }
  });

  // spec: UI-790, UI-520
  test("a refused clipboard is an error toast", async () => {
    const test = setup();
    test.client.thread("t1", HISTORY);
    test.flush();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: () => Promise.reject(new Error("Clipboard access denied")) },
    });
    try {
      act(() => {
        test.host.commands.run("thread.copy");
      });
      await waitFor(() =>
        expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["Clipboard access denied"]),
      );
      expect(test.host.window.get(SUCCESSES)).toEqual([]);
    } finally {
      Reflect.deleteProperty(navigator, "clipboard");
    }
  });

  // spec: UI-791
  test(":export downloads the Markdown as a file named after the title", async () => {
    const test = setup();
    test.client.thread("t1", HISTORY);
    test.flush();
    const downloads = captureDownloads();
    try {
      act(() => {
        expect(test.host.commands.run("export")).toBe(true);
      });
      await waitFor(() =>
        expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual(["Exported “sky-colour.md”"]),
      );
      expect(downloads.files.map((file) => file.name)).toEqual(["sky-colour.md"]);
      expect(await downloads.files[0]?.text).toBe(MARKDOWN);
    } finally {
      downloads.restore();
    }
  });

  // spec: UI-790, UI-791
  test("both are commands of a selected thread without a default key", () => {
    const test = setup();
    const named = (id: string) => test.host.commands.list().find((command) => command.id === id);
    expect([named("thread.copy")?.aliases, named("thread.export")?.aliases]).toEqual([["copy"], ["export"]]);
    const keyed = DEFAULT_BINDINGS.filter((binding) => ["thread.copy", "thread.export"].includes(binding.command));
    expect(keyed).toEqual([]);
    act(() => test.host.window.set(SELECTED_THREAD, undefined));
    expect([test.host.commands.resolve("copy"), test.host.commands.resolve("export")]).toEqual([undefined, undefined]);
  });
});
