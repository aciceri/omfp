/**
 * The transcript's minimap (UI-860..UI-863): the whole conversation in miniature at the transcript's right edge, beside
 * the scrollbar and over the text's edge, while the user scrolls. A zero-height sticky anchor at the top of the scroll
 * container holds the map, so the wheel over it still scrolls the transcript. One canvas holds the drawing
 * (`minimapShapes`) of a band of the map around its window, and one box marks the viewport.
 *
 * The canvas is a cached bitmap: a scroll only moves the box and, when the miniature is taller than the map, the canvas
 * (CSS transforms written at most once per animation frame, from the position the transcript's scroll event already
 * read), never renders React; it redraws the canvas only once the map scrolled out of the band (once per frame at
 * most). Otherwise the canvas is redrawn at most every {@link REDRAW_MS}, only while the map shows, and only when what
 * it shows changed: the rows, their measured heights (the virtualizer's measurements, estimates for rows never
 * rendered, so the drawing
 * shifts a little as they get measured), the search matches, the focused row, the transcript's size or the theme.
 * The map shows on the scrolls the transcript reports as the user's (`ScrollIntent`: their wheel, keys, commands,
 * scrollbar, touch or a drag on the map, never a follow of the bottom, a restore or the jump to the bottom) and hides
 * {@link HIDE_MS} after the last one while the pointer is not on it; its fade is a CSS transition, so the motion level
 * decides it (UI-145), and the `mobile` layout hides it. Pressing or dragging on it scrolls the transcript there.
 */

import type { ThreadState } from "@omfp/core";
import { cssColorReader, watchTheme } from "omfp/kit";
import {
  type ItemExtent,
  type MinimapBox,
  type MinimapPaint,
  type MinimapScene,
  minimapBand,
  minimapBox,
  minimapCovers,
  minimapDragScrollTop,
  minimapMetrics,
  minimapOffset,
  minimapRow,
  minimapScrollTop,
  minimapShapes,
  transcriptRowContent as rowContent,
  type ScrollPosition,
  sameMinimapScene,
  scrollPosition,
  type TranscriptItem,
} from "omfp/sdk";
import { type PointerEvent, type Ref, type RefObject, useEffect, useImperativeHandle, useRef, useState } from "react";

/** The map hides this long after the last scroll. */
const HIDE_MS = 1000;
/** While the map shows, the canvas is redrawn at most this often (a streamed reply, rows being measured). */
const REDRAW_MS = 100;

/**
 * Each paint's theme token (UI-134) and opacity, in drawing order: the rows' tints faint, the boxes behind the words,
 * the marks on top.
 */
const PAINTS: readonly { readonly paint: MinimapPaint; readonly token: string; readonly alpha: number }[] = [
  { paint: "system", token: "outline-variant", alpha: 0.3 },
  { paint: "assistant", token: "on-surface-variant", alpha: 0.05 },
  { paint: "user", token: "tertiary", alpha: 0.2 },
  { paint: "tool", token: "primary", alpha: 0.06 },
  { paint: "error", token: "error", alpha: 0.15 },
  { paint: "codeBlock", token: "on-surface-variant", alpha: 0.14 },
  { paint: "image", token: "on-surface-variant", alpha: 0.25 },
  { paint: "toolBar", token: "primary", alpha: 0.22 },
  { paint: "errorBar", token: "error", alpha: 0.3 },
  { paint: "label", token: "on-surface-variant", alpha: 0.55 },
  { paint: "text", token: "on-surface", alpha: 0.5 },
  { paint: "heading", token: "on-surface", alpha: 0.9 },
  { paint: "code", token: "on-surface-variant", alpha: 0.75 },
  { paint: "errorText", token: "error", alpha: 0.8 },
  { paint: "match", token: "search-current-bg", alpha: 1 },
  { paint: "focus", token: "focus-bar", alpha: 1 },
];

/** The User Timing measure of each redraw, for the transcript benchmark (TST-052). */
const DRAW_MEASURE = "omfp:minimap-draw";

/** What the transcript tells its minimap. */
export interface MinimapHandle {
  /** The view moved to `position`: a scroll the user caused (`byUser`) shows the map, any other only moves its box. */
  scrolled(position: ScrollPosition, byUser: boolean): void;
}

