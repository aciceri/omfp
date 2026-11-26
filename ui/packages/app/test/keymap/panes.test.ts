/** L1: View mode's pane geometry, `Ctrl-w h/j/k/l` as vim's windows (UI-095). */

import { describe, expect, test } from "bun:test";
import { type Box, paneToward } from "../../src/keymap/panes.ts";

function box(left: number, top: number, right: number, bottom: number): Box {
  return { left, top, right, bottom };
}

describe("pane geometry", () => {
  // spec: UI-095
  test("the pane next to the focused one on a side: facing it first, then the nearest, then the most aligned", () => {
    // A transcript in the middle, a files pane on the left, two stacked panes on the right.
    const middle = box(200, 0, 600, 400);
    const panes = [box(0, 0, 200, 400), box(600, 0, 900, 200), box(600, 200, 900, 400), box(200, 400, 600, 500)];
    expect(paneToward(middle, panes, "left")).toBe(0);
    expect(paneToward(middle, panes, "down")).toBe(3);
    expect(paneToward(middle, panes, "up")).toBeUndefined();
    // Both right panes face the middle at the same distance: the one whose middle is nearest wins, the first on a tie.
    expect(paneToward(middle, panes, "right")).toBe(1);
    expect(paneToward(box(200, 250, 600, 400), panes, "right")).toBe(2);
    // From the lower right pane, up is the one above it, not the middle's column.
    expect(paneToward(panes[2] as Box, [middle, ...panes], "up")).toBe(2);
    // A pane further away but facing wins over a nearer one that does not.
    expect(paneToward(box(0, 0, 100, 100), [box(150, 200, 300, 300), box(400, 0, 500, 100)], "right")).toBe(1);
    // 1 px of overlap still counts as beside.
    expect(paneToward(box(0, 0, 100, 100), [box(99, 0, 200, 100)], "right")).toBe(0);
    expect(paneToward(box(0, 0, 100, 100), [box(90, 0, 200, 100)], "right")).toBeUndefined();
  });
});
