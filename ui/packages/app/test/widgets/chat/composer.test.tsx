/** L4: `omfp:composer` (queue chips and composer) over recorded fixtures. */

import { afterEach, describe, expect, test } from "bun:test";
import type { JsonObject, ThreadEvent } from "@omfp/core";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createKeyHandler } from "../../../src/keymap/handler.ts";
import { DEFAULT_BINDINGS } from "../../../src/keymap/keymap.ts";
import {
  draftKey,
  ERRORS,
  FOCUS_COMPOSER,
  FRAME_LAYOUT,
  type OmfpHost,
  SELECTED_THREAD,
} from "../../../src/sdk/index.ts";
import { FILE_QUERY_DELAY_MS } from "../../../src/widgets/chat/completion.tsx";
import { ChatWidget, ComposerWidget } from "../../../src/widgets/chat/index.ts";
import type { FakeClient } from "../../support/host.tsx";
import { PointerMedia } from "../../support/pointer.ts";
import { installResizeObserver, resize } from "../../support/resize.ts";
import { candidates, eventsUntil, fixtureEvents, isFrame } from "./fixtures.ts";
import { ManualClock, renderThread, THREAD } from "./harness.tsx";

let removeKeymap: (() => void) | undefined;

afterEach(() => {
  removeKeymap?.();
  removeKeymap = undefined;
  cleanup();
});

/** The frame's global key handler with the default bindings, as in the app (UI-095). */
function installKeymap(host: OmfpHost, clock: ManualClock): void {
  const handler = createKeyHandler({ bindings: DEFAULT_BINDINGS, commands: host.commands, now: () => clock.now() });
  document.addEventListener("keydown", handler);
  removeKeymap = () => document.removeEventListener("keydown", handler);
}

/** A key press on the focused element, through React and then the document's keymap. */
function press(key: string, init: KeyboardEventInit = {}): void {
  act(() => {
    fireEvent.keyDown(document.activeElement ?? document.body, { key, ...init });
  });
}

const input = () => screen.getByTestId("composer-input") as HTMLTextAreaElement;

/** The `workspaces.find_file` params of a `@query` in {@link THREAD}'s composer (UI-732). */
const findFile = (query: string) => ({ workspaceId: "01W", query, fuzzy: true, threadId: THREAD });

/** The thread right after the follow-up was queued: busy, one chip. */
function queuedEvents(): ThreadEvent[] {
  return fixtureEvents("follow_up_then_abort").slice(0, 12);
}

function busyEvents(): ThreadEvent[] {
  return eventsUntil("text_reply", isFrame("message_update", "text_end"));
}
/** The edit fixture while its first edit is running (the preceding read has settled). */
function activeEditEvents(): ThreadEvent[] {
  let starts = 0;
  const events = fixtureEvents("edit_tool_calls");
  const editStart = events.findIndex((event) => {
    if (!isFrame("tool_execution_start")(event)) {
      return false;
    }
    starts += 1;
    return starts === 2;
  });
  return events.slice(0, editStart + 1);
}

