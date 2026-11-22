/**
 * Live voice in the window (UI-180..UI-189, spike S-09): {@link LiveClient} holds this window's one session. Where the
 * window has WebRTC it is the browser half of omp's `live_connect`: the microphone, a WebRTC peer whose offer the
 * daemon relays (`thread.live_connect`), the remote voice on an `<audio>` element and the analysers the spectrum draws.
 * The desktop webview has no WebRTC: on the daemon's machine (`#local=1`) the window starts omp's own session instead
 * (`thread.live_start`: omp's microphone and speakers), and the spectrum draws omp's `live_levels` (transient frames,
 * PRT-054). The window sends `thread.live_stop` when its peer fails or closes, on the end key, on a thread switch and on
 * unload (UI-185). Both starts carry the voice of `[ui] live_voice` when it is set (UI-999). The browser parts sit
 * behind {@link LiveMedia}, which tests replace.
 */

import { CallError } from "@omfp/client";
import { AnalyserSource, type Levels, LevelsSource, SILENT_SOURCE, type SpectrumSource, Tap } from "@omfp/kit";
import { type LiveVoice, threadTopic } from "@omfp/protocol";
import { strings } from "../strings/index.ts";
import { IDLE, type LiveSupport, type LocalLive, liveSupport } from "../vm/live.ts";
import type { FrameStore } from "./frame-store.ts";
import type { SdkClient, TopicRefs } from "./host.tsx";
import { SELECTED_THREAD } from "./keys.ts";
import type { WindowStore } from "./window-store.ts";

/** The remote voice counts as speaking above this RMS … */
const SPEAKING_LEVEL = 0.015;
/** … and for this long after (UI-184). */
const SPEAKING_HOLD_MS = 350;

/** What the window holds while its session is open. */
interface LiveHold {
  /** What the spectrum draws. */
  readonly source: SpectrumSource;
  /** Enables or disables the local track (UI-183); nothing is local for `live_start`. */
  setMuted(muted: boolean): void;
  /** Releases what the session holds. */
  close(): void;
}

/** The browser half of one session, open: the microphone and a peer that sends it, with its offer. */
export interface LiveLink extends LiveHold {
  /** The peer's SDP offer (one sendrecv Opus audio track and the `oai-events` data channel). */
  readonly offer: string;
  /** The analysers of the microphone and (once it arrives) the remote voice. */
  readonly source: SpectrumSource;
  /** Applies omp's SDP answer. */
  answer(sdp: string): Promise<void>;
  /** Closes the peer, the microphone, the remote audio and the analysers. */
  close(): void;
}

/** Why opening a session failed: the microphone (permission, no device) or the peer. */
export class LiveOpenError extends Error {
  readonly stage: "microphone" | "peer";

  constructor(stage: "microphone" | "peer", message: string) {
    super(message);
    this.name = "LiveOpenError";
    this.stage = stage;
  }
}

/** The browser APIs of a session; {@link browserLiveMedia} in the app, a fake in tests. */
export interface LiveMedia {
  /** How this window holds a session (UI-188). */
  support(): LiveSupport;
  /** Opens the microphone and a peer sending it; `onState` gets the peer's `connectionState` changes. */
  open(onState: (state: RTCPeerConnectionState) => void): Promise<LiveLink>;
  /** The clock of the speaking hold (ms). */
  now(): number;
}

/** The session and the errors the user dismissed, by thread. */
export interface LiveSnapshot {
  readonly local: LocalLive;
  readonly dismissed: Readonly<Record<string, string>>;
}

export interface LiveClientOptions {
  readonly client: SdkClient;
  readonly frames: FrameStore;
  readonly topics: TopicRefs;
  readonly window: WindowStore;
  readonly media: LiveMedia;
}

/** The readable message of a failure: the daemon's message (DMN-104), else the error's. */
function failureMessage(error: unknown): string {
  if (error instanceof CallError && error.failure.kind === "daemon") {
    return error.failure.error.message;
  }
  return error instanceof Error ? error.message : String(error);
}

/** A thread {@link LiveClient.start} makes once the window holds what the session needs: the draft thread's (UI-023). */
export interface NewLiveThread {
  /**
   * Creates the thread and selects it: its id, or `undefined` when that failed (`create` reports it) or gave up;
   * `running()` says whether the start still runs, false once it ended (the mic pressed again): nothing is created then.
   */
  create(running: () => boolean): Promise<string | undefined>;
  /** Reports a start that failed before the thread existed: no live path here, the microphone or the peer. */
  fail(message: string): void;
}

/** Where {@link LiveClient.start} opens its session: an existing thread, or a new one. */
export type LiveTarget = string | NewLiveThread;

