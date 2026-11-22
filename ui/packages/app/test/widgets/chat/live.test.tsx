/**
 * L4: live voice in the chat (UI-180..UI-189, UI-530): the mic button, the live panel and `:mute`, over a fake peer
 * and microphone.
 */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import type { ThreadEvent } from "@omfp/core";
import { threadTopic } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen } from "@testing-library/react";
import type { Levels, SpectrumSource } from "omfp/kit";
import { GlobalCommands } from "../../../src/frame/commands.tsx";
import { sheetGroups } from "../../../src/frame/commands-sheet.ts";
import {
  DRAFT_MESSAGE,
  DRAFT_THREAD,
  EMPTY_DRAFT,
  ERRORS,
  type LiveLink,
  type LiveMedia,
  type LiveSupport,
  SELECTED_THREAD,
  strings,
  threadActions,
} from "../../../src/sdk/index.ts";
import { LiveOpenError } from "../../../src/sdk/live.ts";
import { ComposerWidget } from "../../../src/widgets/chat/index.ts";
import { FakeClient, renderWithHost, type TestHost, testHost } from "../../support/host.tsx";
import { fixtureEvents, summary } from "./fixtures.ts";
import { THREAD } from "./harness.tsx";

const OTHER = "01U";
const OFFER = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\n";
const ANSWER = "v=0\r\no=- 3 4 IN IP4 127.0.0.1\r\n";
const LOGIN = "live voice needs an `openai-codex` login in omp: run `:login openai-codex`, then try again";

afterEach(() => {
  cleanup();
  delete document.documentElement.dataset.motion;
});

class FakeSource implements SpectrumSource {
  read(mic: Float32Array, remote: Float32Array): Levels {
    mic.fill(0);
    remote.fill(0);
    return { mic: 0, remote: 0 };
  }

  levels(): Levels {
    return { mic: 0, remote: 0 };
  }
}

/** One session's browser half: what the window did with the peer and the microphone. */
class FakeLink implements LiveLink {
  readonly offer = OFFER;
  readonly source = new FakeSource();
  readonly answers: string[] = [];
  readonly muted: boolean[] = [];
  closed = false;
  answerError: Error | undefined;

  async answer(sdp: string): Promise<void> {
    if (this.answerError !== undefined) {
      throw this.answerError;
    }
    this.answers.push(sdp);
  }

  setMuted(muted: boolean): void {
    this.muted.push(muted);
  }

  close(): void {
    this.closed = true;
  }
}

/** The browser's live voice APIs, driven by the test: support, the microphone, the peer's states and the clock. */
class FakeMedia implements LiveMedia {
  supported: LiveSupport = "webrtc";
  openError: Error | undefined;
  /** Holds the microphone (its permission prompt) until it resolves. */
  gate: Promise<void> | undefined;
  /** How many times the window asked for the microphone, counted when asked (not when granted). */
  opens = 0;
  readonly links: FakeLink[] = [];
  time = 1000;
  #onState: ((state: RTCPeerConnectionState) => void) | undefined;

  support(): LiveSupport {
    return this.supported;
  }

  async open(onState: (state: RTCPeerConnectionState) => void): Promise<LiveLink> {
    this.opens += 1;
    if (this.gate !== undefined) {
      await this.gate;
    }
    if (this.openError !== undefined) {
      throw this.openError;
    }
    this.#onState = onState;
    const link = new FakeLink();
    this.links.push(link);
    return link;
  }

  now(): number {
    return this.time;
  }

