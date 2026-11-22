/** L4: forking a new thread from the transcript (UI-660, UI-661, UI-663, UI-666) over a fake client. */

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ERRORS, SELECTED_THREAD, SUCCESSES } from "../../../src/sdk/index.ts";
import { TranscriptWidget } from "../../../src/widgets/chat/index.ts";
import { summary } from "./fixtures.ts";
import { renderThread, THREAD } from "./harness.tsx";
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
      { role: "assistant", content: [{ type: "text", text: "first answer" }], timestamp: 2 },
      { role: "user", content: "second question\nwith details", timestamp: 3 },
    ],
  } as ThreadEvent,
];

/** omp's tree for {@link HISTORY}: one branch, the leaf on the second question. */
const TREE = {
  leafId: "u2",
  tree: [
    {
      entry: { id: "u1", parentId: null, type: "message", message: { role: "user", timestamp: 1 } },
      children: [
        {
          entry: { id: "a1", parentId: "u1", type: "message", message: { role: "assistant", timestamp: 2 } },
          children: [
            {
              entry: { id: "u2", parentId: "a1", type: "message", message: { role: "user", timestamp: 3 } },
              children: [],
            },
          ],
        },
      ],
    },
  ],
};

function setup(tree: unknown = TREE) {
  const rendered = renderThread(<TranscriptWidget />, HISTORY);
  rendered.client.answers.set("thread.tree", () => tree);
  rendered.client.answers.set("threads.fork", () => ({ ...summary("01F"), title: "01T (fork)" }));
  return { ...rendered, user: userEvent.setup() };
}

const messages = () => screen.getAllByTestId("message");

describe("fork from the transcript", () => {
  // spec: UI-660, UI-661, UI-666
  test("a user message offers Fork from here: its entry on the branch is forked into a new thread, then shown", async () => {
    const test = setup();
    expect(messages().map((message) => within(message).queryAllByTestId("message-fork").length)).toEqual([1, 0, 1]);
    const fork = within(messages()[2] as HTMLElement).getByTestId("message-fork");
    expect(fork.getAttribute("aria-label")).toBe("Fork a new thread from \u201csecond question\u201d");

    await test.user.click(fork);

    await waitFor(() =>
      expect(test.client.callsTo("threads.fork").map((call) => call.params)).toEqual([
        { threadId: THREAD, entryId: "u2" },
      ]),
    );
    await waitFor(() => expect(test.host.window.get(SELECTED_THREAD)).toBe("01F"));
    expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual([
      "Forked into \u201c01T (fork)\u201d",
    ]);
  });

  // spec: UI-661
  test("a message the session's current branch lacks is not forked, and says why", async () => {
    const test = setup({ leafId: null, tree: [] });
    await test.user.click(within(messages()[0] as HTMLElement).getByTestId("message-fork"));
    await waitFor(() => expect(test.host.window.get(ERRORS).length).toBe(1));
    expect(test.host.window.get(ERRORS)[0]?.text).toContain("not on the session's current branch");
    expect(test.client.callsTo("threads.fork")).toEqual([]);
  });

  // spec: UI-663
  test(":fork forks from the focused message, an answer too; without one it says what to select", async () => {
    const test = setup();
    test.host.commands.run("thread.fork", [], ["transcript"]);
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
      "Select a message to fork from: j and k move between messages",
    ]);

    act(() => (screen.getAllByTestId("transcript-item")[1] as HTMLElement).focus());
    test.host.commands.run("thread.fork", [], ["transcript"]);
    await waitFor(() =>
      expect(test.client.callsTo("threads.fork").map((call) => call.params)).toEqual([
        { threadId: THREAD, entryId: "a1" },
      ]),
    );
  });
});
