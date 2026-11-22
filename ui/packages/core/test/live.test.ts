/** The live voice state (PRT-052) and its readable failures (DMN-104); port of the live tests of `reducer.rs`. */

import { describe, expect, test } from "bun:test";
import { emptyThreadState, liveErrorMessage, reduce, replay, type ThreadEvent } from "../src/index.ts";
import { deepFreeze, event, omp } from "./helpers.ts";

const LOGIN = "live voice needs an `openai-codex` login in omp: run `:login openai-codex`, then try again";

function running(): ThreadEvent {
  return event({ kind: "process", state: { state: "running" } });
}

function liveSession(client: string, connection: number): ThreadEvent {
  return event({ kind: "live_session", owner: { client, connection } });
}

function transcript(role: string, turn: number, text: string, last: boolean): ThreadEvent {
  return omp({ type: "live_transcript", role, turn, text, final: last });
}

describe("live voice state", () => {
  // spec: PRT-052
  test("a session marks where the voice's context ends", () => {
    const typed = (id: string, text: string) =>
      omp({ type: "message_end", messageId: id, message: { role: "user", content: text } });
    let state = replay([
      running(),
      typed("m1", "fix the login"),
      typed("m2", "and the logout"),
      liveSession("omfp/web", 1),
    ]);
    expect(state.live?.context_end).toBe(2);

    state = reduce(state, typed("m3", "typed meanwhile"));
    expect(state.live?.context_end).toBe(2);
  });

  // spec: PRT-052, TST-010
  test("a session opens with its owner and follows omp's phases", () => {
    let state = replay([
      running(),
      omp({ type: "live_end", error: "No Codex OAuth credential is available for a live call." }),
    ]);
    expect(state.live_error).toBe(LOGIN);

    state = reduce(state, liveSession("omfp/web", 7));
    expect(state.live).toEqual({
      owner: { client: "omfp/web", connection: 7 },
      phase: null,
      transcript: [],
      context_end: 0,
    });
    expect(state.live_error).toBeNull();

    for (const phase of ["connecting", "listening", "muted"] as const) {
      state = reduce(state, omp({ type: "live_phase", phase }));
      expect(state.live?.phase).toBe(phase);
    }
    const frozen = deepFreeze(state);
    expect(reduce(frozen, omp({ type: "live_levels", input: 0.4, output: 0.1 }))).toBe(frozen);
  });

  // spec: PRT-052, TST-010
  test("transcripts keep one line per role and turn until final", () => {
    const state = replay([
      running(),
      liveSession("omfp/web", 1),
      transcript("user", 1, "Ciao,", false),
      transcript("user", 1, "Ciao, mi senti", false),
      transcript("assistant", 1, "Sì", false),
      transcript("user", 1, "Ciao, mi senti?", true),
      transcript("user", 1, "late partial", false),
      transcript("assistant", 1, "Sì.", true),
      transcript("user", 2, "Grazie", false),
    ]);
    expect(state.live?.transcript.map((line) => [line.role, line.turn, line.text, line.final])).toEqual([
      ["user", 1, "Ciao, mi senti?", true],
      ["assistant", 1, "Sì.", true],
      ["user", 2, "Grazie", false],
    ]);
  });

  // spec: PRT-052, DMN-104, TST-010
  test("live_end clears the session and names its failure", () => {
    const failed = replay([
      running(),
      liveSession("omfp/web", 1),
      omp({ type: "live_phase", phase: "connecting" }),
      omp({ type: "live_phase", phase: "error" }),
      omp({ type: "live_end", error: "No Codex OAuth credential is available for a live call." }),
      omp({ type: "live_phase", phase: "error" }),
    ]);
    expect(failed.live).toBeNull();
    expect(failed.live_error).toBe(LOGIN);

    const ended = replay([
      running(),
      liveSession("omfp/web", 1),
      omp({ type: "live_phase", phase: "listening" }),
      omp({ type: "live_end" }),
    ]);
    expect([ended.live, ended.live_error]).toEqual([null, null]);
  });

  // spec: PRT-052, DMN-101, DMN-103, TST-010
  test("a session without owner or process is closed", () => {
    const refused = replay([running(), liveSession("omfp/web", 1), event({ kind: "live_session", owner: null })]);
    expect([refused.live, refused.live_error]).toEqual([null, null]);

    const stopped = replay([
      running(),
      liveSession("omfp/web", 1),
      omp({ type: "live_phase", phase: "listening" }),
      event({ kind: "process", state: { state: "stopped" } }),
    ]);
    expect(stopped.live).toBeNull();
  });

  // spec: PRT-052
  test("invalid live input changes nothing or becomes an unknown notice, as in Rust", () => {
    const base = deepFreeze(replay([running(), liveSession("omfp/web", 1)]));
    expect(reduce(base, event({ kind: "live_session", owner: { client: "x", connection: -1 } }))).toBe(base);
    expect(reduce(deepFreeze(emptyThreadState()), omp({ type: "live_phase", phase: "listening" }))).toEqual(
      emptyThreadState(),
    );
    const bad = reduce(base, omp({ type: "live_phase", phase: "singing" }));
    expect(bad.notices.map((notice) => notice.frame.type)).toEqual(["unknown"]);
  });

  // spec: DMN-104
  test("failures read as what the user can do", () => {
    expect(liveErrorMessage("No Codex OAuth credential is available for a live call.")).toBe(LOGIN);
    expect(liveErrorMessage("No OAuth credential available for provider: openai-codex")).toBe(LOGIN);
    expect(liveErrorMessage("Codex live signaling failed (401): unauthorized")).toContain("rejected");
    expect(liveErrorMessage("A live session is already active")).toBe(
      "a live session is already running on this thread",
    );
    expect(liveErrorMessage("End dictation before starting live voice")).toBe(
      "end dictation before starting live voice",
    );
    expect(liveErrorMessage("socket closed")).toBe("live voice failed: socket closed");
  });
});
