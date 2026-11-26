/** L1: which live voice path a window takes (UI-188); whether the voice is behind the chat (UI-199). */

import { describe, expect, test } from "bun:test";
import type { AgentMessage, ThreadState } from "@omfp/core";
import {
  contextChunks,
  IDLE,
  type LocalLive,
  liveBehind,
  liveChatUpdate,
  livePanelView,
  liveSupport,
} from "../../src/vm/live.ts";

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

  // spec: UI-199, UI-213
  test("this window's own live_connect session is never behind: the window tells it of the chat", () => {
    const typed = state([{ role: "user", content: "and the logout?" }]);
    const running = (mode: "peer" | "host"): LocalLive => ({
      kind: "running",
      threadId: "t1",
      mode,
      connected: true,
      speaking: false,
      muted: false,
    });
    const peer = livePanelView("t1", typed, running("peer"), 2, undefined);
    expect(peer.kind === "own" && peer.behind).toBe(false);
    const host = livePanelView("t1", typed, running("host"), 2, undefined);
    expect(host.kind === "own" && host.behind).toBe(true);
  });
});

describe("what the window tells the voice of the chat", () => {
  const entry = (message: AgentMessage) => ({ message_id: null, message });
  const assistant = (text: string): AgentMessage => ({ role: "assistant", content: [{ type: "text", text }] });

  // spec: UI-213
  test("typed prompts and the agent's text, without the turns the voice started", () => {
    const transcript = [
      entry({ role: "user", content: "known before" }),
      entry({ role: "custom", customType: "live-delegation", content: "run the tests" }),
      entry(assistant("Tests pass.")),
      entry({ role: "user", content: "a reminder", synthetic: true }),
      entry({ role: "user", content: [{ type: "text", text: " and the logout? " }] }),
      entry({ role: "toolResult", content: [{ type: "text", text: "tool output" }] }),
      entry(assistant("")),
      entry(assistant("Logout works.")),
    ];

    const update = liveChatUpdate(transcript, 1) ?? "";

    expect(update.startsWith("Chat update, typed in the thread since your last context and not said to you")).toBe(
      true,
    );
    expect(update.endsWith("\nUser: and the logout?\nAgent: Logout works.\n")).toBe(true);
    for (const left of ["Tests pass.", "known before", "tool output", "a reminder"]) {
      expect(update.includes(left)).toBe(false);
    }
    expect(liveChatUpdate(transcript.slice(0, 4), 1)).toBeUndefined();
    expect(liveChatUpdate(transcript, transcript.length)).toBeUndefined();
  });

  // spec: UI-213
  test("context comes in appends of at most 500 UTF-8 bytes, cut at line ends where a line fits", () => {
    const text = `short line\n${"é".repeat(400)}\nlast\n`;

    const chunks = contextChunks(text);

    expect(chunks.join("")).toBe(text);
    expect(chunks.every((chunk) => new TextEncoder().encode(chunk).length <= 500)).toBe(true);
    expect(chunks[0]).toBe("short line\n");
    expect(chunks.length).toBeGreaterThanOrEqual(3);
    expect(contextChunks("")).toEqual([]);
  });
});
