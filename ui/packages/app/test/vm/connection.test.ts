/** L1: the connection banner, the live-region status line and the access token (ports of the Dioxus
 * `vm/connection.rs` and `host/token.rs` tests). */

import { describe, expect, test } from "bun:test";
import type { ConnectionStatus } from "@omfp/client";
import { announcement, banner } from "../../src/vm/connection.ts";
import {
  chooseToken,
  fragmentLocal,
  fragmentToken,
  fragmentWithoutToken,
  TOKEN_KEY,
  type TokenPage,
  takeToken,
} from "../../src/vm/token.ts";
import { thread } from "../support/fixtures.ts";

const connected: ConnectionStatus = {
  kind: "connected",
  hello: {
    protocol: 1,
    daemon: "0.1.0",
    omp: "18.18.6",
    warnings: [],
    widgetBase: "/widgets/test",
    fileBase: "/files/test",
    connection: 1,
  },
};

describe("connection", () => {
  // spec: UI-003
  test("the banner shows reconnect progress and disappears once connected", () => {
    expect(banner({ kind: "reconnecting", attempt: 3, retryInMs: 4200, error: "closed" })).toEqual({
      tone: "warning",
      text: "Connection lost. Reconnecting in 5 s (attempt 3)…",
    });
    expect(banner(connected)).toBeUndefined();
    expect(banner({ kind: "connecting" })?.tone).toBe("info");
    expect(banner({ kind: "unauthenticated" })?.tone).toBe("error");
    expect(banner({ kind: "version_mismatch", supported: [2], message: "wants 2" })?.tone).toBe("error");
  });

  // spec: UI-092
  test("the announcement names the selected thread's state and the waiting threads", () => {
    const threads = [
      thread("t1", "checkout", "w", { needsAttention: true }),
      thread("t2", "flaky", "w", { needsAttention: true }),
    ];
    expect(announcement(connected, threads, "t1")).toBe("checkout: awaiting input. 2 threads await input");
    expect(announcement({ kind: "reconnecting", attempt: 1, retryInMs: 1000, error: "" }, [], undefined)).toBe(
      "Connection lost. Reconnecting in 1 s (attempt 1)…",
    );
    expect(announcement(connected, threads.slice(1), undefined)).toBe("1 thread awaits input");
  });
});

describe("token", () => {
  // spec: UI-002
  test("the token is read from the fragment and removed from it", () => {
    expect(fragmentToken("#token=abc123")).toBe("abc123");
    expect(fragmentWithoutToken("#token=abc123")).toBe("");
    expect(fragmentToken("#thread=t1&token=abc123")).toBe("abc123");
    expect(fragmentWithoutToken("#thread=t1&token=abc123")).toBe("#thread=t1");
    expect(fragmentToken("#token=")).toBeUndefined();
    expect(fragmentToken("")).toBeUndefined();
    expect(fragmentWithoutToken("")).toBe("");
    expect(fragmentToken("#tokens=abc")).toBeUndefined();
    expect(fragmentWithoutToken("#tokens=abc")).toBe("#tokens=abc");
  });

  // spec: UI-002
  test("a token from the address bar replaces the stored one", () => {
    expect(chooseToken("new", "old")).toBe("new");
    expect(chooseToken(undefined, "old")).toBe("old");
    expect(chooseToken(undefined, "  ")).toBeUndefined();
    expect(chooseToken(undefined, null)).toBeUndefined();
  });

  // spec: UI-002
  test("taking the token stores it and replaces the address without the fragment", () => {
    const stored = new Map<string, string>();
    const replaced: string[] = [];
    const page = (hash: string): TokenPage => ({
      location: { hash, pathname: "/", search: "?x=1" },
      history: { replaceState: (_data, _unused, url) => replaced.push(String(url)) },
      storage: { getItem: (key) => stored.get(key) ?? null, setItem: (key, value) => stored.set(key, value) },
    });
    expect(takeToken(page("#thread=t1&token=abc"))).toBe("abc");
    expect(stored.get(TOKEN_KEY)).toBe("abc");
    expect(replaced).toEqual(["/?x=1#thread=t1"]);
    expect(takeToken(page(""))).toBe("abc");
    expect(replaced).toHaveLength(1);
  });

  // spec: UI-188
  test("`local=1` marks a window on the daemon's machine; it stays in the address bar and is never stored", () => {
    expect(fragmentLocal("#token=abc&local=1")).toBe(true);
    expect(fragmentLocal("#local=1")).toBe(true);
    expect(fragmentLocal("#token=abc")).toBe(false);
    expect(fragmentLocal("#local=0")).toBe(false);
    expect(fragmentLocal("#local=10")).toBe(false);
    const stored = new Map<string, string>();
    const replaced: string[] = [];
    const page: TokenPage = {
      location: { hash: "#token=abc&local=1", pathname: "/", search: "" },
      history: { replaceState: (_data, _unused, url) => replaced.push(String(url)) },
      storage: { getItem: (key) => stored.get(key) ?? null, setItem: (key, value) => stored.set(key, value) },
    };
    expect(takeToken(page)).toBe("abc");
    expect(replaced).toEqual(["/#local=1"]);
    expect([...stored.keys()]).toEqual([TOKEN_KEY]);
  });
});
