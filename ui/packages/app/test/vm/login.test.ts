/** L1: logging in to a provider — omp's "needs a login" texts, `:login`/`:logout` completions, outcome messages. */

import { describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import type { AuthProvider } from "@omfp/protocol";
import { errorMessage } from "../../src/sdk/actions.ts";
import {
  importedSummary,
  importOffer,
  loginCompletions,
  loginProblem,
  logoutCompletions,
  logoutSummary,
  needsLogin,
  providerName,
  withAuthenticated,
} from "../../src/vm/login.ts";

const NO_DEFAULT =
  "No default model selected. Use /login, set an API key environment variable, or select a local model with /model or --model.";

const PROVIDERS: AuthProvider[] = [
  { id: "anthropic", name: "Anthropic (Claude Pro/Max)", available: true, authenticated: true },
  { id: "openai-codex", name: "ChatGPT Plus/Pro (Codex)", available: true, authenticated: false },
  { id: "github-copilot", name: "GitHub Copilot", available: false, authenticated: false },
];

describe("needsLogin", () => {
  // spec: UI-381
  test("matches omp's words for no usable model or credential, and nothing else", () => {
    const matching = [
      NO_DEFAULT,
      "No API key found for anthropic.\n\nUse /login, set an API key environment variable, or create /home/u/.omp/agent/agent.db",
      "No model selected.\n\nUse /login, set an API key environment variable, or create /home/u/.omp/agent/agent.db",
      "No models available.",
      "No model available matching enabledModels (anthropic/*) with usable credentials. Use /login …",
      `omp failed to start: server closed (thread_failed)\n${NO_DEFAULT}`,
      "use /login to sign in",
    ];
    const other = [
      "omp failed to start: server closed (thread_failed)",
      "error: unknown option --approval-mode=never",
      "fetch failed: https://example.com/login",
      "",
    ];
    expect(matching.map(needsLogin)).toEqual(matching.map(() => true));
    expect(other.map(needsLogin)).toEqual(other.map(() => false));
    expect([needsLogin(undefined), needsLogin(null)]).toEqual([false, false]);
  });

  // spec: UI-381
  test("a daemon error's message carries omp's stderr tail on the lines after", () => {
    const failed = new CallError({
      kind: "daemon",
      error: {
        code: "thread_failed",
        message: "omp failed to start: server closed",
        details: { stderrTail: `\n${NO_DEFAULT}\n` },
      },
    });
    expect(errorMessage(failed)).toBe(`omp failed to start: server closed (thread_failed)\n${NO_DEFAULT}`);
    expect(needsLogin(errorMessage(failed))).toBe(true);
    const bare = new CallError({ kind: "daemon", error: { code: "not_found", message: "no thread" } });
    expect(errorMessage(bare)).toBe("no thread (not_found)");
    const blank = new CallError({
      kind: "daemon",
      error: { code: "thread_failed", message: "omp failed", details: { stderrTail: "  " } },
    });
    expect(errorMessage(blank)).toBe("omp failed (thread_failed)");
    expect([errorMessage(new Error("x")), errorMessage("plain")]).toEqual(["x", "plain"]);
  });

  // spec: UI-451
  test("omp's login errors read as one short sentence without its CLI advice", () => {
    const keyless =
      "No API key found for anthropic.\n\nUse /login, set an API key environment variable, or create /home/u/.omp/agent/agent.db";
    expect([
      loginProblem(`omp failed to start: omp did not become ready: server closed (thread_failed)\n${NO_DEFAULT}`),
      loginProblem(`prompt failed (agent_error)\n${keyless}`),
      loginProblem("No models available."),
      loginProblem("omp failed to start: server closed (thread_failed)"),
      loginProblem(undefined),
    ]).toEqual([
      "No model available: log in to a provider.",
      "No credential for anthropic: log in to use its models.",
      "No model available: log in to a provider.",
      undefined,
      undefined,
    ]);
  });
});

describe("login completions and outcomes", () => {
  // spec: UI-386
  test(":login completes the available providers, :logout the authenticated ones, first argument only", () => {
    expect(loginCompletions(PROVIDERS, [])).toEqual(["anthropic", "openai-codex"]);
    expect(loginCompletions(PROVIDERS, ["anthropic"])).toEqual([]);
    expect(loginCompletions(undefined, [])).toEqual([]);
    expect(logoutCompletions(PROVIDERS, [])).toEqual(["anthropic"]);
    expect(logoutCompletions(PROVIDERS, ["anthropic"])).toEqual([]);
    expect(logoutCompletions(undefined, [])).toEqual([]);
  });

  // spec: UI-386
  test("a logout says what omp removed and what still authenticates the provider", () => {
    const name = "Anthropic (Claude Pro/Max)";
    expect(logoutSummary({ providerId: "anthropic", removed: 1, remainingSource: null }, name)).toBe(
      "Logged out of Anthropic (Claude Pro/Max): removed 1 stored credential.",
    );
    expect(logoutSummary({ providerId: "anthropic", removed: 2 }, name)).toBe(
      "Logged out of Anthropic (Claude Pro/Max): removed 2 stored credentials.",
    );
    expect(logoutSummary({ providerId: "anthropic", removed: 0, remainingSource: "ANTHROPIC_API_KEY" }, name)).toBe(
      "omp held no stored credential of Anthropic (Claude Pro/Max). Anthropic (Claude Pro/Max) still authenticates with ANTHROPIC_API_KEY.",
    );
  });

  // spec: UI-382, UI-385
  test("provider names fall back to the id, and a login or logout updates the authenticated mark", () => {
    expect([
      providerName(PROVIDERS, "openai-codex"),
      providerName(PROVIDERS, "zai"),
      providerName(undefined, "x"),
    ]).toEqual(["ChatGPT Plus/Pro (Codex)", "zai", "x"]);
    expect(withAuthenticated(PROVIDERS, "openai-codex", true).map((provider) => provider.authenticated)).toEqual([
      true,
      true,
      false,
    ]);
  });

  // spec: UI-505, UI-506
  test("copying omp's logins names their providers and adds the OAuth caveat only for browser logins", () => {
    const keys = [
      { providerId: "anthropic", oauth: false },
      { providerId: "openai", oauth: false },
    ];
    const caveat = "A browser login then lives in both apps: if one of them gets signed out, log in there again.";
    expect(importOffer(keys)).toEqual({
      button: "Use my omp logins (anthropic, openai)",
      note: "Copies only omp's logins, nothing else.",
    });
    expect(importedSummary(keys)).toBe("Copied omp's logins: anthropic, openai");
    const browser = [{ providerId: "openai-codex", oauth: true }];
    expect(importOffer(browser)).toEqual({
      button: "Use my omp logins (openai-codex)",
      note: `Copies only omp's logins, nothing else. ${caveat}`,
    });
    expect(importedSummary(browser)).toBe(`Copied omp's logins: openai-codex. ${caveat}`);
  });
});
