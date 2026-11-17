/** L0: the URLs omp opened in a browser or asks to open (UI-346). */

import { describe, expect, test } from "bun:test";
import { emptyThreadState, type NoticeFrame, type ThreadState } from "@omfp/core";
import { loopbackCallback, OpenedUrlWatch, openedUrls, threadOpenedUrls, urlHost } from "../../src/vm/opened-url.ts";

const AUTH = "https://auth.example.com/authorize?client_id=omp&state=s1&redirect_uri=http%3A%2F%2Flocalhost%3A1455";

const output = (text: string): NoticeFrame => ({ type: "command_output", data: { text } });

/** `/mcp reauth`'s message with the URL omp opened. */
const oauth = (url: string, instructions?: string): NoticeFrame =>
  output(
    "━━━ OAuth Authorization Required ━━━\nOpening your browser to authorize. If it did not open:\n" +
      url +
      (instructions === undefined ? "" : `\n\n${instructions}`),
  );

function withNotices(...frames: NoticeFrame[]): ThreadState {
  return { ...emptyThreadState(), notices: frames.map((frame, position) => ({ position, frame })) };
}

describe("opened URLs", () => {
  // spec: UI-346
  test("an open_url request names its URL; /mcp reauth's output the URL after the browser line", () => {
    expect(openedUrls({ type: "open_url", data: { id: "1", url: "https://login.example/device" } })).toEqual([
      "https://login.example/device",
    ]);
    expect(openedUrls(oauth(AUTH, "Paste the code if asked."))).toEqual([AUTH]);
    // The first message of a reauth says it opens the browser but has no URL yet.
    expect(openedUrls(output('Reauthorizing "docs"… opening your browser (5 minute timeout).'))).toEqual([]);
  });

  // spec: UI-346
  test("the pattern is generic: any 'opens/opened your browser' line followed by a URL, sentence punctuation off", () => {
    expect(openedUrls(output("Opened https://example.org/login. in your browser"))).toEqual([]);
    expect(openedUrls(output("omp opened your browser at https://example.org/login."))).toEqual([
      "https://example.org/login",
    ]);
    expect(openedUrls(output("Open this link in your browser:\n  https://example.org/a?x=1\n"))).toEqual([
      "https://example.org/a?x=1",
    ]);
    // A URL without a browser phrase before it is just text (e.g. `/dirs` output, an MCP server's URL).
    expect(openedUrls(output("Added https://example.org/mcp as docs"))).toEqual([]);
    expect(
      openedUrls({ type: "notice", data: { level: "info", message: "Open your browser at https://x.y" } }),
    ).toEqual([]);
  });

  // spec: UI-346
  test("a thread's opened URLs come once each, in transcript order", () => {
    const state = withNotices(oauth(AUTH), output("done"), { type: "open_url", data: { id: "2", url: AUTH } });
    expect(threadOpenedUrls(state)).toEqual([AUTH]);
  });

  // spec: UI-346
  test("the watch reports a URL once, and none of those present when it first sees the thread", () => {
    const watch = new OpenedUrlWatch();
    const before = { type: "open_url", data: { id: "1", url: "https://old.example/" } } as const;
    expect(watch.take("t1", withNotices(before))).toEqual([]);
    const later = withNotices(before, oauth(AUTH));
    expect(watch.take("t1", later)).toEqual([AUTH]);
    expect(watch.take("t1", later)).toEqual([]);
    // Another thread starts its own history.
    expect(watch.take("t2", withNotices(oauth("https://other.example/")))).toEqual([]);
  });

  // spec: UI-346
  test("the host a toast names", () => {
    expect(urlHost(AUTH)).toBe("auth.example.com");
    expect(urlHost("not a url")).toBe("not a url");
  });

  // spec: UI-761
  test("an authorization URL names the loopback callback its sign-in returns to; other URLs none", () => {
    const authorize = (redirect: string) =>
      `https://auth.example.com/authorize?state=s1&redirect_uri=${encodeURIComponent(redirect)}`;
    expect(loopbackCallback(AUTH)).toBe("http://localhost:1455/");
    expect(loopbackCallback(authorize("http://localhost:1455/auth/callback"))).toBe(
      "http://localhost:1455/auth/callback",
    );
    expect(loopbackCallback(authorize("http://127.0.0.1:8080/callback"))).toBe("http://localhost:8080/callback");
    expect(loopbackCallback(authorize("http://localhost/callback"))).toBe("http://localhost:80/callback");
    for (const elsewhere of [
      "https://localhost:1455/callback",
      "http://example.com:1455/callback",
      "http://[::1]:1455/callback",
      "http://user@localhost:1455/callback",
      "vscode://gitlab.gitlab-workflow/authentication",
      "not a url",
    ]) {
      expect([elsewhere, loopbackCallback(authorize(elsewhere))]).toEqual([elsewhere, undefined]);
    }
    expect(loopbackCallback("https://login.example.com/device?code=1")).toBeUndefined();
    expect(loopbackCallback("not a url")).toBeUndefined();
  });
});
