/** L4: `omfp:header` over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import type { ThreadEvent } from "@omfp/core";
import type { ThreadSummary } from "@omfp/protocol";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import {
  AUTH_REVISION,
  DRAFT_THREAD,
  draftKey,
  ERRORS,
  FRAME_LAYOUT,
  LOGIN,
  OmfpProvider,
  RENAMING_THREAD,
  REVEAL_WIDGET,
  SELECTED_THREAD,
  SUCCESSES,
} from "../../src/sdk/index.ts";
import { Composer } from "../../src/widgets/chat/composer.tsx";
import { ThreadMoreButton, ThreadRename } from "../../src/widgets/header/header.tsx";
import { HeaderWidget } from "../../src/widgets/header/index.ts";
import { captureDownloads } from "../support/downloads.ts";
import { repo, thread, workspace } from "../support/fixtures.ts";
import { type FakeClient, renderWithHost, testHost } from "../support/host.tsx";
import { installResizeObserver, resize } from "../support/resize.ts";
import { fixtureEvents, isFrame, priced } from "./chat/fixtures.ts";

afterEach(cleanup);

const opus = { id: "opus", name: "Opus", provider: "anthropic" };

function answer(client: FakeClient): void {
  const answers: Partial<Record<CallMethod, unknown>> = {
    "thread.state": {
      sessionId: "s",
      model: opus,
      thinkingLevel: "high",
      contextUsage: { tokens: 1, contextWindow: 200_000, percent: 42 },
    },
    "thread.models": { models: [opus, { id: "haiku", name: "Haiku", provider: "anthropic" }] },
    "thread.thinking_levels": { levels: ["off", "high"] },
    "threads.update": {},
    "threads.archive": {},
    "threads.unarchive": {},
    "threads.delete": {},
    "thread.stop": {},
    "thread.restart": {},
    "thread.set_model": {},
    "thread.set_thinking": {},
    "thread.subagents": { subagents: [] },
  };
  for (const [method, result] of Object.entries(answers)) {
    client.answers.set(method as CallMethod, () => result);
  }
}

function setup(summary: ThreadSummary = thread("t1", "checkout", "w1")) {
  const base = testHost();
  answer(base.client);
  base.host.window.set(SELECTED_THREAD, summary.id);
  const test = renderWithHost(<HeaderWidget />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1", [repo("w1", "fe", "main")])], threads: [summary] });
  test.flush();
  return { ...test, user: userEvent.setup() };
}

async function choose(user: UserEvent, testid: string, option: string): Promise<void> {
  await user.click(within(screen.getByTestId(testid)).getByRole("button"));
  await user.click(await screen.findByRole("option", { name: option }));
}

/** Presses `testid` in the header's `⋯` popover (UI-482). */
async function fromMore(user: UserEvent, testid: string): Promise<void> {
  await user.click(screen.getByTestId("header-more"));
  await user.click(within(await screen.findByTestId("header-more-dialog")).getByTestId(testid));
}

