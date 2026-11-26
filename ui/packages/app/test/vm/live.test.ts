/** L1: which live voice path a window takes (UI-188); whether the voice is behind the chat (UI-199). */

import { describe, expect, test } from "bun:test";
import type { AgentMessage, ThreadState } from "@omfp/core";
import { IDLE, liveBehind, livePanelView, liveSupport } from "../../src/vm/live.ts";

describe("live support", () => {
  // spec: UI-188, UI-180
  test("WebRTC where it works, omp's own session on the daemon's machine, else why not", () => {
    const at = (secure: boolean, webrtc: boolean, local: boolean) => liveSupport({ secure, webrtc, local });
    // A browser on localhost or HTTPS, wherever the daemon runs.
    expect(at(true, true, false)).toBe("webrtc");
    expect(at(true, true, true)).toBe("webrtc");
    // The desktop webview (no WebRTC) on a local daemon.
    expect(at(true, false, true)).toBe("host");
    expect(at(false, false, true)).toBe("host");
    expect(at(false, true, true)).toBe("host");
    // The desktop webview on a remote daemon (`--connect`), or a browser without WebRTC.
    expect(at(true, false, false)).toBe("unsupported");
    expect(at(false, false, false)).toBe("unsupported");
    // A browser on a LAN address.
    expect(at(false, true, false)).toBe("insecure");
  });
});

describe("the voice behind the chat", () => {
  const entry = (message: AgentMessage) => ({ message_id: null, message });
  const state = (later: AgentMessage[]): Pick<ThreadState, "live" | "live_error" | "transcript"> => ({
    live: { owner: { client: "omfp/web", connection: 2 }, phase: "listening", transcript: [], context_end: 1 },
    live_error: null,
    transcript: [entry({ role: "user", content: "fix the login" }), ...later.map(entry)],
  });

  // spec: UI-199
  test("only a prompt typed after the session started puts the voice behind", () => {
    expect(liveBehind(state([]))).toBe(false);
    expect(liveBehind(state([{ role: "custom", customType: "live-delegation", content: "run the tests" }]))).toBe(
      false,
    );
    expect(liveBehind(state([{ role: "user", content: "a reminder", synthetic: true }]))).toBe(false);
    expect(liveBehind(state([{ role: "user", content: "and the logout?" }]))).toBe(true);
    expect(liveBehind({ live: null, live_error: null, transcript: [] })).toBe(false);
  });

  // spec: UI-199
  test("another window's session shows it too", () => {
    const view = livePanelView("t1", state([{ role: "user", content: "and the logout?" }]), IDLE, 7, undefined);
    expect(view.kind === "elsewhere" && view.behind).toBe(true);
  });
});
