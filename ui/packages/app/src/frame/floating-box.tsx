/**
 * A panel floating over the view (the file panel, UI-115; the floating widgets, UI-230): its box and minimised state
 * in a window key, moved by a grip and resized by a corner (pointer drag, or arrow keys; Shift moves four times as far).
 * A panel without a box of its own opens below the bars at the top of the view, side by side with the panels before it
 * while they fit across the view, then cascaded (UI-454).
 */

import { IconButton } from "@omfp/kit";
import type { CSSProperties, KeyboardEvent, PointerEvent, ReactNode, RefObject } from "react";
import { useLayoutEffect, useRef, useState } from "react";
import {
  type FloatingPanel,
  movePanel,
  resizePanel,
  type Update,
  useWindowState,
  type WindowKey,
} from "../sdk/index.ts";

/** How far an arrow key moves or resizes the panel; with Shift, four times as far. */
const KEY_STEP = 16;

/** The gap between panels side by side, in px, as the stylesheet's `s3` at normal density (UI-454). */
const PANEL_GAP = 12;

/** The bars a new panel must not cover: the header widget and the tab bars at the top of the view (UI-454). */
const TOP_BARS = '[data-widget="omfp:header"], .omfp-view__tabbar';

/**
 * Where the bars at the top of the shown view end, in px below the top of `container` (the panels' containing block):
 * the header widget and the tab bars starting at the view's top edge (UI-454); 0 without a view.
 */
export function topBarsBottom(container: HTMLElement): number {
  const view = container.querySelector('[data-testid="view"]');
  if (view === null) {
    return 0;
  }
  const viewTop = view.getBoundingClientRect().top;
  let bottom = viewTop;
  for (const bar of view.querySelectorAll(TOP_BARS)) {
    const box = bar.getBoundingClientRect();
    if (box.height > 0 && box.top <= viewTop + 1) {
      bottom = Math.max(bottom, box.bottom);
    }
  }
  return Math.max(0, bottom - container.getBoundingClientRect().top);
}

/**
 * The opening place of the `index`th panel without a box (UI-454): its column from the right while panels of
 * `panelWidth` fit side by side across `containerWidth`, then its row of the cascade.
 */
export function openingSlot(
  index: number,
  containerWidth: number,
  panelWidth: number,
): { readonly column: number; readonly row: number } {
  const columns = Math.max(1, Math.floor((containerWidth + PANEL_GAP) / (panelWidth + PANEL_GAP)));
  return { column: index % columns, row: Math.floor(index / columns) };
}

interface Opening {
  readonly top: number;
  readonly column: number;
  readonly row: number;
}

/** Measures where a panel without a box opens, again when the window or the view's top bars change size. */
function useOpening(root: RefObject<HTMLElement | null>, index: number, placed: boolean): Opening | undefined {
  const [opening, setOpening] = useState<Opening | undefined>(undefined);
  useLayoutEffect(() => {
    if (placed) {
      return undefined;
    }
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(() => measure());
    function measure(): void {
      const element = root.current;
      const container = element?.offsetParent;
      if (element === null || element === undefined || !(container instanceof HTMLElement)) {
        return;
      }
      const top = topBarsBottom(container);
      const { column, row } = openingSlot(index, container.clientWidth, element.offsetWidth);
      setOpening((now) => (now?.top === top && now.column === column && now.row === row ? now : { top, column, row }));
      observer?.disconnect();
      for (const bar of container.querySelectorAll(`[data-testid="view"] :is(${TOP_BARS})`)) {
        observer?.observe(bar);
      }
    }
    measure();
    window.addEventListener("resize", measure);
    return () => {
      window.removeEventListener("resize", measure);
      observer?.disconnect();
    };
  }, [root, index, placed]);
  return opening;
}

type Box = NonNullable<FloatingPanel["box"]>;
export type Gesture = (panel: FloatingPanel, from: Box, dx: number, dy: number) => FloatingPanel;

/** The pointer and key handlers of a grip or a resize corner. */
export interface GestureHandlers {
  readonly onPointerDown: (event: PointerEvent<HTMLElement>) => void;
  readonly onPointerMove: (event: PointerEvent<HTMLElement>) => void;
  readonly onPointerUp: () => void;
  readonly onPointerCancel: () => void;
  readonly onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
}

