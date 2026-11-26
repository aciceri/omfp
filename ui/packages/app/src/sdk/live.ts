/**
 * Live voice in the window (UI-180..UI-189, spike S-09): {@link LiveClient} holds this window's one session. Where the
 * window has WebRTC it is the browser half of omp's `live_connect`: the microphone, a WebRTC peer whose offer the
 * daemon relays (`thread.live_connect`), the remote voice through Web Audio at twice its level and the analysers the
 * spectrum draws. Once the peer's data channel opens it appends the live instructions the daemon returned as silent
 * context (DMN-118).
 * The desktop webview has no WebRTC: on the daemon's machine (`#local=1`) the window starts omp's own session instead
 * (`thread.live_start`: omp's microphone and speakers), and the spectrum draws omp's `live_levels` (transient frames,
 * PRT-054). The window sends `thread.live_stop` when its peer fails or closes, on the end key, on a thread switch and on
 * unload (UI-185). Both starts carry the voice of `[ui] live_voice` when it is set (UI-999). While the window holds the
 * microphone it keeps the screen on (UI-531). The browser parts sit behind {@link LiveMedia}, which tests replace.
 */

import { CallError } from "@omfp/client";
import { AnalyserSource, type Levels, LevelsSource, SILENT_SOURCE, type SpectrumSource, Tap } from "@omfp/kit";
import { type LiveVoice, threadTopic } from "@omfp/protocol";
import { strings } from "../strings/index.ts";
import { IDLE, type LiveSupport, type LocalLive, liveSupport } from "../vm/live.ts";
import type { FrameStore } from "./frame-store.ts";
import type { SdkClient, TopicRefs } from "./host.tsx";
import { DRAFT_THREAD, SELECTED_THREAD } from "./keys.ts";
import { holdBrowserScreenAwake } from "./wake-lock.ts";
import type { WindowStore } from "./window-store.ts";

/** The remote voice counts as speaking above this RMS … */
const SPEAKING_LEVEL = 0.015;
/** … and for this long after (UI-184). */
const SPEAKING_HOLD_MS = 350;
/** The gain on the remote voice (UI-181): a phone at full volume is still too quiet. */
const VOICE_GAIN = 2;

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
  /**
   * Appends `chunks` to the realtime session as silent context, in order, once the data channel is open (DMN-118):
   * the realtime endpoint ignores the instructions omp passes at the start.
   */
  context(chunks: readonly string[]): void;
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
  /** Creates the thread and selects it: its id, or `undefined` when that failed (`create` reports it) or gave up. */
  create(start: LiveCreation): Promise<string | undefined>;
  /** Reports a start that failed before the thread existed: no live path here, the microphone or the peer. */
  fail(message: string): void;
  /** Whether the window still shows what the start began from (the draft pressed), asked whenever the draft changes
   * until the thread exists: false ends the start at once. */
  shown(): boolean;
}

/** What {@link NewLiveThread.create} knows of its start. */
export interface LiveCreation {
  /** Whether the start still runs: false once it ended (the mic pressed again, another thread shown, the page left),
   * when nothing may be created. */
  running(): boolean;
  /** Says the thread exists, before it is selected: the window may show it; anything else shown ends the start. */
  created(threadId: string): void;
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
    // Until the session opens on its thread, showing another thread (a new thread's start: anything but the draft, then
    // the new thread), dropping the draft pressed or leaving the page ends the start at once (UI-185), the microphone
    // with it.
    const watch = this.#watchShown(starting, target);
    let link: LiveLink | undefined;
    let threadId: string | undefined;
    try {
      // omp's own session (`live_start`) needs nothing of the window.
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
      threadId = typeof target === "string" ? target : await this.#create(target, starting, link, watch.show);
    } finally {
      watch.stop();
    }
    if (threadId !== undefined) {
      await (link === undefined ? this.#startHost(threadId) : this.#connect(threadId, link));
    }
  }

