/** L4: `:copy` in the transcript (UI-790): the focused message, else the whole conversation. */

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import { act, cleanup, screen, waitFor } from "@testing-library/react";
import { SUCCESSES } from "../../../src/sdk/index.ts";
import { TranscriptWidget } from "../../../src/widgets/chat/index.ts";
import { renderThread } from "./harness.tsx";
import { fakeLayout } from "./layout.ts";

let restoreLayout: () => void = () => {};
beforeEach(() => {
  restoreLayout = fakeLayout();
});
afterEach(() => {
  cleanup();
  restoreLayout();
});

const HISTORY: ThreadEvent[] = [
  {
    kind: "history",
    session: { session_id: "s", resumed: true },
    messages: [
      { role: "user", content: "first question", timestamp: 1 },
      {
        role: "assistant",
        content: [
          { type: "thinking", thinking: "easy" },
          { type: "text", text: "first **answer**" },
        ],
        timestamp: 2,
      },
    ],
  } as ThreadEvent,
];

describe(":copy in the transcript", () => {
  // spec: UI-790
  test("copies the focused message without its heading, else the whole conversation", async () => {
    const test = renderThread(<TranscriptWidget />, HISTORY);
    const successes = () => test.host.window.get(SUCCESSES).map((success) => success.text);
    act(() => (screen.getAllByTestId("transcript-item")[1] as HTMLElement).focus());
    act(() => {
      test.host.commands.run("thread.copy", [], ["transcript"]);
    });
    await waitFor(() => expect(successes()).toEqual(["Copied the message"]));
    expect(await navigator.clipboard.readText()).toBe("first **answer**\n");

    (document.activeElement as HTMLElement | null)?.blur();
    act(() => {
      test.host.commands.run("thread.copy", [], ["transcript"]);
    });
    await waitFor(() => expect(successes()).toEqual(["Copied the message", "Copied the conversation"]));
    expect(await navigator.clipboard.readText()).toBe(
      "# 01T\n\n## You\n\nfirst question\n\n## Assistant\n\nfirst **answer**\n",
    );
  });
});
