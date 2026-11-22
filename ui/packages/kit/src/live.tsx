/**
 * Live voice (UI-182, 09-design): the spectrum, its static meter and the live panel. Presentational: the app feeds
 * the audio (a {@link SpectrumSource}), the status and the texts.
 *
 * The spectrum is a row of {@link SPECTRUM_BARS} log-spaced bars on a canvas, the microphone and the remote voice
 * mixed (the louder one wins each bar, each with its own gradient), drawn every animation frame from Web Audio
 * analysers ({@link AnalyserSource}), or from omp's levels alone in a `live_start` session ({@link LevelsSource});
 * colors are the theme's role tokens. Below motion `full` it is a static level meter refreshed four times a second
 * (UI-145).
 */
import { type HTMLAttributes, type ReactNode, type Ref, useEffect, useRef, useState } from "react";
import { Meter as AriaMeter } from "react-aria-components";
import { BorderBox } from "./cards.tsx";
import { cx } from "./class.ts";
import { Kbd, KeyHint } from "./kbd.tsx";
import { type MotionLevel, useMotionLevel } from "./motion.ts";

/** Bars of the spectrum. */
export const SPECTRUM_BARS = 64;
/** The band of the bars (Hz): voice lives here. */
const LOW_HZ = 70;
const HIGH_HZ = 9000;
/** How often the static meter reads the levels. */
const METER_MS = 250;
/** How often the canvas reads the theme's colors again (a theme or scheme change needs no event). */
const COLORS_MS = 500;

/** What a live session shows: omp's phases, `speaking` derived on the client for `live_connect` (UI-184), and `idle`. */
export type LiveStatus = "idle" | "connecting" | "listening" | "working" | "speaking" | "muted" | "error";

/** RMS levels of the microphone and the remote voice in [0, 1]. */
export interface Levels {
  readonly mic: number;
  readonly remote: number;
}

/** Where the spectrum's audio comes from. */
export interface SpectrumSource {
  /** Fills the bar heights in [0, 1] of the microphone and the remote voice at `now` (ms); returns their RMS. */
  read(mic: Float32Array, remote: Float32Array, now: number): Levels;
  /** The RMS levels only (the static meter). */
  levels(now: number): Levels;
}

/** A source without audio: flat bars. */
export const SILENT_SOURCE: SpectrumSource = {
  read(mic, remote) {
    mic.fill(0);
    remote.fill(0);
    return { mic: 0, remote: 0 };
  },
  levels: () => ({ mic: 0, remote: 0 }),
};

/** An analyser on one audio node, with its buffers. */
export class Tap {
  readonly analyser: AnalyserNode;
  readonly #freq: Uint8Array<ArrayBuffer>;
  readonly #time: Float32Array<ArrayBuffer>;

  constructor(context: BaseAudioContext, node: AudioNode) {
    this.analyser = context.createAnalyser();
    this.analyser.fftSize = 1024;
    this.analyser.smoothingTimeConstant = 0.72;
    this.analyser.minDecibels = -95;
    this.analyser.maxDecibels = -25;
    node.connect(this.analyser);
    this.#freq = new Uint8Array(this.analyser.frequencyBinCount);
    this.#time = new Float32Array(this.analyser.fftSize);
  }