describe("omfp:header", () => {
  // spec: UI-274
  test("a badge counts what runs in the background and shows omfp:background", async () => {
    const test = setup();
    expect(screen.queryByTestId("background-badge")).toBeNull();
    const events = fixtureEvents("async_bash_tool_call");
    const ended = events.findIndex(isFrame("tool_execution_end")) + 1;
    test.client.thread("t1", events.slice(0, ended));
    test.flush();
    const badge = screen.getByTestId("background-badge");
    expect(badge.textContent).toBe("1 in background");
    await test.user.click(badge);
    expect(test.host.window.get(REVEAL_WIDGET)?.reference).toBe("omfp:background");
    test.client.threadEvents("t1", ...events.slice(ended));
    test.flush();
    expect(screen.queryByTestId("background-badge")).toBeNull();
  });

  // spec: UI-690, UI-691, UI-692
  test("the cost: the session's own, then its subagents' while they run and once done; the same in the compact ⋯", async () => {
    const restore = installResizeObserver();
    try {
      const test = setup();
      // omp's `get_session_stats` also counts its calls outside the transcript (titles, judgments).
      test.client.answers.set("thread.session", () => ({ usage: { cost: 2 } }));
      const events = priced(fixtureEvents("task_tool_call"), { assistant: 0.125, task: 1.25, progress: 0.5 });
      const started = events.findIndex(isFrame("subagent_lifecycle"));
      const ended = events.findIndex((event, index) => index > started && isFrame("subagent_lifecycle")(event));
      test.client.thread("t1", events.slice(0, started));
      test.flush();
      const cost = () => screen.getByTestId("session-cost");
      const label = () => [cost().textContent, cost().getAttribute("aria-label"), cost().getAttribute("title")];
      expect(label()).toEqual(["$0.1250", "Cost: session $0.1250", "Cost: session $0.1250"]);
      test.client.threadEvents("t1", ...events.slice(started, ended));
      test.flush();
      const running = "Cost: session $0.1250, subagents $0.5000";
      expect(label()).toEqual(["$0.1250 (+$0.5000)", running, running]);
      // Read once the thread settles.
      expect(test.client.callsTo("thread.session").length).toBe(0);
      test.client.threadEvents("t1", ...events.slice(ended));
      test.flush();
      const done = "Cost: session $0.7500, subagents $1.25";
      await waitFor(() => expect(label()).toEqual(["$0.7500 (+$1.25)", done, done]));
      resize(screen.getByTestId("thread-header"), 600);
      await test.user.click(screen.getByTestId("header-more"));
      const more = await screen.findByTestId("header-more-dialog");
      await waitFor(() => expect(within(more).getByTestId("session-cost").textContent).toBe("$0.7500 (+$1.25)"));
    } finally {
      restore();
    }
  });

  // spec: UI-340
  test("one workspace chip: name and repository count, the repositories as its tooltip; a press shows omfp:workspace", async () => {
    const test = setup();
    const chip = screen.getByTestId("workspace-chip");
    expect([chip.textContent, chip.dataset.repos]).toEqual(["w1 · 1 repo", "1"]);
    const label = chip.querySelector("[title]");
    expect(label?.getAttribute("title")).toBe("fe:main (worktree)");
    const description = document.getElementById(chip.getAttribute("aria-describedby") ?? "");
    expect(description?.textContent).toBe("fe:main (worktree)");
    await test.user.click(chip);
    expect(test.host.window.get(REVEAL_WIDGET)?.reference).toBe("omfp:workspace");
  });

  // spec: UI-484
  test("the workspace chip is a bordered button; in the compact ⋯ a press closes the popover and shows omfp:workspace", async () => {
    const restore = installResizeObserver();
    try {
      const test = setup();
      const chip = screen.getByTestId("workspace-chip");
      expect([chip.tagName, chip.dataset.variant]).toEqual(["BUTTON", "secondary"]);
      resize(screen.getByTestId("thread-header"), 600);
      await test.user.click(screen.getByTestId("header-more"));
      const more = await screen.findByTestId("header-more-dialog");
      await test.user.click(within(more).getByTestId("workspace-chip"));
      expect(screen.queryAllByTestId("header-more-dialog").length).toBe(0);
      expect(test.host.window.get(REVEAL_WIDGET)?.reference).toBe("omfp:workspace");
    } finally {
      restore();
    }
  });

  // spec: UI-030
  test("without a selected thread the header says so", () => {
    renderWithHost(<HeaderWidget />);
    expect(screen.getByTestId("header-empty").textContent).toBe("No thread selected");
  });

  // spec: UI-030, UI-020
  test("it shows the title, chips, process state and the session's model, thinking and context; no tags", async () => {
    setup();
    expect(screen.getByTestId("thread-title").textContent).toBe("checkout");
    expect(screen.queryByTestId("edit-tags")).toBeNull();
    expect(screen.getAllByTestId("workspace-chip").map((chip) => chip.textContent)).toEqual(["w1 · 1 repo"]);
    expect(screen.getByTestId("thread-state").dataset.status).toBe("idle");
    await waitFor(() => expect(screen.getByTestId("context-usage").textContent).toBe("84k/200k"));
    expect(screen.getByTestId("context-usage").getAttribute("aria-label")).toBe("Context window usage: 42% of 200k");
    expect(within(screen.getByTestId("model-select")).getByRole("button").textContent).toContain("Opus (anthropic)");
    expect(within(screen.getByTestId("thinking-select")).getByRole("button").textContent).toContain("high");
  });

  // spec: UI-202
  test("the full header's title carries the full title as its tooltip, as the compact one does", () => {
    const long = "a thread title long enough to be cut with an ellipsis in the full header";
    setup(thread("t1", long, "w1"));
    expect(screen.getByTestId("thread-title").getAttribute("title")).toBe(long);
  });

  // spec: UI-030, UI-023, UI-092, UI-286
  test("rename in place: Enter saves through threads.update, Escape cancels, both back on the title; unnamed threads start empty", async () => {
    const { client, user } = setup(thread("t1", "", "w1"));
    const title = screen.getByTestId("thread-title");
    expect([title.textContent, title.dataset.placeholder]).toEqual(["New thread", "true"]);
    await fromMore(user, "rename-thread");
    const input = screen.getByTestId("title-input");
    expect(document.activeElement === input).toBe(true);
    expect((input as HTMLInputElement).value).toBe("");
    await user.type(input, "checkout v2{Enter}");
    expect(client.callsTo("threads.update").map((call) => call.params)).toEqual([
      { threadId: "t1", title: "checkout v2" },
    ]);
    // The focus stays on the renamed item: its title.
    expect(document.activeElement?.getAttribute("data-testid")).toBe("thread-title");
    await fromMore(user, "rename-thread");
    await user.type(screen.getByTestId("title-input"), "x{Escape}");
    expect(screen.queryByTestId("title-input")).toBeNull();
    expect(document.activeElement?.getAttribute("data-testid")).toBe("thread-title");
    expect(client.callsTo("threads.update")).toHaveLength(1);
  });

  // spec: UI-030
  test("choosing a model or thinking level sets it and refreshes the session", async () => {
    const { client, user } = setup();
    await waitFor(() => expect(screen.getByTestId("context-usage")).toBeDefined());
    await choose(user, "model-select", "Haiku (anthropic)");
    expect(client.callsTo("thread.set_model").map((call) => call.params)).toEqual([
      { threadId: "t1", provider: "anthropic", modelId: "haiku" },
    ]);
    await choose(user, "thinking-select", "off");
    expect(client.callsTo("thread.set_thinking").map((call) => call.params)).toEqual([
      { threadId: "t1", level: "off" },
    ]);
    await waitFor(() => expect(client.callsTo("thread.state")).toHaveLength(3));
  });

  // spec: UI-030
  test("the context usage is read again when a turn settles and when a compaction ends, not only on selection", async () => {
    const test = setup();
    const usage = () => screen.getByTestId("context-usage").textContent;
    await waitFor(() => expect(usage()).toBe("84k/200k"));
    const contextAt = (percent: number) =>
      test.client.answers.set("thread.state", () => ({
        sessionId: "s",
        model: opus,
        thinkingLevel: "high",
        contextUsage: { tokens: 1, contextWindow: 200_000, percent },
      }));
    const settled = (value: boolean) => {
      test.client.index({
        workspaces: [workspace("w1", [repo("w1", "fe", "main")])],
        threads: [thread("t1", "checkout", "w1", { settled: value })],
      });
      test.flush();
    };
    test.client.thread("t1", []);
    test.flush();
    settled(false);
    contextAt(60);
    settled(true);
    await waitFor(() => expect(usage()).toBe("120k/200k"));
    contextAt(10);
    test.client.threadEvents(
      "t1",
      { kind: "omp", frame: { type: "manual_compaction_start" } },
      { kind: "omp", frame: { type: "manual_compaction_end", aborted: false } },
    );
    test.flush();
    await waitFor(() => expect(usage()).toBe("20k/200k"));
    contextAt(30);
    test.client.threadEvents("t1", {
      kind: "omp",
      frame: { type: "auto_compaction_end", action: "context-full", aborted: false, willRetry: false },
    });
    test.flush();
    await waitFor(() => expect(usage()).toBe("60k/200k"));
  });

  // spec: UI-030
  test("sending waits for the selected model to be applied without losing the first message", async () => {
    const base = testHost();
    answer(base.client);
    base.client.answers.delete("thread.set_model");
    base.host.window.set(SELECTED_THREAD, "t1");
    const { client, flush } = renderWithHost(
      <>
        <HeaderWidget />
        <Composer threadId="t1" busy={false} abortable={false} commands={[]} />
      </>,
      base,
    );
    client.connected();
    client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "", "w1")] });
    flush();
    const user = userEvent.setup();
    await user.type(screen.getByTestId("composer-input"), "first message");
    await choose(user, "model-select", "Haiku (anthropic)");
    expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(true);
    await user.type(screen.getByTestId("composer-input"), "{Enter}");
    expect(client.callsTo("thread.prompt")).toHaveLength(0);
    expect(screen.getByTestId("composer-input")).toHaveProperty("value", "first message");
    act(() => client.callsTo("thread.set_model")[0]?.resolve({}));
    await waitFor(() => expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(false));
    await user.click(screen.getByTestId("send"));
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: "t1", text: "first message" },
    ]);
  });

  // spec: UI-030, UI-482, UI-524
  test("stop for a running process, restart with the failure for a failed one", async () => {
    const { client, user } = setup();
    expect(screen.getByTestId("process-action").textContent).toBe("Stop process");
    await user.click(screen.getByTestId("process-action"));
    expect(client.callsTo("thread.stop")).toHaveLength(1);
    cleanup();
    const failed = setup(thread("t1", "x", "w1", { process: { state: "failed", message: "omp exited" } }));
    // Why it failed: the state's tooltip and description, and the ⋯ popover's first line; not a text of the row.
    const state = screen.getByTestId("thread-state");
    expect([state.getAttribute("title"), state.getAttribute("aria-description")]).toEqual(["omp exited", "omp exited"]);
    expect(screen.queryAllByTestId("thread-failure").length).toBe(0);
    await failed.user.click(screen.getByTestId("header-more"));
    expect((await screen.findByTestId("thread-failure")).textContent).toBe("omp exited");
    await failed.user.keyboard("{Escape}");
    await failed.user.click(screen.getByTestId("process-action"));
    expect(failed.client.callsTo("thread.restart")).toHaveLength(1);
  });

  // spec: UI-026, UI-482
  test.each(["delete-thread", "archive-thread"])("%s clears the selection and draft immediately", async (control) => {
    const { client, host, user } = setup();
    host.window.set(draftKey("t1"), { text: "half", images: [], pastes: [] });
    await fromMore(user, control);
    const method = control === "delete-thread" ? "threads.delete" : "threads.archive";
    expect(client.callsTo(method).map((call) => call.params)).toEqual([{ threadId: "t1" }]);
    expect(client.callsTo(control === "delete-thread" ? "threads.archive" : "threads.delete")).toHaveLength(0);
    expect(host.window.get(SELECTED_THREAD)).toBeUndefined();
    expect(host.window.get(draftKey("t1")).text).toBe("");
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  // spec: UI-026, UI-482
  test("an archived thread offers unarchive", async () => {
    const { client, user } = setup(thread("t1", "x", "w1", { archivedAt: 2 }));
    await user.click(screen.getByTestId("header-more"));
    const archive = within(await screen.findByTestId("header-more-dialog")).getByTestId("archive-thread");
    expect(archive.textContent).toBe("Unarchive");
    await user.click(archive);
    expect(client.callsTo("threads.unarchive")).toHaveLength(1);
  });

  // spec: UI-482
  test("the title row: the title, the state, its one action and ⋯; rename, archive and delete only in the ⋯", async () => {
    const { user } = setup();
    const top = screen.getByTestId("thread-header").querySelector(".omfp-chat-header__top");
    const ids = [...(top?.querySelectorAll("[data-testid]") ?? [])].map((e) => e.getAttribute("data-testid"));
    expect(ids).toEqual(["thread-title", "thread-state", "process-action", "header-more"]);
    await user.click(screen.getByTestId("header-more"));
    const more = await screen.findByTestId("header-more-dialog");
    expect(
      ["rename-thread", "archive-thread", "delete-thread", "process-action", "thinking-select", "workspace-chip"].map(
        (testid) => within(more).queryAllByTestId(testid).length,
      ),
    ).toEqual([1, 1, 1, 0, 0, 0]);
  });

  // spec: UI-483
  test("what is not a control has no box; the controls are bordered buttons, the one call to action primary", () => {
    setup(
      thread("t1", "x", "w1", {
        process: {
          state: "failed",
          message: "omp failed to start: server closed",
          stderr_tail: "No default model selected. Use /login, or pass --model.",
        },
      }),
    );
    expect(screen.getByTestId("thread-state").dataset.bare).toBe("true");
    const variant = (testid: string) => screen.getByTestId(testid).dataset.variant;
    expect(["process-action", "header-more", "header-login"].map(variant)).toEqual([
      "secondary",
      "secondary",
      "primary",
    ]);
  });

  // spec: UI-202
  test("narrower than 48rem the header is one row: title, state, model and a ⋯ popover with the rest", async () => {
    const restore = installResizeObserver();
    try {
      const { client, user } = setup();
      const header = screen.getByTestId("thread-header");
      expect(within(header).queryAllByTestId("header-more").length).toBe(1);
      resize(header, 600);
      expect(header.dataset.compact).toBe("true");
      const inHeader = (testid: string) => within(header).queryAllByTestId(testid).length;
      expect(
        ["thread-title", "thread-state", "model-select", "header-more", "process-action", "archive-thread"].map(
          inHeader,
        ),
      ).toEqual([1, 1, 1, 1, 0, 0]);
      expect(["thinking-select", "workspace-chip", "rename-thread"].map(inHeader)).toEqual([0, 0, 0]);
      await user.click(screen.getByTestId("header-more"));
      const more = await screen.findByTestId("header-more-dialog");
      expect(
        ["rename-thread", "process-action", "archive-thread", "delete-thread", "thinking-select"].map(
          (testid) => within(more).queryAllByTestId(testid).length,
        ),
      ).toEqual([1, 1, 1, 1, 1]);
      expect(
        within(more)
          .getAllByTestId("workspace-chip")
          .map((chip) => chip.textContent),
      ).toEqual(["w1 · 1 repo"]);
      await user.click(within(more).getByTestId("rename-thread"));
      expect(screen.queryAllByTestId("header-more-dialog").length).toBe(0);
      expect(within(header).queryAllByTestId("title-input").length).toBe(1);
      await user.keyboard("{Escape}");
      resize(header, 1000);
      expect(header.dataset.compact).toBeUndefined();
      expect(inHeader("process-action")).toBe(1);
      resize(header, 600);
      await user.click(screen.getByTestId("header-more"));
      await user.click(within(await screen.findByTestId("header-more-dialog")).getByTestId("archive-thread"));
      expect(client.callsTo("threads.archive").map((call) => call.params)).toEqual([{ threadId: "t1" }]);
    } finally {
      restore();
    }
  });

  // spec: UI-349
  test("the held-back restart shows named by its reasons; a refusal is an error toast", async () => {
    expect(setup().view.container.querySelector("[data-testid=restart-pending]")).toBeNull();
    cleanup();
    const { client, host, user } = setup(thread("t1", "checkout", "w1", { restartPending: ["login"] }));
    const reload = screen.getByTestId("restart-pending");
    expect(reload.textContent).toBe("Restart to load new models");
    await user.click(reload);
    const [reconnect] = client.callsTo("thread.mcp_reconnect");
    expect(reconnect?.params).toEqual({ threadId: "t1" });
    reconnect?.reject(new Error("busy: the turn has not settled"));
    await waitFor(() =>
      expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["busy: the turn has not settled"]),
    );
  });

  // spec: UI-388
  test("a start that failed for want of a login offers Log in beside Restart; another failure does not", async () => {
    const noDefault =
      "No default model selected. Use /login, set an API key environment variable, or select a local model with /model or --model.";
    const { host, user } = setup(
      thread("t1", "x", "w1", {
        process: { state: "failed", message: "omp failed to start: server closed", stderr_tail: noDefault },
      }),
    );
    expect(screen.getByTestId("process-action").dataset.action).toBe("restart");
    const login = screen.getByTestId("header-login");
    expect(login.textContent).toBe("Log in");
    await user.click(login);
    expect(host.window.get(LOGIN)).toEqual({ providerId: undefined });
    cleanup();
    setup(thread("t1", "x", "w1", { process: { state: "failed", message: "omp exited" } }));
    expect(screen.queryAllByTestId("header-login").length).toBe(0);
  });

  // spec: UI-751
  test("a failed start's unusable saved model: the header says so, offers the usable models and Log in, and only a pick changes it", async () => {
    const base = testHost();
    answer(base.client);
    base.client.answers.set("auth.models", () => ({ models: [opus] }));
    base.client.answers.set("thread.session", () => ({ sessionModel: "anthropic/claude-opus-5-5" }));
    base.host.window.set(SELECTED_THREAD, "t1");
    const { client, host, flush } = renderWithHost(<HeaderWidget />, base);
    client.connected();
    const failed = thread("t1", "x", "w1", {
      process: {
        state: "failed",
        message: "omp failed to start: omp did not become ready: server closed",
        stderr_tail: "SessionModelRestoreError: Could not restore model anthropic/claude-opus-5-5",
      },
    });
    client.index({ workspaces: [workspace("w1")], threads: [failed] });
    flush();
    const user = userEvent.setup();

    const note = await screen.findByTestId("model-unavailable");
    expect(note.textContent).toBe(
      "The saved model anthropic/claude-opus-5-5 is unavailable. Choose another model or log in to resume this conversation.",
    );
    const select = screen.getByTestId("model-select");
    expect(select.dataset.unavailable).toBe("true");
    expect(within(select).getByRole("button").textContent).toContain("anthropic/claude-opus-5-5 (unavailable)");
    // Read without starting omp; nothing is switched by itself.
    expect(client.callsTo("thread.state").length).toBe(0);
    expect(client.callsTo("thread.set_model").length).toBe(0);

    // Log in beside Restart opens the provider picker.
    expect(screen.getByTestId("process-action").dataset.action).toBe("restart");
    await user.click(screen.getByTestId("header-login"));
    expect(host.window.get(LOGIN)).toEqual({ providerId: undefined });

    // The saved model cannot be picked again; a usable one is stored and resumes the conversation.
    await user.click(within(select).getByRole("button"));
    const saved = await screen.findByRole("option", { name: "anthropic/claude-opus-5-5 (unavailable)" });
    expect(saved.getAttribute("aria-disabled")).toBe("true");
    await user.click(await screen.findByRole("option", { name: "Opus (anthropic)" }));
    expect(client.callsTo("thread.set_model").map((call) => call.params)).toEqual([
      { threadId: "t1", provider: "anthropic", modelId: "opus" },
    ]);
  });

  // spec: UI-385, UI-388
  test("with no model the model list offers Log in, and a login of the window reads the models again", async () => {
    setup();
    // The default answer lists models: the selector shows.
    await waitFor(() => expect(screen.queryAllByTestId("model-select").length).toBe(1));
    expect(screen.queryAllByTestId("models-login").length).toBe(0);
    cleanup();

    const base = testHost();
    answer(base.client);
    base.client.answers.set("thread.models", () => ({ models: [] }));
    base.client.answers.set("thread.state", () => ({ sessionId: "s" }));
    base.host.window.set(SELECTED_THREAD, "t1");
    const { client, host, flush } = renderWithHost(<HeaderWidget />, base);
    client.connected();
    client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "checkout", "w1")] });
    flush();
    const login = await screen.findByTestId("models-login");
    expect(screen.getByTestId("no-models").textContent).toBe("No modelsLog in");
    await userEvent.setup().click(login);
    expect(host.window.get(LOGIN)).toEqual({ providerId: undefined });

    // A login worked: the models are read again and the list shows.
    expect(client.callsTo("thread.models").length).toBe(1);
    client.answers.set("thread.models", () => ({ models: [opus] }));
    act(() => host.window.set(AUTH_REVISION, (revision) => revision + 1));
    await waitFor(() => expect(client.callsTo("thread.models").length).toBe(2));
    await screen.findByTestId("model-select");
    expect(screen.queryAllByTestId("models-login").length).toBe(0);
  });

  // spec: UI-998, UI-030
  test("stopped threads shown one after the other read the usable models once, again after a login", async () => {
    const base = testHost();
    answer(base.client);
    base.client.answers.set("auth.models", () => ({ models: [opus] }));
    base.host.window.set(SELECTED_THREAD, "t1");
    const { client, host, flush } = renderWithHost(<HeaderWidget />, base);
    client.connected();
    const stopped = { process: { state: "stopped" } } as const;
    client.index({
      workspaces: [workspace("w1")],
      threads: [thread("t1", "one", "w1", stopped), thread("t2", "two", "w1", stopped)],
    });
    flush();
    await screen.findByTestId("model-select");
    act(() => host.window.set(SELECTED_THREAD, "t2"));
    await waitFor(() => expect(screen.getByTestId("thread-header").dataset.threadId).toBe("t2"));
    await screen.findByTestId("model-select");
    // Each `auth.models` starts a short-lived omp; nothing asked would start a thread's.
    expect(client.calls.map((call) => call.method)).toEqual(["auth.models"]);

    act(() => host.window.set(AUTH_REVISION, (revision) => revision + 1));
    await waitFor(() => expect(client.callsTo("auth.models").length).toBe(2));
  });
});

