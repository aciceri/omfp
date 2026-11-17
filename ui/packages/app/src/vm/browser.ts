/**
 * The `omfp:browser` widget (UI-870..UI-873): the browser tabs the selected thread's agent drove, read from its `eval`
 * calls. omp 18.22.0 runs its browser as an eval prelude (`browser.open({ url })`, `tab.click(…)`), so an RPC host
 * sees nothing but the eval tool's results: each cell's status events, where a successful browser call is
 * `{ op: "browser", detail }` with omp's one-line description of it (`describeBrowserCall` of its
 * `tools/browser.ts`: `open main https://…`, `main.click("#go")`, `main.goto("https://…")`, `close main`,
 * `close all`, `tabs`), and the call's images, the browser's screenshots among them marked `detail: "original"`. omp
 * sends no browser event stream and no CDP endpoint of its Chromium: this is what the calls reported, not the live
 * page.
 */

import type { JsonValue, ThreadState, ToolRun } from "@omfp/core";
import { isObject as isRecord } from "../sdk/json.ts";

/** An image of a tool result: base64 `data` of type `mime`. */
export interface BrowserScreenshot {
  readonly mime: string;
  readonly data: string;
}

/** One tab the agent drove, as its calls left it. */
export interface BrowserTab {
  /** The tab's name (`browser.open({ name })`, `main` by default). */
  readonly name: string;
  /** Where its last `open` landed or its last `goto` sent it; `undefined` before either. */
  readonly url: string | undefined;
  /** omp's description of the last browser call on the tab, as omp wrote it. */
  readonly action: string;
  /** The agent closed it (`close <name>` or `close all`) and did not use it again. */
  readonly closed: boolean;
  /** The last screenshot its calls produced (UI-872). */
  readonly screenshot: BrowserScreenshot | undefined;
}

export interface BrowserView {
  /** The tabs, the most recently used first. */
  readonly tabs: readonly BrowserTab[];
}

/** One browser call of a status event. */
type BrowserCall =
  | { readonly kind: "open"; readonly name: string; readonly url: string | undefined }
  | { readonly kind: "close"; readonly name: string }
  | { readonly kind: "closeAll" }
  | { readonly kind: "call"; readonly name: string; readonly chain: string };