  /** The frequency bins (0..255) of the latest block. */
  frequencies(): Uint8Array<ArrayBuffer> {
    this.analyser.getByteFrequencyData(this.#freq);
    return this.#freq;
  }

  rms(): number {
    this.analyser.getFloatTimeDomainData(this.#time);
    let sum = 0;
    for (const sample of this.#time) {
      sum += sample * sample;
    }
    return Math.sqrt(sum / this.#time.length);
  }

  disconnect(): void {
    this.analyser.disconnect();
  }
}

/** The bars from Web Audio analysers on the microphone and the remote track (`live_connect`: omp sends no levels). */
export class AnalyserSource implements SpectrumSource {
  mic: Tap | undefined;
  remote: Tap | undefined;
  readonly #bands = new Uint16Array(SPECTRUM_BARS + 1);
  #bandsFor = 0;

  read(mic: Float32Array, remote: Float32Array): Levels {
    return { mic: this.#fill(this.mic, mic), remote: this.#fill(this.remote, remote) };
  }

  levels(): Levels {
    return { mic: this.mic?.rms() ?? 0, remote: this.remote?.rms() ?? 0 };
  }

  #fill(tap: Tap | undefined, out: Float32Array): number {
    if (tap === undefined) {
      out.fill(0);
      return 0;
    }
    const { analyser } = tap;
    const bins = analyser.frequencyBinCount;
    if (this.#bandsFor !== analyser.context.sampleRate) {
      this.#bandsFor = analyser.context.sampleRate;
      bandEdges(this.#bandsFor / 2 / bins, bins, this.#bands);
    }
    const freq = tap.frequencies();
    for (let bar = 0; bar < SPECTRUM_BARS; bar++) {
      let peak = 0;
      for (let bin = this.#bands[bar] ?? 0; bin < (this.#bands[bar + 1] ?? 0); bin++) {
        peak = Math.max(peak, freq[bin] ?? 0);
      }
      // Steeper than linear: quiet bins stay low, speech peaks stand out.
      out[bar] = (peak / 255) ** 1.6;
    }
    return tap.rms();
  }
}

/** How fast the levels source follows omp's levels (ms, time constant): omp sends at most ten a second. */
const LEVELS_EASE_MS = 60;

/** The voice band's shape over the bars (0..1): most energy in the low middle, never flat zero. */
function voiceProfile(bar: number): number {
  const x = bar / (SPECTRUM_BARS - 1);
  return 0.35 + 0.65 * Math.exp(-(((x - 0.3) / 0.25) ** 2));
}

/** A slow ripple (0.6..1) so steady levels still move like speech. */
function ripple(bar: number, now: number): number {
  return 0.8 + 0.2 * Math.sin(now / 90 + bar * 0.9) * Math.sin(now / 210 - bar * 0.37);
}

/**
 * The bars from levels alone (UI-189): omp's `live_levels` of a `live_start` session, where omp owns the audio and
 * sends only the RMS of the microphone and the speaker (S-09). Each voice's level, eased between omp's frames, is
 * spread over a voice-shaped profile with a slow ripple.
 */
export class LevelsSource implements SpectrumSource {
  #target: Levels = { mic: 0, remote: 0 };
  #shown: Levels = { mic: 0, remote: 0 };
  #at = 0;

  /** The latest levels, at `now` (ms). */
  set(levels: Levels, now: number): void {
    this.#ease(now);
    this.#target = levels;
  }

  read(mic: Float32Array, remote: Float32Array, now: number): Levels {
    const levels = this.levels(now);
    const micScale = Math.min(1, Math.sqrt(levels.mic) * 1.5);
    const remoteScale = Math.min(1, Math.sqrt(levels.remote) * 1.5);
    for (let bar = 0; bar < SPECTRUM_BARS; bar++) {
      const shape = voiceProfile(bar);
      mic[bar] = micScale * shape * ripple(bar, now);
      remote[bar] = remoteScale * shape * ripple(bar + SPECTRUM_BARS / 2, now);
    }
    return levels;
  }

  levels(now: number): Levels {
    this.#ease(now);
    return this.#shown;
  }

  #ease(now: number): void {
    const elapsed = now - this.#at;
    if (elapsed <= 0) {
      return;
    }
    this.#at = now;
    const keep = Math.exp(-elapsed / LEVELS_EASE_MS);
    const { mic, remote } = this.#target;
    this.#shown = {
      mic: mic + (this.#shown.mic - mic) * keep,
      remote: remote + (this.#shown.remote - remote) * keep,
    };
  }
}

/** The first FFT bin of each bar (and the end of the last): log-spaced between 70 Hz and 9 kHz, one bin at least. */
export function bandEdges(hzPerBin: number, bins: number, out: Uint16Array): void {
  for (let edge = 0; edge <= SPECTRUM_BARS; edge++) {
    const hz = LOW_HZ * (HIGH_HZ / LOW_HZ) ** (edge / SPECTRUM_BARS);
    out[edge] = Math.min(bins - 1, Math.max(1, Math.round(hz / hzPerBin)));
  }
  for (let edge = 1; edge <= SPECTRUM_BARS; edge++) {
    const previous = out[edge - 1] ?? 0;
    if ((out[edge] ?? 0) <= previous) {
      out[edge] = previous + 1;
    }
  }
}

/**
 * One frame of bars: `heights` in px (at least the floor) and which voice each bar shows (`fromRemote` 1). The
 * microphone is silent while muted; `connecting` sweeps a low wave, `working` breathes; idle and error stay flat.
 */
export function spectrumFrame(
  status: LiveStatus,
  mic: Float32Array,
  remote: Float32Array,
  now: number,
  height: number,
  heights: Float32Array,
  fromRemote: Uint8Array,
): void {
  const floor = Math.max(2, Math.round(height * 0.06));
  const quiet = status === "idle" || status === "error";
  const micOn = !quiet && status !== "muted";
  for (let bar = 0; bar < SPECTRUM_BARS; bar++) {
    const m = micOn ? (mic[bar] ?? 0) : 0;
    const r = quiet ? 0 : (remote[bar] ?? 0);
    let value = Math.max(m, r);
    if (status === "connecting") {
      value = 0.02 + 0.06 * Math.max(0, Math.sin(now / 240 - bar / 5));
    } else if (status === "working") {
      value = Math.max(value, 0.02 + 0.02 * (1 + Math.sin(now / 600)));
    }
    heights[bar] = Math.max(floor, value * height);
    fromRemote[bar] = r > m ? 1 : 0;
  }
}

/** The meter's value (0..100) for `levels`: the louder voice, on a square-root scale (speech RMS sits at 0.02–0.3). */
export function meterValue(status: LiveStatus, levels: Levels): number {
  if (status === "idle" || status === "error") {
    return 0;
  }
  const mic = status === "muted" ? 0 : levels.mic;
  return Math.min(100, Math.round(Math.sqrt(Math.max(mic, levels.remote)) * 160));
}

interface Colors {
  readonly user: string;
  readonly assistant: string;
  readonly muted: string;
  readonly idle: string;
}

/** The theme's resolved colors of the spectrum's roles, read through `probe` (its `color` set to each token). */
function readColors(probe: HTMLElement): Colors {
  const resolve = (token: string) => {
    probe.style.color = `var(${token})`;
    return getComputedStyle(probe).color;
  };
  return {
    user: resolve("--omfp-tertiary"),
    assistant: resolve("--omfp-primary"),
    muted: resolve("--omfp-outline"),
    idle: resolve("--omfp-outline-variant"),
  };
}

export interface SpectrumProps {
  id?: string | undefined;
  className?: string | undefined;
  "data-testid"?: string | undefined;
  source: SpectrumSource;
  status: LiveStatus;
  /** The meter's accessible name (below motion `full`). */
  label: string;
  /** Called with the levels of every frame (canvas) or meter reading, e.g. to derive `speaking` (UI-184). */
  onLevels?: ((levels: Levels) => void) | undefined;
  /** The motion level to draw for; default: the effective one (UI-145). */
  motion?: MotionLevel | undefined;
}

/** The live spectrum (UI-182): canvas bars at motion `full`, else {@link VoiceMeter}. */
export function Spectrum({ motion, ...props }: SpectrumProps) {
  const effective = useMotionLevel();
  return (motion ?? effective) === "full" ? <SpectrumCanvas {...props} /> : <VoiceMeter {...props} />;
}

/** The canvas is decorative: the panel's status line says what happens. */
function SpectrumCanvas({
  source,
  status,
  onLevels,
  className,
  id,
  "data-testid": testId,
}: Omit<SpectrumProps, "motion">) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const probeRef = useRef<HTMLSpanElement>(null);
  const latest = useRef({ status, onLevels });
  latest.current = { status, onLevels };
  useEffect(() => {
    const canvas = canvasRef.current;
    const probe = probeRef.current;
    if (canvas === null || probe === null) {
      return undefined;
    }
    const context = canvas.getContext("2d");
    const mic = new Float32Array(SPECTRUM_BARS);
    const remote = new Float32Array(SPECTRUM_BARS);
    const heights = new Float32Array(SPECTRUM_BARS);
    const fromRemote = new Uint8Array(SPECTRUM_BARS);
    let width = 0;
    let height = 0;
    let colors: Colors | undefined;
    let colorsAt = Number.NEGATIVE_INFINITY;
    let userFill: CanvasGradient | string = "";
    let assistantFill: CanvasGradient | string = "";
    const paints = () => {
      if (context === null || colors === undefined) {
        return;
      }
      // Each voice runs from its own role into the other's.
      const user = context.createLinearGradient(0, 0, width, 0);
      user.addColorStop(0, colors.user);
      user.addColorStop(1, colors.assistant);
      userFill = user;
      const assistant = context.createLinearGradient(0, 0, width, 0);
      assistant.addColorStop(0, colors.assistant);
      assistant.addColorStop(1, colors.user);
      assistantFill = assistant;
    };
    const resize = () => {
      const ratio = window.devicePixelRatio || 1;
      width = Math.max(1, Math.round(canvas.clientWidth * ratio));
      height = Math.max(1, Math.round(canvas.clientHeight * ratio));
      canvas.width = width;
      canvas.height = height;
      paints();
    };
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(resize) : undefined;
    observer?.observe(canvas);
    resize();
    let frame = 0;
    const draw = (now: number) => {
      frame = requestAnimationFrame(draw);
      const { status: current, onLevels: report } = latest.current;
      const levels = source.read(mic, remote, now);
      report?.(levels);
      if (context === null) {
        return;
      }
      if (now - colorsAt > COLORS_MS) {
        colorsAt = now;
        const next = readColors(probe);
        if (
          colors === undefined ||
          Object.entries(next).some(([key, value]) => colors?.[key as keyof Colors] !== value)
        ) {
          colors = next;
          paints();
        }
      }
      if (colors === undefined) {
        return;
      }
      spectrumFrame(current, mic, remote, now, height, heights, fromRemote);
      context.clearRect(0, 0, width, height);
      const gap = Math.max(1, Math.round(width / SPECTRUM_BARS / 4));
      const bar = (width - gap * (SPECTRUM_BARS - 1)) / SPECTRUM_BARS;
      const flat = current === "idle" || current === "error";
      // Mirrored around the middle; one path per fill keeps it at two fill() calls a frame.
      for (const pass of [0, 1]) {
        context.fillStyle = flat
          ? colors.idle
          : pass === 1
            ? assistantFill
            : current === "muted"
              ? colors.muted
              : userFill;
        context.beginPath();
        for (let index = 0; index < SPECTRUM_BARS; index++) {
          if (fromRemote[index] === pass) {
            const h = heights[index] ?? 0;
            context.rect(index * (bar + gap), (height - h) / 2, bar, h);
          }
        }
        context.fill();
      }
    };
    frame = requestAnimationFrame(draw);
    return () => {
      cancelAnimationFrame(frame);
      observer?.disconnect();
    };
  }, [source]);
  return (
    <div
      id={id}
      className={cx("omfp-spectrum", className)}
      data-status={status}
      data-testid={testId}
      aria-hidden="true"
    >
      <canvas ref={canvasRef} className="omfp-spectrum__canvas" />
      <span ref={probeRef} className="omfp-spectrum__probe" />
    </div>
  );
}

/** The spectrum below motion `full`: one level meter (the louder voice), read four times a second, not animated. */
export function VoiceMeter({
  source,
  status,
  label,
  onLevels,
  className,
  id,
  "data-testid": testId,
}: Omit<SpectrumProps, "motion">) {
  const [levels, setLevels] = useState<Levels>(() => source.levels(performance.now()));
  const latest = useRef(onLevels);
  latest.current = onLevels;
  useEffect(() => {
    const timer = setInterval(() => {
      const next = source.levels(performance.now());
      latest.current?.(next);
      setLevels(next);
    }, METER_MS);
    return () => clearInterval(timer);
  }, [source]);
  const value = meterValue(status, levels);
  const voice = levels.remote > (status === "muted" ? 0 : levels.mic) ? "assistant" : "user";
  return (
    <AriaMeter
      value={value}
      aria-label={label}
      {...(id === undefined ? {} : { id })}
      className={cx("omfp-voice-meter", className)}
      data-status={status}
      data-voice={voice}
      {...(testId === undefined ? {} : { "data-testid": testId })}
    >
      <span className="omfp-voice-meter__track">
        <span className="omfp-voice-meter__fill" style={{ inlineSize: `${value}%` }} />
      </span>
    </AriaMeter>
  );
}

/** A transcript line of the live panel. */
export interface LiveLineView {
  readonly role: "user" | "assistant";
  readonly text: string;
}

/** A key chip and what it does, in the status line (`spc mute`). */
export interface LiveHint {
  readonly keys: string;
  readonly label: string;
}

export interface LivePanelProps extends Omit<HTMLAttributes<HTMLElement>, "title"> {
  /** The panel's element, e.g. to focus it so its keys work. */
  ref?: Ref<HTMLElement> | undefined;
  status: LiveStatus;
  /** The title on the panel's top border (UI-187). */
  title: ReactNode;
  /** The status word after the dot (`listening`). */
  statusLabel: string;
  /** The spectrum (or nothing, when this window holds no audio). */
  spectrum?: ReactNode;
  /** The latest transcript lines. */
  lines?: readonly LiveLineView[];
  /** A failure, shown instead of the lines. */
  message?: ReactNode;
  /** Key chips after the status (`· spc mute · esc end`). */
  hints?: readonly LiveHint[];
  /** Notes after the status (`in another window`). */
  note?: ReactNode;
  /** Buttons at the end of the status line. */
  actions?: ReactNode;
}

/**
 * The live panel (UI-182): its title on the top border (UI-187), the spectrum, the transcript lines in the prose
 * family (the user's in `on-surface-variant`), then the status line `● listening · spc mute · esc end` in the code
 * family, its dot in the status's role.
 */
export function LivePanel({
  status,
  title,
  statusLabel,
  spectrum,
  lines = [],
  message,
  hints = [],
  note,
  actions,
  className,
  ...props
}: LivePanelProps) {
  return (
    <BorderBox
      {...props}
      title={title}
      className={cx("omfp-live", className)}
      bodyClassName="omfp-live__body"
      data-status={status}
    >
      {spectrum}
      {message === undefined ? (
        lines.length > 0 && (
          <div className="omfp-live__lines">
            {lines.map((line) => (
              <p key={line.role} className="omfp-live__line omfp-prose" data-role={line.role}>
                {line.text}
              </p>
            ))}
          </div>
        )
      ) : (
        <p className="omfp-live__message" role="alert">
          {message}
        </p>
      )}
      <div className="omfp-live__status">
        <span className="omfp-live__dot" aria-hidden="true">
          ●
        </span>
        <span className="omfp-live__state">{statusLabel}</span>
        {note !== undefined && (
          <>
            <span className="omfp-live__sep" aria-hidden="true">
              ·
            </span>
            <span className="omfp-live__note">{note}</span>
          </>
        )}
        {hints.map((hint) => (
          <KeyHint key={hint.keys} className="omfp-live__hint">
            <span className="omfp-live__sep" aria-hidden="true">
              ·
            </span>
            <Kbd>{hint.keys}</Kbd> {hint.label}
          </KeyHint>
        ))}
        {actions !== undefined && <span className="omfp-live__actions">{actions}</span>}
      </div>
    </BorderBox>
  );
}