/** This window's live session (at most one), read by the widgets through {@link LiveClient.subscribe}. */
export class LiveClient {
  readonly #client: SdkClient;
  readonly #frames: FrameStore;
  readonly #topics: TopicRefs;
  readonly #window: WindowStore;
  readonly #media: LiveMedia;
  #snapshot: LiveSnapshot = { local: IDLE, dismissed: {} };
  readonly #listeners = new Set<() => void>();
  /** What the open session holds (its peer, or the levels of omp's session), and what it retains while open. */
  #link: LiveHold | undefined;
  #release: (() => void)[] = [];
  /** The thread state showed this window's session: from then on its absence means it ended. */
  #seen = false;
  /** The open session's peer reached `connected`. */
  #connected = false;
  #speakingUntil = 0;

  constructor(options: LiveClientOptions) {
    this.#client = options.client;
    this.#frames = options.frames;
    this.#topics = options.topics;
    this.#window = options.window;
    this.#media = options.media;
  }

  readonly getSnapshot = (): LiveSnapshot => this.#snapshot;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  support(): LiveSupport {
    return this.#media.support();
  }

  /** What the spectrum draws: the open session's analysers or omp's levels, else silence. */
  source(): SpectrumSource {
    return this.#link?.source ?? SILENT_SOURCE;
  }

  /** The voice of `[ui] live_voice` for a start's params (UI-999); without one, none: omp picks its own. */
  #voice(): { voice?: LiveVoice } {
    const voice = this.#frames.getSnapshot().index.settings.ui.liveVoice;
    return voice === undefined || voice === null ? {} : { voice };
  }

  /**
   * Starts a session on `target`, a thread or one {@link NewLiveThread} makes: microphone, peer and offer,
   * `thread.live_connect`, then omp's answer (UI-181); without WebRTC on the daemon's machine, `thread.live_start`
   * (UI-188). Ends this window's other session first. The microphone opens at once, before anything is awaited:
   * browsers grant it reliably only within the press (Safari); a new thread is made after it (UI-023). A failure shows
   * in the thread's panel (UI-186), or through the new thread's `fail` before the thread exists.
   */
  async start(target: LiveTarget): Promise<void> {
    const support = this.#media.support();
    if (support !== "webrtc" && support !== "host") {
      this.#failed(target, strings.chat.live[support]);
      return;
    }
    this.end();
    const starting: LocalLive = { kind: "starting", threadId: typeof target === "string" ? target : undefined };
    this.#set(starting);
    this.#seen = false;
    this.#connected = false;
    this.#speakingUntil = 0;
    // omp's own session (`live_start`) needs nothing of the window.
    let link: LiveLink | undefined;
    if (support === "webrtc") {
      try {
        link = await this.#media.open((state) => this.#peerState(link, state));
      } catch (error) {
        if (this.#snapshot.local === starting) {
          const stage = error instanceof LiveOpenError ? error.stage : "peer";
          this.#failed(target, strings.chat.live[stage](failureMessage(error)));
        }
        return;
      }
      if (this.#snapshot.local !== starting) {
        link.close();
        return;
      }
    }
    const threadId = typeof target === "string" ? target : await this.#create(target, starting, link);
    if (threadId !== undefined) {
      await (link === undefined ? this.#startHost(threadId) : this.#connect(threadId, link));
    }
  }

  /**
   * Makes the thread of a start once the window holds what its session needs: its id, now the session's; `undefined`
   * when that failed or the start ended meanwhile. The window holds `link` meanwhile, so ending the start releases the
   * microphone at once, not when the thread exists.
   */
  async #create(thread: NewLiveThread, starting: LocalLive, link: LiveLink | undefined): Promise<string | undefined> {
    this.#link = link;
    const threadId = await thread.create(() => this.#snapshot.local === starting);
    if (this.#snapshot.local === starting && threadId !== undefined) {
      this.#set({ kind: "starting", threadId });
      return threadId;
    }
    if (this.#link === link) {
      this.#close();
    }
    if (this.#snapshot.local === starting) {
      this.#set(IDLE);
    }
    return undefined;
  }

  /** The peer's session on `threadId`: `thread.live_connect` with `link`'s offer, then omp's answer (UI-181). */
  async #connect(threadId: string, link: LiveLink): Promise<void> {
    this.#open(threadId, link);
    let answer: string;
    try {
      ({ sdpAnswer: answer } = await this.#client.call("thread.live_connect", {
        threadId,
        sdpOffer: link.offer,
        ...this.#voice(),
      }));
    } catch (error) {
      if (this.#link === link) {
        this.#close();
        this.#set({ kind: "failed", threadId, message: failureMessage(error) });
      }
      return;
    }
    if (this.#link !== link) {
      return;
    }
    try {
      await link.answer(answer);
    } catch (error) {
      if (this.#link === link) {
        this.#stop(threadId);
        this.#close();
        this.#set({ kind: "failed", threadId, message: strings.chat.live.peer(failureMessage(error)) });
      }
      return;
    }
    if (this.#link === link) {
      this.#set({ kind: "running", threadId, mode: "peer", connected: this.#connected, speaking: false, muted: false });
      this.#check();
    }
  }

  /**
   * Starts omp's own session on the daemon's machine (`thread.live_start`, UI-188): no microphone or peer here; the
   * spectrum draws the thread's `live_levels` (UI-189).
   */
  async #startHost(threadId: string): Promise<void> {
    const source = new LevelsSource();
    const topic = threadTopic(threadId);
    const link: LiveHold = {
      source,
      setMuted: () => {},
      close: this.#client.onTransient((frame) => {
        if (frame.topic === topic && frame.event.kind === "live_levels") {
          source.set({ mic: frame.event.input, remote: frame.event.output }, this.#media.now());
        }
      }),
    };
    this.#open(threadId, link);
    try {
      await this.#client.call("thread.live_start", { threadId, ...this.#voice() });
    } catch (error) {
      if (this.#link === link) {
        this.#close();
        this.#set({ kind: "failed", threadId, message: failureMessage(error) });
      }
      return;
    }
    if (this.#link === link) {
      // omp answers once it records: its phases show from now on, `speaking` included.
      this.#set({ kind: "running", threadId, mode: "host", connected: true, speaking: false, muted: false });
      this.#check();
    }
  }

  /**
   * A start that failed before its session ran: in the thread's panel (UI-186); a thread not made yet has no panel,
   * so its `fail` reports it and the window holds nothing.
   */
  #failed(target: LiveTarget, message: string): void {
    if (typeof target === "string") {
      this.#set({ kind: "failed", threadId: target, message });
      return;
    }
    const local = this.#snapshot.local;
    if (local.kind === "starting" && local.threadId === undefined) {
      this.#set(IDLE);
    }
    target.fail(message);
  }

  /** Ends this window's session (the end key, a thread switch, unload, UI-185): `thread.live_stop`, then closes it. */
  end(): void {
    const local = this.#snapshot.local;
    if (local.kind !== "starting" && local.kind !== "running") {
      return;
    }
    // A session still starting may reach omp yet: stop it either way.
    this.#close();
    this.#set(IDLE);
    if (local.threadId !== undefined) {
      this.#stop(local.threadId);
    }
  }

  /** Ends the session of thread `threadId`, this window's or another's (UI-182). */
  endThread(threadId: string): void {
    const local = this.#snapshot.local;
    if ((local.kind === "starting" || local.kind === "running") && local.threadId === threadId) {
      this.end();
    } else {
      this.#stop(threadId);
    }
  }

  /** Mutes or unmutes this window's running session (UI-183): the local track (if any), then `thread.live_mute`. */
  toggleMute(): void {
    const local = this.#snapshot.local;
    if (local.kind !== "running") {
      return;
    }
    const muted = !local.muted;
    this.#link?.setMuted(muted);
    this.#set({ ...local, muted });
    this.#client.call("thread.live_mute", { threadId: local.threadId, muted }).catch(() => {
      // omp refuses without a session (`conflict`): the session ended meanwhile and its `live_end` closes it.
    });
  }

  /**
   * The levels the spectrum read (every frame, or four times a second below motion `full`): `speaking` while the
   * remote voice is audible (UI-184; omp reports no `speaking` phase for `live_connect`, but does for `live_start`).
   */
  readonly noteLevels = (levels: Levels): void => {
    const local = this.#snapshot.local;
    if (local.kind !== "running" || local.mode === "host") {
      return;
    }
    const now = this.#media.now();
    if (levels.remote > SPEAKING_LEVEL) {
      this.#speakingUntil = now + SPEAKING_HOLD_MS;
    }
    const speaking = now < this.#speakingUntil;
    if (speaking !== local.speaking) {
      this.#set({ ...local, speaking });
    }
  };

  /** Hides the failure shown on `threadId`: this window's failed start, and `message`, the thread's `live_error`. */
  dismiss(threadId: string, message: string): void {
    const local = this.#snapshot.local;
    this.#snapshot = {
      local: local.kind === "failed" && local.threadId === threadId ? IDLE : local,
      dismissed: { ...this.#snapshot.dismissed, [threadId]: message },
    };
    this.#emit();
  }

  #open(threadId: string, link: LiveHold): void {
    this.#link = link;
    const unload = () => this.end();
    window.addEventListener("pagehide", unload);
    this.#release = [
      this.#topics.retain(threadTopic(threadId)),
      this.#frames.subscribe(() => this.#check()),
      this.#window.subscribe(SELECTED_THREAD, () => {
        if (this.#window.get(SELECTED_THREAD) !== threadId) {
          this.end();
        }
      }),
      () => window.removeEventListener("pagehide", unload),
    ];
  }

  #close(): void {
    const link = this.#link;
    this.#link = undefined;
    for (const release of this.#release) {
      release();
    }
    this.#release = [];
    link?.close();
  }

  #stop(threadId: string): void {
    this.#client.call("thread.live_stop", { threadId }).catch((error: unknown) => {
      // Shown unless a new session started meanwhile.
      if (this.#snapshot.local.kind === "idle") {
        this.#set({ kind: "failed", threadId, message: strings.chat.live.stopFailed(failureMessage(error)) });
      }
    });
  }

  /** Closes the session once the thread state shows it ended (`live_end`, another owner, UI-185). */
  #check(): void {
    const local = this.#snapshot.local;
    if (local.kind !== "running") {
      return;
    }
    const snapshot = this.#frames.getSnapshot();
    const view = snapshot.threads.get(local.threadId);
    if (view === undefined) {
      return;
    }
    const connection = snapshot.connection.kind === "connected" ? snapshot.connection.hello.connection : undefined;
    const live = view.state.live;
    if (live !== null && live.owner.connection === connection) {
      this.#seen = true;
    } else if (this.#seen) {
      this.#close();
      this.#set(IDLE);
    }
  }

  #peerState(link: LiveLink | undefined, state: RTCPeerConnectionState): void {
    if (link === undefined || this.#link !== link) {
      return;
    }
    if (state === "failed" || state === "closed") {
      // omp cannot see the peer: tell it the call is gone (UI-185).
      this.end();
    } else if (state === "connected") {
      this.#connected = true;
      const local = this.#snapshot.local;
      if (local.kind === "running" && !local.connected) {
        this.#set({ ...local, connected: true });
      }
    }
  }

  #set(local: LocalLive): void {
    this.#snapshot = { ...this.#snapshot, local };
    this.#emit();
  }

  #emit(): void {
    for (const listener of [...this.#listeners]) {
      listener();
    }
  }
}

