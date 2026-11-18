/** L4: the first run's welcome when omp has no usable model (UI-500..UI-504), over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import { act, cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Frame } from "../../src/frame/frame.tsx";
import { ERRORS, SELECTED_THREAD, SUCCESSES, type WidgetComponent } from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, testHost } from "../support/host.tsx";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
});

const stub =
  (name: string): WidgetComponent =>
  () => <p>{name}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>(
  ["header", "chat", "workspace", "changes", "background"].map((name) => [`omfp:${name}`, stub(name)]),
);

/** A connected window whose index knows `models` (`undefined`: the daemon has not checked yet); `importable` answers
 * `auth.importable` (else it stays pending). */
function setup(models: number | undefined, importable?: () => unknown) {
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: new ManualTimers(),
    noticeTimers: new ManualTimers(),
  });
  base.client.answers.set("widgets.report", () => ({}));
  if (importable !== undefined) {
    base.client.answers.set("auth.importable", importable);
  }
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1")],
    threads: [thread("t1", "checkout", "w1")],
    ...(models === undefined ? {} : { models }),
  });
  test.flush();
  return { ...test, user: userEvent.setup() };
}

/** Whether the welcome shows instead of the view. */
function welcomed(): boolean {
  const welcome = screen.queryAllByTestId("welcome").length === 1;
  expect(screen.queryAllByTestId("view").length).toBe(welcome ? 0 : 1);
  return welcome;
}