export interface MinimapProps {
  ref: Ref<MinimapHandle>;
  /** The transcript's scroll container and its list. */
  container: RefObject<HTMLElement | null>;
  list: RefObject<HTMLElement | null>;
  /** The virtualizer's measurements, by row; a new array whenever a row's size changed. */
  measurements: () => readonly ItemExtent[];
  rows: readonly TranscriptItem[];
  state: ThreadState;
  /** Rows matching the conversation search while it is open (UI-730). */
  matches: readonly number[];
  /** The focused row (UI-095). */
  current: number | undefined;
  /** Scrolls the transcript to `scrollTop` for the user. */
  onJump: (scrollTop: number) => void;
}

interface Elements {
  readonly map: RefObject<HTMLDivElement | null>;
  readonly canvas: RefObject<HTMLCanvasElement | null>;
  readonly box: RefObject<HTMLDivElement | null>;
}

type Colors = ReadonlyMap<string, string>;

/** What the canvas was last drawn from. */
interface Drawn extends MinimapScene {
  readonly colors: Colors;
}

/** Canvas takes `rgb()` and hex colors everywhere; any other notation goes through the reader's 1×1 canvas. */
const CANVAS_COLOR = /^(?:rgba?\(|#)/;

/** The theme's colors of the paints, as the canvas takes them. */
function readColors(within: HTMLElement): Colors {
  const read = cssColorReader(within.ownerDocument, (color) => (CANVAS_COLOR.test(color) ? color : undefined), within);
  const colors = new Map<string, string>();
  for (const { token } of PAINTS) {
    const color = colors.has(token) ? undefined : read(token);
    if (color !== undefined) {
      colors.set(token, color);
    }
  }
  return colors;
}

/** Draws `scene` on `canvas`, as tall as its band, at the screen's pixel ratio. */
function drawScene(canvas: HTMLCanvasElement, scene: MinimapScene, colors: Colors): void {
  const ratio = window.devicePixelRatio || 1;
  const width = Math.round(scene.width * ratio);
  const height = Math.round(scene.height * ratio);
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) {
    canvas.height = height;
    canvas.style.height = `${scene.height}px`;
  }
  const context = canvas.getContext("2d");
  if (context === null) {
    return;
  }
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, scene.width, scene.height);
  const shapes = minimapShapes(scene);
  // One path per paint: a fill style set once, one fill.
  for (const { paint, token, alpha } of PAINTS) {
    const rects = shapes.get(paint);
    const color = colors.get(token);
    if (rects === undefined || color === undefined) {
      continue;
    }
    context.globalAlpha = alpha;
    context.fillStyle = color;
    context.beginPath();
    for (let at = 0; at + 3 < rects.length; at += 4) {
      context.rect(rects[at] ?? 0, rects[at + 1] ?? 0, rects[at + 2] ?? 0, rects[at + 3] ?? 0);
    }
    context.fill();
  }
  context.globalAlpha = 1;
}

/** The map's state outside React: shown or not, what the canvas was drawn from, the box's place, the timers. */
class MinimapView implements MinimapHandle {
  readonly #elements: Elements;
  readonly #props: RefObject<MinimapProps>;
  #shown = false;
  #hovered = false;
  #dragging = false;
  /** What the canvas shows may have changed since it was drawn. */
  #stale = true;
  #drawn: Drawn | undefined;
  #colors: Colors | undefined;
  #drawnAt = Number.NEGATIVE_INFINITY;
  #position: ScrollPosition | undefined;
  /** The map's height, the transcript's viewport's, and its width, the miniature's. */
  #height = 0;
  #width = 0;
  #box: MinimapBox = { top: Number.NaN, height: Number.NaN };
  /** The band of the map the canvas holds, and where the canvas was last moved to. */
  #band: MinimapBox | undefined;
  #canvasTop = Number.NaN;
  /** How far below the viewport box's top the drag holds it. */
  #grab = 0;
  /** What the drawing was last told it shows (the rows, their measurements, the state, the matches, the focus). */
  #inputs: readonly unknown[] = [];
  #frame: number | undefined;
  #redraw: number | undefined;
  #hide: number | undefined;

  constructor(elements: Elements, props: RefObject<MinimapProps>) {
    this.#elements = elements;
    this.#props = props;
  }

  scrolled(position: ScrollPosition, byUser: boolean): void {
    this.#position = position;
    if (byUser) {
      this.#shown = true;
      this.#hideLater();
    }
    if (this.#shown) {
      this.#request();
    }
  }

