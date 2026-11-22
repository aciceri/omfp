/** L4: logging in to a provider from the window (UI-380..UI-387), over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import type { AuthProvider, IndexEvent, WindowRequest } from "@omfp/protocol";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { completions } from "../../src/frame/cmdline.ts";
import { Frame } from "../../src/frame/frame.tsx";
import { AUTH_REVISION, ERRORS, reportError, SUCCESSES, type WidgetComponent } from "../../src/sdk/index.ts";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { ManualTimers, renderWithHost, type TestHost, testHost } from "../support/host.tsx";

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

const PROVIDERS: AuthProvider[] = [
  { id: "anthropic", name: "Anthropic (Claude Pro/Max)", available: true, authenticated: true },
  { id: "openai-codex", name: "ChatGPT Plus/Pro (Codex)", available: true, authenticated: false },
  { id: "github-copilot", name: "GitHub Copilot", available: false, authenticated: false },
];

const NO_DEFAULT =
  "No default model selected. Use /login, set an API key environment variable, or select a local model with /model or --model.";

const ANTHROPIC = { providerId: "anthropic", name: "Anthropic (Claude Pro/Max)" };

function setup() {
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: new ManualTimers(),
    noticeTimers: new ManualTimers(),
  });
  base.client.answers.set("widgets.report", () => ({}));
  base.client.answers.set("windows.answer", () => ({}));
  base.client.answers.set("auth.login_cancel", () => ({ providerId: "anthropic" }));
  const test = renderWithHost(<Frame registry={registry} now={() => 0} />, base);
  test.client.connected();
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "checkout", "w1")] });
  test.flush();
  return { ...test, user: userEvent.setup() };
}

function run(test: TestHost, line: string): void {
  const [name = "", ...args] = line.split(" ");
  act(() => {
    test.host.commands.run(name, args);
  });
}

function request(test: TestHost, id: string, asked: WindowRequest): void {
  act(() => test.client.windowRequest({ type: "window_request", id, request: asked }));
}

/** The answers the window sent with `windows.answer`. */
function answers(test: TestHost): unknown[] {
  return test.client.callsTo("windows.answer").map((call) => call.params);
}

/** Settles a pending call made by the code under test. */
async function settle(action: () => void): Promise<void> {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}

