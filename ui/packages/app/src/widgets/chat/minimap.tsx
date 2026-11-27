/**
 * The transcript's minimap (UI-860..UI-863): a faded overview of the whole conversation at the transcript's right
 * edge, beside the scrollbar, while the user scrolls. A zero-height sticky anchor at the top of the scroll container
 * holds the map, so the wheel over it still scrolls the transcript. One canvas holds the schematic drawing
 * (`minimapShapes`) and one box marks the viewport.
 *
 * The canvas is a cached bitmap: a scroll only moves the box (a CSS transform written at most once per animation frame,
 * from the position the transcript's scroll event already read), never renders React nor redraws the canvas. The
 * canvas is redrawn at most every {@link REDRAW_MS}, only while the map shows, and only when what it shows changed:
 * the rows, their measured heights (the virtualizer's measurements, estimates for rows never rendered, so the drawing
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
  minimapBox,
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

/** Each paint's theme token (UI-134) and opacity, in drawing order: blocks faint, parts and marks stronger. */
const PAINTS: readonly { readonly paint: MinimapPaint; readonly token: string; readonly alpha: number }[] = [
  { paint: "system", token: "outline-variant", alpha: 0.5 },
  { paint: "assistant", token: "on-surface-variant", alpha: 0.16 },
  { paint: "user", token: "tertiary", alpha: 0.3 },
  { paint: "tool", token: "primary", alpha: 0.2 },
  { paint: "error", token: "error", alpha: 0.3 },
  { paint: "line", token: "on-surface-variant", alpha: 0.45 },
  { paint: "toolBar", token: "primary", alpha: 0.55 },
  { paint: "errorBar", token: "error", alpha: 0.75 },
  { paint: "match", token: "search-current-bg", alpha: 1 },
  { paint: "focus", token: "focus-bar", alpha: 1 },
];

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

/** Draws `scene` on `canvas` at the screen's pixel ratio. */
function drawScene(canvas: HTMLCanvasElement, scene: MinimapScene, colors: Colors): void {
  const ratio = window.devicePixelRatio || 1;
  const width = Math.round(scene.width * ratio);
  const height = Math.round(scene.height * ratio);
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
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
  /** The map's height, the transcript's viewport's. */
  #height = 0;
  #box: MinimapBox = { top: Number.NaN, height: Number.NaN };
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
    this.#jump(event.clientY);
  }

  drag(event: PointerEvent<HTMLDivElement>): void {
    if (this.#dragging) {
      this.#jump(event.clientY);
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

  #jump(clientY: number): void {
    const map = this.#elements.map.current;
    const container = this.#props.current.container.current;
    if (map !== null && container !== null) {
      const y = clientY - map.getBoundingClientRect().top;
      this.#props.current.onJump(minimapScrollTop(y, this.#height, scrollPosition(container)));
    }
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
    if (this.#stale) {
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
    this.#placeBox();
    const map = this.#elements.map.current;
    if (map !== null && map.dataset.visible !== "true") {
      map.dataset.visible = "true";
    }
  }

  /** Reads where the rows are and redraws the canvas unless that draws the same picture. */
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
    const width = map.clientWidth;
    const colors = this.#colors ?? readColors(map);
    this.#colors = colors;
    this.#position = position;
    if (this.#height !== position.clientHeight) {
      this.#height = position.clientHeight;
      map.style.height = `${position.clientHeight}px`;
    }
    const scene: MinimapScene = {
      rows: rows.map((row) => {
        const content = rowContent(state, row);
        return content === undefined ? undefined : minimapRow(content);
      }),
      items: measurements(),
      listTop,
      scrollHeight: position.scrollHeight,
      width,
      height: position.clientHeight,
      matches,
      current,
    };
    if (this.#drawn !== undefined && this.#drawn.colors === colors && sameMinimapScene(this.#drawn, scene)) {
      return;
    }
    this.#drawn = { ...scene, colors };
    this.#drawnAt = performance.now();
    drawScene(canvas, scene, colors);
  }

  /** Moves the viewport's box to where the view is. */
  #placeBox(): void {
    const box = this.#elements.box.current;
    if (box === null || this.#position === undefined) {
      return;
    }
    const next = minimapBox(this.#position, this.#height);
    if (next.top !== this.#box.top) {
      box.style.transform = `translateY(${next.top}px)`;
    }
    if (next.height !== this.#box.height) {
      box.style.height = `${next.height}px`;
    }
    this.#box = next;
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