describe("composer", () => {
  // spec: UI-060
  test("multi-line input: Alt+Enter is a newline, Enter sends and clears the draft", async () => {
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("first line{Alt>}{Enter}{/Alt}second line");
    expect(input().value).toBe("first line\nsecond line");
    expect(screen.getByTestId("send").dataset.mode).toBe("prompt");
    await userEvent.keyboard("{Enter}");
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: THREAD, text: "first line\nsecond line" },
    ]);
    expect(input().value).toBe("");
  });

  // spec: UI-064
  test("a prompt that fails to send comes back into the draft", async () => {
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("hello{Enter}");
    await userEvent.keyboard("more");
    await act(async () => client.callsTo("thread.prompt")[0]?.reject(new Error("offline")));
    expect(input().value).toBe("hello\n\nmore");
  });

  // spec: UI-060
  test("`/` completes from the thread's commands, kept fresh by available_commands_update", async () => {
    const { client, flush } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("/secu");
    const menu = await screen.findByTestId("slash-suggestions");
    expect(within(menu).getAllByTestId("slash-suggestion")[0]?.textContent).toContain("/security");
    await userEvent.keyboard("{Tab}");
    expect(input().value).toBe("/security ");
    expect(screen.queryByTestId("slash-suggestions")).toBeNull();

    client.threadEvents(THREAD, {
      kind: "omp",
      frame: {
        type: "available_commands_update",
        commands: [{ name: "zebra", source: "custom", description: "Stripes" }],
      },
    });
    flush();
    await userEvent.clear(input());
    await userEvent.keyboard("/ze");
    const fresh = await screen.findByTestId("slash-suggestions");
    expect(
      within(fresh)
        .getAllByTestId("slash-suggestion")
        .map((item) => item.textContent),
    ).toEqual(["/zebra Stripes"]);
  });

  // spec: UI-998, UI-060
  test("a thread without announced commands asks thread.commands once `/` is typed, not when shown", async () => {
    const { client } = renderThread(<ComposerWidget />, []);
    await userEvent.click(input());
    await userEvent.keyboard("hi");
    // Only the word completion asks while plain words are typed (UI-175).
    expect(client.calls.filter((call) => call.method !== "prompts.predict_word").map((call) => call.method)).toEqual(
      [],
    );

    await userEvent.clear(input());
    await userEvent.keyboard("/");
    const [asked, ...again] = client.callsTo("thread.commands");
    expect([asked?.params, again.length]).toEqual([{ threadId: THREAD }, 0]);
    await act(async () => asked?.resolve({ commands: [{ name: "compost", source: "custom" }] }));
    await userEvent.keyboard("comp");
    const menu = await screen.findByTestId("slash-suggestions");
    expect(
      within(menu)
        .getAllByTestId("slash-suggestion")
        .map((item) => item.textContent),
    ).toEqual(["/compost"]);
    await userEvent.clear(input());
    await userEvent.keyboard("/");
    expect(client.callsTo("thread.commands").length).toBe(1);
  });

  // spec: UI-978
  test("`/` lists omp's TUI-only builtins: the routed ones like any command, the others disabled with the reason", async () => {
    renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("/set");
    const menu = await screen.findByTestId("slash-suggestions");
    expect(
      within(menu)
        .getAllByTestId("slash-suggestion")
        .map((item) => [item.textContent, item.getAttribute("aria-disabled")]),
    ).toEqual([
      ["/settings Needs omp's terminal UI", "true"],
      ["/setup Needs omp's terminal UI; :login logs in to a provider", "true"],
    ]);
    // Tab picks no disabled command.
    await userEvent.keyboard("{Tab}");
    expect(input().value).toBe("/set");

    await userEvent.clear(input());
    await userEvent.keyboard("/log");
    const routed = await screen.findByTestId("slash-suggestions");
    expect(
      within(routed)
        .getAllByTestId("slash-suggestion")
        .map((item) => [item.textContent, item.getAttribute("aria-disabled")]),
    ).toEqual([
      ["/login [<provider>] Log in to a provider (:login)", null],
      ["/logout [<provider>] Log out of a provider (:logout)", null],
    ]);
    await userEvent.keyboard("{Tab}");
    expect(input().value).toBe("/login ");
  });

  // spec: UI-976
  test("a routed command runs omfp's equivalent instead of thread.prompt and clears the draft", async () => {
    const { client, host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    const ran: string[][] = [];
    act(() => {
      host.commands.register({ id: "auth.login", title: "Log in", run: (args) => void ran.push([...args]) });
      host.commands.register({
        id: "thread.compact",
        title: "Compact",
        run: (args) => void ran.push(["compact", ...args]),
      });
    });
    await userEvent.click(input());
    await userEvent.keyboard("/login anthropic{Enter}");
    expect(ran).toEqual([["anthropic"]]);
    expect(input().value).toBe("");
    // `/compact`, its modes included, runs `:compact` with the text as typed (UI-910).
    await userEvent.keyboard("/compact keep the test plan{Enter}");
    expect(ran.at(-1)).toEqual(["compact", "keep the test plan"]);
    // Enter on the open menu would pick the highlighted command: Tab completes it, then Enter sends.
    await userEvent.keyboard("/fork{Tab}{Enter}");
    expect(client.callsTo("threads.fork").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
    expect(client.callsTo("thread.prompt")).toEqual([]);
    // `/model` opens the header's model list: without a header nothing runs it, and the error says so.
    await userEvent.keyboard("/model{Tab}{Enter}");
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["/model: nothing in this window runs it now"]);
    await userEvent.keyboard("/compact soft{Enter}");
    expect(ran.at(-1)).toEqual(["compact", "soft"]);
    expect(client.callsTo("thread.prompt")).toEqual([]);
  });

  // spec: UI-977
  test("a TUI-only builtin is not sent: the reason is an error and the draft stays", async () => {
    const { client, host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("/settings{Enter}");
    expect(client.callsTo("thread.prompt")).toEqual([]);
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual([
      "/settings was not sent: Needs omp's terminal UI",
    ]);
    expect(input().value).toBe("/settings");
  });

  // spec: UI-979
  test("a command omp answers with an error says why and comes back into the draft; a failed message stays quiet", async () => {
    const { client, host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("/dirs{Tab}{Enter}");
    await act(async () => client.callsTo("thread.prompt")[0]?.reject(new Error("Cannot list the directories")));
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["/dirs failed: Cannot list the directories"]);
    expect(input().value).toBe("/dirs");

    await userEvent.clear(input());
    await userEvent.keyboard("hello{Enter}");
    await act(async () => client.callsTo("thread.prompt")[1]?.reject(new Error("offline")));
    expect(host.window.get(ERRORS)).toHaveLength(1);
    expect(input().value).toBe("hello");
  });

  // spec: UI-732
  test("`@` asks find_file fuzzily once typing pauses; Tab inserts the workspace-relative `@path `", async () => {
    const clock = new ManualClock(1_000);
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"), clock);
    await userEvent.click(input());
    await userEvent.keyboard("look at @ma");
    expect(client.callsTo("workspaces.find_file")).toHaveLength(0);
    act(() => clock.advance(FILE_QUERY_DELAY_MS));
    const [found, ...more] = client.callsTo("workspaces.find_file");
    expect([found?.params, more.length]).toEqual([findFile("ma"), 0]);
    await act(async () => found?.resolve(candidates("src/main.rs", "docs/manual.md")));
    const menu = await screen.findByTestId("file-suggestions");
    expect(
      within(menu)
        .getAllByTestId("file-suggestion")
        .map((item) => item.textContent),
    ).toEqual(["@src/main.rs", "@docs/manual.md"]);
    await userEvent.keyboard("{Tab}");
    expect(input().value).toBe("look at @src/main.rs ");
    expect(screen.queryByTestId("file-suggestions")).toBeNull();
    await userEvent.keyboard("please{Enter}");
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: THREAD, text: "look at @src/main.rs please" },
    ]);
  });

  // spec: UI-732
  test("an answer to an older `@` query is dropped; the last one, narrowed, shows meanwhile; Esc closes the menu", async () => {
    const clock = new ManualClock(1_000);
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"), clock);
    await userEvent.click(input());
    await userEvent.keyboard("@s");
    act(() => clock.advance(FILE_QUERY_DELAY_MS));
    await act(async () => client.callsTo("workspaces.find_file")[0]?.resolve(candidates("src/lib.rs", "src/main.rs")));
    await userEvent.keyboard("m");
    act(() => clock.advance(FILE_QUERY_DELAY_MS));
    await userEvent.keyboard("a");
    // The answer to `sm` comes after `sma` was typed: dropped.
    await act(async () => client.callsTo("workspaces.find_file")[1]?.resolve(candidates("stale/sm.rs")));
    const items = () =>
      within(screen.getByTestId("file-suggestions"))
        .getAllByTestId("file-suggestion")
        .map((item) => item.textContent);
    expect(items()).toEqual(["@src/main.rs"]);
    act(() => clock.advance(FILE_QUERY_DELAY_MS));
    expect(client.callsTo("workspaces.find_file").map((call) => call.params)).toEqual([
      findFile("s"),
      findFile("sm"),
      findFile("sma"),
    ]);
    await act(async () => client.callsTo("workspaces.find_file")[2]?.resolve(candidates("src/main.rs", "sma.txt")));
    expect(items()).toEqual(["@src/main.rs", "@sma.txt"]);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByTestId("file-suggestions")).toBeNull();
    expect(input().value).toBe("@sma");
  });

  // spec: UI-732, UI-060
  test("`/` commands keep their menu, `@` inside a word asks for nothing, `@` after a command's space completes files", async () => {
    const clock = new ManualClock(1_000);
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"), clock);
    await userEvent.click(input());
    await userEvent.keyboard("/secu");
    act(() => clock.advance(FILE_QUERY_DELAY_MS));
    expect(await screen.findByTestId("slash-suggestions")).toBeDefined();
    await userEvent.keyboard("{Tab}");
    expect(input().value).toBe("/security ");
    await userEvent.keyboard("mail me@exa");
    act(() => clock.advance(FILE_QUERY_DELAY_MS));
    expect(client.callsTo("workspaces.find_file")).toHaveLength(0);
    await userEvent.keyboard(" @rea");
    act(() => clock.advance(FILE_QUERY_DELAY_MS));
    await act(async () => client.callsTo("workspaces.find_file")[0]?.resolve(candidates("README.md")));
    const menu = await screen.findByTestId("file-suggestions");
    expect(within(menu).getAllByTestId("file-suggestion")[0]?.textContent).toBe("@README.md");
    expect(screen.queryByTestId("slash-suggestions")).toBeNull();
    await userEvent.keyboard("{Tab}");
    expect(input().value).toBe("/security mail me@exa @README.md ");
  });
  // spec: UI-693
  test("active work is named and timed above the composer, with the one Abort action", async () => {
    const clock = new ManualClock(1_000);
    const { client } = renderThread(<ComposerWidget />, activeEditEvents(), clock);
    const activity = screen.getByTestId("composer-activity");
    expect([activity.dataset.kind, within(activity).getByTestId("activity-label").textContent]).toEqual([
      "tool",
      "Saturating the addition",
    ]);
    expect(within(activity).getByTestId("activity-duration").textContent).toBe("0 ms");
    act(() => clock.advance(1_250));
    expect(within(activity).getByTestId("activity-duration").textContent).toBe("1.3 s");
    expect(screen.getAllByTestId("abort").length).toBe(1);
    await userEvent.click(within(activity).getByTestId("abort"));
    expect(client.callsTo("thread.abort").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
  });

  // spec: UI-693
  test("streamed reasoning previews its latest line as plain text above the composer until a tool or settling", () => {
    const update = (assistantMessageEvent: JsonObject): ThreadEvent => ({
      kind: "omp",
      frame: { type: "message_update", messageId: "live", message: { role: "assistant" }, assistantMessageEvent },
    });
    const { client, flush, host } = renderThread(<ComposerWidget />, [
      { kind: "omp", frame: { type: "agent_start" } },
      { kind: "omp", frame: { type: "message_start", messageId: "live", message: { role: "assistant", content: [] } } },
      update({ type: "thinking_start", contentIndex: 0, block: { type: "thinking", thinking: "" } }),
    ]);
    const label = () => screen.getByTestId("activity-label").textContent;
    const kind = () => screen.getByTestId("composer-activity").dataset.kind;
    expect([kind(), label()]).toEqual(["thinking", "Thinking…"]);
    client.threadEvents(THREAD, update({ type: "thinking_delta", contentIndex: 0, delta: "**Assessing Slack" }));
    flush();
    expect(label()).toBe("Assessing Slack");
    client.threadEvents(
      THREAD,
      update({
        type: "thinking_delta",
        contentIndex: 0,
        delta: " unread activity options**\n\nThe user wants `unread`",
      }),
    );
    flush();
    expect(label()).toBe("The user wants unread");
    // The row is no live region: deltas are not announced.
    expect(screen.getByTestId("composer-activity").closest("[aria-live], [role='status'], [role='log']")).toBeNull();
    client.threadEvents(THREAD, {
      kind: "omp",
      frame: { type: "tool_execution_start", toolCallId: "c1", toolName: "read", args: {}, intent: "Reading notes" },
    });
    flush();
    expect([kind(), label()]).toEqual(["tool", "Reading notes"]);
    client.threadEvents(THREAD, { kind: "omp", frame: { type: "session_settled" } });
    flush();
    expect(screen.queryByTestId("composer-activity")).toBeNull();
    expect(host.announcer.getSnapshot().message).not.toContain("Assessing");
  });

  // spec: UI-060
  test("pasted and dropped images become attachments sent with the prompt", async () => {
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    const png = new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], "a.png", { type: "image/png" });
    const text = new File(["hi"], "a.txt", { type: "text/plain" });
    fireEvent.paste(input(), { clipboardData: { files: [png], getData: () => "" } });
    await waitFor(() => expect(screen.getAllByTestId("attachment").length).toBe(1));
    fireEvent.drop(screen.getByTestId("composer"), { dataTransfer: { files: [png, text] } });
    await waitFor(() => expect(screen.getAllByTestId("attachment").length).toBe(2));
    await userEvent.click(screen.getAllByTestId("attachment-remove")[1] as HTMLElement);
    expect(screen.getAllByTestId("attachment").length).toBe(1);
    await userEvent.click(screen.getByTestId("send"));
    expect(client.callsTo("thread.prompt")[0]?.params).toEqual({
      threadId: THREAD,
      text: "",
      images: [{ type: "image", data: "iVBORw==", mimeType: "image/png" }],
    });
  });

  // spec: UI-061, UI-148, UI-172
  test("while busy, sending steers; ctrl+enter, shift+enter and ctrl+q queue a follow-up; the button says which", async () => {
    const { client } = renderThread(<ComposerWidget />, busyEvents());
    const send = screen.getByTestId("send");
    expect([send.dataset.mode, send.textContent]).toEqual(["steer", "Steer nowenter"]);
    const alternative = screen.getByTestId("send-alternative");
    expect([alternative.dataset.mode, alternative.textContent]).toEqual(["follow-up", "Queue follow-upctrl+enter"]);
    await userEvent.click(input());
    await userEvent.keyboard("now{Enter}");
    await userEvent.keyboard("{Control>}");
    expect(screen.getByTestId("send").dataset.mode).toBe("follow-up");
    expect(screen.getByTestId("send").textContent).toBe("Queue follow-upctrl+enter");
    await userEvent.keyboard("{/Control}later{Control>}{Enter}{Enter>3/}{/Control}");
    await userEvent.keyboard("shifted{Shift>}{Enter}{/Shift}");
    await userEvent.keyboard("chord{Control>}q{/Control}");
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: THREAD, text: "now", behavior: "steer" },
      { threadId: THREAD, text: "later", behavior: "followUp" },
      { threadId: THREAD, text: "shifted", behavior: "followUp" },
      { threadId: THREAD, text: "chord", behavior: "followUp" },
    ]);
    expect(screen.getByTestId("send").dataset.mode).toBe("steer");
  });

  // spec: UI-732, UI-061
  test("with the `@` menu open on a busy thread, ctrl+q and ctrl+enter still queue, shift+enter too", async () => {
    const clock = new ManualClock(1_000);
    const { client } = renderThread(<ComposerWidget />, busyEvents(), clock);
    await userEvent.click(input());
    const openMenu = async (typed: string) => {
      await userEvent.keyboard(typed);
      act(() => clock.advance(FILE_QUERY_DELAY_MS));
      const calls = client.callsTo("workspaces.find_file");
      await act(async () => calls[calls.length - 1]?.resolve(candidates("src/main.rs")));
      await screen.findByTestId("file-suggestions");
    };
    await openMenu("see @ma");
    await userEvent.keyboard("{Control>}q{/Control}");
    await openMenu("then @ma");
    await userEvent.keyboard("{Control>}{Enter}{/Control}");
    await openMenu("last @ma");
    await userEvent.keyboard("{Shift>}{Enter}{/Shift}");
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: THREAD, text: "see @ma", behavior: "followUp" },
      { threadId: THREAD, text: "then @ma", behavior: "followUp" },
      { threadId: THREAD, text: "last @ma", behavior: "followUp" },
    ]);
  });

  // spec: UI-172, UI-098
  test("the composer box is in insert mode (gradient border) while its input has the focus", async () => {
    renderThread(<ComposerWidget />, busyEvents());
    const box = () => screen.getByTestId("composer").querySelector(".omfp-compose");
    expect(box()?.getAttribute("data-mode")).toBe("normal");
    await userEvent.click(input());
    expect(box()?.getAttribute("data-mode")).toBe("insert");
    act(() => input().blur());
    expect(box()?.getAttribute("data-mode")).toBe("normal");
  });

  // spec: UI-063, UI-092, UI-095, UI-148
  test("Esc while busy only leaves the composer, the agent keeps running; the Abort button restores the queue", async () => {
    const { client, host } = renderThread(<ChatWidget />, queuedEvents());
    expect(input().getAttribute("placeholder")).toBe("Steer the agent (ctrl+enter queues, ctrl+c interrupts)");
    expect(screen.getByTestId("composer").textContent).toContain("ctrl+c interrupt");
    await userEvent.click(input());
    await userEvent.keyboard("typing");
    await userEvent.keyboard("{Escape}");
    expect(document.activeElement === screen.getByTestId("transcript")).toBe(true);
    expect(client.callsTo("thread.abort")).toHaveLength(0);
    expect(screen.getByTestId("composer").dataset.busy).toBe("true");
    await userEvent.click(screen.getByTestId("abort"));
    const [abort] = client.callsTo("thread.abort");
    expect(abort?.params).toEqual({ threadId: THREAD });
    await act(async () => abort?.resolve({ steering: [], followUp: [{ text: "later please" }] }));
    expect(input().value).toBe("later please\n\ntyping");
    expect(host.announcer.getSnapshot().message).toBe("Turn aborted");
  });

  // spec: UI-368, UI-063
  test("Ctrl-C in normal mode aborts the busy thread and restores its queue; with text selected it copies", async () => {
    const clock = new ManualClock(0);
    const { client, host } = renderThread(<ChatWidget />, queuedEvents(), clock);
    installKeymap(host, clock);
    await userEvent.click(input());
    await userEvent.keyboard("typing{Escape}");
    const transcript = screen.getByTestId("transcript");
    expect(document.activeElement === transcript).toBe(true);
    // Text selected in the page (a queued message's chip; the virtualized transcript draws no rows here).
    const selection = document.getSelection();
    const range = document.createRange();
    range.selectNodeContents(screen.getByTestId("queue-chip"));
    // A range left by an earlier test would keep this one from being added.
    selection?.removeAllRanges();
    selection?.addRange(range);
    expect(selection?.toString()).toContain("later please");
    press("c", { ctrlKey: true });
    expect(client.callsTo("thread.abort")).toHaveLength(0);
    selection?.removeAllRanges();
    press("c", { ctrlKey: true });
    const [abort] = client.callsTo("thread.abort");
    expect(abort?.params).toEqual({ threadId: THREAD });
    await act(async () => abort?.resolve({ steering: [], followUp: [{ text: "later please" }] }));
    expect(input().value).toBe("later please\n\ntyping");
    expect(host.announcer.getSnapshot().message).toBe("Turn aborted");
  });

  // spec: UI-368
  test("Ctrl-C in normal mode does nothing on an idle thread", () => {
    const clock = new ManualClock(0);
    const { client, host } = renderThread(<ChatWidget />, fixtureEvents("text_reply"), clock);
    installKeymap(host, clock);
    act(() => screen.getByTestId("transcript").focus());
    press("c", { ctrlKey: true });
    expect(client.callsTo("thread.abort")).toHaveLength(0);
  });

  // spec: UI-095
  test("Esc while idle leaves the composer for the transcript; `composer.focus` comes back", async () => {
    const { host } = renderThread(<ChatWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("{Escape}");
    expect(document.activeElement === screen.getByTestId("transcript")).toBe(true);
    act(() => void host.commands.run("composer.focus"));
    expect(document.activeElement === input()).toBe(true);
  });

  // spec: UI-095, UI-400
  test("Esc goes to the transcript of the composer's own chat, not the first one in the page", async () => {
    // Another transcript earlier in the page: a kept tab's or a second chat's.
    const other = document.createElement("div");
    other.setAttribute("data-omfp-scope", "transcript");
    other.setAttribute("data-testid", "other-transcript");
    other.tabIndex = 0;
    document.body.prepend(other);
    try {
      renderThread(<ChatWidget />, fixtureEvents("text_reply"));
      await userEvent.click(input());
      await userEvent.keyboard("{Escape}");
      expect(document.activeElement?.getAttribute("data-testid")).toBe("transcript");
    } finally {
      other.remove();
    }
  });

  // spec: UI-064
  test("drafts are kept per thread across thread switches", async () => {
    const { client, flush, host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("draft for one");
    act(() => host.window.set(SELECTED_THREAD, "02T"));
    client.thread("02T", fixtureEvents("text_reply"));
    flush();
    expect(input().value).toBe("");
    await userEvent.click(input());
    await userEvent.keyboard("other");
    act(() => host.window.set(SELECTED_THREAD, THREAD));
    client.thread(THREAD, fixtureEvents("text_reply"));
    flush();
    expect(input().value).toBe("draft for one");
    expect(host.window.get(draftKey("02T")).text).toBe("other");
  });

  // spec: UI-063, UI-095
  test("Ctrl-C clears the draft (text and images) without aborting an idle thread; u restores it", async () => {
    const clock = new ManualClock(0);
    const { client, host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"), clock);
    installKeymap(host, clock);
    const image = { type: "image" as const, data: "AA==", mimeType: "image/png" };
    await userEvent.click(input());
    await userEvent.keyboard("draft one");
    act(() => host.window.set(draftKey(THREAD), (draft) => ({ ...draft, images: [image] })));
    press("c", { ctrlKey: true });
    expect(input().value).toBe("");
    expect(host.window.get(draftKey(THREAD)).images).toEqual([]);
    expect(screen.queryByTestId("interrupt-hint")).toBeNull();
    expect(screen.getByTestId("restore-draft-hint")).toBeDefined();
    // Empty draft on an idle thread: nothing to abort.
    act(() => clock.advance(2000));
    expect(screen.getByTestId("restore-draft-hint")).toBeDefined();
    press("c", { ctrlKey: true });
    expect(client.callsTo("thread.abort")).toHaveLength(0);
    await userEvent.keyboard("{Escape}");
    press("u");
    expect(input().value).toBe("draft one");
    expect(host.window.get(draftKey(THREAD)).images).toEqual([image]);
    expect(screen.queryByTestId("restore-draft-hint")).toBeNull();
  });

  // spec: UI-063, UI-095, UI-148
  test("busy: Ctrl-C clears with a hint, a second press within 1 s aborts without restoring the queue", async () => {
    const clock = new ManualClock(0);
    const { client, host } = renderThread(<ComposerWidget />, queuedEvents(), clock);
    installKeymap(host, clock);
    await userEvent.click(input());
    await userEvent.keyboard("typing");
    press("c", { ctrlKey: true });
    expect(input().value).toBe("");
    expect(screen.getByTestId("interrupt-hint").textContent).toBe("ctrl+c again to abort");
    expect(client.callsTo("thread.abort")).toHaveLength(0);
    act(() => clock.advance(500));
    expect(screen.getByTestId("interrupt-hint")).toBeDefined();
    press("c", { ctrlKey: true });
    const [abort] = client.callsTo("thread.abort");
    expect(abort?.params).toEqual({ threadId: THREAD });
    await act(async () => abort?.resolve({ steering: [], followUp: [{ text: "later please" }] }));
    expect(input().value).toBe("");
    expect(screen.queryByTestId("interrupt-hint")).toBeNull();
    expect(host.announcer.getSnapshot().message).toBe("Turn aborted");
    // After the window the hint goes away and a press clears again instead of aborting.
    await userEvent.keyboard("again");
    press("c", { ctrlKey: true });
    expect(screen.getByTestId("interrupt-hint")).toBeDefined();
    act(() => clock.advance(1000));
    expect(screen.getByTestId("interrupt-hint")).toBeDefined();
    act(() => clock.advance(1));
    expect(screen.queryByTestId("interrupt-hint")).toBeNull();
    await userEvent.keyboard("more");
    press("c", { ctrlKey: true });
    expect(input().value).toBe("");
    expect(client.callsTo("thread.abort")).toHaveLength(1);
    // `u` restores the last cleared draft.
    act(() => input().blur());
    press("u");
    expect(input().value).toBe("more");
  });

  // spec: UI-063
  test("typing after a Ctrl-C ends the sequence: the next press within 1 s clears instead of aborting", async () => {
    const clock = new ManualClock(0);
    const { client, host } = renderThread(<ComposerWidget />, queuedEvents(), clock);
    installKeymap(host, clock);
    await userEvent.click(input());
    await userEvent.keyboard("first");
    press("c", { ctrlKey: true });
    expect(screen.getByTestId("interrupt-hint")).toBeDefined();
    act(() => clock.advance(300));
    await userEvent.keyboard("second");
    expect(screen.queryByTestId("interrupt-hint")).toBeNull();
    press("c", { ctrlKey: true });
    expect(input().value).toBe("");
    expect(screen.getByTestId("interrupt-hint")).toBeDefined();
    expect(client.callsTo("thread.abort")).toHaveLength(0);
    press("c", { ctrlKey: true });
    expect(client.callsTo("thread.abort")).toHaveLength(1);
  });

  // spec: UI-063
  test("Ctrl-C on an empty draft of a busy thread aborts at once", async () => {
    const clock = new ManualClock(0);
    const { client, host } = renderThread(<ComposerWidget />, queuedEvents(), clock);
    installKeymap(host, clock);
    await userEvent.click(input());
    press("c", { ctrlKey: true });
    expect(client.callsTo("thread.abort").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
  });
});

describe("queue chips", () => {
  // spec: UI-062
  test("chips come from queue_update only; remove waits for the daemon's answer", async () => {
    const { client, flush } = renderThread(<ComposerWidget />, queuedEvents());
    const chip = screen.getByTestId("queue-chip");
    expect([chip.dataset.queue, chip.getAttribute("aria-busy")]).toEqual(["follow-up", "false"]);
    await userEvent.click(within(chip).getByTestId("queue-remove"));
    const [remove] = client.callsTo("thread.queue.remove");
    expect(remove?.params).toEqual({ threadId: THREAD, message: "later please", queue: "followUp" });
    expect(screen.getByTestId("queue-chip").getAttribute("aria-busy")).toBe("true");
    await act(async () => remove?.resolve({ removed: true }));
    // Still shown until the next queue_update says otherwise.
    expect(screen.getByTestId("queue-chip").getAttribute("aria-busy")).toBe("false");
    client.threadEvents(THREAD, ...fixtureEvents("follow_up_then_abort").slice(12, 13));
    flush();
    expect(screen.queryByTestId("queue-chip")).toBeNull();
  });

  // spec: UI-062, UI-092
  test("a promote the daemon could not apply is announced and leaves the chip", async () => {
    const { client, host } = renderThread(<ComposerWidget />, queuedEvents());
    await userEvent.click(screen.getByTestId("queue-promote"));
    await act(async () => client.callsTo("thread.queue.promote")[0]?.resolve({ promoted: false }));
    expect(host.announcer.getSnapshot().message).toBe("\u201clater please\u201d is no longer queued");
    expect(screen.getByTestId("queue-chip").getAttribute("aria-busy")).toBe("false");
  });
});

describe("the composer's bar by pointer and width", () => {
  let pointer: PointerMedia | undefined;

  afterEach(() => {
    pointer?.restore();
    pointer = undefined;
  });

  // spec: UI-421
  test("Enter makes a new line and the send buttons send; ctrl+enter from a hardware keyboard still queues", async () => {
    pointer = new PointerMedia(true);
    const { client } = renderThread(<ComposerWidget />, busyEvents());
    expect(input().getAttribute("enterkeyhint")).toBe("enter");
    await userEvent.click(input());
    await userEvent.keyboard("first{Enter}second");
    expect(input().value).toBe("first\nsecond");
    expect(client.callsTo("thread.prompt")).toHaveLength(0);
    await userEvent.click(screen.getByTestId("send"));
    await userEvent.click(input());
    await userEvent.keyboard("later");
    await userEvent.click(screen.getByTestId("send-alternative"));
    await userEvent.click(input());
    await userEvent.keyboard("keyed{Control>}{Enter}{/Control}");
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: THREAD, text: "first\nsecond", behavior: "steer" },
      { threadId: THREAD, text: "later", behavior: "followUp" },
      { threadId: THREAD, text: "keyed", behavior: "followUp" },
    ]);
  });

  // spec: UI-060, UI-421
  test("with a mouse Enter still sends, and the keyboard's Enter key reads send", async () => {
    pointer = new PointerMedia(false);
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    expect(input().getAttribute("enterkeyhint")).toBe("send");
    await userEvent.click(input());
    await userEvent.keyboard("hello{Enter}");
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([{ threadId: THREAD, text: "hello" }]);
  });

  // spec: UI-422
  test("the composer takes prose: autocorrect, capitals, the spell check and the keyboard's suggestions are on, touch or not", () => {
    for (const coarse of [false, true]) {
      pointer = new PointerMedia(coarse);
      renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
      // Not React Aria's `autocomplete="off"`: Chrome on Android hands it to the keyboard as "no suggestions", which
      // also turns off its autocorrect (#352).
      expect(
        ["autocorrect", "autocapitalize", "spellcheck", "autocomplete"].map((name) => input().getAttribute(name)),
      ).toEqual(["on", "sentences", "true", "on"]);
      cleanup();
      pointer.restore();
    }
  });

  // spec: UI-423
  test("one row without key hints: Abort as an icon, short send labels; a mouse brings the desktop bar back", () => {
    pointer = new PointerMedia(true);
    renderThread(<ComposerWidget />, queuedEvents());
    const composer = screen.getByTestId("composer");
    const send = screen.getByTestId("send");
    const alternative = screen.getByTestId("send-alternative");
    const abort = screen.getByTestId("abort");
    expect(composer.dataset.touch).toBe("true");
    expect(composer.querySelectorAll(".omfp-compose__hint, .omfp-kbd").length).toBe(0);
    expect(input().placeholder).toBe("Steer the agent, or queue a follow-up");
    expect([abort.getAttribute("aria-label"), abort.textContent]).toEqual(["Abort", ""]);
    expect([send.textContent, alternative.textContent]).toEqual(["Steer", "Queue"]);
    act(() => pointer?.set(false));
    expect(composer.dataset.touch).toBeUndefined();
    expect(composer.querySelectorAll(".omfp-compose__hint").length).toBe(3);
    expect(input().placeholder).toBe("Steer the agent (ctrl+enter queues, ctrl+c interrupts)");
    expect([screen.getByTestId("abort").textContent, screen.getByTestId("send").textContent]).toEqual([
      "Abort",
      "Steer nowenter",
    ]);
  });

  // spec: UI-423, UI-421
  test("on a phone with a keyboard the bar shows no keys either, and Enter still sends", async () => {
    pointer = new PointerMedia(false);
    const { client, host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    act(() => host.window.set(FRAME_LAYOUT, "mobile"));
    expect(screen.getByTestId("composer").querySelectorAll(".omfp-compose__hint, .omfp-kbd").length).toBe(0);
    await userEvent.click(input());
    await userEvent.keyboard("hello{Enter}");
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([{ threadId: THREAD, text: "hello" }]);
  });

  /** The composer bar's two groups: the hints' text, and the actions' test ids in order. */
  function bar(): { hints: string[]; actions: string[] } {
    const composer = screen.getByTestId("composer");
    const hints = composer.querySelectorAll(".omfp-compose__bar > .omfp-compose__hints > .omfp-compose__hint");
    const actions = composer.querySelectorAll(".omfp-compose__bar > .omfp-compose__actions [data-testid]");
    return {
      hints: [...hints].map((hint) => hint.textContent ?? ""),
      actions: [...actions].map((action) => action.getAttribute("data-testid") ?? ""),
    };
  }

  // spec: UI-480
  test("the hints at the bar's start, the actions in one group at its end, the primary last, idle and busy", () => {
    pointer = new PointerMedia(false);
    renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    const idle = bar();
    expect(idle.hints).toEqual(["alt+enter newline", "esc view"]);
    expect(idle.actions.filter((id) => id !== "live-mic")).toEqual(["send"]);
    cleanup();
    renderThread(<ComposerWidget />, queuedEvents());
    const busy = bar();
    // The ctrl+c hint remains; the live activity row owns the one Abort action.
    expect(busy.hints).toEqual(["ctrl+c interrupt", "alt+enter newline", "esc view"]);
    expect(busy.actions.filter((id) => id !== "live-mic")).toEqual(["send-alternative", "send"]);
    expect(busy.actions.indexOf("live-mic")).toBe(idle.actions.indexOf("live-mic"));
  });

  // spec: UI-481
  test("narrower than 34rem the composer is tight: Abort as an icon, short send labels, the key chips kept", () => {
    pointer = new PointerMedia(false);
    const restore = installResizeObserver();
    try {
      renderThread(<ComposerWidget />, queuedEvents());
      const composer = screen.getByTestId("composer");
      resize(composer, 400);
      expect(composer.dataset.tight).toBe("true");
      const abort = screen.getByTestId("abort");
      expect([abort.getAttribute("aria-label"), abort.textContent]).toEqual(["Abort", ""]);
      expect([screen.getByTestId("send").textContent, screen.getByTestId("send-alternative").textContent]).toEqual([
        "Steerenter",
        "Queuectrl+enter",
      ]);
      expect(bar().hints.length).toBe(3);
      resize(composer, 800);
      expect(composer.dataset.tight).toBeUndefined();
      expect([screen.getByTestId("abort").textContent, screen.getByTestId("send").textContent]).toEqual([
        "Abort",
        "Steer nowenter",
      ]);
    } finally {
      restore();
    }
  });
});