  /**
   * Makes the thread of a start once the window holds what its session needs: its id, now the session's; `undefined`
   * when that failed or the start ended meanwhile. The window holds `link` meanwhile, so ending the start releases the
   * microphone at once, not when the thread exists.
   */
  async #create(
    thread: NewLiveThread,
    starting: LocalLive,
    link: LiveLink | undefined,
    show: (threadId: string) => void,
  ): Promise<string | undefined> {
    this.#link = link;
    const threadId = await thread.create({
      running: () => this.#snapshot.local === starting,
      created: show,
    });
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

  /**
   * Ends the start `starting` when the window leaves the page or shows another thread than now (the draft: none) or
   * than the one `show` names; for a new thread, also once `target.shown()` is false when the draft changes (the draft
   * pressed replaced or dismissed), until `show` (the thread exists, its draft is done). Until `stop`.
   */
  #watchShown(starting: LocalLive, target: LiveTarget): { show(threadId: string): void; stop(): void } {
    let shown = this.#window.get(SELECTED_THREAD);
    const end = () => {
      if (this.#snapshot.local === starting) {
        this.end();
      }
    };
    const unsubscribe = this.#window.subscribe(SELECTED_THREAD, () => {
      if (this.#window.get(SELECTED_THREAD) !== shown) {
        end();
      }
    });
    let unsubscribeDraft =
      typeof target === "string"
        ? () => {}
        : this.#window.subscribe(DRAFT_THREAD, () => {
            if (!target.shown()) {
              end();
            }
          });
    window.addEventListener("pagehide", end);
    return {
      show: (threadId) => {
        shown = threadId;
        unsubscribeDraft();
        unsubscribeDraft = () => {};
      },
      stop: () => {
        unsubscribe();
        unsubscribeDraft();
        window.removeEventListener("pagehide", end);
      },
    };
  }

  /** The peer's session on `threadId`: `thread.live_connect` with `link`'s offer, then omp's answer (UI-181). */
  async #connect(threadId: string, link: LiveLink): Promise<void> {
    this.#open(threadId, link);
    let answer: string;
    let context: readonly string[];
    try {
      ({ sdpAnswer: answer, context = [] } = await this.#client.call("thread.live_connect", {
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
      link.context(context);
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
  // The screen stays on while the window holds the microphone (UI-531): a phone's screen turning off would stop it.
  const awake = holdBrowserScreenAwake();
  const source = new AnalyserSource();
  const context = new AudioContext();
  const peer = new RTCPeerConnection();
  const audio = new Audio();
  const close = () => {
    awake();
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
    const output = voiceOutput(context);
    // The element plays the voice itself only while the context waits for a user activation.
    const follow = () => {
      audio.muted = context.state === "running";
    };
    context.addEventListener("statechange", follow);
    source.mic = new Tap(context, context.createMediaStreamSource(microphone));
    const [track] = microphone.getAudioTracks();
    if (track === undefined) {
      throw new Error("the microphone has no audio track");
    }
    peer.addTransceiver(track, { direction: "sendrecv", streams: [microphone] });
    // omp's realtime model talks over the `oai-events` channel; the window only appends the live instructions there,
    // which the realtime endpoint ignores at the start (DMN-118).
    const events = peer.createDataChannel("oai-events");
    const pending: string[] = [];
    const flush = () => {
      for (const text of pending.splice(0)) {
        events.send(
          JSON.stringify({
            type: "session.context.append",
            channel: "commentary",
            content: [{ type: "input_text", text }],
          }),
        );
      }
    };
    events.addEventListener("open", flush);
    peer.addEventListener("track", (event) => {
      const stream = event.streams[0] ?? new MediaStream([event.track]);
      // Chromium feeds a remote WebRTC stream into Web Audio only while a media element plays it.
      audio.autoplay = true;
      audio.srcObject = stream;
      void audio.play().catch(() => undefined);
      const remote = context.createMediaStreamSource(stream);
      remote.connect(output);
      source.remote = new Tap(context, remote);
      follow();
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
      context(chunks) {
        pending.push(...chunks);
        if (events.readyState === "open") {
          flush();
        }
      },
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

/** The remote voice's way to the speakers: {@link VOICE_GAIN}, then a limiter so the louder peaks do not clip. */
function voiceOutput(context: AudioContext): AudioNode {
  const gain = context.createGain();
  gain.gain.value = VOICE_GAIN;
  const limiter = context.createDynamicsCompressor();
  limiter.threshold.value = -3;
  limiter.knee.value = 0;
  limiter.ratio.value = 20;
  limiter.attack.value = 0.003;
  limiter.release.value = 0.1;
  gain.connect(limiter).connect(context.destination);
  return gain;
}
