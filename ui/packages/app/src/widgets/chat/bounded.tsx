/**
 * A long tool-card body kept bounded (UI-935). Live, a window about `BODY_LINES` tall that follows the newest lines,
 * "… N earlier lines" above it; scrolled up by the user it stops following, and follows again once scrolled back to
 * its end. Settled, its preview (`cut`) and a control expanding it to the whole body, which scrolls inside the card at
 * most about a viewport tall; an output's preview is its end, its control above it. The window and the expanded body
 * take the focus and scroll themselves on the arrow and paging keys, which never reach the transcript around them; nor
 * does a wheel turn they scroll themselves, so the transcript stays pinned (UI-092).
 */

import { Button } from "omfp/kit";
import {
  BODY_LINES,
  bodyCut,
  type Cut,
  earlierLines,
  follows,
  keyScrollTop,
  type Noun,
  strings,
  type WindowScroll,
  wheelScrolls,
} from "omfp/sdk";
import { type KeyboardEvent, type ReactNode, useLayoutEffect, useRef, useState, type WheelEvent } from "react";

const text = strings.chat.tool;

export interface BoundedBodyProps {
  /** The body's lines, or a list's items. */
  readonly total: number;
  /** Whether the call is live (`liveBody`): the body shows in a window following its newest lines. */
  readonly live: boolean;
  /** The lines a settled body's preview shows (`bodyCut`, `diffCut`); `undefined` when the whole body fits. */
  readonly cut: Cut | undefined;
  /** The body's lines `[start, end)`. */
  readonly render: (start: number, end: number) => ReactNode;
  /** What its controls count: lines unless it is a list. */
  readonly noun?: Noun | undefined;
  /** How many a live window shows before counting earlier ones, until it is laid out: `BODY_LINES` by default. */
  readonly limit?: number | undefined;
}

/** A long body: a live window while the call is live, else its settled preview. */
export function BoundedBody({ total, live, cut, render, noun, limit }: BoundedBodyProps): ReactNode {
  return live ? (
    <LiveWindow total={total} noun={noun} limit={limit}>
      {render(0, total)}
    </LiveWindow>
  ) : (
    <Preview total={total} cut={cut} render={render} noun={noun} />
  );
}

/**
 * A list of `items` bounded as a body (UI-935): live, in a window following its newest items; settled, its first
 * `limit` and a control showing the rest. `render` lays out the items shown.
 */
export function BoundedList<T>({
  items,
  limit,
  noun,
  live,
  render,
}: {
  items: readonly T[];
  limit: number;
  noun: Noun;
  live: boolean;
  render: (shown: readonly T[]) => ReactNode;
}): ReactNode {
  return (
    <BoundedBody
      total={items.length}
      live={live}
      cut={bodyCut(items.length, limit)}
      noun={noun}
      limit={limit}
      render={(start, end) => render(start === 0 && end === items.length ? items : items.slice(start, end))}
    />
  );
}

function scrollOf(element: HTMLElement): WindowScroll {
  return { top: element.scrollTop, height: element.scrollHeight, client: element.clientHeight };
}

/**
 * A live body of `total` lines (or items, `noun`) in a window following its newest ones; `children` may hold more than
 * the lines (an edit preview's file names).
 */
