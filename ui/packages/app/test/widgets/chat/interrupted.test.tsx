/** L4: the chat's notice of a turn a daemon restart interrupted, with Resume (UI-932, UI-933). */

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { ThreadSummary } from "@omfp/protocol";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ERRORS, SELECTED_THREAD } from "../../../src/sdk/index.ts";
import { ChatWidget, TranscriptWidget } from "../../../src/widgets/chat/index.ts";
import { renderWithHost, testHost } from "../../support/host.tsx";
import { fixtureEvents, summary } from "./fixtures.ts";
import { THREAD } from "./harness.tsx";
import { fakeLayout } from "./layout.ts";

let restoreLayout: () => void = () => {};
beforeEach(() => {
  restoreLayout = fakeLayout();
});
afterEach(() => {
  cleanup();
  restoreLayout();
});

const AT = new Date(2026, 10, 22, 8, 40).getTime();

function interrupted(): ThreadSummary {
  return { ...summary(THREAD), process: { state: "stopped" }, interruptedAt: AT };
}

function setup(thread: ThreadSummary, widget = <ChatWidget />) {
  const test = testHost();
  test.client.connected();
  test.client.index({ threads: [thread] });
  test.host.window.set(SELECTED_THREAD, THREAD);
  const rendered = renderWithHost(widget, test);
  test.client.thread(THREAD, fixtureEvents("text_reply"));
  test.flush();
  return { ...rendered, user: userEvent.setup() };
}

describe("interrupted turn", () => {
  // spec: UI-932
  test("the chat and the transcript show the notice only while the summary carries interruptedAt", () => {
    setup(summary(THREAD));
    expect(screen.queryAllByTestId("interrupted-turn").length).toBe(0);
    cleanup();

    setup(interrupted());
    const notice = within(screen.getByTestId("transcript")).getByTestId("interrupted-turn");
    expect(notice.dataset.level).toBe("warning");
    expect(notice.textContent).toContain("interrupted when the daemon restarted");
    expect(within(notice).getByTestId("resume-turn").textContent).toBe("Resume");
    cleanup();

    setup(interrupted(), <TranscriptWidget />);
    expect(screen.getAllByTestId("interrupted-turn").length).toBe(1);
  });

  // spec: UI-933, PRT-230
  test("Resume calls thread.resume; the notice goes when the daemon clears the mark, a refusal is an error", async () => {
    const { client, host, flush, user } = setup(interrupted());

    await user.click(screen.getByTestId("resume-turn"));
    const [first] = client.callsTo("thread.resume");
    expect(first?.params).toEqual({ threadId: THREAD });
    expect(screen.getByTestId("resume-turn").dataset.pending).toBe("true");
    await act(async () => {
      first?.reject(new Error("conflict: the thread has no turn interrupted by a daemon restart"));
    });
    await waitFor(() =>
      expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual([
        "conflict: the thread has no turn interrupted by a daemon restart",
      ]),
    );
    await waitFor(() => expect(screen.getByTestId("resume-turn").dataset.pending).toBeUndefined());

    await user.click(screen.getByTestId("resume-turn"));
    await act(async () => {
      client.callsTo("thread.resume")[1]?.resolve({ promptId: "p1", agentInvoked: true });
      client.indexEvents({ kind: "thread_upserted", thread: { ...interrupted(), interruptedAt: null } });
      flush();
    });
    expect(screen.queryAllByTestId("interrupted-turn").length).toBe(0);
  });
});
