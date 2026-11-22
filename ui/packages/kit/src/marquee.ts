/**
 * A one-line title longer than its room scrolls to its end and back (the chat title, the mode line's thread title,
 * UI-202): this hook measures; kit.css animates `[data-overflow]` at motion `full` only, else the ellipsis stays.
 */
import { type RefObject, useEffect } from "react";

/** Scroll speed of an overflowing title, the share of a pass it moves (it rests at each end), the shortest pass. */
const PX_PER_S = 30;
const MOVING_SHARE = 0.6;
const MIN_S = 4;

/**
 * Marks a one-line element whose text overflows it (`data-overflow`) and sets how far its text moves to show its end
 * (`--marquee-shift`) and how long one pass lasts (`--marquee-time`). Marks again after every render (the text
 * changed) and on every resize. The marquee animates `text-indent`, which moves the text and so changes `scrollWidth`
 * (never below `clientWidth`): the measure holds the indent at 0 for its reading (an `!important` inline value wins
 * over the animation), so a pass under way neither skews the mark nor restarts.
 */
export function useMarquee(ref: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    const element = ref.current;
    if (element === null) {
      return undefined;
    }
    const mark = () => {
      element.style.setProperty("text-indent", "0", "important");
      const overflow = Math.round(element.scrollWidth - element.clientWidth);
      element.style.removeProperty("text-indent");
      if (overflow > 1) {
        const seconds = Math.max(MIN_S, overflow / PX_PER_S / MOVING_SHARE);
        element.style.setProperty("--marquee-shift", `${-overflow}px`);
        element.style.setProperty("--marquee-time", `${seconds.toFixed(1)}s`);
        element.setAttribute("data-overflow", "");
      } else {
        element.removeAttribute("data-overflow");
      }
    };
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(mark);
    observer?.observe(element);
    mark();
    return () => observer?.disconnect();
  });
}