describe("the first run", () => {
  // spec: UI-500
  test("without a usable model and no thread selected, the main area welcomes and says why instead of the view", () => {
    setup(0);
    expect(welcomed()).toBe(true);
    const welcome = screen.getByTestId("welcome");
    expect(welcome.querySelector("h1")?.textContent).toBe("Welcome to omfp");
    expect(screen.getByTestId("welcome-why").textContent).toBe(
      "omp has no model it can use yet. Log in to a provider to start your first thread.",
    );
    expect(screen.getByTestId("welcome-keys").textContent).toContain("ANTHROPIC_API_KEY");
    // The rest of the frame stays: the threads column and the mode line.
    expect(screen.queryAllByTestId("left-column").length).toBe(1);
    expect(screen.queryAllByTestId("mode-bar").length).toBe(1);
  });

  // spec: UI-501
  test("its primary action opens the provider picker, and is busy while a login runs", async () => {
    const test = setup(0);
    test.client.answers.set("auth.providers", () => ({
      providers: [{ id: "anthropic", name: "Anthropic (Claude Pro/Max)", available: true, authenticated: false }],
    }));
    const button = screen.getByTestId("welcome-login");
    expect(button.textContent).toBe("Log in to a provider");
    expect(button.dataset.variant).toBe("primary");

    await test.user.click(button);

    await screen.findByTestId("login-picker");
    expect(test.client.callsTo("auth.providers").length).toBe(1);
    await test.user.click(await screen.findByTestId("login-provider"));
    await waitFor(() => expect(test.client.callsTo("auth.login").length).toBe(1));
    expect(screen.getByTestId("welcome-login").dataset.pending).toBe("true");
  });

  // spec: UI-502
  test("the window asks auth.status once per connection while the index does not know; a failure changes nothing", async () => {
    const test = setup(undefined);
    expect(test.client.callsTo("auth.status").length).toBe(1);
    expect(welcomed()).toBe(false);
    const [status] = test.client.callsTo("auth.status");
    await act(async () => {
      status?.reject(new CallError({ kind: "daemon", error: { code: "thread_failed", message: "omp is missing" } }));
      await Promise.resolve();
    });
    expect(welcomed()).toBe(false);
    expect(screen.queryAllByTestId("error").length).toBe(0);
    test.flush();
    expect(test.client.callsTo("auth.status").length).toBe(1);

    // A known count asks nothing.
    cleanup();
    const known = setup(3);
    expect(known.client.callsTo("auth.status").length).toBe(0);
  });

  // spec: UI-503
  test("the count follows the index live: a login that made models usable shows the view, losing them welcomes", () => {
    const test = setup(undefined);
    const models = (count: number) =>
      act(() => {
        test.client.indexEvents({ kind: "models_changed", models: count });
        test.flush();
      });
    models(0);
    expect(welcomed()).toBe(true);
    models(2);
    expect(welcomed()).toBe(false);
    models(0);
    expect(welcomed()).toBe(true);
  });

  // spec: UI-504
  test("with usable models, an unknown count or a selected thread, nothing changes", () => {
    const configured = setup(4);
    expect(welcomed()).toBe(false);
    expect(configured.client.callsTo("auth.status").length).toBe(0);
    cleanup();
    setup(undefined);
    expect(welcomed()).toBe(false);
    cleanup();
    const selected = setup(0);
    expect(welcomed()).toBe(true);
    act(() => selected.host.window.set(SELECTED_THREAD, "t1"));
    expect(welcomed()).toBe(false);
  });

  // spec: UI-505
  test("the welcome asks auth.importable once per connection and offers nothing when omp has no login or it fails", async () => {
    const empty = setup(0, () => ({ providers: [] }));
    await act(async () => {
      await Promise.resolve();
    });
    expect(welcomed()).toBe(true);
    expect(empty.client.callsTo("auth.importable").length).toBe(1);
    expect(screen.queryAllByTestId("welcome-import").length).toBe(0);
    expect(screen.queryAllByTestId("welcome-import-note").length).toBe(0);
    // The welcome going and coming back on the same connection asks nothing again.
    act(() => {
      empty.client.indexEvents({ kind: "models_changed", models: 2 });
      empty.flush();
    });
    act(() => {
      empty.client.indexEvents({ kind: "models_changed", models: 0 });
      empty.flush();
    });
    expect(welcomed()).toBe(true);
    expect(empty.client.callsTo("auth.importable").length).toBe(1);

    cleanup();
    const failing = setup(0);
    const [asked] = failing.client.callsTo("auth.importable");
    await act(async () => {
      asked?.reject(new CallError({ kind: "daemon", error: { code: "omp_error", message: "auth broker configured" } }));
      await Promise.resolve();
    });
    expect(welcomed()).toBe(true);
    expect(screen.queryAllByTestId("welcome-import").length).toBe(0);
    expect(failing.host.window.get(ERRORS)).toEqual([]);

    // A window with usable models never asks.
    cleanup();
    const configured = setup(3);
    expect(configured.client.callsTo("auth.importable").length).toBe(0);
  });

  // spec: UI-505
  test("a new connection asks again even with the same connection number, and an earlier connection's late answer is dropped", async () => {
    const test = setup(0);
    const [first] = test.client.callsTo("auth.importable");
    // The daemon restarted and numbers this connection 1 again: still a new connection.
    act(() => {
      test.client.model.setConnection({ kind: "reconnecting", attempt: 1, retryInMs: 500, error: "closed" });
      test.client.notify();
      test.flush();
    });
    act(() => {
      test.client.connected();
      test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "checkout", "w1")], models: 0 });
      test.flush();
    });
    expect(test.client.callsTo("auth.importable").length).toBe(2);
    await act(async () => {
      first?.resolve({ providers: [{ providerId: "anthropic", oauth: false }] });
      await Promise.resolve();
    });
    expect(screen.queryAllByTestId("welcome-import").length).toBe(0);

    const [, second] = test.client.callsTo("auth.importable");
    await act(async () => {
      second?.resolve({ providers: [{ providerId: "openai", oauth: false }] });
      await Promise.resolve();
    });
    expect(screen.getByTestId("welcome-import").textContent).toBe("Use my omp logins (openai)");
  });

  // spec: UI-505
  test("with omp's logins the welcome offers to copy them under the login, with the OAuth caveat only for browser logins", async () => {
    setup(0, () => ({
      providers: [
        { providerId: "anthropic", oauth: false },
        { providerId: "openai", oauth: false },
      ],
    }));
    const button = await screen.findByTestId("welcome-import");
    expect([button.textContent, button.dataset.variant]).toEqual([
      "Use my omp logins (anthropic, openai)",
      "secondary",
    ]);
    expect(screen.getByTestId("welcome-login").dataset.variant).toBe("primary");
    const note = screen.getByTestId("welcome-import-note");
    expect(note.textContent).toBe("Copies only omp's logins, nothing else.");
    expect(button.getAttribute("aria-describedby")).toBe(note.id);
    expect(note.id).not.toBe("");

    cleanup();
    setup(0, () => ({
      providers: [
        { providerId: "anthropic", oauth: true },
        { providerId: "openai", oauth: false },
      ],
    }));
    await screen.findByTestId("welcome-import");
    expect(screen.getByTestId("welcome-import-note").textContent).toBe(
      "Copies only omp's logins, nothing else. A browser login then lives in both apps: if one of them gets signed out, log in there again.",
    );
  });

  // spec: UI-505, UI-506
  test("pressing the offer copies omp's logins with auth.import, busy while it runs, and says what it copied", async () => {
    const test = setup(0, () => ({ providers: [{ providerId: "anthropic", oauth: false }] }));
    await test.user.click(await screen.findByTestId("welcome-import"));
    expect(test.client.callsTo("auth.import").map((call) => call.params)).toEqual([{}]);
    expect(screen.getByTestId("welcome-import").dataset.pending).toBe("true");
    await test.user.click(screen.getByTestId("welcome-import"));
    expect(test.client.callsTo("auth.import").length).toBe(1);

    const [copy] = test.client.callsTo("auth.import");
    await act(async () => {
      copy?.resolve({ providers: [{ providerId: "anthropic", oauth: false }] });
      await Promise.resolve();
    });
    expect(test.host.window.get(SUCCESSES).map((shown) => shown.text)).toEqual(["Copied omp's logins: anthropic"]);
    expect(screen.getByTestId("welcome-import").dataset.pending).toBeUndefined();
  });
});