  /** After a render: marks the drawing changed when one of `inputs` is not the one it had. */
  track(inputs: readonly unknown[]): void {
    if (inputs.length !== this.#inputs.length || inputs.some((input, index) => input !== this.#inputs[index])) {
      this.#inputs = inputs;
      this.changed();
    }
  }

  /** What the canvas shows may have changed: redrawn now if the map shows, else when it shows next. */
  changed(): void {
    this.#stale = true;
    if (this.#shown) {
      this.#request();
    }
  }

  themeChanged(): void {
    this.#colors = undefined;
    this.changed();
  }

  enter(): void {
    this.#hovered = true;
    window.clearTimeout(this.#hide);
  }

  leave(): void {
    this.#hovered = false;
    this.#hideLater();
  }

  press(event: PointerEvent<HTMLDivElement>): void {
    if (event.button !== 0 || !this.#shown) {
      return;
    }
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    this.#dragging = true;
    window.clearTimeout(this.#hide);
    const pointer = this.#pointer(event.clientY);
    if (pointer !== undefined) {
      // The view centres on the place pressed; the drag then holds the box where the pointer took it.
      const { y, position } = pointer;
      const top = minimapScrollTop(y, this.#height, position);
      this.#grab = y - minimapBox({ ...position, scrollTop: top }, this.#height).top;
      this.#props.current.onJump(top);
    }
  }

  drag(event: PointerEvent<HTMLDivElement>): void {
    const pointer = this.#dragging ? this.#pointer(event.clientY) : undefined;
    if (pointer !== undefined) {
      this.#props.current.onJump(minimapDragScrollTop(pointer.y - this.#grab, this.#height, pointer.position));
    }
  }

  release(): void {
    this.#dragging = false;
    this.#hideLater();
  }

  dispose(): void {
    if (this.#frame !== undefined) {
      cancelAnimationFrame(this.#frame);
      this.#frame = undefined;
    }
    window.clearTimeout(this.#redraw);
    this.#redraw = undefined;
    window.clearTimeout(this.#hide);
  }

  /** Where a pointer at `clientY` is on the map, with the transcript's scroll position. */
  #pointer(clientY: number): { readonly y: number; readonly position: ScrollPosition } | undefined {
    const map = this.#elements.map.current;
    const container = this.#props.current.container.current;
    return map === null || container === null
      ? undefined
      : { y: clientY - map.getBoundingClientRect().top, position: scrollPosition(container) };
  }

  #hideLater(): void {
    window.clearTimeout(this.#hide);
    if (this.#shown && !this.#hovered && !this.#dragging) {
      this.#hide = window.setTimeout(() => {
        this.#shown = false;
        this.#elements.map.current?.removeAttribute("data-visible");
      }, HIDE_MS);
    }
  }

  #request(): void {
    this.#frame ??= requestAnimationFrame(() => {
      this.#frame = undefined;
      this.#tick();
    });
  }