describe("`:model`, omp's `/model` in omfp", () => {
  // spec: UI-980
  test("thread.pickModel opens the header's model list; without a model it logs in", async () => {
    const { client, host, user } = setup();
    await waitFor(() => expect(screen.getByTestId("context-usage")).toBeDefined());
    expect(host.commands.resolve("model")?.id).toBe("thread.pickModel");
    act(() => {
      host.commands.run("thread.pickModel");
    });
    await user.click(await screen.findByRole("option", { name: "Haiku (anthropic)" }));
    expect(client.callsTo("thread.set_model").map((call) => call.params)).toEqual([
      { threadId: "t1", provider: "anthropic", modelId: "haiku" },
    ]);
    cleanup();

    const base = testHost();
    answer(base.client);
    base.client.answers.set("thread.models", () => ({ models: [] }));
    base.client.answers.set("thread.state", () => ({ sessionId: "s" }));
    base.host.window.set(SELECTED_THREAD, "t1");
    const empty = renderWithHost(<HeaderWidget />, base);
    empty.client.connected();
    empty.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "checkout", "w1")] });
    empty.flush();
    await screen.findByTestId("models-login");
    act(() => {
      empty.host.commands.run("model");
    });
    expect(empty.host.window.get(LOGIN)).toEqual({ providerId: undefined });
  });
});

