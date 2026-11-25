/**
 * A long tool-card body kept bounded (UI-935). Live, a window about `BODY_LINES` tall that follows the newest lines,
 * "… N earlier lines" above it; scrolled up by the user it stops following, and follows again once scrolled back to
 * its end. Settled, its preview (`cut`) and a control expanding it to the whole body, which scrolls inside the card at
 * most about a viewport tall; an output's preview is its end, its control above it.
 */

import { Button } from "omfp/kit";
import { type Cut, earlierLines, follows, strings, type WindowScroll } from "omfp/sdk";
import { type ReactNode, useLayoutEffect, useRef, useState } from "react";

const text = strings.chat.tool;

export interface BoundedBodyProps {
  /** The body's lines. */
  readonly total: number;
  /** Whether the call is live (`liveBody`): the body shows in a window following its newest lines. */
  readonly live: boolean;
  /** The lines a settled body's preview shows (`bodyCut`, `diffCut`); `undefined` when the whole body fits. */
  readonly cut: Cut | undefined;
  /** The body's lines `[start, end)`. */
  readonly render: (start: number, end: number) => ReactNode;
}

/** A long body: a live window while the call is live, else its settled preview. */
export function BoundedBody({ total, live, cut, render }: BoundedBodyProps): ReactNode {
  return live ? (
    <LiveWindow total={total}>{render(0, total)}</LiveWindow>
  ) : (
    <Preview total={total} cut={cut} render={render} />
  );
}

function scrollOf(element: HTMLElement): WindowScroll {
  return { top: element.scrollTop, height: element.scrollHeight, client: element.clientHeight };
}

/**
 * A live body of `total` lines in a window following its newest lines; `children` may hold more than the lines (an
 * edit preview's file names).
 */
export function LiveWindow({ total, children }: { total: number; children: ReactNode }): ReactNode {
  const box = useRef<HTMLDivElement>(null);
  // Where the window last scrolled itself: a scroll elsewhere is the user's.
  const written = useRef(0);
  const [following, setFollowing] = useState(true);
  const [earlier, setEarlier] = useState(() => earlierLines(total, { top: 0, height: 0, client: 0 }));
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
    setEarlier(earlierLines(total, scrollOf(element)));
  });
  const onScroll = () => {
    const element = box.current;
    if (element === null) {
      return;
    }
    const scroll = scrollOf(element);
    setFollowing((was) => follows(was, scroll, written.current));
    setEarlier(earlierLines(total, scroll));
  };
  return (
    <div className="omfp-bounded" data-live="true">
      {earlier > 0 && (
        <p className="omfp-bounded__marker" data-testid="tool-earlier">
          {text.earlierLines(earlier)}
        </p>
      )}
      <div
        ref={box}
        className="omfp-bounded__body"
        data-testid="tool-window"
        data-following={following}
        onScroll={onScroll}
      >
        {children}
      </div>
    </div>
  );
}

/** A settled body: its preview, expandable to the whole body; the whole body when it fits. */
function Preview({ total, cut, render }: Omit<BoundedBodyProps, "live">): ReactNode {
  const [expanded, setExpanded] = useState(false);
  const box = useRef<HTMLDivElement>(null);
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
      {expanded ? text.fewerLines : tail ? text.earlierLines(hidden) : text.moreLines(hidden)}
    </Button>
  );
  return (
    <div className="omfp-bounded" data-cut={!expanded} data-tail={tail} data-expanded={expanded}>
      {tail && toggle}
      <div ref={box} className="omfp-bounded__body">
        {expanded ? render(0, total) : render(cut.start, cut.end)}
      </div>
      {!tail && toggle}
    </div>
  );
}