export function LiveWindow({
  total,
  noun = text.nouns.line,
  limit = BODY_LINES,
  children,
}: {
  total: number;
  noun?: Noun | undefined;
  limit?: number | undefined;
  children: ReactNode;
}): ReactNode {
  const box = useRef<HTMLElement>(null);
  // Where the window last scrolled itself: a scroll elsewhere is the user's.
  const written = useRef(0);
  const [following, setFollowing] = useState(true);
  const [earlier, setEarlier] = useState(() => earlierLines(total, { top: 0, height: 0, client: 0 }, limit));
  // After every render, since the body may have grown: back to its end while following, then what is above.
  useLayoutEffect(() => {
    const element = box.current;
    if (element === null) {
      return;
    }
    const follow = follows(following, scrollOf(element), written.current);
    if (follow) {
      element.scrollTop = element.scrollHeight;
      written.current = element.scrollTop;
    }
    setFollowing(follow);
    setEarlier(earlierLines(total, scrollOf(element), limit));
  });
  const onScroll = () => {
    const element = box.current;
    if (element === null) {
      return;
    }
    const scroll = scrollOf(element);
    setFollowing((was) => follows(was, scroll, written.current));
    setEarlier(earlierLines(total, scroll, limit));
  };
  return (
    <div className="omfp-bounded" data-live="true">
      {earlier > 0 && (
        <p className="omfp-bounded__marker" data-testid="tool-earlier">
          {text.earlier(earlier, noun)}
        </p>
      )}
      <section
        ref={box}
        className="omfp-bounded__body"
        aria-label={text.bodyLabel}
        // biome-ignore lint/a11y/noNoninteractiveTabindex: a scrollable region must be focusable to scroll by keyboard.
        tabIndex={0}
        data-testid="tool-window"
        data-following={following}
        onScroll={onScroll}
        onKeyDown={scrollByKey}
        onWheel={keepWheel}
      >
        {children}
      </section>
    </div>
  );
}

/**
 * The paging keys scroll a focused body itself (WebKitGTK scrolls no focused scroller on them), and neither they nor
 * the arrows reach the transcript, which would scroll or unpin itself instead (UI-092).
 */
function scrollByKey(event: KeyboardEvent<HTMLElement>): void {
  const box = event.currentTarget;
  const top = keyScrollTop(event.key, box.scrollTop, box.clientHeight, box.scrollHeight);
  if (top !== undefined) {
    event.preventDefault();
    box.scrollTop = top;
  }
  if (top !== undefined || event.key === "ArrowUp" || event.key === "ArrowDown") {
    event.stopPropagation();
  }
}

/**
 * A wheel turn a body scrolls itself stays there: the transcript would take it as the user scrolling the conversation
 * up and stop following it. At the body's edge the turn goes on, scrolling the transcript as usual.
 */
function keepWheel(event: WheelEvent<HTMLElement>): void {
  if (!event.ctrlKey && wheelScrolls(scrollOf(event.currentTarget), event.deltaY)) {
    event.stopPropagation();
  }
}

/** A settled body: its preview, expandable to the whole body; the whole body when it fits. */
function Preview({ total, cut, render, noun = text.nouns.line }: Omit<BoundedBodyProps, "live" | "limit">): ReactNode {
  const [expanded, setExpanded] = useState(false);
  const box = useRef<HTMLElement>(null);
  const tail = cut !== undefined && cut.start > 0;
  // An output's whole body opens at its end, where its preview was.
  useLayoutEffect(() => {
    if (expanded && tail && box.current !== null) {
      box.current.scrollTop = box.current.scrollHeight;
    }
  }, [expanded, tail]);
  if (cut === undefined) {
    return render(0, total);
  }
  const hidden = total - (cut.end - cut.start);
  const toggle = (
    <Button
      size="sm"
      variant="quiet"
      aria-expanded={expanded}
      data-testid="tool-more"
      onPress={() => setExpanded((open) => !open)}
    >
      {expanded ? text.fewer(noun) : tail ? text.earlier(hidden, noun) : text.more(hidden, noun)}
    </Button>
  );
  return (
    <div className="omfp-bounded" data-cut={!expanded} data-tail={tail} data-expanded={expanded}>
      {tail && toggle}
      {/* One element whether expanded or not, so what the body holds (a nested list's toggle, a live window) stays. */}
      <section
        ref={box}
        className="omfp-bounded__body"
        aria-label={expanded ? text.bodyLabel : undefined}
        tabIndex={expanded ? 0 : undefined}
        onKeyDown={expanded ? scrollByKey : undefined}
        onWheel={expanded ? keepWheel : undefined}
      >
        {expanded ? render(0, total) : render(cut.start, cut.end)}
      </section>
      {!tail && toggle}
    </div>
  );
}