describe("the header on a phone", () => {
  // spec: UI-419
  test("a thread's header gives way to the mode line; its ⋯ there holds the model, which :model opens", async () => {
    const base = testHost();
    answer(base.client);
    base.host.window.set(SELECTED_THREAD, "t1");
    base.host.window.set(FRAME_LAYOUT, "mobile");
    const summary = thread("t1", "checkout", "w1");
    const test = renderWithHost(
      <>
        <HeaderWidget />
        <ThreadMoreButton summary={summary} shown />
      </>,
      base,
    );
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1", [repo("w1", "fe", "main")])], threads: [summary] });
    test.flush();
    const user = userEvent.setup();
    expect(screen.queryAllByTestId("thread-header").length).toBe(0);
    // The model is in the popover only: `:model` opens the popover with its list.
    expect(screen.queryAllByTestId("model-select").length).toBe(0);
    act(() => {
      test.host.commands.run("model");
    });
    const more = await screen.findByTestId("header-more-dialog");
    expect(within(more).queryAllByTestId("model-select").length).toBe(1);
    await user.click(await screen.findByRole("option", { name: "Haiku (anthropic)" }));
    expect(test.client.callsTo("thread.set_model").map((call) => call.params)).toEqual([
      { threadId: "t1", provider: "anthropic", modelId: "haiku" },
    ]);
  });

  // spec: UI-419
  test("a draft keeps its header: its workspace and model have nowhere else to go", () => {
    const base = testHost();
    answer(base.client);
    base.host.window.set(FRAME_LAYOUT, "mobile");
    base.host.window.set(DRAFT_THREAD, { workspaceId: "w1" });
    const test = renderWithHost(<HeaderWidget />, base);
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [] });
    test.flush();
    expect(screen.getByTestId("thread-header").dataset.draft).toBe("true");
  });

  // spec: UI-419, UI-980
  test("while the : line hides the ⋯, :model stays registered and opens the popover with its list once it shows", async () => {
    const base = testHost();
    answer(base.client);
    base.host.window.set(SELECTED_THREAD, "t1");
    const summary = thread("t1", "checkout", "w1");
    const more = (shown: boolean) => (
      <OmfpProvider host={base.host}>
        <ThreadMoreButton summary={summary} shown={shown} />
      </OmfpProvider>
    );
    const test = renderWithHost(<ThreadMoreButton summary={summary} shown={false} />, base);
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [summary] });
    test.flush();
    expect(screen.queryAllByTestId("header-more").length).toBe(0);
    act(() => {
      expect(test.host.commands.run("model")).toBe(true);
    });
    test.view.rerender(more(true));
    const dialog = await screen.findByTestId("header-more-dialog");
    expect(within(dialog).queryAllByTestId("model-select").length).toBe(1);
    expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toContain("Haiku (anthropic)");
  });

  // spec: UI-419
  test("renaming in the mode line: the editor takes the title's place, Enter saves, Escape leaves it as it was", async () => {
    const base = testHost();
    answer(base.client);
    const summary = thread("t1", "checkout", "w1");
    base.host.window.set(RENAMING_THREAD, "t1");
    const test = renderWithHost(<ThreadRename summary={summary} />, base);
    const user = userEvent.setup();
    const input = screen.getByTestId("title-input") as HTMLInputElement;
    expect(input.value).toBe("checkout");
    await user.clear(input);
    await user.type(input, "Fix checkout{Enter}");
    expect(test.client.callsTo("threads.update").map((call) => call.params)).toEqual([
      { threadId: "t1", title: "Fix checkout" },
    ]);
    expect([test.host.window.get(RENAMING_THREAD), screen.queryAllByTestId("title-input").length]).toEqual([
      undefined,
      0,
    ]);
    act(() => test.host.window.set(RENAMING_THREAD, "t1"));
    await user.type(screen.getByTestId("title-input"), " later{Escape}");
    expect(test.client.callsTo("threads.update").length).toBe(1);
    expect(test.host.window.get(RENAMING_THREAD)).toBeUndefined();
  });
});