/** An absolute address at the end of an `open` line (`https://…`, `about:blank`, `file:///…`). */
const OPEN_URL = /^(.+) ([a-z][a-z0-9+.-]*:\S*)$/i;
/** `<name>.<method>(`: the tab of a call chain, the shortest prefix before a method call. */
const CALL = /^(.+?)\.[A-Za-z_$][\w$]*\(/;
/** A chain starting with `goto("<url>")`. */
const GOTO = /^goto\(("(?:[^"\\]|\\.)*")/;
/** A chain that can take a screenshot: a `screenshot(` step (of the tab or a frame), or code run in the tab. */
const CAPTURES = /(?:^|\.)(?:screenshot|run)\(/;

/** The browser call omp's status line `detail` describes; `undefined` for `tabs`, `targets` and unknown lines. */
function browserCall(detail: string): BrowserCall | undefined {
  if (detail === "close all") {
    return { kind: "closeAll" };
  }
  if (detail.startsWith("close ")) {
    return { kind: "close", name: detail.slice("close ".length) };
  }
  if (detail.startsWith("open ")) {
    const rest = detail.slice("open ".length);
    const match = OPEN_URL.exec(rest);
    return { kind: "open", name: match?.[1] ?? rest, url: match?.[2] };
  }
  const match = CALL.exec(detail);
  if (match?.[1] === undefined) {
    return undefined;
  }
  return { kind: "call", name: match[1], chain: detail.slice(match[1].length + 1) };
}

/** The address of a chain starting with `goto("<url>")`. */
function gotoUrl(chain: string): string | undefined {
  const literal = GOTO.exec(chain)?.[1];
  if (literal === undefined) {
    return undefined;
  }
  try {
    const url: unknown = JSON.parse(literal);
    return typeof url === "string" ? url : undefined;
  } catch {
    return undefined;
  }
}

function records(value: unknown): readonly Readonly<Record<string, unknown>>[] {
  return Array.isArray(value) ? value.filter(isRecord) : [];
}

/** What one eval call says about the browser: its browser status lines in order and whether it drove the computer. */
function statusLines(details: Readonly<Record<string, unknown>>): { browser: string[]; computer: boolean } {
  const browser: string[] = [];
  let computer = false;
  for (const cell of records(details.cells)) {
    for (const event of records(cell.statusEvents)) {
      if (event.op === "browser" && typeof event.detail === "string") {
        browser.push(event.detail);
      } else if (event.op === "computer") {
        computer = true;
      }
    }
  }
  return { browser, computer };
}

/**
 * The last screenshot-sized image of an eval result (`detail: "original"`, which omp gives the browser's screenshots
 * and the computer's frames only): in its content once it ended, in `details.images` while it runs.
 */
function lastScreenshot(
  result: Readonly<Record<string, unknown>>,
  details: Readonly<Record<string, unknown>>,
): BrowserScreenshot | undefined {
  const content = records(result.content).filter((part) => part.type === "image");
  const images = content.length > 0 ? content : records(details.images);
  for (const image of images.toReversed()) {
    if (image.detail === "original" && typeof image.data === "string" && image.data !== "") {
      return { mime: typeof image.mimeType === "string" ? image.mimeType : "image/png", data: image.data };
    }
  }
  return undefined;
}

/**
 * The `eval` calls of the thread in order, each with its result (or its latest partial result while it runs). Runs
 * the transcript does not call are added only while in flight: a finished one belongs to a branch the session left
 * (`history` replaces the transcript, not the runs).
 */
function evalResults(state: Pick<ThreadState, "transcript" | "tools">): JsonValue[] {
  const results = new Map<string, unknown>();
  const called: string[] = [];
  for (const { message } of state.transcript) {
    if (message.role === "assistant") {
      for (const block of message.content ?? []) {
        if (block.type === "toolCall" && block.name === "eval" && block.id !== undefined) {
          called.push(block.id);
        }
      }
    } else if (message.role === "toolResult" && message.toolCallId !== undefined && !results.has(message.toolCallId)) {
      results.set(message.toolCallId, message);
    }
  }
  const runs = new Map<string, ToolRun>();
  for (const run of state.tools) {
    if (run.tool_name === "eval" && !runs.has(run.tool_call_id)) {
      runs.set(run.tool_call_id, run);
    }
  }
  const seen = new Set(called);
  const inFlight = [...runs.values()].filter(
    (run) => !seen.has(run.tool_call_id) && (run.status === "running" || run.status === "pending"),
  );
  const ids = [...called, ...inFlight.map((run) => run.tool_call_id)];
  return ids.flatMap((id) => {
    const run = runs.get(id);
    const result = results.get(id) ?? run?.result ?? run?.partial_result ?? null;
    return result === null ? [] : [result as JsonValue];
  });
}

interface TabState {
  name: string;
  url: string | undefined;
  action: string;
  closed: boolean;
  screenshot: BrowserScreenshot | undefined;
  /** When it was last used: the position of its last call among all of them. */
  used: number;
}

/**
 * The browser tabs of a thread from its `eval` calls (UI-870, UI-871): a tab appears with its first `open` or call,
 * takes the address of each `open` and `goto`, and closes with `close <name>` or `close all`; failed browser calls
 * report no tab and change nothing. A call's last screenshot belongs to a tab only when every browser call of that
 * eval call that can take one names that same tab and it did not drive the computer (UI-872).
 */
export function browserView(state: Pick<ThreadState, "transcript" | "tools">): BrowserView {
  const tabs = new Map<string, TabState>();
  let used = 0;
  const tab = (name: string, action: string): TabState => {
    const found = tabs.get(name);
    const current = found ?? { name, url: undefined, action, closed: false, screenshot: undefined, used };
    current.action = action;
    current.closed = false;
    current.used = used;
    if (found === undefined) {
      tabs.set(name, current);
    }
    return current;
  };
  for (const result of evalResults(state)) {
    if (!isRecord(result)) {
      continue;
    }
    const details = isRecord(result.details) ? result.details : {};
    const { browser, computer } = statusLines(details);
    const capturing = new Set<string>();
    for (const detail of browser) {
      const call = browserCall(detail);
      if (call === undefined) {
        continue;
      }
      used += 1;
      switch (call.kind) {
        case "open": {
          const opened = tab(call.name, detail);
          opened.url = call.url ?? opened.url;
          break;
        }
        case "close": {
          const closed = tabs.get(call.name);
          if (closed !== undefined) {
            Object.assign(closed, { action: detail, closed: true, used });
          }
          break;
        }
        case "closeAll":
          for (const closed of tabs.values()) {
            if (!closed.closed) {
              Object.assign(closed, { action: detail, closed: true, used });
            }
          }
          break;
        case "call": {
          const driven = tab(call.name, detail);
          driven.url = gotoUrl(call.chain) ?? driven.url;
          if (CAPTURES.test(call.chain)) {
            capturing.add(call.name);
          }
          break;
        }
      }
    }
    const [only] = capturing;
    const screenshot = capturing.size === 1 && !computer ? lastScreenshot(result, details) : undefined;
    const owner = only === undefined ? undefined : tabs.get(only);
    if (screenshot !== undefined && owner !== undefined) {
      owner.screenshot = screenshot;
    }
  }
  return {
    tabs: [...tabs.values()]
      .sort((a, b) => b.used - a.used)
      .map(({ name, url, action, closed, screenshot }) => ({ name, url, action, closed, screenshot })),
  };
}