  #tick(): void {
    if (!this.#shown) {
      return;
    }
    if (
      this.#band === undefined ||
      this.#position === undefined ||
      !minimapCovers(this.#band, this.#position, this.#height)
    ) {
      // The map scrolled out of what the canvas holds: drawn now, not to show it blank.
      this.#draw();
    } else if (this.#stale) {
      const wait = this.#drawnAt + REDRAW_MS - performance.now();
      if (wait <= 0) {
        this.#draw();
      } else {
        this.#redraw ??= window.setTimeout(() => {
          this.#redraw = undefined;
          this.#request();
        }, wait);
      }
    }
    this.#place();
    const map = this.#elements.map.current;
    if (map !== null && map.dataset.visible !== "true") {
      map.dataset.visible = "true";
    }
  }

  /**
   * Reads where the rows are and the map's metrics, and redraws the canvas unless that draws the same picture: the
   * band it holds stays while it still holds the map's window, else one centred on it. Each redraw is a User Timing
   * measure ({@link DRAW_MEASURE}); the page keeps only the latest, a `PerformanceObserver` sees them all.
   */
  #draw(): void {
    const { container, list, measurements, rows, state, matches, current } = this.#props.current;
    const element = container.current;
    const listElement = list.current;
    const map = this.#elements.map.current;
    const canvas = this.#elements.canvas.current;
    if (element === null || listElement === null || map === null || canvas === null) {
      return;
    }
    this.#stale = false;
    // Reads first, then writes: one layout at most.
    const position = scrollPosition(element);
    const listTop = listElement.getBoundingClientRect().top - element.getBoundingClientRect().top + position.scrollTop;
    const font = getComputedStyle(map);
    const metrics = minimapMetrics(
      listElement.clientWidth,
      Number.parseFloat(font.fontSize),
      Number.parseFloat(font.lineHeight),
    );
    const colors = this.#colors ?? readColors(map);
    this.#colors = colors;
    this.#position = position;
    if (this.#height !== position.clientHeight) {
      this.#height = position.clientHeight;
      map.style.height = `${position.clientHeight}px`;
    }
    if (this.#width !== metrics.width) {
      this.#width = metrics.width;
      map.style.width = `${metrics.width}px`;
    }
    const fresh = minimapBand(position, this.#height);
    const band =
      this.#band !== undefined &&
      this.#band.height === fresh.height &&
      minimapCovers(this.#band, position, this.#height)
        ? this.#band
        : fresh;
    const scene: MinimapScene = {
      rows: rows.map((row) => {
        const content = rowContent(state, row);
        return content === undefined ? undefined : minimapRow(content);
      }),
      items: measurements(),
      listTop,
      width: metrics.width,
      top: band.top,
      height: band.height,
      columns: metrics.columns,
      pitch: metrics.pitch,
      matches,
      current,
    };
    if (this.#drawn !== undefined && this.#drawn.colors === colors && sameMinimapScene(this.#drawn, scene)) {
      return;
    }
    this.#drawn = { ...scene, colors };
    this.#band = band;
    const started = performance.now();
    this.#drawnAt = started;
    drawScene(canvas, scene, colors);
    performance.clearMeasures(DRAW_MEASURE);
    performance.measure(DRAW_MEASURE, { start: started });
  }

  /** Moves the viewport's box to where the view is, and the canvas to where the map is scrolled. */
  #place(): void {
    const position = this.#position;
    if (position === undefined) {
      return;
    }
    const box = this.#elements.box.current;
    const next = minimapBox(position, this.#height);
    if (box !== null && next.top !== this.#box.top) {
      box.style.transform = `translateY(${next.top}px)`;
    }
    if (box !== null && next.height !== this.#box.height) {
      box.style.height = `${next.height}px`;
    }
    this.#box = next;
    const canvas = this.#elements.canvas.current;
    const top = this.#band === undefined ? 0 : this.#band.top - minimapOffset(position, this.#height);
    if (canvas !== null && top !== this.#canvasTop) {
      canvas.style.transform = `translateY(${top}px)`;
      this.#canvasTop = top;
    }
  }
}

/** The minimap of one transcript; the transcript calls its handle's `scrolled` on every scroll of its view. */
export function TranscriptMinimap({ ref, ...props }: MinimapProps) {
  const map = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const latest = useRef<MinimapProps>({ ref, ...props });
  latest.current = { ref, ...props };
  const [view] = useState(() => new MinimapView({ map, canvas, box }, latest));
  useImperativeHandle(ref, () => view, [view]);
  const { measurements, rows, state, matches, current, container } = props;
  // After every render: redrawn only once what the drawing shows changed.
  useEffect(() => {
    view.track([measurements(), rows, state, matches, current]);
  });
  useEffect(() => {
    const element = container.current;
    const observer =
      element === null || typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(() => view.changed());
    if (element !== null) {
      observer?.observe(element);
    }
    const unwatch = watchTheme(document, () => view.themeChanged());
    return () => {
      observer?.disconnect();
      unwatch();
      view.dispose();
    };
  }, [view, container]);
  return (
    <div className="omfp-minimap" aria-hidden="true">
      <div
        ref={map}
        className="omfp-minimap__map"
        data-testid="transcript-minimap"
        onPointerDown={(event) => view.press(event)}
        onPointerMove={(event) => view.drag(event)}
        onPointerUp={() => view.release()}
        onPointerCancel={() => view.release()}
        onPointerEnter={() => view.enter()}
        onPointerLeave={() => view.leave()}
      >
        <canvas ref={canvas} className="omfp-minimap__canvas" />
        <div ref={box} className="omfp-minimap__box" />
      </div>
    </div>
  );
}
