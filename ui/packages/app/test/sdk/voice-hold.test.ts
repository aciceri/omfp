/** L1: the voice's audio stays silent from the first context append until the user first speaks (UI-181). */

import { describe, expect, test } from "bun:test";
import { VoiceHold, voicePlayback } from "../../src/sdk/live.ts";

/** A hold whose playback the test reads, as `openPeer` drives the media element and the gain. */
function playback(running: boolean) {
  const applied: { elementMuted: boolean; gain: number }[] = [];
  const hold = new VoiceHold(() => applied.push(voicePlayback(hold.held, running)));
  return { hold, applied, last: () => applied.at(-1) };
}

describe("the voice's startup hold", () => {
  // spec: UI-181
  test("both playback paths go silent at the first append and come back when the user speaks", () => {
    // The audio context runs: Web Audio plays, the element stays muted.
    const web = playback(true);
    web.hold.apply();
    expect(web.last()).toEqual({ elementMuted: true, gain: 2 });
    web.hold.start();
    expect(web.last()).toEqual({ elementMuted: true, gain: 0 });
    web.hold.message(JSON.stringify({ type: "input_transcript.added", item: { text: "Ciao" } }));
    expect(web.last()).toEqual({ elementMuted: true, gain: 2 });

    // The context waits for a user activation: the element plays, unless held.
    const element = playback(false);
    element.hold.start();
    expect(element.last()).toEqual({ elementMuted: true, gain: 0 });
    element.hold.message(JSON.stringify({ type: "input_transcript.added" }));
    expect(element.last()).toEqual({ elementMuted: false, gain: 2 });
  });

  // spec: UI-181
  test("only the user's first transcript releases it; the voice's own output and other messages do not", () => {
    const { hold, applied, last } = playback(true);
    hold.start();
    for (const message of [
      JSON.stringify({ type: "output_transcript.added", item: { text: " Mm-hmm" } }),
      JSON.stringify({ type: "turn.done", turn: { role: "assistant", transcript: "Mm-hmm" } }),
      JSON.stringify({ type: "session.context.appended" }),
      JSON.stringify({ text: "input_transcript.added" }),
      "not json input_transcript.added",
      new ArrayBuffer(4),
    ]) {
      hold.message(message);
    }
    expect(hold.held).toBe(true);
    expect(last()).toEqual({ elementMuted: true, gain: 0 });
    hold.message(JSON.stringify({ type: "input_transcript.added" }));
    expect(hold.held).toBe(false);
    const count = applied.length;
    hold.message(JSON.stringify({ type: "input_transcript.added" }));
    expect(applied.length).toBe(count);
  });

  // spec: UI-181
  test("without a context append nothing is held", () => {
    const { hold, last } = playback(true);
    hold.message(JSON.stringify({ type: "input_transcript.added" }));
    expect(hold.held).toBe(false);
    hold.apply();
    expect(last()).toEqual({ elementMuted: true, gain: 2 });
  });
});
