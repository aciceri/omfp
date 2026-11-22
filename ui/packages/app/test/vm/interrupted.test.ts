/** L1: the notice of a turn a daemon restart interrupted (UI-932). */

import { describe, expect, test } from "bun:test";
import { interruptedTurn } from "../../src/vm/interrupted.ts";
import { formatWhen } from "../../src/vm/jobs.ts";

describe("interrupted turn", () => {
  // spec: UI-932, PRT-229
  test("a summary with interruptedAt says when the daemon stopped; without one there is no notice", () => {
    const at = new Date(2026, 10, 22, 8, 40).getTime();
    const sameDay = new Date(2026, 10, 22, 9, 15).getTime();
    expect(interruptedTurn({ interruptedAt: at }, sameDay)).toEqual({
      at,
      text: "The agent's last turn was interrupted when the daemon restarted (08:40).",
    });
    const nextDay = new Date(2026, 10, 23, 9, 15).getTime();
    expect(interruptedTurn({ interruptedAt: at }, nextDay)?.text).toContain(`(${formatWhen(at, nextDay)})`);
    expect(formatWhen(at, nextDay)).toBe("2026-11-22 08:40");
    expect(interruptedTurn({ interruptedAt: null }, sameDay)).toBeUndefined();
    expect(interruptedTurn({}, sameDay)).toBeUndefined();
    expect(interruptedTurn(undefined, sameDay)).toBeUndefined();
  });
});