/**
 * The browser's {@link LiveMedia}: `getUserMedia`, `RTCPeerConnection`, an `<audio>` element and Web Audio; `local`
 * says the window runs on the daemon's machine (`#local=1`), where a window without WebRTC uses omp's own session.
 */
export function browserLiveMedia(local: boolean): LiveMedia {
  return {
    support: () =>
      liveSupport({
        secure: window.isSecureContext,
        webrtc: typeof RTCPeerConnection === "function" && navigator.mediaDevices?.getUserMedia !== undefined,
        local,
      }),
    now: () => performance.now(),
    open: openPeer,
  };
}

/** Opens the microphone and a peer sending it (`live_connect`). */
async function openPeer(onState: (state: RTCPeerConnectionState) => void): Promise<LiveLink> {
  let microphone: MediaStream;
  try {
    microphone = await navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
    });
  } catch (error) {
    throw new LiveOpenError("microphone", failureMessage(error));
  }
  const source = new AnalyserSource();
  const context = new AudioContext();
  const peer = new RTCPeerConnection();
  const audio = new Audio();
  const close = () => {
    peer.close();
    for (const track of microphone.getTracks()) {
      track.stop();
    }
    audio.pause();
    audio.srcObject = null;
    source.mic?.disconnect();
    source.remote?.disconnect();
    void context.close();
  };
  try {
    // Not awaited: without a user activation `resume` waits for one; the analysers stay silent until then.
    void context.resume().catch(() => undefined);
    source.mic = new Tap(context, context.createMediaStreamSource(microphone));
    const [track] = microphone.getAudioTracks();
    if (track === undefined) {
      throw new Error("the microphone has no audio track");
    }
    peer.addTransceiver(track, { direction: "sendrecv", streams: [microphone] });
    // omp's realtime model talks over the `oai-events` channel; the browser only has to open it.
    peer.createDataChannel("oai-events");
    peer.addEventListener("track", (event) => {
      const stream = event.streams[0] ?? new MediaStream([event.track]);
      // Chromium feeds a remote WebRTC stream into Web Audio only while a media element plays it.
      audio.autoplay = true;
      audio.srcObject = stream;
      void audio.play().catch(() => undefined);
      source.remote = new Tap(context, context.createMediaStreamSource(stream));
    });
    peer.addEventListener("connectionstatechange", () => onState(peer.connectionState));
    await peer.setLocalDescription(await peer.createOffer());
    const offer = peer.localDescription?.sdp;
    if (offer === undefined || offer === "") {
      throw new Error("the browser made no SDP offer");
    }
    return {
      offer,
      source,
      answer: (sdp) => peer.setRemoteDescription({ type: "answer", sdp }),
      setMuted(muted) {
        for (const local of microphone.getAudioTracks()) {
          local.enabled = !muted;
        }
      },
      close,
    };
  } catch (error) {
    close();
    throw new LiveOpenError("peer", failureMessage(error));
  }
}