describe("copying and exporting from the header", () => {
  // spec: UI-793
  test("the ⋯ popover copies and exports the whole conversation, then closes", async () => {
    const test = setup(thread("t1", "Sky", "w1"));
    test.client.thread("t1", [
      {
        kind: "history",
        session: { session_id: "s", resumed: true },
        messages: [{ role: "user", content: "Why is the sky blue?", timestamp: 1 }],
      } as ThreadEvent,
    ]);
    test.flush();
    const markdown = "# Sky\n\n## You\n\nWhy is the sky blue?\n";
    await fromMore(test.user, "copy-conversation");
    await waitFor(() => expect(screen.queryByTestId("header-more-dialog")).toBeNull());
    expect(await navigator.clipboard.readText()).toBe(markdown);
    const downloads = captureDownloads();
    try {
      await fromMore(test.user, "export-conversation");
      await waitFor(() =>
        expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual([
          "Copied the conversation",
          "Exported “sky.md”",
        ]),
      );
      expect(downloads.files.map((file) => file.name)).toEqual(["sky.md"]);
      expect(await downloads.files[0]?.text).toBe(markdown);
    } finally {
      downloads.restore();
    }
  });
});

describe("compacting from the header", () => {
  // spec: UI-912, UI-910, UI-911
  test("the ⋯ popover compacts with the focus typed, closes and says how big the context was", async () => {
    const test = setup();
    test.client.answers.set("thread.compact", () => ({ summary: "s", firstKeptEntryId: "e1", tokensBefore: 4200 }));
    await test.user.click(screen.getByTestId("header-more"));
    const dialog = await screen.findByTestId("header-more-dialog");
    await test.user.type(within(dialog).getByTestId("compaction-focus"), "the failing test");
    await test.user.click(within(dialog).getByTestId("compact-context"));
    await waitFor(() => expect(screen.queryByTestId("header-more-dialog")).toBeNull());
    await waitFor(() =>
      expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual([
        "Compacted the context (it held 4.2k tokens)",
      ]),
    );
    expect(test.client.callsTo("thread.compact").map((call) => call.params)).toEqual([
      { threadId: "t1", instructions: "the failing test" },
    ]);
  });

  // spec: UI-912, UI-910
  test("while the thread works the popover's compaction is refused", async () => {
    const test = setup(thread("t1", "checkout", "w1", { settled: false }));
    await fromMore(test.user, "compact-context");
    await waitFor(() =>
      expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
        "The thread is working: compact it once its turn ends (Abort ends it now)",
      ]),
    );
    expect(test.client.callsTo("thread.compact")).toEqual([]);
  });

  // spec: UI-912
  test("an archived thread's popover offers no compaction", async () => {
    const test = setup(thread("t1", "checkout", "w1", { archivedAt: 5 }));
    await test.user.click(screen.getByTestId("header-more"));
    const dialog = await screen.findByTestId("header-more-dialog");
    expect(within(dialog).queryAllByTestId("compaction").length).toBe(0);
  });
});

