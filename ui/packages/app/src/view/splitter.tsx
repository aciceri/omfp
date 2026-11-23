/**
 * A splitter's behaviour (UI-206, UI-207) over the kit's `Splitter`: a pointer drag (previewed while it moves,
 * applied once released), the arrow keys in 5 % steps and a double click that resets. Deltas are shares of the
 * splitter's parent along its axis.
 */

import { Splitter } from "@omfp/kit";
import { type HTMLAttributes, type KeyboardEvent, type PointerEvent, type ReactNode, useRef, useState } from "react";
import { RESIZE_STEP } from "./edit.ts";

const STEPS: Readonly<Record<string, number>> = { ArrowLeft: -1, ArrowUp: -1, ArrowRight: 1, ArrowDown: 1 };

export interface ResizerProps extends Omit<HTMLAttributes<HTMLDivElement>, "role" | "children" | "onReset"> {
  /** `vertical`: between cells side by side; `horizontal`: between stacked cells. */
  readonly orientation: "vertical" | "horizontal";
  readonly label: string;
  /** Share of the cell before it, 0–100. */
  readonly value: number;
  /** A drag in progress: the share of the parent moved so far (`undefined`: the drag ended or was cancelled). */
  readonly onPreview: (delta: number | undefined) => void;
  /** A finished change: `delta` is the share of the parent moved to the cell before the splitter. */
  readonly onResize: (delta: number) => void;
  /** Double click: back to the default sizes; absent: double click does nothing. */
  readonly onReset?: (() => void) | undefined;
}

export function Resizer({ orientation, onPreview, onResize, onReset, ...props }: ResizerProps): ReactNode {
  const drag = useRef<{ readonly start: number; readonly length: number }>(undefined);
  const [dragging, setDragging] = useState(false);
  const position = (event: PointerEvent) => (orientation === "vertical" ? event.clientX : event.clientY);
  const delta = (event: PointerEvent) =>
    drag.current === undefined ? 0 : (position(event) - drag.current.start) / drag.current.length;
  const end = () => {
    drag.current = undefined;
    setDragging(false);
  };
  return (
    <Splitter
      {...props}
      orientation={orientation}
      data-dragging={dragging || undefined}
      onKeyDown={(event: KeyboardEvent) => {
        const step = STEPS[event.key];
        if (step !== undefined) {
          event.preventDefault();
          onResize(step * RESIZE_STEP);
        }
      }}
      onDoubleClick={onReset}
      onPointerDown={(event) => {
        const box = event.currentTarget.parentElement?.getBoundingClientRect();
        const length = box === undefined ? 0 : orientation === "vertical" ? box.width : box.height;
        if (length > 0 && event.button === 0) {
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = { start: position(event), length };
          setDragging(true);
        }
      }}
      onPointerMove={(event) => {
        if (drag.current !== undefined) {
          onPreview(delta(event));
        }
      }}
      onPointerUp={(event) => {
        if (drag.current === undefined) {
          return;
        }
        const moved = delta(event);
        end();
        if (moved === 0) {
          onPreview(undefined);
        } else {
          onPreview(moved);
          onResize(moved);
        }
      }}
      onPointerCancel={() => {
        end();
        onPreview(undefined);
      }}
    />
  );
}