describe("logging in", () => {
  // spec: UI-382, UI-383
  test(":login opens the provider picker, which lists omp's providers and starts the login of the picked one", async () => {
    const test = setup();
    run(test, "login");
    const picker = await screen.findByTestId("login-picker");
    expect(within(picker).queryAllByTestId("login-providers-loading").length).toBe(1);
    const [providers] = test.client.callsTo("auth.providers");
    await settle(() => providers?.resolve({ providers: PROVIDERS }));
    const rows = within(picker).getAllByTestId("login-provider");
    expect(rows.map((row) => [row.dataset.provider, row.textContent])).toEqual([
      ["anthropic", "Anthropic (Claude Pro/Max)anthropiclogged in"],
      ["openai-codex", "ChatGPT Plus/Pro (Codex)openai-codex"],
      ["github-copilot", "GitHub Copilotgithub-copilotnot available here"],
    ]);
    expect(rows.map((row) => row.getAttribute("aria-disabled") === "true")).toEqual([false, false, true]);

    await test.user.click(rows[1] as HTMLElement);
    expect(test.client.callsTo("auth.login").map((call) => call.params)).toEqual([{ providerId: "openai-codex" }]);
    expect(screen.queryAllByTestId("login-picker").length).toBe(0);
    const progress = screen.getByTestId("login-progress");
    expect(progress.textContent).toContain("Logging in to ChatGPT Plus/Pro (Codex)… finish in your browser");

    request(test, "p1", { kind: "login_progress", providerId: "openai-codex", name: "Codex", message: "Waiting…" });
    expect(screen.getByTestId("login-progress-message").textContent).toBe("Waiting…");

    // Cancel: `auth.login_cancel`, and the rejection that follows is no error.
    await test.user.click(within(progress).getByTestId("login-cancel"));
    expect(test.client.callsTo("auth.login_cancel").length).toBe(1);
    const [login] = test.client.callsTo("auth.login");
    await settle(() =>
      login?.reject(new CallError({ kind: "daemon", error: { code: "omp_error", message: "Login cancelled" } })),
    );
    expect(screen.queryAllByTestId("login-progress").length).toBe(0);
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([]);
  });

  // spec: UI-382, UI-386, UI-389
  test(":login <provider> starts at once; :login and :logout complete the provider ids", async () => {
    const test = setup();
    run(test, "login anthropic");
    expect(test.client.callsTo("auth.login").map((call) => call.params)).toEqual([{ providerId: "anthropic" }]);
    expect(screen.queryAllByTestId("login-picker").length).toBe(0);

    // The first completion asks for the providers; once they are known the `:` line completes them.
    expect(completions("login ", test.host.commands.list())).toEqual([]);
    await act(async () => {
      await Promise.resolve();
    });
    const [providers] = test.client.callsTo("auth.providers");
    await settle(() => providers?.resolve({ providers: PROVIDERS }));
    expect(completions("login ", test.host.commands.list()).map((option) => option.value)).toEqual([
      "login anthropic",
      "login openai-codex",
    ]);
    expect(completions("logout ", test.host.commands.list()).map((option) => option.value)).toEqual([
      "logout anthropic",
    ]);
    expect(test.client.callsTo("auth.providers").length).toBe(1);
  });

  // spec: UI-382
  test("each provider row puts the long name on its own line over the id and status, and keyboard picking still works", async () => {
    const test = setup();
    run(test, "login");
    const picker = await screen.findByTestId("login-picker");
    const [providers] = test.client.callsTo("auth.providers");
    const long = "ChatGPT Plus/Pro (Codex Subscription) with a name long enough to wrap in a narrow dialog";
    await settle(() =>
      providers?.resolve({
        providers: [
          {
            id: "openai-codex-subscription-with-a-very-long-identifier",
            name: long,
            available: true,
            authenticated: true,
          },
          { id: "zai-coding-plan-unavailable", name: "Z.AI Coding Plan", available: false, authenticated: false },
          { id: "anthropic", name: "Anthropic (Claude Pro/Max)", available: true, authenticated: false },
        ],
      }),
    );
    const rows = within(picker).getAllByTestId("login-provider");
    // Two lines per row: the whole name first, then the id with the row's status, nothing truncated.
    const structure = rows.map((row) =>
      Array.from(row.children, (child) => [
        (child as HTMLElement).dataset.testid,
        Array.from(child.children.length === 0 ? [child] : child.children, (part) => [
          part.className,
          part.textContent,
        ]),
      ]),
    );
    expect(structure).toEqual([
      [
        ["login-provider-name", [["omfp-login__name", long]]],
        [
          "login-provider-meta",
          [
            ["omfp-login__id", "openai-codex-subscription-with-a-very-long-identifier"],
            ["omfp-login__mark", "logged in"],
          ],
        ],
      ],
      [
        ["login-provider-name", [["omfp-login__name", "Z.AI Coding Plan"]]],
        [
          "login-provider-meta",
          [
            ["omfp-login__id", "zai-coding-plan-unavailable"],
            ["omfp-login__note", "not available here"],
          ],
        ],
      ],
      [
        ["login-provider-name", [["omfp-login__name", "Anthropic (Claude Pro/Max)"]]],
        ["login-provider-meta", [["omfp-login__id", "anthropic"]]],
      ],
    ]);
    expect(rows.map((row) => row.getAttribute("aria-disabled") === "true")).toEqual([false, true, false]);

    // The keyboard moves over the rows, skipping the disabled one, and Enter picks the focused provider.
    act(() => (rows[0] as HTMLElement).focus());
    await test.user.keyboard("{ArrowDown}");
    expect(document.activeElement?.getAttribute("data-provider")).toBe("anthropic");
    await test.user.keyboard("{Enter}");
    expect(test.client.callsTo("auth.login").map((call) => call.params)).toEqual([{ providerId: "anthropic" }]);
    expect(screen.queryAllByTestId("login-picker").length).toBe(0);
  });

  // spec: UI-380
  test("a login_url request opens the page and shows its toast with Open, Copy and the instructions until the login ends", async () => {
    const test = setup();
    const opened: unknown[][] = [];
    const copied: string[] = [];
    const open = window.open;
    window.open = (...args: unknown[]) => {
      opened.push(args);
      return null;
    };
    const clipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: (text: string) => Promise.resolve(void copied.push(text)) },
    });
    try {
      run(test, "login anthropic");
      const url = "https://claude.ai/oauth/authorize?code=true&state=s";
      request(test, "u1", {
        kind: "login_url",
        ...ANTHROPIC,
        url,
        launchUrl: "http://127.0.0.1:7420/login/1",
        instructions: "Paste the code the page shows",
      });
      expect(opened).toEqual([[url, "_blank", "noopener"]]);
      const toast = screen.getByTestId("opened-url-toast");
      expect([toast.dataset.url, toast.dataset.login]).toEqual([url, "true"]);
      expect(toast.textContent).toContain("Log in to Anthropic (Claude Pro/Max) at claude.ai");
      expect(within(toast).getByTestId("login-instructions").textContent).toBe("Paste the code the page shows");
      // Popup blockers can reject the asynchronous open. The focused paste dialog must offer the same URL:
      // the toast behind its modal overlay cannot be clicked.
      request(test, "i1", {
        kind: "login_input",
        ...ANTHROPIC,
        message: "Paste the authorization code",
        placeholder: null,
      });
      const dialog = screen.getByTestId("login-input-dialog");
      expect(within(dialog).getByTestId("login-page").textContent).toContain("claude.ai");
      await test.user.click(within(dialog).getByTestId("login-page-open"));
      await test.user.click(within(dialog).getByTestId("login-page-copy"));
      expect([opened.length, copied]).toEqual([2, [url]]);
      await test.user.click(within(toast).getByTestId("opened-url-open"));
      await test.user.click(within(toast).getByTestId("opened-url-copy"));
      expect([opened.length, copied]).toEqual([3, [url, url]]);
      // A login_url is a notice: never answered.
      expect(answers(test)).toEqual([]);

      const [login] = test.client.callsTo("auth.login");
      await settle(() => login?.resolve(ANTHROPIC));
      expect(screen.queryAllByTestId("opened-url-toast").length).toBe(0);
    } finally {
      window.open = open;
      if (clipboard === undefined) {
        Reflect.deleteProperty(navigator, "clipboard");
      } else {
        Object.defineProperty(navigator, "clipboard", clipboard);
      }
    }
  });

  // spec: UI-761
  test("a login page returning to omp's loopback listener takes the address the browser ended on, for the login", async () => {
    const test = setup();
    const open = window.open;
    window.open = () => null;
    try {
      run(test, "login openai-codex");
      const url = `https://auth.openai.com/oauth/authorize?state=s&redirect_uri=${encodeURIComponent(
        "http://localhost:1455/auth/callback",
      )}`;
      request(test, "u1", { kind: "login_url", ...ANTHROPIC, url, launchUrl: null, instructions: null });
      const toast = screen.getByTestId("opened-url-toast");
      const pasted = "http://localhost:1455/auth/callback?code=c0de&state=s";
      await test.user.type(within(toast).getByTestId("oauth-callback-input"), pasted);
      await test.user.click(within(toast).getByTestId("oauth-callback-send"));
      const [relay] = test.client.callsTo("oauth.callback");
      // No thread: the daemon relays it to the running login's sign-in.
      expect(relay?.params).toEqual({ url: pasted });
      await settle(() => relay?.resolve({ status: 200 }));
      expect(screen.queryAllByTestId("opened-url-toast").length).toBe(0);
      // The login itself goes on until omp stored the credential.
      expect(screen.queryAllByTestId("login-progress").length).toBe(1);
    } finally {
      window.open = open;
    }
  });

  // spec: UI-640
  test("a login this window did not start shows its progress and its page from the index, until it ends", async () => {
    const test = setup();
    const opened: unknown[][] = [];
    const copied: string[] = [];
    const open = window.open;
    window.open = (...args: unknown[]) => {
      opened.push(args);
      return null;
    };
    const clipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: (text: string) => Promise.resolve(void copied.push(text)) },
    });
    const url = "https://auth.kimi.com/device?code=ABCD";
    const changed = (event: IndexEvent) =>
      act(() => {
        test.client.indexEvents(event);
        test.flush();
      });
    const kimi = { providerId: "kimi-code", name: "Kimi Code", url: null, instructions: null, progress: null };
    try {
      // An agent's `auth.login`: nothing of it was started here.
      changed({ kind: "login_changed", login: kimi });
      const progress = screen.getByTestId("login-progress");
      expect([progress.dataset.provider, progress.dataset.own]).toEqual(["kimi-code", "false"]);
      expect(progress.textContent).toContain("Logging in to Kimi Code… finish in your browser");
      expect(screen.queryAllByTestId("opened-url-toast").length).toBe(0);

      // The focused window gets the flow's requests: it opens the page, whose one toast comes from the index.
      request(test, "u1", { kind: "login_url", ...kimi, url, launchUrl: null, instructions: "Enter ABCD" });
      changed({ kind: "login_changed", login: { ...kimi, url, instructions: "Enter ABCD" } });
      request(test, "p1", { kind: "login_progress", ...kimi, message: "Waiting…" });
      changed({ kind: "login_changed", login: { ...kimi, url, instructions: "Enter ABCD", progress: "Waiting…" } });
      expect(opened).toEqual([[url, "_blank", "noopener"]]);
      const toasts = screen.getAllByTestId("opened-url-toast");
      expect(toasts.map((toast) => [toast.dataset.url, toast.dataset.login])).toEqual([[url, "true"]]);
      const toast = toasts[0] as HTMLElement;
      expect(toast.textContent).toContain("Log in to Kimi Code at auth.kimi.com");
      expect(within(toast).getByTestId("login-instructions").textContent).toBe("Enter ABCD");
      expect(screen.getByTestId("login-progress-message").textContent).toBe("Waiting…");
      await test.user.click(within(toast).getByTestId("opened-url-open"));
      await test.user.click(within(toast).getByTestId("opened-url-copy"));
      expect([opened.length, copied]).toEqual([2, [url]]);

      // Its paste dialog goes when it ends; so do its toasts.
      request(test, "i1", { kind: "login_input", ...kimi, message: "Paste the code", placeholder: null });
      expect(screen.queryAllByTestId("login-input-dialog").length).toBe(1);
      changed({ kind: "login_changed", login: null });
      await waitFor(() => expect(screen.queryAllByTestId("login-input-dialog").length).toBe(0));
      expect(screen.queryAllByTestId("login-progress").length).toBe(0);
      expect(screen.queryAllByTestId("opened-url-toast").length).toBe(0);
      expect(answers(test)).toEqual([]);

      // Cancel works for it too, and a dismissed page stays dismissed.
      changed({ kind: "login_changed", login: { ...kimi, url } });
      await test.user.click(within(screen.getByTestId("login-progress")).getByTestId("login-cancel"));
      expect(test.client.callsTo("auth.login_cancel").length).toBe(1);
      await test.user.click(within(screen.getByTestId("opened-url-toast")).getByRole("button", { name: "Dismiss" }));
      expect(screen.queryAllByTestId("opened-url-toast").length).toBe(0);

      // A login this window runs shows only its own toasts.
      changed({ kind: "login_changed", login: { ...ANTHROPIC, url, instructions: null, progress: null } });
      run(test, "login anthropic");
      expect(screen.getAllByTestId("login-progress").map((shown) => shown.dataset.own)).toEqual(["true"]);
    } finally {
      window.open = open;
      if (clipboard === undefined) {
        Reflect.deleteProperty(navigator, "clipboard");
      } else {
        Object.defineProperty(navigator, "clipboard", clipboard);
      }
    }
  });

  // spec: UI-384
  test("a login_input request asks for the code: Enter answers it, Escape or Cancel answers null", async () => {
    const test = setup();
    run(test, "login anthropic");
    const asked = { kind: "login_input", ...ANTHROPIC, message: "Paste the authorization code" } as const;
    request(test, "i1", { ...asked, placeholder: "code#state" });
    const dialog = await screen.findByTestId("login-input-dialog");
    expect(within(dialog).getByRole("heading").textContent).toBe("Log in to Anthropic (Claude Pro/Max)");
    const field = within(dialog).getByTestId("login-input");
    expect([field.getAttribute("placeholder"), document.activeElement === field]).toEqual(["code#state", true]);
    await test.user.type(field, "abc#xyz{Enter}");
    expect(answers(test)).toEqual([{ requestId: "i1", answer: { kind: "login_input", value: "abc#xyz" } }]);
    expect(screen.queryAllByTestId("login-input-dialog").length).toBe(0);

    request(test, "i2", asked);
    await screen.findByTestId("login-input-dialog");
    await test.user.keyboard("{Escape}");
    request(test, "i3", asked);
    await test.user.click(await screen.findByTestId("login-input-cancel"));
    expect(answers(test).slice(1)).toEqual([
      { requestId: "i2", answer: { kind: "login_input", value: null } },
      { requestId: "i3", answer: { kind: "login_input", value: null } },
    ]);
    // Cancelling the input cancels the login: its rejection is no error.
    const [login] = test.client.callsTo("auth.login");
    await settle(() => login?.reject(new Error("Login cancelled")));
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([]);
  });

  // spec: UI-385
  test("a login that worked says so and bumps the auth revision; one that failed is an error toast", async () => {
    const test = setup();
    run(test, "login anthropic");
    const [first] = test.client.callsTo("auth.login");
    await settle(() => first?.resolve(ANTHROPIC));
    const success = screen.getByTestId("success");
    expect([success.textContent, success.dataset.tone]).toEqual(["Logged in to Anthropic (Claude Pro/Max)", "success"]);
    expect(test.host.window.get(AUTH_REVISION)).toBe(1);
    expect(screen.queryAllByTestId("login-progress").length).toBe(0);

    run(test, "login openai-codex");
    const [, second] = test.client.callsTo("auth.login");
    await settle(() =>
      second?.reject(
        new CallError({ kind: "daemon", error: { code: "omp_error", message: "Token exchange failed: 400" } }),
      ),
    );
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["Token exchange failed: 400 (omp_error)"]);
    expect(test.host.window.get(AUTH_REVISION)).toBe(1);
  });

  // spec: UI-386, UI-389
  test(":logout removes the provider's credentials and says what still authenticates it", async () => {
    const test = setup();
    test.client.answers.set("auth.logout", () => ({
      providerId: "anthropic",
      removed: 1,
      remainingSource: "ANTHROPIC_API_KEY",
    }));
    run(test, "logout anthropic");
    await waitFor(() => expect(screen.queryAllByTestId("success").length).toBe(1));
    expect(test.client.callsTo("auth.logout").map((call) => call.params)).toEqual([{ providerId: "anthropic" }]);
    expect(screen.getByTestId("success").textContent).toBe(
      "Logged out of anthropic: removed 1 stored credential. anthropic still authenticates with ANTHROPIC_API_KEY.",
    );
    run(test, "logout");
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["Usage: :logout <provider>"]);
  });

  // spec: UI-506
  test(":import-logins copies omp's logins with auth.import and says what it copied, that there was none, or why it failed", async () => {
    const test = setup();
    const command = test.host.commands.list().find((listed) => listed.id === "auth.import");
    expect([command?.title, command?.aliases]).toEqual(["Copy omp's logins (:import-logins)", ["import-logins"]]);
    expect(command?.description).toContain("never its configuration or MCP credentials");

    run(test, "import-logins");
    const [first] = test.client.callsTo("auth.import");
    expect(first?.params).toEqual({});
    await settle(() =>
      first?.resolve({
        providers: [
          { providerId: "anthropic", oauth: true },
          { providerId: "openai", oauth: false },
        ],
      }),
    );
    expect(test.host.window.get(SUCCESSES).map((shown) => [shown.text, shown.tone])).toEqual([
      [
        "Copied omp's logins: anthropic, openai. A browser login then lives in both apps: if one of them gets signed out, log in there again.",
        undefined,
      ],
    ]);
    expect(test.host.window.get(AUTH_REVISION)).toBe(1);

    run(test, "import-logins");
    const [, second] = test.client.callsTo("auth.import");
    await settle(() => second?.resolve({ providers: [] }));
    expect(test.host.window.get(SUCCESSES).at(-1)).toMatchObject({ text: "omp has no logins to copy", tone: "info" });
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([]);

    run(test, "import-logins");
    const [, , third] = test.client.callsTo("auth.import");
    await settle(() =>
      third?.reject(
        new CallError({ kind: "daemon", error: { code: "omp_error", message: "auth broker is configured" } }),
      ),
    );
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["auth broker is configured (omp_error)"]);

    run(test, "import-logins");
    const [, , , fourth] = test.client.callsTo("auth.import");
    await settle(() =>
      fourth?.reject(
        new CallError({
          kind: "daemon",
          error: { code: "conflict", message: "a login is running; copy omp's logins after it ends" },
        }),
      ),
    );
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
      "auth broker is configured (omp_error)",
      "a login is running; copy omp's logins after it ends (conflict)",
    ]);
    expect(test.host.window.get(AUTH_REVISION)).toBe(1);
  });

  // spec: UI-381, UI-387, UI-451
  test("an error toast with omp's no-model stderr says so in short, keeps omp's words under Details and offers Log in", async () => {
    const test = setup();
    act(() =>
      reportError(
        test.host,
        new CallError({
          kind: "daemon",
          error: {
            code: "thread_failed",
            message: "omp failed to start: server closed",
            details: { stderrTail: NO_DEFAULT },
          },
        }),
      ),
    );
    act(() => reportError(test.host, new Error("git failed")));
    const [needing, other] = screen.getAllByTestId("error");
    const text = needing?.querySelector(".omfp-toast__text");
    expect(text?.firstChild?.textContent).toBe("No model available: log in to a provider.");
    const details = within(needing as HTMLElement).getByTestId("error-details");
    expect(details.getAttribute("aria-expanded")).toBe("false");
    await test.user.click(details);
    expect(details.getAttribute("aria-expanded")).toBe("true");
    expect(within(needing as HTMLElement).getByTestId("error-raw").textContent).toBe(
      `omp failed to start: server closed (thread_failed)\n${NO_DEFAULT}`,
    );
    expect(other?.querySelector(".omfp-toast__text")?.textContent).toBe("git failed");
    expect(other !== undefined && within(other).queryAllByTestId("error-login").length).toBe(0);
    await test.user.click(within(needing as HTMLElement).getByTestId("error-login"));
    await screen.findByTestId("login-picker");
    expect(test.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["git failed"]);
    expect(test.client.callsTo("auth.providers").length).toBe(1);
  });

  // spec: UI-380
  test("the agents' window requests leave the login's requests unanswered", () => {
    const test = setup();
    request(test, "x1", { kind: "login_progress", ...ANTHROPIC, message: "Waiting" });
    request(test, "x2", { kind: "login_input", ...ANTHROPIC, message: "Paste the code", placeholder: null });
    expect(answers(test)).toEqual([]);
  });
});