  /** The latest peer's `connectionstatechange`. */
  peer(state: RTCPeerConnectionState): void {
    act(() => this.#onState?.(state));
  }

  get link(): FakeLink {
    const link = this.links.at(-1);
    if (link === undefined) {
      throw new Error("no session was opened");
    }
    return link;
  }
}

interface Live extends TestHost {
  media: FakeMedia;
  /** Applies thread events and commits the frame. */
  events(...events: ThreadEvent[]): void;
}

/** `omfp:composer` on {@link THREAD} (and {@link OTHER} in the index) and the frame's commands, with a fake peer and
 * microphone. */
function renderLive(history: readonly ThreadEvent[] = fixtureEvents("text_reply"), supported?: LiveSupport): Live {
  const media = new FakeMedia();
  media.supported = supported ?? "webrtc";
  const test = testHost(new FakeClient(), { liveMedia: media });
  test.client.connected();
  test.client.index({ threads: [summary(THREAD), summary(OTHER)] });
  test.host.window.set(SELECTED_THREAD, THREAD);
  renderWithHost(
    <>
      <GlobalCommands viewIds={[]} currentView={undefined} widgets={[]} addWidget={() => Promise.resolve(false)} />
      <ComposerWidget />
    </>,
    test,
  );
  test.client.thread(THREAD, history);
  test.flush();
  return {
    ...test,
    media,
    events: (...events) => {
      test.client.threadEvents(THREAD, ...events);
      test.flush();
    },
  };
}

const session = (connection: number, client = "omfp/web"): ThreadEvent => ({
  kind: "live_session",
  owner: { client, connection },
});
const phase = (value: "connecting" | "listening" | "working" | "speaking" | "muted" | "error"): ThreadEvent => ({
  kind: "omp",
  frame: { type: "live_phase", phase: value },
});
const line = (role: "user" | "assistant", turn: number, text: string, final: boolean): ThreadEvent => ({
  kind: "omp",
  frame: { type: "live_transcript", role, turn, text, final },
});
const end = (error?: string): ThreadEvent => ({
  kind: "omp",
  frame: error === undefined ? { type: "live_end" } : { type: "live_end", error },
});

const mic = () => screen.getByTestId("live-mic");
const panel = () => screen.getByTestId("live-panel");
const status = () => panel().dataset.status;
const muteButton = () => screen.getByTestId("live-mute");
/** The mute button's pressed state, name and icon. */
const muteState = () => [
  muteButton().getAttribute("aria-pressed"),
  muteButton().getAttribute("aria-label"),
  muteButton().querySelector("svg")?.getAttribute("data-icon"),
];

async function press(element: HTMLElement): Promise<void> {
  await act(async () => {
    fireEvent.click(element);
  });
}

async function key(element: Element, value: string): Promise<void> {
  await act(async () => {
    fireEvent.keyDown(element, { key: value });
  });
}

/** Presses the mic button and lets the daemon open the session, as it does before answering (DMN-100). */
async function start(live: Live): Promise<void> {
  await press(mic());
  live.events(session(1), phase("connecting"), phase("listening"));
}

/** Answers the pending `thread.live_connect`. */
async function answer(live: Live): Promise<void> {
  const [call] = live.client.callsTo("thread.live_connect");
  await act(async () => call?.resolve({ sdpAnswer: ANSWER, voice: "sol" }));
}

async function connected(live: Live): Promise<void> {
  await start(live);
  await answer(live);
  live.media.peer("connected");
}

describe("live voice", () => {
  // spec: UI-180, UI-181
  test("the mic button opens the microphone and a peer, sends its offer and applies omp's answer", async () => {
    const live = renderLive();
    expect(mic().dataset.live).toBe("start");
    expect(screen.queryByTestId("live-panel")).toBeNull();
    await press(mic());
    expect(live.client.callsTo("thread.live_connect").map((call) => call.params)).toEqual([
      { threadId: THREAD, sdpOffer: OFFER },
    ]);
    expect(status()).toBe("connecting");
    expect(mic().dataset.live).toBe("end");
    await answer(live);
    expect(live.media.link.answers).toEqual([ANSWER]);
    expect(live.media.link.closed).toBe(false);
  });

  // spec: UI-108
  test(":live starts the session like the mic button, and ends it", async () => {
    const live = renderLive();
    await act(async () => {
      expect(live.host.commands.run("live")).toBe(true);
    });
    expect(live.client.callsTo("thread.live_connect").map((call) => call.params)).toEqual([
      { threadId: THREAD, sdpOffer: OFFER },
    ]);
    live.events(session(1), phase("connecting"), phase("listening"));
    await answer(live);
    live.media.peer("connected");
    await act(async () => {
      expect(live.host.commands.run("live.toggle")).toBe(true);
    });
    expect(live.client.callsTo("thread.live_stop").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
  });

  // spec: UI-180
  test("outside a secure context the mic button only says why: no microphone, no call", async () => {
    const live = renderLive(fixtureEvents("text_reply"), "insecure");
    expect(mic().dataset.live).toBe("unavailable");
    const reason = document.getElementById(mic().getAttribute("aria-describedby") ?? "");
    expect(reason?.textContent).toContain("localhost, 127.0.0.1 or HTTPS");
    await press(mic());
    expect(live.media.links).toEqual([]);
    expect(live.client.callsTo("thread.live_connect")).toEqual([]);
    expect(panel().dataset.view).toBe("error");
    expect(screen.getByRole("alert").textContent).toContain("secure page");
  });

  // spec: UI-181
  test("the panel shows connecting until the peer is connected, then omp's phase", async () => {
    const live = renderLive();
    await start(live);
    expect(status()).toBe("connecting");
    await answer(live);
    // omp already listens, but the peer is not connected: what is said now is lost.
    expect(status()).toBe("connecting");
    live.media.peer("connecting");
    expect(status()).toBe("connecting");
    live.media.peer("connected");
    expect(status()).toBe("listening");
    live.events(phase("working"));
    expect(status()).toBe("working");
  });

  // spec: UI-182
  test("spectrum, the latest line per role and the status line with its keys", async () => {
    document.documentElement.dataset.motion = "none";
    const live = renderLive();
    await connected(live);
    live.events(
      line("user", 1, "Ciao,", false),
      line("user", 1, "Ciao, mi senti?", false),
      line("assistant", 1, "Sì", false),
      line("user", 1, "Ciao, mi senti? Dimmi solo sì", true),
      line("assistant", 1, "Sì.", true),
    );
    expect(screen.getByRole("meter", { name: "Voice level" })).toBeDefined();
    const lines = [...panel().querySelectorAll(".omfp-live__line")].map((node) => [
      node.getAttribute("data-role"),
      node.textContent,
    ]);
    expect(lines).toEqual([
      ["user", "Ciao, mi senti? Dimmi solo sì"],
      ["assistant", "Sì."],
    ]);
    // The status line, its buttons left out.
    const statusLine = [...(panel().querySelector(".omfp-live__status")?.childNodes ?? [])]
      .filter((node) => !(node instanceof Element && node.classList.contains("omfp-live__actions")))
      .map((node) => node.textContent)
      .join("");
    expect(statusLine).toBe("●listening·spc mute·esc end");
    live.events(line("user", 2, "E adesso?", false));
    expect([...panel().querySelectorAll(".omfp-live__line")].map((node) => node.textContent)).toEqual([
      "Sì.",
      "E adesso?",
    ]);
  });

  // spec: UI-182, UI-183
  test("Space and Esc act only while the panel has the focus; it takes the focus when the session starts", async () => {
    const live = renderLive();
    await connected(live);
    expect(document.activeElement).toBe(panel());
    const input = screen.getByTestId("composer-input");
    act(() => input.focus());
    await key(input, " ");
    await key(input, "Escape");
    expect(live.client.callsTo("thread.live_mute")).toEqual([]);
    expect(live.client.callsTo("thread.live_stop")).toEqual([]);
    await key(panel(), " ");
    expect(live.client.callsTo("thread.live_mute").map((call) => call.params)).toEqual([
      { threadId: THREAD, muted: true },
    ]);
  });

  // spec: UI-183
  test("mute disables the local track and sends live_mute; the panel shows muted from the thread state", async () => {
    const live = renderLive();
    await connected(live);
    await key(panel(), " ");
    expect(live.media.link.muted).toEqual([true]);
    expect(live.client.callsTo("thread.live_mute").map((call) => call.params)).toEqual([
      { threadId: THREAD, muted: true },
    ]);
    expect(panel().textContent).toContain("unmute");
    live.events(phase("muted"));
    expect(status()).toBe("muted");
    await key(panel(), " ");
    expect(live.media.link.muted).toEqual([true, false]);
    expect(live.client.callsTo("thread.live_mute").map((call) => call.params)).toEqual([
      { threadId: THREAD, muted: true },
      { threadId: THREAD, muted: false },
    ]);
    live.events(phase("listening"));
    expect(status()).toBe("listening");
  });

  // spec: UI-182, UI-183
  test("the Mute button mutes and unmutes like Space, one name, pressed from the state; End ends the session", async () => {
    const live = renderLive();
    await start(live);
    // Nothing to mute until omp answered.
    expect((muteButton() as HTMLButtonElement).disabled).toBe(true);
    await answer(live);
    live.media.peer("connected");
    expect((muteButton() as HTMLButtonElement).disabled).toBe(false);
    expect(muteState()).toEqual(["false", "Mute", "mic"]);
    await press(muteButton());
    expect(live.media.link.muted).toEqual([true]);
    expect(live.client.callsTo("thread.live_mute").map((call) => call.params)).toEqual([
      { threadId: THREAD, muted: true },
    ]);
    expect(muteState()).toEqual(["true", "Mute", "mic-off"]);
    expect(panel().dataset.muted).toBe("true");
    live.events(phase("muted"));
    expect(status()).toBe("muted");
    // Space on the focused button is the button's: it unmutes once, the panel does not toggle again.
    await act(async () => {
      muteButton().focus();
      fireEvent.keyDown(muteButton(), { key: " " });
      fireEvent.keyUp(muteButton(), { key: " " });
    });
    expect(live.media.link.muted).toEqual([true, false]);
    expect(live.client.callsTo("thread.live_mute").map((call) => call.params)).toEqual([
      { threadId: THREAD, muted: true },
      { threadId: THREAD, muted: false },
    ]);
    expect(muteState()).toEqual(["false", "Mute", "mic"]);
    expect(live.client.callsTo("thread.live_stop")).toEqual([]);
    await press(screen.getByTestId("live-end"));
    expect(live.client.callsTo("thread.live_stop").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
    expect(live.media.link.closed).toBe(true);
    expect(screen.queryByTestId("live-panel")).toBeNull();
  });

  // spec: UI-530
  test(":mute toggles this window's running session and is offered only while one runs", async () => {
    const live = renderLive();
    const sheetIds = () =>
      sheetGroups(live.host.commands.list(), [], "").flatMap((group) => group.commands.map((command) => command.id));
    expect(live.host.commands.run("mute")).toBe(false);
    expect(sheetIds()).not.toContain("live.mute");
    await connected(live);
    expect(sheetIds()).toContain("live.mute");
    act(() => {
      expect(live.host.commands.run("mute")).toBe(true);
    });
    expect(live.media.link.muted).toEqual([true]);
    expect(muteState()).toEqual(["true", "Mute", "mic-off"]);
    act(() => {
      expect(live.host.commands.run("live.mute")).toBe(true);
    });
    expect(live.client.callsTo("thread.live_mute").map((call) => call.params)).toEqual([
      { threadId: THREAD, muted: true },
      { threadId: THREAD, muted: false },
    ]);
    await key(panel(), "Escape");
    expect(live.host.commands.run("mute")).toBe(false);
    expect(live.client.callsTo("thread.live_mute")).toHaveLength(2);
  });

  // spec: UI-184
  test("speaking is derived from the remote voice's level, held briefly after it drops", async () => {
    const live = renderLive();
    await connected(live);
    expect(status()).toBe("listening");
    act(() => live.host.live.noteLevels({ mic: 0.3, remote: 0 }));
    expect(status()).toBe("listening");
    act(() => live.host.live.noteLevels({ mic: 0, remote: 0.2 }));
    expect(status()).toBe("speaking");
    live.media.time += 200;
    act(() => live.host.live.noteLevels({ mic: 0, remote: 0 }));
    expect(status()).toBe("speaking");
    live.media.time += 200;
    act(() => live.host.live.noteLevels({ mic: 0, remote: 0 }));
    expect(status()).toBe("listening");
    // omp's own phases win: a muted or working session is not shown speaking.
    act(() => live.host.live.noteLevels({ mic: 0, remote: 0.2 }));
    live.events(phase("working"));
    expect(status()).toBe("working");
  });

  // spec: UI-185
  test("Esc ends the session: live_stop, the peer and the microphone close, the panel goes", async () => {
    const live = renderLive();
    await connected(live);
    await key(panel(), "Escape");
    expect(live.client.callsTo("thread.live_stop").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
    expect(live.media.link.closed).toBe(true);
    // The daemon's `live_end` follows.
    expect(screen.queryByTestId("live-panel")).toBeNull();
    live.events(end());
    expect(screen.queryByTestId("live-panel")).toBeNull();
    expect(mic().dataset.live).toBe("start");
  });

  // spec: UI-185
  test("a failed or closed peer sends live_stop: omp cannot see it", async () => {
    for (const state of ["failed", "closed"] as const) {
      const live = renderLive();
      await connected(live);
      live.media.peer(state);
      expect(live.client.callsTo("thread.live_stop").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
      expect(live.media.link.closed).toBe(true);
      cleanup();
    }
  });

  // spec: UI-185
  test("switching threads and unloading the page end the session", async () => {
    const live = renderLive();
    await connected(live);
    act(() => live.host.window.set(SELECTED_THREAD, OTHER));
    expect(live.client.callsTo("thread.live_stop").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
    expect(live.media.link.closed).toBe(true);

    act(() => live.host.window.set(SELECTED_THREAD, THREAD));
    // The thread's topic was released with the switch: the daemon answers it again.
    live.client.thread(THREAD, fixtureEvents("text_reply"));
    live.flush();
    await connected(live);
    act(() => {
      window.dispatchEvent(new Event("pagehide"));
    });
    expect(live.client.callsTo("thread.live_stop")).toHaveLength(2);
    expect(live.media.link.closed).toBe(true);
  });

  // spec: UI-185
  test("when the state shows the session ended, the window closes its peer without stopping again", async () => {
    const live = renderLive();
    await connected(live);
    live.events(phase("listening"), end());
    expect(live.media.link.closed).toBe(true);
    expect(live.client.callsTo("thread.live_stop")).toEqual([]);
    expect(screen.queryByTestId("live-panel")).toBeNull();
  });

  // spec: UI-182
  test("another window's session shows its state and an end button, no audio", async () => {
    const live = renderLive();
    live.events(session(2, "omfp/desktop"), phase("listening"), line("assistant", 1, "Sì.", true));
    expect(panel().dataset.view).toBe("elsewhere");
    expect(status()).toBe("listening");
    expect(panel().textContent).toContain("in another window (omfp/desktop)");
    expect(panel().textContent).toContain("Sì.");
    expect(panel().querySelector(".omfp-spectrum, [role=meter]")).toBeNull();
    expect(panel().querySelectorAll("kbd")).toHaveLength(0);
    await key(panel(), " ");
    expect(live.client.callsTo("thread.live_mute")).toEqual([]);
    expect(mic().dataset.live).toBe("end");
    await press(screen.getByTestId("live-end"));
    expect(live.client.callsTo("thread.live_stop").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
    expect(live.media.links).toEqual([]);
  });

  // spec: UI-186
  test("a refused start shows the daemon's readable message until dismissed", async () => {
    const live = renderLive();
    await press(mic());
    const [call] = live.client.callsTo("thread.live_connect");
    await act(async () =>
      call?.reject(
        new CallError({
          kind: "daemon",
          error: {
            code: "omp_error",
            message: LOGIN,
            details: { ompError: "No Codex OAuth credential is available for a live call." },
          },
        }),
      ),
    );
    expect(panel().dataset.view).toBe("error");
    expect(screen.getByRole("alert").textContent).toBe(LOGIN);
    expect(live.media.link.closed).toBe(true);
    expect(mic().dataset.live).toBe("start");
    await press(screen.getByTestId("live-dismiss"));
    expect(screen.queryByTestId("live-panel")).toBeNull();
  });

  // spec: UI-186
  test("a session that ended on an error shows live_error; Esc on the panel dismisses it", async () => {
    const live = renderLive(fixtureEvents("live_no_credential"));
    expect(panel().dataset.view).toBe("error");
    expect(screen.getByRole("alert").textContent).toBe(LOGIN);
    await key(panel(), "Escape");
    expect(screen.queryByTestId("live-panel")).toBeNull();
    // The next failure shows again.
    live.events(session(2), phase("error"), end("Codex live signaling failed (401): unauthorized"));
    expect(screen.getByRole("alert").textContent).toBe(
      "live voice: the `openai-codex` login was rejected; log in again with `:login openai-codex`",
    );
  });

  // spec: UI-186
  test("a microphone the browser refuses shows why, and nothing reaches the daemon", async () => {
    const live = renderLive();
    live.media.openError = new LiveOpenError("microphone", "Permission denied");
    await press(mic());
    expect(screen.getByRole("alert").textContent).toBe("live voice could not open the microphone: Permission denied");
    expect(live.client.callsTo("thread.live_connect")).toEqual([]);
  });

  // spec: UI-181, UI-186
  test("an answer the peer rejects stops the session and says why", async () => {
    const live = renderLive();
    await start(live);
    live.media.link.answerError = new Error("bad answer");
    await answer(live);
    expect(live.client.callsTo("thread.live_stop").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
    expect(live.media.link.closed).toBe(true);
    expect(screen.getByRole("alert").textContent).toBe("live voice could not set up the call: bad answer");
  });
});

describe("live voice on the daemon's machine", () => {
  /** omp's levels of the thread, as a transient frame (PRT-054). */
  const levels = (live: Live, input: number, output: number, threadId = THREAD) =>
    act(() =>
      live.client.transient({
        type: "transient",
        topic: threadTopic(threadId),
        event: { kind: "live_levels", input, output },
      }),
    );

  /** Presses the mic button; the daemon opens the session and omp answers once it records. */
  async function startHost(live: Live): Promise<void> {
    await press(mic());
    live.events(session(1), phase("connecting"));
    const [call] = live.client.callsTo("thread.live_start");
    await act(async () => call?.resolve({ voice: "sol" }));
    live.events(phase("listening"));
  }

  // spec: UI-188, UI-189
  test("without WebRTC the mic starts omp's own session: live_start, no microphone, no peer, omp's phases", async () => {
    const live = renderLive(fixtureEvents("text_reply"), "host");
    expect(mic().dataset.live).toBe("start");
    await press(mic());
    expect(live.client.callsTo("thread.live_start").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
    expect(live.client.callsTo("thread.live_connect")).toEqual([]);
    expect(live.media.links).toEqual([]);
    expect(status()).toBe("connecting");
    live.events(session(1), phase("connecting"));
    const [call] = live.client.callsTo("thread.live_start");
    await act(async () => call?.resolve({ voice: "sol" }));
    expect(status()).toBe("connecting");
    // No peer to wait for: omp's phase shows as omp reports it, `speaking` included.
    live.events(phase("listening"));
    expect(status()).toBe("listening");
    live.events(phase("speaking"));
    expect(status()).toBe("speaking");
    act(() => live.host.live.noteLevels({ mic: 0, remote: 0 }));
    expect(status()).toBe("speaking");
    live.events(phase("listening"));
    act(() => live.host.live.noteLevels({ mic: 0, remote: 0.5 }));
    expect(status()).toBe("listening");
  });

  // spec: UI-189, DMN-110
  test("the spectrum draws omp's levels of this thread", async () => {
    const live = renderLive(fixtureEvents("text_reply"), "host");
    // The spectrum's canvas reads on the page's clock: the levels arrive on it too.
    live.media.time = performance.now();
    await startHost(live);
    levels(live, 0.2, 0.4);
    levels(live, 0.9, 0.9, OTHER);
    const later = performance.now() + 1000;
    const shown = live.host.live.source().levels(later);
    expect(shown.mic).toBeCloseTo(0.2, 3);
    expect(shown.remote).toBeCloseTo(0.4, 3);
    document.documentElement.dataset.motion = "none";
    live.events(phase("working"));
    expect(screen.getByRole("meter", { name: "Voice level" })).toBeDefined();

    await key(panel(), "Escape");
    expect(live.host.live.source().levels(later)).toEqual({ mic: 0, remote: 0 });
  });

  // spec: UI-189, UI-183, UI-185
  test("Space mutes omp's microphone with live_mute and Esc ends the session with live_stop", async () => {
    const live = renderLive(fixtureEvents("text_reply"), "host");
    await startHost(live);
    expect(document.activeElement).toBe(panel());
    await key(panel(), " ");
    expect(live.client.callsTo("thread.live_mute").map((call) => call.params)).toEqual([
      { threadId: THREAD, muted: true },
    ]);
    live.events(phase("muted"));
    expect(status()).toBe("muted");
    await key(panel(), "Escape");
    expect(live.client.callsTo("thread.live_stop").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
    expect(screen.queryByTestId("live-panel")).toBeNull();
  });

  // spec: UI-189, UI-183
  test("the Mute button on omp's own session sends only live_mute", async () => {
    const live = renderLive(fixtureEvents("text_reply"), "host");
    await startHost(live);
    await press(muteButton());
    expect(live.media.links).toEqual([]);
    expect(live.client.callsTo("thread.live_mute").map((call) => call.params)).toEqual([
      { threadId: THREAD, muted: true },
    ]);
    expect(muteState()).toEqual(["true", "Mute", "mic-off"]);
    await press(screen.getByTestId("live-end"));
    expect(live.client.callsTo("thread.live_stop").map((call) => call.params)).toEqual([{ threadId: THREAD }]);
  });

  // spec: UI-188, UI-186
  test("a refused live_start shows the daemon's readable message", async () => {
    const live = renderLive(fixtureEvents("text_reply"), "host");
    await press(mic());
    const [call] = live.client.callsTo("thread.live_start");
    const message = "live voice failed: Failed to open the default speaker: no Linux playback backend available";
    await act(async () => call?.reject(new CallError({ kind: "daemon", error: { code: "omp_error", message } })));
    expect(panel().dataset.view).toBe("error");
    expect(screen.getByRole("alert").textContent).toBe(message);
    expect(mic().dataset.live).toBe("start");
  });

  // spec: UI-188, UI-180
  test("without WebRTC away from the daemon's machine the mic only says why", async () => {
    const live = renderLive(fixtureEvents("text_reply"), "unsupported");
    expect(mic().dataset.live).toBe("unavailable");
    const reason = document.getElementById(mic().getAttribute("aria-describedby") ?? "");
    expect(reason?.textContent).toContain("live voice in the desktop app needs the daemon on this machine");
    await press(mic());
    expect(live.client.callsTo("thread.live_start")).toEqual([]);
    expect(live.client.callsTo("thread.live_connect")).toEqual([]);
    expect(screen.getByRole("alert").textContent).toContain("needs the daemon on this machine");
  });
});

describe("the voice of live sessions", () => {
  /** Sets `[ui] live_voice` as the daemon pushes a settings change (UI-097). */
  const setVoice = (live: Live, liveVoice: "maple" | "vale") => {
    live.client.indexEvents({ kind: "settings_changed", settings: { ui: { liveVoice }, keymap: [], error: null } });
    live.flush();
  };

  // spec: UI-999, PRT-228
  test("a start over WebRTC carries the voice set last", async () => {
    const live = renderLive();
    setVoice(live, "vale");
    setVoice(live, "maple");
    await press(mic());
    expect(live.client.callsTo("thread.live_connect").map((call) => call.params)).toEqual([
      { threadId: THREAD, sdpOffer: OFFER, voice: "maple" },
    ]);
  });

  // spec: UI-999, UI-188
  test("omp's own session on the daemon's machine carries the voice too", async () => {
    const live = renderLive(fixtureEvents("text_reply"), "host");
    setVoice(live, "vale");
    await press(mic());
    expect(live.client.callsTo("thread.live_start").map((call) => call.params)).toEqual([
      { threadId: THREAD, voice: "vale" },
    ]);
  });
});

const NEW = "01N";

/** `omfp:composer` on the draft thread of workspace `01W` holding `text`, with a fake peer and microphone. */
function renderDraft(supported: LiveSupport = "webrtc", text = ""): Live {
  const media = new FakeMedia();
  media.supported = supported;
  const test = testHost(new FakeClient(), { liveMedia: media });
  test.client.connected();
  test.client.index({ threads: [summary(THREAD)] });
  threadActions(test.host).openDraft("01W");
  test.host.window.set(DRAFT_MESSAGE, { ...EMPTY_DRAFT, text });
  renderWithHost(<ComposerWidget />, test);
  test.flush();
  return {
    ...test,
    media,
    events: (...events) => {
      test.client.threadEvents(NEW, ...events);
      test.flush();
    },
  };
}

/** Answers the pending `threads.create` with thread {@link NEW}, whose state then arrives. */
async function created(live: Live): Promise<void> {
  const [call] = live.client.callsTo("threads.create");
  await act(async () => call?.resolve(summary(NEW)));
  live.client.index({ threads: [summary(THREAD), summary(NEW)] });
  live.client.thread(NEW, []);
  live.flush();
}

describe("the draft thread's mic", () => {
  // spec: UI-023, UI-180, UI-181
  test("it opens the microphone within the press, then creates the thread, selects it and talks to it", async () => {
    const live = renderDraft();
    const microphone = Promise.withResolvers<void>();
    live.media.gate = microphone.promise;
    expect(mic().dataset.live).toBe("start");
    await press(mic());
    // Asked before anything is awaited: browsers grant the microphone reliably only within the press.
    expect(live.media.opens).toBe(1);
    expect(live.client.callsTo("threads.create")).toHaveLength(0);
    expect(mic().dataset.live).toBe("end");
    expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(true);
    await act(async () => microphone.resolve());
    expect(live.client.callsTo("threads.create").map((call) => call.params)).toEqual([{ workspace: { id: "01W" } }]);
    expect(live.client.callsTo("thread.live_connect")).toHaveLength(0);
    await created(live);
    expect(live.host.window.get(SELECTED_THREAD)).toBe(NEW);
    expect(live.host.window.get(DRAFT_THREAD)).toBeUndefined();
    expect(live.client.callsTo("thread.live_connect").map((call) => call.params)).toEqual([
      { threadId: NEW, sdpOffer: OFFER },
    ]);
    expect(live.media.opens).toBe(1);
    expect(status()).toBe("connecting");
    const [call] = live.client.callsTo("thread.live_connect");
    await act(async () => call?.resolve({ sdpAnswer: ANSWER, voice: "sol" }));
    expect(live.media.link.answers).toEqual([ANSWER]);
    expect(live.media.link.closed).toBe(false);
  });

  // spec: UI-023
  test("the draft's message goes with it as the thread's first message", async () => {
    const live = renderDraft("webrtc", "  fix the build ");
    await press(mic());
    expect(live.client.callsTo("threads.create").map((call) => call.params)).toEqual([
      { workspace: { id: "01W" }, prompt: "fix the build" },
    ]);
  });

  // spec: UI-023, UI-186
  test("a failed creation keeps the draft, reports the error and releases the microphone", async () => {
    const live = renderDraft("webrtc", "hello");
    await press(mic());
    const [call] = live.client.callsTo("threads.create");
    await act(async () => call?.reject(new Error("daemon down")));
    expect(live.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["daemon down"]);
    expect(live.host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "01W" });
    expect(live.host.window.get(DRAFT_MESSAGE).text).toBe("hello");
    expect(live.media.link.closed).toBe(true);
    expect(live.client.callsTo("thread.live_connect")).toHaveLength(0);
    expect(mic().dataset.live).toBe("start");
    expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(false);
  });

  // spec: UI-023, UI-186
  test("a refused microphone creates no thread and is reported; the draft stays", async () => {
    const live = renderDraft("webrtc", "hello");
    live.media.openError = new LiveOpenError("microphone", "Permission denied");
    await press(mic());
    expect(live.client.callsTo("threads.create")).toHaveLength(0);
    expect(live.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
      "live voice could not open the microphone: Permission denied",
    ]);
    expect(live.host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "01W" });
    expect(mic().dataset.live).toBe("start");
  });

  // spec: UI-180, UI-185
  test("pressed again while the microphone opens, it gives up: no thread, the microphone closes", async () => {
    const live = renderDraft();
    const microphone = Promise.withResolvers<void>();
    live.media.gate = microphone.promise;
    await press(mic());
    await press(mic());
    expect(mic().dataset.live).toBe("start");
    await act(async () => microphone.resolve());
    expect(live.media.link.closed).toBe(true);
    expect(live.client.callsTo("threads.create")).toHaveLength(0);
    expect(live.client.callsTo("thread.live_stop")).toHaveLength(0);
    expect(live.host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "01W" });
  });

  // spec: UI-186
  test("a refused live_connect on the new thread shows in its live panel", async () => {
    const live = renderDraft();
    await press(mic());
    await created(live);
    const [call] = live.client.callsTo("thread.live_connect");
    await act(async () =>
      call?.reject(new CallError({ kind: "daemon", error: { code: "omp_error", message: LOGIN } })),
    );
    expect(panel().dataset.view).toBe("error");
    expect(screen.getByRole("alert").textContent).toBe(LOGIN);
    expect(live.media.link.closed).toBe(true);
  });

  // spec: UI-188, UI-023
  test("without WebRTC on the daemon's machine it creates the thread, then starts omp's own session", async () => {
    const live = renderDraft("host");
    await press(mic());
    expect(live.media.opens).toBe(0);
    await created(live);
    expect(live.client.callsTo("thread.live_start").map((call) => call.params)).toEqual([{ threadId: NEW }]);
  });

  // spec: UI-180, UI-023
  test("where live voice is unavailable it only says why: no microphone, no thread", async () => {
    const live = renderDraft("insecure");
    expect(mic().dataset.live).toBe("unavailable");
    await press(mic());
    expect(live.media.opens).toBe(0);
    expect(live.client.callsTo("threads.create")).toHaveLength(0);
    expect(live.host.window.get(ERRORS).map(({ text }) => text)).toEqual([strings.chat.live.insecure]);
  });

  // spec: UI-108, UI-023
  test("`:live` on the draft thread does what its mic does: makes the thread and talks to it, or gives up", async () => {
    const live = renderDraft();
    const microphone = Promise.withResolvers<void>();
    live.media.gate = microphone.promise;
    await act(async () => {
      expect(live.host.commands.run("live")).toBe(true);
    });
    expect(live.media.opens).toBe(1);
    expect(mic().dataset.live).toBe("end");
    // Pressed again while the microphone opens, it gives up as the mic does.
    await act(async () => {
      expect(live.host.commands.run("live.toggle")).toBe(true);
    });
    await act(async () => microphone.resolve());
    expect(live.media.link.closed).toBe(true);
    expect(live.client.callsTo("threads.create")).toHaveLength(0);
    live.media.gate = undefined;
    await act(async () => {
      live.host.commands.run("live");
    });
    await created(live);
    expect(live.client.callsTo("thread.live_connect").map((call) => call.params)).toEqual([
      { threadId: NEW, sdpOffer: OFFER },
    ]);
  });

  // spec: UI-180, UI-185, UI-023
  test("pressed again while the thread is created, it releases the microphone at once; the thread comes without a session", async () => {
    const live = renderDraft("webrtc", "hello");
    await press(mic());
    expect(live.client.callsTo("threads.create")).toHaveLength(1);
    await press(mic());
    // Not when the daemon answers: the microphone stops capturing now.
    expect(live.media.link.closed).toBe(true);
    expect(mic().dataset.live).toBe("start");
    // The message is still on its way: no edits, no other send, no other thread until the daemon answers.
    expect(screen.getByTestId("composer-input")).toHaveProperty("readOnly", true);
    expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(true);
    expect(mic().hasAttribute("disabled")).toBe(true);
    await created(live);
    expect(live.host.window.get(SELECTED_THREAD)).toBe(NEW);
    expect(live.host.window.get(DRAFT_THREAD)).toBeUndefined();
    expect(live.client.callsTo("thread.live_connect")).toHaveLength(0);
  });

  // spec: UI-023, UI-185
  test("given up while the thread is created, a failed creation unlocks the draft with its message", async () => {
    const live = renderDraft("webrtc", "hello");
    await press(mic());
    await press(mic());
    const [call] = live.client.callsTo("threads.create");
    await act(async () => call?.reject(new Error("daemon down")));
    expect(live.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["daemon down"]);
    expect(live.host.window.get(DRAFT_MESSAGE).text).toBe("hello");
    expect(screen.getByTestId("composer-input")).toHaveProperty("readOnly", false);
    expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(false);
    expect(mic().hasAttribute("disabled")).toBe(false);
  });

  // spec: UI-023
  test("a draft left for another workspace's while the microphone opens creates nothing", async () => {
    const live = renderDraft("webrtc", "for the first workspace");
    const microphone = Promise.withResolvers<void>();
    live.media.gate = microphone.promise;
    await press(mic());
    act(() => threadActions(live.host).openDraft("02W"));
    await act(async () => microphone.resolve());
    expect(live.client.callsTo("threads.create")).toHaveLength(0);
    expect(live.media.link.closed).toBe(true);
    expect(live.host.window.get(DRAFT_THREAD)?.workspaceId).toBe("02W");
    expect(mic().dataset.live).toBe("start");
  });

  // spec: UI-023
  test("while the mic creates the thread the draft is read-only and takes no images", async () => {
    const live = renderDraft("webrtc", "hello");
    await press(mic());
    const input = screen.getByTestId("composer-input");
    expect(input).toHaveProperty("readOnly", true);
    fireEvent.paste(input, { clipboardData: { files: [pngFile()], getData: () => "" } });
    const [call] = live.client.callsTo("threads.create");
    await act(async () => call?.reject(new Error("daemon down")));
    expect(live.host.window.get(DRAFT_MESSAGE).images).toHaveLength(0);
    expect(screen.getByTestId("composer-input")).toHaveProperty("readOnly", false);
  });

  // spec: UI-023, UI-357, UI-060
  test("while the mic creates the thread, paste chips, attachments, large pastes and Alt+Enter change nothing", async () => {
    const live = renderDraft();
    const long = Array.from({ length: 30 }, (_, index) => `row ${index}`).join("\n");
    const message = { text: "look: [Paste #1, +30 lines]", images: [PNG], pastes: [{ id: 1, text: long }] };
    act(() => live.host.window.set(DRAFT_MESSAGE, message));
    await press(mic());
    expect(live.client.callsTo("threads.create")).toHaveLength(1);
    await press(screen.getByTestId("paste-toggle"));
    expect(screen.getByTestId("paste-text")).toHaveProperty("readOnly", true);
    expect(screen.queryAllByTestId("paste-remove")).toHaveLength(0);
    expect(screen.getByTestId("attachment-remove").hasAttribute("disabled")).toBe(true);
    const input = screen.getByTestId("composer-input");
    fireEvent.paste(input, { clipboardData: { files: [], getData: () => `${long}\nmore` } });
    fireEvent.keyDown(input, { key: "Enter", altKey: true });
    expect(live.host.window.get(DRAFT_MESSAGE)).toEqual(message);
    expect(input).toHaveProperty("value", message.text);
    // Once the creation failed, the draft is the user's again.
    const [call] = live.client.callsTo("threads.create");
    await act(async () => call?.reject(new Error("daemon down")));
    expect(screen.getByTestId("paste-text")).toHaveProperty("readOnly", false);
    expect(screen.queryAllByTestId("paste-remove")).toHaveLength(1);
    expect(screen.getByTestId("attachment-remove").hasAttribute("disabled")).toBe(false);
  });

  // spec: UI-023, UI-060
  test("an image still being read when the mic is pressed goes with the first message", async () => {
    const live = renderDraft("host", "what is this?");
    const slow = slowPngFile();
    fireEvent.paste(screen.getByTestId("composer-input"), { clipboardData: { files: [slow.file], getData: () => "" } });
    await press(mic());
    expect(live.client.callsTo("threads.create")).toHaveLength(0);
    await act(async () => {
      slow.release();
      await slow.read;
    });
    expect(live.client.callsTo("threads.create").map((call) => call.params)).toEqual([
      { workspace: { id: "01W" }, prompt: "what is this?", images: [PNG] },
    ]);
  });

  // spec: UI-180, UI-185
  test("pressed again while an image is still being read, it creates nothing once the read ends", async () => {
    const live = renderDraft("webrtc", "what is this?");
    const slow = slowPngFile();
    fireEvent.paste(screen.getByTestId("composer-input"), { clipboardData: { files: [slow.file], getData: () => "" } });
    await press(mic());
    await press(mic());
    expect(live.media.link.closed).toBe(true);
    expect(mic().dataset.live).toBe("start");
    await act(async () => {
      slow.release();
      await slow.read;
    });
    expect(live.client.callsTo("threads.create")).toHaveLength(0);
    expect(live.host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "01W" });
  });

  // spec: UI-185, UI-023
  test("opening another workspace's draft while the thread is created ends the start at once; no session follows", async () => {
    const live = renderDraft("webrtc", "for the first workspace");
    await press(mic());
    act(() => threadActions(live.host).openDraft("02W"));
    // Now, not when the daemon answers: the microphone stops capturing.
    expect(live.media.link.closed).toBe(true);
    expect(mic().dataset.live).toBe("start");
    await created(live);
    expect(live.client.callsTo("thread.live_connect")).toHaveLength(0);
    // The other workspace's draft stays.
    expect(live.host.window.get(DRAFT_THREAD)?.workspaceId).toBe("02W");
  });

  // spec: UI-185, UI-023
  test("showing another thread while the thread is created ends the start at once; no session follows", async () => {
    const live = renderDraft();
    await press(mic());
    act(() => live.host.window.set(SELECTED_THREAD, THREAD));
    // Now, not when the daemon answers: the microphone stops capturing.
    expect(live.media.link.closed).toBe(true);
    await created(live);
    // The thread the message went to is shown, as a send does, but nobody talks to it.
    expect(live.host.window.get(SELECTED_THREAD)).toBe(NEW);
    expect(live.client.callsTo("thread.live_connect")).toHaveLength(0);
    expect(live.client.callsTo("thread.live_stop")).toHaveLength(0);
  });

  // spec: UI-185, UI-023
  test("leaving the page while the microphone opens ends the start: no thread", async () => {
    const live = renderDraft();
    const microphone = Promise.withResolvers<void>();
    live.media.gate = microphone.promise;
    await press(mic());
    act(() => void window.dispatchEvent(new Event("pagehide")));
    expect(mic().dataset.live).toBe("start");
    await act(async () => microphone.resolve());
    expect(live.media.link.closed).toBe(true);
    expect(live.client.callsTo("threads.create")).toHaveLength(0);
  });

  // spec: UI-185
  test("a thread's mic: switching threads while the microphone opens ends the start, no live_connect", async () => {
    const live = renderLive();
    const microphone = Promise.withResolvers<void>();
    live.media.gate = microphone.promise;
    await press(mic());
    act(() => live.host.window.set(SELECTED_THREAD, OTHER));
    await act(async () => microphone.resolve());
    expect(live.media.link.closed).toBe(true);
    expect(live.client.callsTo("thread.live_connect")).toHaveLength(0);
  });
});

/** A tiny PNG file, as pasted, and the attachment it becomes (UI-060). */
function pngFile(): File {
  return new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], "a.png", { type: "image/png" });
}
const PNG = { type: "image" as const, data: "iVBORw==", mimeType: "image/png" };

/** A PNG file whose bytes are read only once `release` is called; `read` is that read. */
function slowPngFile(): { file: File; release: () => void; read: Promise<ArrayBuffer> } {
  const file = pngFile();
  const bytes = file.arrayBuffer();
  const gate = Promise.withResolvers<void>();
  const read = gate.promise.then(() => bytes);
  Object.defineProperty(file, "arrayBuffer", { value: () => read });
  return { file, release: () => gate.resolve(), read };
}
