/**
 * The URLs omp opened in a browser, or asks the client to open (UI-346): an `open_url` request (`login`) names its
 * URL; a command's output saying it opens the browser (`/mcp reauth`'s "Opening your browser to authorize. If it did
 * not open:" and the like) names it on the lines after. omp opens them on the daemon's host, so a window elsewhere
 * offers them again.
 */

import type { NoticeFrame, ThreadState } from "@omfp/core";

/** "opens/opened/opening … browser", "open … in your browser": the phrase before the URL, on one line. */
const BROWSER_PHRASE = /\bopen(?:s|ed|ing)?\b[^\n]*?\bbrowsers?\b/gi;

const URL_PATTERN = /\bhttps?:\/\/[^\s<>"'`]+/i;

/** Sentence punctuation a URL at the end of a sentence carries. */
const TRAILING = /[.,;:!?)\]}]+$/;

/** The URLs `frame` says omp opened or asks to open, in order, each once; `[]` for other notices. */
export function openedUrls(frame: NoticeFrame): readonly string[] {
  switch (frame.type) {
    case "open_url":
      return [frame.data.url];
    case "command_output":
      return urlsAfterBrowserPhrase(frame.data.text);
    default:
      return [];
  }
}

/** The first URL after each mention of opening a browser. */
export function urlsAfterBrowserPhrase(text: string): readonly string[] {
  const urls: string[] = [];
  for (const phrase of text.matchAll(BROWSER_PHRASE)) {
    const found = URL_PATTERN.exec(text.slice(phrase.index + phrase[0].length));
    const url = found?.[0].replace(TRAILING, "");
    if (url !== undefined && url !== "" && !urls.includes(url)) {
      urls.push(url);
    }
  }
  return urls;
}

/** {@link openedUrls} per frame: the states of a thread share their notices' frames. */
const KNOWN = new WeakMap<NoticeFrame, readonly string[]>();

function cachedUrls(frame: NoticeFrame): readonly string[] {
  let urls = KNOWN.get(frame);
  if (urls === undefined) {
    urls = openedUrls(frame);
    KNOWN.set(frame, urls);
  }
  return urls;
}

/** The URLs every notice of `state` says omp opened, in transcript order, each once. */
export function threadOpenedUrls(state: ThreadState): readonly string[] {
  const urls: string[] = [];
  for (const { frame } of state.notices) {
    for (const url of cachedUrls(frame)) {
      if (!urls.includes(url)) {
        urls.push(url);
      }
    }
  }
  return urls;
}

/** The host a toast names (`example.com`), the URL itself when it does not parse. */
export function urlHost(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/** The hosts omp's OAuth callback listener answers on, the only ones the daemon relays to (DMN-341). */
const LOOPBACK_HOSTS: readonly string[] = ["localhost", "127.0.0.1"];

/**
 * The loopback callback an authorization URL omp opened sends the browser back to (UI-761): its `redirect_uri` when
 * that is a plain `http` address on `localhost` or `127.0.0.1`, as `http://localhost:<port><path>`; `undefined` for
 * any other URL (a device page, a non-loopback redirect), where pasting the address would not help.
 */
export function loopbackCallback(url: string): string | undefined {
  let redirect: URL;
  try {
    const raw = new URL(url).searchParams.get("redirect_uri");
    if (raw === null) {
      return undefined;
    }
    redirect = new URL(raw);
  } catch {
    return undefined;
  }
  if (
    redirect.protocol !== "http:" ||
    redirect.username !== "" ||
    redirect.password !== "" ||
    !LOOPBACK_HOSTS.includes(redirect.hostname)
  ) {
    return undefined;
  }
  return `http://localhost:${redirect.port === "" ? "80" : redirect.port}${redirect.pathname}`;
}

/**
 * Which opened URLs are new to a window (UI-346): those present when the window first sees a thread's state are not,
 * and each URL of a thread is new once.
 */
export class OpenedUrlWatch {
  readonly #seen = new Map<string, Set<string>>();

  /** The URLs of `threadId`'s `state` this watch has not reported yet; none on the first look at the thread. */
  take(threadId: string, state: ThreadState): readonly string[] {
    const urls = threadOpenedUrls(state);
    const seen = this.#seen.get(threadId);
    if (seen === undefined) {
      this.#seen.set(threadId, new Set(urls));
      return [];
    }
    const fresh = urls.filter((url) => !seen.has(url));
    for (const url of fresh) {
      seen.add(url);
    }
    return fresh;
  }
}