describe("a fork's header", () => {
  // spec: UI-664
  test("says where the thread comes from; the source's title shows it, a deleted source's title stays", async () => {
    const source = thread("t0", "checkout", "w1");
    const fork = {
      ...thread("t1", "checkout (fork)", "w1"),
      forkedFrom: { threadId: "t0", title: "old title", entryId: "e1", message: "Fix the cart" },
    };
    const test = setup(fork);
    test.client.index({ workspaces: [workspace("w1", [repo("w1", "fe", "main")])], threads: [source, fork] });
    test.flush();
    const lineage = await screen.findByTestId("fork-lineage");
    expect(lineage.textContent).toBe("Forked from checkout at \u201cFix the cart\u201d");
    expect(lineage.getAttribute("aria-label")).toBe("Go to checkout, the thread this one was forked from");
    await test.user.click(lineage);
    expect(test.host.window.get(SELECTED_THREAD)).toBe("t0");

    cleanup();
    setup(fork);
    const gone = await screen.findByTestId("fork-lineage");
    expect(gone.tagName).toBe("SPAN");
    expect(gone.textContent).toBe("Forked from old title at \u201cFix the cart\u201d");
    cleanup();
    setup();
    expect(screen.queryAllByTestId("fork-lineage").length).toBe(0);
  });
});