export interface FloatingBox {
  readonly panel: FloatingPanel;
  readonly setPanel: (update: Update<FloatingPanel>) => void;
  /** The panel's element, measured when a gesture starts before the panel has a box of its own. */
  readonly root: RefObject<HTMLElement | null>;
  /** The handlers applying `gesture` (`movePanel`, `resizePanel`). */
  readonly handle: (gesture: Gesture) => GestureHandlers;
  /**
   * The stored box as inline style; without one, the opening place for the stylesheet (`--panel-top`, the bottom of
   * the view's top bars, `--panel-column` and `--panel-row`, UI-454).
   */
  readonly style: CSSProperties;
}

/** The panel of window key `key`; `index` is its place among the panels opening together (UI-454). */
export function useFloatingBox(key: WindowKey<FloatingPanel>, index = 0): FloatingBox {
  const [panel, setPanel] = useWindowState(key);
  const root = useRef<HTMLElement>(null);
  const opening = useOpening(root, index, panel.box !== undefined);
  const drag = useRef<{ readonly gesture: Gesture; readonly from: Box; readonly x: number; readonly y: number }>(
    undefined,
  );
  /** The panel's box now: the stored one, else where the stylesheet put it. */
  const currentBox = (): Box => {
    const element = root.current;
    return (
      panel.box ?? {
        left: element?.offsetLeft ?? 0,
        top: element?.offsetTop ?? 0,
        width: element?.offsetWidth ?? 0,
        height: element?.offsetHeight ?? 0,
      }
    );
  };
  const handle = (gesture: Gesture): GestureHandlers => ({
    onPointerDown: (event) => {
      if (event.button !== 0) {
        return;
      }
      event.preventDefault();
      event.currentTarget.setPointerCapture?.(event.pointerId);
      drag.current = { gesture, from: currentBox(), x: event.clientX, y: event.clientY };
    },
    onPointerMove: (event) => {
      const started = drag.current;
      if (started !== undefined) {
        setPanel((now) => started.gesture(now, started.from, event.clientX - started.x, event.clientY - started.y));
      }
    },
    onPointerUp: () => {
      drag.current = undefined;
    },
    onPointerCancel: () => {
      drag.current = undefined;
    },
    onKeyDown: (event) => {
      const step = event.shiftKey ? KEY_STEP * 4 : KEY_STEP;
      const delta = (
        {
          ArrowLeft: [-step, 0],
          ArrowRight: [step, 0],
          ArrowUp: [0, -step],
          ArrowDown: [0, step],
        } as Readonly<Record<string, readonly [number, number]>>
      )[event.key];
      if (delta !== undefined) {
        event.preventDefault();
        event.stopPropagation();
        setPanel((now) => gesture(now, currentBox(), delta[0], delta[1]));
      }
    },
  });
  const style: CSSProperties =
    panel.box === undefined
      ? ({
          "--panel-top": `${opening?.top ?? 0}px`,
          "--panel-column": opening?.column ?? index,
          "--panel-row": opening?.row ?? 0,
        } as CSSProperties)
      : {
          left: panel.box.left,
          top: panel.box.top,
          width: panel.box.width,
          height: panel.minimised ? undefined : panel.box.height,
          right: "auto",
          bottom: "auto",
        };
  return { panel, setPanel, root, handle, style };
}

/** The labels of a panel's controls. */
export interface FloatingLabels {
  readonly move: string;
  readonly resize: string;
  readonly minimise: string;
  readonly restore: string;
}

/**
 * The controls of `box`: the grip moving it, the button minimising it to its bar (or showing it again) and the corner
 * resizing it (none while minimised); their test ids are `testId` with `-move`, `-minimise` and `-resize`.
 */
export function floatingControls(
  box: FloatingBox,
  testId: string,
  labels: FloatingLabels,
): { readonly grip: ReactNode; readonly minimise: ReactNode; readonly resize: ReactNode } {
  const { minimised } = box.panel;
  return {
    grip: (
      <button
        type="button"
        className="omfp-floating-file__grip omfp-focus-ring"
        aria-label={labels.move}
        title={labels.move}
        data-testid={`${testId}-move`}
        {...box.handle(movePanel)}
      >
        ⠿
      </button>
    ),
    minimise: (
      <IconButton
        label={minimised ? labels.restore : labels.minimise}
        icon={minimised ? "□" : "–"}
        onPress={() => box.setPanel((now) => ({ ...now, minimised: !now.minimised }))}
        data-testid={`${testId}-minimise`}
      />
    ),
    resize: !minimised && (
      <button
        type="button"
        className="omfp-floating-file__resize omfp-focus-ring"
        aria-label={labels.resize}
        title={labels.resize}
        data-testid={`${testId}-resize`}
        {...box.handle(resizePanel)}
      />
    ),
  };
}