describe("focus requests", () => {
  // spec: UI-092, UI-286
  test("the composer of the thread named by FOCUS_COMPOSER takes the focus and clears the request", () => {
    const { host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    act(() => host.window.set(FOCUS_COMPOSER, "other"));
    expect(document.activeElement?.getAttribute("data-testid")).not.toBe("composer-input");
    act(() => host.window.set(FOCUS_COMPOSER, THREAD));
    expect(document.activeElement?.getAttribute("data-testid")).toBe("composer-input");
    expect(host.window.get(FOCUS_COMPOSER)).toBeUndefined();
  });
});

const lines = (count: number) => Array.from({ length: count }, (_, index) => `line ${index + 1}`).join("\n");

/** Pastes `text` at the caret; `false` when the composer took it over (the default action was prevented). */
function paste(text: string): boolean {
  let native = true;
  act(() => {
    native = fireEvent.paste(input(), { clipboardData: { files: [], getData: () => text } });
  });
  return native;
}

const chipLabels = () => screen.queryAllByTestId("paste-toggle").map((chip) => chip.textContent);

describe("large pastes", () => {
  // spec: UI-357, UI-360
  test("a paste above the threshold becomes a numbered chip and a token; sending expands it in place", async () => {
    const { client, host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("before  after");
    input().setSelectionRange(7, 7);
    expect(paste(lines(120))).toBe(false);
    expect(input().value).toBe("before [Paste #1, +120 lines] after");
    expect(input().selectionStart).toBe("before [Paste #1, +120 lines]".length);
    expect(chipLabels()).toEqual(["Pasted text #1 · 120 lines"]);
    expect(within(screen.getByRole("list", { name: "Pasted texts" })).getAllByTestId("paste-chip").length).toBe(1);
    expect(host.announcer.getSnapshot().message).toBe("Pasted 120 lines as pasted text #1; ctrl+shift+v pastes it raw");
    input().setSelectionRange(input().value.length, input().value.length);
    expect(paste("y".repeat(1001))).toBe(false);
    expect(chipLabels()).toEqual(["Pasted text #1 · 120 lines", "Pasted text #2 · 1001 chars"]);
    await userEvent.click(screen.getByTestId("send"));
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: THREAD, text: `before ${lines(120)} after${"y".repeat(1001)}` },
    ]);
    expect(chipLabels()).toEqual([]);
  });

  // spec: UI-359
  test("small pastes and Ctrl+Shift+V pastes are left to the text area", async () => {
    renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    expect(paste(lines(10))).toBe(true);
    act(() => {
      fireEvent.keyDown(input(), { key: "V", ctrlKey: true, shiftKey: true });
    });
    expect(paste(lines(50))).toBe(true);
    // The raw keys count for the next paste only.
    expect(paste(lines(50))).toBe(false);
    expect(chipLabels()).toEqual(["Pasted text #1 · 50 lines"]);
  });

  // spec: UI-358, UI-092
  test("the chip expands to its editable text; × removes the paste and its token", async () => {
    const { host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    paste(lines(12));
    const toggle = screen.getByTestId("paste-toggle");
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await userEvent.click(toggle);
    expect(screen.getByTestId("paste-toggle").getAttribute("aria-expanded")).toBe("true");
    const area = screen.getByTestId("paste-text") as HTMLTextAreaElement;
    expect(area.value).toBe(lines(12));
    expect(screen.getByLabelText("Text of pasted text #1").getAttribute("data-testid")).toBe("paste-text");
    await userEvent.clear(area);
    await userEvent.type(area, "short");
    expect(input().value).toBe("[Paste #1, 5 chars]");
    expect(chipLabels()).toEqual(["Pasted text #1 · 5 chars"]);
    await userEvent.click(screen.getByRole("button", { name: "Remove pasted text #1" }));
    expect(input().value).toBe("");
    expect(chipLabels()).toEqual([]);
    expect(host.window.get(draftKey(THREAD)).pastes).toEqual([]);
    expect(host.announcer.getSnapshot().message).toBe("Removed pasted text #1");
  });

  // spec: UI-358
  test("Backspace on a token removes the whole paste", async () => {
    renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("see ");
    paste(lines(11));
    await userEvent.keyboard("{Backspace}");
    expect(input().value).toBe("see ");
    expect(chipLabels()).toEqual([]);
  });

  // spec: UI-360, UI-064, UI-063
  test("pastes stay in the thread's draft: thread switches, Ctrl-C then u, and a failed send", async () => {
    const clock = new ManualClock(0);
    const { client, flush, host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"), clock);
    installKeymap(host, clock);
    await userEvent.click(input());
    paste(lines(11));
    act(() => host.window.set(SELECTED_THREAD, "02T"));
    client.thread("02T", fixtureEvents("text_reply"));
    flush();
    expect(chipLabels()).toEqual([]);
    act(() => host.window.set(SELECTED_THREAD, THREAD));
    client.thread(THREAD, fixtureEvents("text_reply"));
    flush();
    expect(input().value).toBe("[Paste #1, +11 lines]");
    expect(chipLabels()).toEqual(["Pasted text #1 · 11 lines"]);

    await userEvent.click(input());
    press("c", { ctrlKey: true });
    expect(chipLabels()).toEqual([]);
    act(() => input().blur());
    press("u");
    expect(chipLabels()).toEqual(["Pasted text #1 · 11 lines"]);

    await userEvent.click(screen.getByTestId("send"));
    expect(client.callsTo("thread.prompt")[0]?.params).toEqual({ threadId: THREAD, text: lines(11) });
    await act(async () => client.callsTo("thread.prompt")[0]?.reject(new Error("offline")));
    expect(input().value).toBe("[Paste #1, +11 lines]");
    expect(chipLabels()).toEqual(["Pasted text #1 · 11 lines"]);
  });
});

describe("prompt history", () => {
  const entry = (text: string) => ({ text, threadId: THREAD, workspaceId: "w", at: 1 });

  // spec: UI-361, UI-362, UI-363
  test("Up on the first line recalls older prompts, Down goes back and restores the draft; typing ends the walk", async () => {
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("half typed");
    press("ArrowUp");
    const asked = client.callsTo("prompts.history");
    expect(asked.length).toBe(1);
    expect(asked[0]?.params).toMatchObject({ threadId: THREAD, limit: 100 });
    await act(async () => asked[0]?.resolve({ prompts: [entry("second"), entry("first\nline two")] }));
    expect(input().value).toBe("second");
    press("ArrowUp");
    expect(input().value).toBe("first\nline two");
    // The caret is on the recalled text's last line: Up moves it, the walk waits.
    press("ArrowUp");
    expect(input().value).toBe("first\nline two");
    press("ArrowDown");
    expect(input().value).toBe("second");
    press("ArrowDown");
    expect(input().value).toBe("half typed");
    expect(client.callsTo("prompts.history").length).toBe(1);

    press("ArrowUp");
    await act(async () => client.callsTo("prompts.history")[1]?.resolve({ prompts: [entry("second")] }));
    expect(input().value).toBe("second");
    await userEvent.keyboard("!");
    press("ArrowDown");
    expect(input().value).toBe("second!");
    // Modifiers leave the arrows to the keymap and the field.
    press("ArrowUp", { shiftKey: true });
    expect(client.callsTo("prompts.history").length).toBe(2);
  });
});

describe("readline keys", () => {
  /** The field's text with `|` at the caret, `[` `]` around a selection. */
  const shown = () => {
    const { value, selectionStart: start, selectionEnd: end } = input();
    return start === end
      ? `${value.slice(0, start)}|${value.slice(start)}`
      : `${value.slice(0, start)}[${value.slice(start, end)}]${value.slice(end)}`;
  };
  const caret = (at: number) => act(() => input().setSelectionRange(at, at));
  const ctrl = (key: string) => press(key, { ctrlKey: true });
  const alt = (key: string) => press(key, { altKey: true });

  async function typed(text: string) {
    const clock = new ManualClock(0);
    const rendered = renderThread(<ComposerWidget />, fixtureEvents("text_reply"), clock);
    installKeymap(rendered.host, clock);
    await userEvent.click(input());
    await userEvent.keyboard(text);
    return rendered;
  }

  // spec: UI-760
  test("each key moves the caret or edits the draft, in the thread's composer", async () => {
    const { host } = await typed("hello big world{Alt>}{Enter}{/Alt}second line");
    caret(8);
    ctrl("a");
    expect(shown()).toBe("|hello big world\nsecond line");
    ctrl("e");
    expect(shown()).toBe("hello big world|\nsecond line");
    ctrl("b");
    expect(input().selectionStart).toBe("hello big world".length - 1);
    ctrl("f");
    expect(shown()).toBe("hello big world|\nsecond line");
    alt("b");
    expect(shown()).toBe("hello big |world\nsecond line");
    alt("f");
    expect(shown()).toBe("hello big world|\nsecond line");
    ctrl("n");
    expect(shown()).toBe("hello big world\nsecond line|");
    ctrl("p");
    // The column stays: 11 characters into the first line.
    expect(shown()).toBe("hello big w|orld\nsecond line");
    caret(6);
    ctrl("d");
    expect(shown()).toBe("hello |ig world\nsecond line");
    ctrl("h");
    expect(shown()).toBe("hello|ig world\nsecond line");
    ctrl("t");
    expect(shown()).toBe("hellio|g world\nsecond line");
    // Every edit is the draft's (UI-064).
    expect(host.window.get(draftKey(THREAD)).text).toBe("helliog world\nsecond line");
    ctrl("A");
    expect(shown()).toBe("|helliog world\nsecond line");
    press("A", { ctrlKey: true, shiftKey: true });
    expect(shown()).toBe("[helliog world\nsecond line]");
  });

  // spec: UI-760
  test("kills go to the kill buffer and Ctrl-y yanks them; a selection is what goes or is replaced", async () => {
    const { host } = await typed("alpha beta gamma");
    alt("Backspace");
    expect(shown()).toBe("alpha beta |");
    ctrl("a");
    ctrl("y");
    expect(shown()).toBe("gamma|alpha beta ");
    alt("d");
    expect(shown()).toBe("gamma| beta ");
    ctrl("e");
    ctrl("y");
    expect(shown()).toBe("gamma beta alpha|");
    caret(5);
    ctrl("k");
    expect(shown()).toBe("gamma|");
    ctrl("u");
    expect(shown()).toBe("|");
    ctrl("y");
    expect(shown()).toBe("gamma|");
    act(() => input().setSelectionRange(1, 4));
    ctrl("y");
    expect(shown()).toBe("ggamma|a");
    act(() => input().setSelectionRange(0, 3));
    ctrl("w");
    expect(shown()).toBe("|mmaa");
    ctrl("y");
    expect(shown()).toBe("gga|mmaa");
    expect(host.window.get(draftKey(THREAD)).text).toBe("ggammaa");
  });

  // spec: UI-760, UI-358
  test("editing keys keep paste chips: a kill beside a token leaves it, a kill over it removes the paste", async () => {
    await typed("see ");
    paste(lines(11));
    await userEvent.keyboard(" now");
    alt("Backspace");
    expect(input().value).toBe("see [Paste #1, +11 lines] ");
    expect(chipLabels()).toEqual(["Pasted text #1 · 11 lines"]);
    // A killed token takes its text along: the yank brings the text back, never an orphaned token.
    ctrl("u");
    expect(input().value).toBe("");
    expect(chipLabels()).toEqual([]);
    ctrl("y");
    expect(input().value).toBe(`see ${lines(11)} `);
  });

  // spec: UI-760
  test("with two composers mounted the keys edit the focused one", async () => {
    const clock = new ManualClock(0);
    const { host } = renderThread(
      <>
        <ComposerWidget />
        <ComposerWidget />
      </>,
      fixtureEvents("text_reply"),
      clock,
    );
    installKeymap(host, clock);
    const first = screen.getAllByTestId("composer-input")[0] as HTMLTextAreaElement;
    await userEvent.click(first);
    await userEvent.keyboard("abc");
    ctrl("a");
    expect(first.selectionStart).toBe(0);
    ctrl("d");
    expect(first.value).toBe("bc");
  });

  // spec: UI-760, UI-361, UI-362
  test("Ctrl-p on the first line and Ctrl-n on the last walk the prompt history like Up and Down", async () => {
    const { client } = await typed("draft{Alt>}{Enter}{/Alt}two");
    ctrl("p");
    expect(shown()).toBe("dra|ft\ntwo");
    expect(client.callsTo("prompts.history")).toHaveLength(0);
    ctrl("p");
    expect(client.callsTo("prompts.history")).toHaveLength(1);
    await act(async () =>
      client.callsTo("prompts.history")[0]?.resolve({
        prompts: [{ text: "older", threadId: THREAD, workspaceId: "w", at: 1 }],
      }),
    );
    expect(input().value).toBe("older");
    ctrl("n");
    expect(input().value).toBe("draft\ntwo");
  });

  // spec: UI-760, UI-060
  test("while the `/` suggestions are open Ctrl-p does not walk the history", async () => {
    const { client } = await typed("/secu");
    await screen.findByTestId("slash-suggestions");
    ctrl("p");
    expect(client.callsTo("prompts.history")).toHaveLength(0);
    expect(input().value).toBe("/secu");
  });

  // spec: UI-760, UI-095
  test("insert-mode Ctrl-c and Esc keep their commands; the keys do nothing outside the composer", async () => {
    await typed("abc");
    ctrl("c");
    expect(input().value).toBe("");
    await userEvent.keyboard("abc");
    press("Escape");
    expect(document.activeElement === input()).toBe(false);
    ctrl("a");
    expect(input().value).toBe("abc");
  });
});

describe("word completion", () => {
  let pointer: PointerMedia | undefined;

  afterEach(() => {
    pointer?.restore();
    pointer = undefined;
  });

  const ghost = () => screen.queryByTestId("composer-ghost")?.textContent ?? null;
  const asked = (client: FakeClient) => client.callsTo("prompts.predict_word");
  const feedback = (client: FakeClient) => client.callsTo("prompts.predict_word_feedback").map((call) => call.params);

  /** The thread's composer with `text` typed and the answer `suffix` for it shown. */
  async function showing(text: string, suffix: string) {
    const rendered = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard(text);
    const last = asked(rendered.client).at(-1);
    expect(last?.params).toEqual({ text, cursor: text.length });
    await act(async () => last?.resolve({ suffix }));
    expect(ghost()).toBe(suffix);
    return rendered;
  }

  // spec: UI-175
  test("typing at a word's end asks for the rest of the word and shows it dimmed after the caret, not in the value", async () => {
    const { client } = await showing("say hel", "lo");
    // Asked after each edit ending a word; not after the space.
    expect(asked(client).map((call) => call.params)).toEqual(
      ["s", "sa", "say", "say h", "say he", "say hel"].map((text) => ({ text, cursor: text.length })),
    );
    expect(input().value).toBe("say hel");
    const mirror = screen.getByTestId("composer-ghost-text");
    expect(mirror.getAttribute("aria-hidden")).toBe("true");
    expect(mirror.textContent).toBe("say hello\u200b");
    expect(input().getAttribute("aria-keyshortcuts")?.split(" ")).toContain("ArrowRight");
  });

  // spec: UI-179
  test("the suffix hangs from a zero-width anchor at the caret, out of the text's flow", async () => {
    await showing("say hel", "lo");
    const anchor = screen.getByTestId("composer-ghost").parentElement;
    expect(anchor?.className).toBe("omfp-compose__ghost-anchor");
    expect(anchor?.textContent).toBe("lo");
    // The text before the anchor is exactly the text before the caret, so it wraps as the text area's does.
    expect(anchor?.previousSibling?.textContent).toBe("say hel");
    expect(anchor?.parentElement?.getAttribute("data-testid")).toBe("composer-ghost-text");
  });

  // spec: UI-175
  test("typing at a word's end with text after it on the same line asks nothing", async () => {
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("hel world");
    const before = asked(client).length;
    act(() => input().setSelectionRange(3, 3));
    await userEvent.keyboard("p");
    expect(input().value).toBe("help world");
    expect(asked(client)).toHaveLength(before);
    expect(ghost()).toBeNull();
  });

  // spec: UI-175
  test("an answer for an older text is dropped; a failed one shows nothing and no error", async () => {
    const { client, host } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("hel");
    await act(async () => asked(client)[1]?.resolve({ suffix: "llo" }));
    expect(ghost()).toBeNull();
    await act(async () => asked(client)[2]?.reject(new Error("omp not started")));
    expect(ghost()).toBeNull();
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual([]);
  });

  // spec: UI-175, UI-177
  test("typing the ghost's next character shortens it at once and asks again; the rest typed reports nothing", async () => {
    const { client } = await showing("hel", "lo");
    await userEvent.keyboard("l");
    expect(ghost()).toBe("o");
    expect(asked(client).at(-1)?.params).toEqual({ text: "hell", cursor: 4 });
    await act(async () => asked(client).at(-1)?.resolve({ suffix: "o" }));
    expect(ghost()).toBe("o");
    await userEvent.keyboard("o");
    expect(ghost()).toBeNull();
    expect(feedback(client)).toEqual([]);
  });

  // spec: UI-176, UI-177
  test("Tab inserts the suffix and a space, the caret after them, and reports it accepted", async () => {
    const { client } = await showing("say hel", "lo");
    await userEvent.keyboard("{Tab}");
    expect(input().value).toBe("say hello ");
    expect([input().selectionStart, input().selectionEnd]).toEqual([10, 10]);
    expect(ghost()).toBeNull();
    expect(feedback(client)).toEqual([{ text: "say hel", cursor: 7, suggestion: "lo", accepted: true }]);
    expect(document.activeElement === input()).toBe(true);
  });

  // spec: UI-176, UI-177
  test("an accept inserts through the text area's own edit, so it can be undone, and is reported accepted once", async () => {
    const { client, host } = await showing("say hel", "lo");
    const original = document.execCommand;
    const commands: Array<[string, string | undefined]> = [];
    // As a browser does: the text replaces the selection, the caret after it, and the field fires `input`.
    document.execCommand = (command: string, _ui?: boolean, value?: string) => {
      commands.push([command, value]);
      const field = input();
      const { selectionStart, selectionEnd, value: text } = field;
      const inserted = value ?? "";
      fireEvent.input(field, {
        target: { value: `${text.slice(0, selectionStart)}${inserted}${text.slice(selectionEnd)}` },
      });
      field.setSelectionRange(selectionStart + inserted.length, selectionStart + inserted.length);
      return true;
    };
    try {
      await userEvent.keyboard("{Tab}");
      expect(commands).toEqual([["insertText", "lo "]]);
      expect(input().value).toBe("say hello ");
      expect([input().selectionStart, input().selectionEnd]).toEqual([10, 10]);
      expect(host.window.get(draftKey(THREAD)).text).toBe("say hello ");
      expect(ghost()).toBeNull();
      expect(feedback(client)).toEqual([{ text: "say hel", cursor: 7, suggestion: "lo", accepted: true }]);

      // The insertion's own `input` neither refuses the accepted ghost nor asks for the completed word. (Typed as a
      // browser's text input does, through `insertText`.)
      act(() => void document.execCommand("insertText", false, "wor"));
      await act(async () => asked(client).at(-1)?.resolve({ suffix: "ld" }));
      expect(ghost()).toBe("ld");
      const before = asked(client).length;
      press("ArrowRight");
      expect(commands.at(-1)).toEqual(["insertText", "ld"]);
      expect(input().value).toBe("say hello world");
      expect(asked(client)).toHaveLength(before);
      expect(ghost()).toBeNull();
      expect(feedback(client)).toEqual([
        { text: "say hel", cursor: 7, suggestion: "lo", accepted: true },
        { text: "say hello wor", cursor: 13, suggestion: "ld", accepted: true },
      ]);
    } finally {
      document.execCommand = original;
    }
  });

  // spec: UI-176, UI-177
  test("ArrowRight inserts the suffix alone, the caret after it, and reports it accepted", async () => {
    const { client } = await showing("say hel", "lo");
    await userEvent.keyboard("{ArrowRight}");
    expect(input().value).toBe("say hello");
    expect([input().selectionStart, input().selectionEnd]).toEqual([9, 9]);
    expect(ghost()).toBeNull();
    expect(feedback(client)).toEqual([{ text: "say hel", cursor: 7, suggestion: "lo", accepted: true }]);
  });

  // spec: UI-177
  test("a ghost typed past is reported refused once", async () => {
    const { client } = await showing("hel", "lo");
    await userEvent.keyboard("p");
    expect(ghost()).toBeNull();
    await userEvent.keyboard("x");
    expect(feedback(client)).toEqual([{ text: "hel", cursor: 3, suggestion: "lo", accepted: false }]);
  });

  // spec: UI-177
  test("a deletion or a caret move ends the ghost, reported refused", async () => {
    const { client } = await showing("hel", "lo");
    await userEvent.keyboard("{Backspace}");
    expect(ghost()).toBeNull();
    expect(feedback(client)).toEqual([{ text: "hel", cursor: 3, suggestion: "lo", accepted: false }]);

    await act(async () => asked(client).at(-1)?.resolve({ suffix: "llo" }));
    expect(ghost()).toBe("llo");
    await userEvent.keyboard("{ArrowLeft}");
    expect(ghost()).toBeNull();
    expect(input().value).toBe("he");
    expect(feedback(client).at(-1)).toEqual({ text: "he", cursor: 2, suggestion: "llo", accepted: false });
    expect(feedback(client)).toHaveLength(2);
  });

  // spec: UI-177
  test("blur, send and a prompt-history recall end the ghost without feedback", async () => {
    const { client } = await showing("hel", "lo");
    act(() => input().blur());
    expect(ghost()).toBeNull();

    act(() => input().focus());
    await userEvent.keyboard("p");
    await act(async () => asked(client).at(-1)?.resolve({ suffix: "er" }));
    expect(ghost()).toBe("er");
    press("ArrowUp");
    expect(client.callsTo("prompts.history")).toHaveLength(1);
    expect(ghost()).toBeNull();

    await userEvent.keyboard("s");
    await act(async () => asked(client).at(-1)?.resolve({ suffix: "o" }));
    expect(ghost()).toBe("o");
    await userEvent.keyboard("{Enter}");
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([{ threadId: THREAD, text: "helps" }]);
    expect(ghost()).toBeNull();
    expect(feedback(client)).toEqual([]);
  });

  // spec: UI-176, UI-060
  test("an open `/` list keeps Tab and asks nothing", async () => {
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("/secu");
    await screen.findByTestId("slash-suggestions");
    expect(asked(client)).toHaveLength(0);
    await userEvent.keyboard("{Tab}");
    expect(input().value).toBe("/security ");
    expect(feedback(client)).toEqual([]);
  });

  // spec: UI-178
  test("on a touch screen nothing is asked and no ghost shows", async () => {
    pointer = new PointerMedia(true);
    const { client } = renderThread(<ComposerWidget />, fixtureEvents("text_reply"));
    await userEvent.click(input());
    await userEvent.keyboard("hel");
    expect(asked(client)).toHaveLength(0);
    expect(ghost()).toBeNull();
  });
});