describe("a job's thread in the header", () => {
  // spec: UI-746
  test("a thread a job made says `job <name>` where a fork names its source, and reveals omfp:jobs", async () => {
    const made = thread("t1", "mail · 2026-11-16 07:00", "w1", {
      provenance: { createdBy: { kind: "job", job: "mail" }, client: "omfp-daemon" },
    });
    const test = setup(made);
    const mark = await screen.findByTestId("thread-job");
    expect(mark.textContent).toBe("job mail");
    expect(mark.getAttribute("aria-label")).toBe("Made by job mail: show the jobs");
    await test.user.click(mark);
    expect(test.host.window.get(REVEAL_WIDGET)).toMatchObject({ reference: "omfp:jobs", workspaceId: "w1" });
    cleanup();
    setup();
    expect(screen.queryAllByTestId("thread-job").length).toBe(0);
  });
});

describe("one meaning of job in the header", () => {
  // spec: UI-747
  test("the background badge's tooltip names background commands, not jobs", () => {
    const test = setup();
    const events = fixtureEvents("async_bash_tool_call");
    test.client.thread("t1", events.slice(0, events.findIndex(isFrame("tool_execution_end")) + 1));
    test.flush();
    const tip = screen.getByTestId("background-badge").querySelector("[title]")?.getAttribute("title");
    expect(tip).toBe("1 background command or subagent runs: show them");
  });
});
