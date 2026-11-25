/**
 * `eval` calls driving omp's browser, as omp 19.2.0 sends them to an RPC host (UI-870..UI-872). The recorded fixtures
 * cannot hold one: the contract tests run omp behind `DenyProxy`, where its Chromium can be neither downloaded nor
 * reached. The shapes follow omp's sources: each cell's `statusEvents` hold `{ op: "browser", detail }` from
 * `describeBrowserCall` (`tools/browser.ts`) and `{ op, error }` for a failed call (`eval/js/tool-bridge.ts`); a
 * screenshot is an image of the call's content marked `detail: "original"` (`tools/browser/screenshot.ts`), in
 * `details.images` while the call runs (`tools/eval.ts`).
 */

import type { JsonObject, ThreadEvent } from "@omfp/core";

/** A status event of an eval cell. */
export type StatusEvent = { op: string; detail: string } | { op: string; error: string };

/** A screenshot as an eval result carries it: base64 `data` (here a short tag) marked `detail: "original"`. */
export function screenshot(tag: string): JsonObject {
  return { type: "image", data: btoa(tag), mimeType: "image/png", detail: "original" };
}

function omp(frame: JsonObject): ThreadEvent {
  return { kind: "omp", frame } as ThreadEvent;
}

function details(events: readonly StatusEvent[], images?: readonly JsonObject[]): JsonObject {
  const cell = { index: 0, code: "", language: "js", output: "", status: "complete", statusEvents: [...events] };
  return images === undefined
    ? { language: "js", cells: [cell] }
    : { language: "js", cells: [cell], images: [...images] };
}

/** The assistant message calling `eval` with id `id`, and the start of that call. */
function started(id: string): ThreadEvent[] {
  const call = { type: "toolCall", id, name: "eval", arguments: { language: "js", code: "await browser.open()" } };
  return [
    omp({ type: "message_end", messageId: `msg-${id}`, message: { role: "assistant", content: [call] } }),
    omp({ type: "tool_execution_start", toolCallId: id, toolName: "eval", args: call.arguments }),
  ];
}

/** An `eval` call that ended: its status events and the images of its content. */
export function evalCall(
  id: string,
  events: readonly StatusEvent[],
  images: readonly JsonObject[] = [],
): ThreadEvent[] {
  const result = { content: [{ type: "text", text: "(output)" }, ...images], details: details(events) };
  return [
    ...started(id),
    omp({ type: "tool_execution_end", toolCallId: id, toolName: "eval", result, isError: false }),
    omp({
      type: "message_end",
      messageId: `msg-${id}-result`,
      message: { role: "toolResult", toolCallId: id, toolName: "eval", isError: false, ...result },
    }),
  ];
}

/** An `eval` call still running: its partial result so far, the images in `details.images`. */
export function runningEvalCall(
  id: string,
  events: readonly StatusEvent[],
  images: readonly JsonObject[] = [],
): ThreadEvent[] {
  const partialResult = { content: [{ type: "text", text: "(output so far)" }], details: details(events, images) };
  return [
    ...started(id),
    omp({ type: "tool_execution_update", toolCallId: id, toolName: "eval", args: {}, partialResult }),
  ];
}

/**
 * A session's browser work: `main` opened and shot; `docs` opened and sent elsewhere while `main` is clicked and one
 * call fails; then both shot in one call (which screenshot is whose is unknown); a `bash` call between; `main`
 * closed; and `docs` being shot by a call that still runs.
 */
export function browserSession(): ThreadEvent[] {
  return [
    ...evalCall(
      "call_eval_1",
      [
        { op: "browser", detail: "open main https://example.com/" },
        { op: "browser", detail: "main.screenshot()" },
      ],
      [screenshot("main-1")],
    ),
    ...evalCall("call_eval_2", [
      { op: "browser", detail: "open docs https://docs.example.com/start" },
      { op: "browser", detail: 'docs.goto("https://docs.example.com/api")' },
      { op: "browser", detail: 'main.click("#more")' },
      { op: "browser", error: "Timed out waiting for selector #missing" },
      { op: "browser", detail: "tabs" },
    ]),
    ...evalCall(
      "call_eval_3",
      [
        { op: "browser", detail: "main.screenshot()" },
        { op: "browser", detail: "docs.screenshot()" },
      ],
      [screenshot("main-2"), screenshot("docs-1")],
    ),
    omp({
      type: "message_end",
      messageId: "msg-bash",
      message: { role: "assistant", content: [{ type: "toolCall", id: "call_bash", name: "bash", arguments: {} }] },
    }),
    ...evalCall("call_eval_4", [{ op: "browser", detail: "close main" }]),
    ...runningEvalCall(
      "call_eval_5",
      [{ op: "browser", detail: 'docs.screenshot({"fullPage":true})' }],
      [screenshot("docs-2")],
    ),
  ];
}
