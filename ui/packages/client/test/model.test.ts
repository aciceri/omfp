/** L0 tests of the client model: index and thread events, snapshots, replay, resync and change tracking. */

import { describe, expect, test } from "bun:test";
import { replay, type ThreadEvent } from "@omfp/core";
import {
  type EventFrame,
  INDEX_TOPIC,
  type JobState,
  type JobsState,
  type JobView,
  jobsTopic,
  jobTopic,
  type RunInfo,
  type SubscribeResult,
  type TerminalState,
  type TopicEvent,
  terminalTopic,
  threadTopic,
  type ViewsState,
  viewsTopic,
  type WorkspaceBinding,
} from "@omfp/protocol";
import { decodeServerFrame } from "../src/decode.ts";
import {
  type Dirty,
  isDirtyEmpty,
  JOB_TEXT_LIMIT,
  Model,
  RUN_EVENTS_KEPT,
  TERMINAL_BYTES_LIMIT,
  TERMINAL_CHUNKS_LIMIT,
  type ThreadView,
} from "../src/index.ts";
import { FIXTURES, fixture, indexFrame, indexState, summary, threadFrame, workspace } from "./support.ts";

const THREAD = "01THREAD";
const TOPIC = threadTopic(THREAD);

function snapshot(epoch: string, seq: number, state: unknown): SubscribeResult {
  return { mode: "snapshot", epoch, seq, state };
}

/** A model following `thread/<THREAD>` from a snapshot of `events` at `epoch`/`events.length`. */
function following(epoch: string, events: ThreadEvent[]): Model {
  const model = new Model();
  expect(model.track(TOPIC)).toBe(true);
  const started = model.applySubscribed(TOPIC, snapshot(epoch, events.length, replay(events)));
  expect(started).toEqual({ ok: true, mode: "snapshot" });
  return model;
}

function applyAll(model: Model, epoch: string, firstSeq: number, events: ThreadEvent[]): void {
  for (const [offset, event] of events.entries()) {
    const seq = firstSeq + offset;
    expect({ seq, applied: model.applyEvent(threadFrame(THREAD, epoch, seq, event)) }).toEqual({
      seq,
      applied: "applied",
    });
  }
}

function view(model: Model): ThreadView {
  const thread = model.thread(THREAD);
  if (thread === undefined) {
    throw new Error("thread not followed");
  }
  return thread;
}

function eventAt(events: ThreadEvent[], at: number): ThreadEvent {
  const event = events[at];
  if (event === undefined) {
    throw new Error(`no event ${at}`);
  }
  return event;
}

describe("model", () => {
  // spec: UI-010, PRT-021
  test("a snapshot plus the following events equals the reducer over every event", () => {
    for (const name of FIXTURES) {
      const events = fixture(name);
      for (const split of [0, Math.floor(events.length / 2), events.length]) {
        const model = following("e1", events.slice(0, split));

        applyAll(model, "e1", split + 1, events.slice(split));

        expect({ name, split, state: view(model).state }).toEqual({ name, split, state: replay(events) });
        expect(model.topic(TOPIC)?.position).toEqual({ epoch: "e1", seq: events.length });
      }
    }
  });

  // spec: PRT-020, UI-003
  test("after a connection loss the topic resubscribes from its position and replay continues", () => {
    const events = fixture("bash_tool_call");
    const model = following("e1", events.slice(0, 10));
    applyAll(model, "e1", 11, events.slice(10, 20));

    model.connectionLost();

    expect(model.topic(TOPIC)?.sync).toEqual({ kind: "subscribing" });
    // Events of the old connection no longer apply.
    expect(model.applyEvent(threadFrame(THREAD, "e1", 21, eventAt(events, 20)))).toBe("ignored");
    expect(model.subscribeParams(TOPIC)?.since).toEqual({ epoch: "e1", seq: 20 });
    const replayed = model.applySubscribed(TOPIC, { mode: "replay", epoch: "e1", fromSeq: 21 });
    expect(replayed).toEqual({ ok: true, mode: "replay" });
    applyAll(model, "e1", 21, events.slice(20));
    expect(view(model).state).toEqual(replay(events));
    expect(model.topic(TOPIC)?.sync).toEqual({ kind: "live", mode: "replay" });
  });

  // spec: PRT-020
  test("a replay that does not continue the position asks for a snapshot", () => {
    const events = fixture("text_reply");
    const answers: SubscribeResult[] = [
      { mode: "replay", epoch: "e2", fromSeq: 6 },
      { mode: "replay", epoch: "e1", fromSeq: 9 },
    ];
    for (const answer of answers) {
      const model = following("e1", events.slice(0, 5));
      model.connectionLost();

      const result = model.applySubscribed(TOPIC, answer);

      expect(result.ok === false && result.error.kind).toBe("replay_mismatch");
      expect(model.subscribeParams(TOPIC)?.since).toBeUndefined();
    }
  });

  // spec: PRT-022
  test("duplicates are ignored and a gap drops the position", () => {
    const events = fixture("text_reply");
    const model = following("e1", events.slice(0, 3));
    const before = view(model).state;

    expect(model.applyEvent(threadFrame(THREAD, "e1", 3, eventAt(events, 2)))).toBe("ignored");
    expect(view(model).state).toEqual(before);

    expect(model.applyEvent(threadFrame(THREAD, "e1", 5, eventAt(events, 4)))).toBe("gap");
    expect(view(model).state).toEqual(before);
    expect(model.topic(TOPIC)?.sync).toEqual({ kind: "subscribing" });
    expect(model.subscribeParams(TOPIC)?.since).toBeUndefined();
  });

  // spec: PRT-022
  test("a new epoch continues at seq one and anything else is a gap", () => {
    const events = fixture("restart_with_history");
    const model = following("e1", events.slice(0, 4));

    // A new omp process restarts numbering; the daemon's state carries over.
    applyAll(model, "e2", 1, events.slice(4));
    expect(view(model).state).toEqual(replay(events));

    const jumped = following("e1", events.slice(0, 4));
    expect(jumped.applyEvent(threadFrame(THREAD, "e2", 3, eventAt(events, 4)))).toBe("gap");
  });

  // spec: PRT-023
  test("resync keeps the data until the snapshot replaces it", () => {
    const events = fixture("text_reply");
    const model = following("e1", events.slice(0, 10));
    const stale = view(model).state;

    expect(model.resync(TOPIC)).toBe(true);

    expect(view(model).state).toEqual(stale);
    expect(model.applyEvent(threadFrame(THREAD, "e1", 11, eventAt(events, 10)))).toBe("ignored");
    expect(model.subscribeParams(TOPIC)?.since).toBeUndefined();
    const fresh = replay(events);
    expect(model.applySubscribed(TOPIC, snapshot("e1", events.length, fresh))).toEqual({ ok: true, mode: "snapshot" });
    expect(view(model).state).toEqual(fresh);
    // Untracked topics are not resynced.
    expect(model.resync(INDEX_TOPIC)).toBe(false);
  });

  test("the index is keyed and ordered by id and follows its events", () => {
    const model = new Model();
    model.track(INDEX_TOPIC);
    const state = indexState({
      workspaces: [workspace("01W2"), workspace("01W1")],
      threads: [summary("01T2", "second"), summary("01T1", "first")],
    });
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i1", 7, state)).ok).toBe(true);

    const renamed = summary("01T1", "renamed");
    const frames: EventFrame[] = [
      indexFrame("i1", 8, { kind: "thread_upserted", thread: renamed }),
      indexFrame("i1", 9, { kind: "thread_upserted", thread: summary("01T0", "zeroth") }),
      indexFrame("i1", 10, { kind: "thread_removed", threadId: "01T2" }),
      indexFrame("i1", 11, { kind: "workspace_removed", workspaceId: "01W2" }),
    ];
    for (const frame of frames) {
      expect(model.applyEvent(frame)).toBe("applied");
    }

    const { index } = model;
    expect([...index.threads.keys()]).toEqual(["01T0", "01T1"]);
    expect(index.threads.get("01T1")).toEqual(renamed);
    expect([...index.workspaces.keys()]).toEqual(["01W1"]);
  });

  // spec: DMN-075
  test("the index carries the user's theme and follows theme changes", () => {
    const model = new Model();
    expect(model.index.theme).toEqual({ css: null });
    model.track(INDEX_TOPIC);
    const themed = indexState({ theme: { css: ":root { --omfp-primary: red; }" } });
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i1", 1, themed)).ok).toBe(true);
    expect(model.index.theme).toEqual({ css: ":root { --omfp-primary: red; }" });
    model.takeDirty();

    const cleared = indexFrame("i1", 2, { kind: "theme_changed", theme: { css: null } });
    expect(model.applyEvent(cleared)).toBe("applied");

    expect(model.index.theme).toEqual({ css: null });
    expect(model.takeDirty().index).toBe(true);
  });

  // spec: DMN-075
  test("a snapshot without a theme has the built-in themes only", () => {
    const model = new Model();
    model.track(INDEX_TOPIC);
    const { theme: _, ...older } = indexState({ threads: [summary("01T1", "one")] });
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i1", 1, older)).ok).toBe(true);
    expect(model.index.theme).toEqual({ css: null });
    expect(model.index.threads.size).toBe(1);

    const broken = { ...indexState(), theme: { css: 42 } };
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i2", 1, broken)).ok).toBe(false);
  });

  // spec: DMN-009, UI-097
  test("the index carries the UI settings and follows settings changes", () => {
    const model = new Model();
    expect(model.index.settings).toEqual({ ui: {}, keymap: [], error: null });
    model.track(INDEX_TOPIC);
    const { settings: _, ...older } = indexState();
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i1", 1, older)).ok).toBe(true);
    expect(model.index.settings).toEqual({ ui: {}, keymap: [], error: null });
    model.takeDirty();

    const settings = {
      ui: { density: "normal", fonts: { prose: "Literata, serif" } },
      keymap: [{ mode: "normal", keys: "Space t", command: "theme.set" }],
      error: "config.toml: unknown field `densty`",
    } as const;
    const changed = indexFrame("i1", 2, {
      kind: "settings_changed",
      settings: { ...settings, keymap: [...settings.keymap] },
    });
    expect(model.applyEvent(changed)).toBe("applied");

    expect(model.index.settings).toEqual({ ...settings, keymap: [...settings.keymap] });
    expect(model.takeDirty().index).toBe(true);
    const broken = { ...indexState(), settings: { ui: {}, keymap: [{ keys: 1 }], error: null } };
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i2", 1, broken)).ok).toBe(false);
  });

  test("a snapshot replaces the previous state and position", () => {
    const model = new Model();
    model.track(INDEX_TOPIC);
    const first = indexState({ workspaces: [workspace("01W1")], threads: [summary("01T1", "one")] });
    model.applySubscribed(INDEX_TOPIC, snapshot("i1", 3, first));
    const second = indexState({ threads: [summary("01T9", "nine")] });

    model.applySubscribed(INDEX_TOPIC, snapshot("i2", 1, second));

    expect([...model.index.threads.keys()]).toEqual(["01T9"]);
    expect(model.index.workspaces.size).toBe(0);
    expect(model.topic(INDEX_TOPIC)?.position).toEqual({ epoch: "i2", seq: 1 });
  });

  test("events of unsubscribed topics or before the snapshot are ignored", () => {
    const model = new Model();
    const event: ThreadEvent = { kind: "process", state: { state: "starting" } };
    expect(model.applyEvent(threadFrame(THREAD, "e1", 1, event))).toBe("ignored");

    model.track(TOPIC);
    expect(model.applyEvent(threadFrame(THREAD, "e1", 1, event))).toBe("ignored");
    expect(model.thread(THREAD)).toBeUndefined();
  });

  test("an invalid snapshot fails the topic", () => {
    const model = new Model();
    model.track(TOPIC);

    const result = model.applySubscribed(TOPIC, snapshot("e1", 1, { nope: true }));

    expect(result.ok === false && result.error.kind).toBe("snapshot");
    const sync = model.topic(TOPIC)?.sync;
    expect(sync?.kind === "failed" && sync.error.code).toBe("internal");
    // A failed topic is subscribed again.
    expect(model.track(TOPIC)).toBe(true);
  });

  test("unsubscribing drops the topic and its data", () => {
    const model = following("e1", fixture("text_reply"));
    model.takeDirty();

    expect(model.untrack(TOPIC)).toBe(true);

    expect(model.thread(THREAD)).toBeUndefined();
    expect(model.topics).toEqual([]);
    const dirty = model.takeDirty();
    expect(dirty.topics).toBe(true);
    expect([...dirty.threads]).toEqual([THREAD]);
    expect(model.untrack(TOPIC)).toBe(false);
  });

  test("workspace topics keep the git status of each repository", () => {
    const model = new Model();
    const topic = "workspace/01W";
    model.track(topic);

    expect(model.applySubscribed(topic, snapshot("w1", 0, [])).ok).toBe(true);
    expect(model.workspaces.get("01W")).toEqual([]);

    const frame: EventFrame = {
      type: "event",
      topic,
      epoch: "w1",
      seq: 1,
      event: {
        kind: "status",
        repos: [{ name: "app", status: { ahead: 1, behind: 0, dirty: false, files: [] } }],
      },
    };
    expect(model.applyEvent(frame)).toBe("applied");
    const repos = model.workspaces.get("01W") ?? [];
    expect(repos.map((repo) => [repo.name, repo.status.ahead])).toEqual([["app", 1]]);
  });

  // spec: PRT-040
  test("views topics keep the manifest, the widget folders and the stored values", () => {
    const model = new Model();
    const topic = viewsTopic("01W");
    model.track(topic);
    const state: ViewsState = {
      manifest: null,
      error: null,
      widgets: [{ source: "workspace", name: "tests", version: "v1", entry: "index.tsx" }],
      stored: { tests: { "job:make": "01JOB" } },
      keymap: [],
      keymapError: null,
      init: null,
      viewFiles: null,
    };
    expect(model.applySubscribed(topic, snapshot("v1", 0, state))).toEqual({ ok: true, mode: "snapshot" });
    expect(model.views.get("01W")).toEqual(state);
    model.takeDirty();

    const manifest = { views: { main: { layout: "omfp:chat" } } };
    const events: TopicEvent[] = [
      { kind: "views_manifest", manifest, error: null },
      { kind: "views_widgets", widgets: [{ source: "user", name: "clock", version: "u1", entry: null }] },
      { kind: "widget_stored", instance: "tests", key: "count", value: 3 },
      { kind: "widget_stored", instance: "tests", key: "job:make", value: null },
      { kind: "widget_stored", instance: "other", key: "k", value: { a: [1] } },
    ];
    for (const [at, event] of events.entries()) {
      expect(model.applyEvent({ type: "event", topic, epoch: "v1", seq: at + 1, event })).toBe("applied");
    }

    expect(model.views.get("01W")).toEqual({
      manifest,
      error: null,
      widgets: [{ source: "user", name: "clock", version: "u1", entry: null }],
      stored: { tests: { count: 3 }, other: { k: { a: [1] } } },
      keymap: [],
      keymapError: null,
      init: null,
      viewFiles: null,
    });
    expect([...model.takeDirty().views]).toEqual(["01W"]);

    const invalid = { kind: "views_manifest", error: "$.views: at least one view" } as const;
    expect(model.applyEvent({ type: "event", topic, epoch: "v1", seq: 6, event: invalid })).toBe("applied");
    expect(model.views.get("01W")?.manifest).toBeNull();
    expect(model.views.get("01W")?.error).toBe("$.views: at least one view");
    const last = { kind: "widget_stored", instance: "other", key: "k", value: null } as const;
    expect(model.applyEvent({ type: "event", topic, epoch: "v1", seq: 7, event: last })).toBe("applied");
    expect(model.views.get("01W")?.stored).toEqual({ tests: { count: 3 } });

    expect(model.untrack(topic)).toBe(true);
    expect(model.views.has("01W")).toBe(false);
    expect([...model.takeDirty().views]).toEqual(["01W"]);
  });

  // spec: PRT-060
  test("a views topic keeps the workspace keymap; a snapshot without one has no bindings", () => {
    const model = new Model();
    const topic = viewsTopic("01W");
    model.track(topic);
    const older = { manifest: null, error: null, widgets: [], stored: {} };
    expect(model.applySubscribed(topic, snapshot("v1", 0, older))).toEqual({ ok: true, mode: "snapshot" });
    expect(model.views.get("01W")).toEqual({ ...older, keymap: [], keymapError: null, init: null, viewFiles: null });

    const keymap: WorkspaceBinding[] = [
      { mode: "normal", keys: "g t", command: "tests.run", args: ["unit"] },
      { mode: "insert", keys: "Ctrl-t", command: "" },
    ];
    const changed: TopicEvent = { kind: "views_keymap", keymap, error: null };
    expect(model.applyEvent({ type: "event", topic, epoch: "v1", seq: 1, event: changed })).toBe("applied");
    expect(model.views.get("01W")?.keymap).toEqual(keymap);
    const invalid: TopicEvent = { kind: "views_keymap", keymap, error: "bindings[0].keys: expected" };
    expect(model.applyEvent({ type: "event", topic, epoch: "v1", seq: 2, event: invalid })).toBe("applied");
    expect(model.views.get("01W")?.keymap).toEqual(keymap);
    expect(model.views.get("01W")?.keymapError).toBe("bindings[0].keys: expected");

    const frame = (bindings: unknown) => ({
      type: "event",
      topic,
      epoch: "v1",
      seq: 3,
      event: { kind: "views_keymap", keymap: bindings, error: null },
    });
    expect(decodeServerFrame(frame(keymap)).ok).toBe(true);
    expect(decodeServerFrame(frame([{ keys: "g t" }])).ok).toBe(false);
    expect(decodeServerFrame(frame([{ mode: "normal", keys: "g t", command: "x", args: [1] }])).ok).toBe(false);
  });

  // spec: PRT-095
  test("the init versions: the index and views topics follow them; snapshots without one have none", () => {
    const model = new Model();
    expect(model.index.init).toBeNull();
    model.track(INDEX_TOPIC);
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i1", 1, indexState())).ok).toBe(true);
    expect(model.index.init).toBeNull();
    model.takeDirty();
    expect(model.applyEvent(indexFrame("i1", 2, { kind: "init_changed", init: "0123456789abcdef" }))).toBe("applied");
    expect(model.index.init).toBe("0123456789abcdef");
    expect(model.takeDirty().index).toBe(true);
    expect(model.applyEvent(indexFrame("i1", 3, { kind: "init_changed", init: null }))).toBe("applied");
    expect(model.index.init).toBeNull();
    expect(
      model.applySubscribed(INDEX_TOPIC, snapshot("i2", 1, { ...indexState(), init: "aaaaaaaaaaaaaaaa" })).ok,
    ).toBe(true);
    expect(model.index.init).toBe("aaaaaaaaaaaaaaaa");
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i3", 1, { ...indexState(), init: 1 })).ok).toBe(false);

    const topic = viewsTopic("01W");
    model.track(topic);
    const older = { manifest: null, error: null, widgets: [], stored: {} };
    expect(model.applySubscribed(topic, snapshot("v1", 0, older)).ok).toBe(true);
    expect(model.views.get("01W")?.init).toBeNull();
    const changed: TopicEvent = { kind: "views_init", init: "fedcba9876543210" };
    expect(model.applyEvent({ type: "event", topic, epoch: "v1", seq: 1, event: changed })).toBe("applied");
    expect(model.views.get("01W")?.init).toBe("fedcba9876543210");
    const removed: TopicEvent = { kind: "views_init", init: null };
    expect(model.applyEvent({ type: "event", topic, epoch: "v1", seq: 2, event: removed })).toBe("applied");
    expect(model.views.get("01W")?.init).toBeNull();
    expect(model.applySubscribed(topic, snapshot("v2", 0, { ...older, init: 2 })).ok).toBe(false);

    const frame = (kind: string, init: unknown) => ({
      type: "event",
      topic,
      epoch: "v1",
      seq: 3,
      event: { kind, init },
    });
    expect(decodeServerFrame(frame("views_init", "x")).ok).toBe(true);
    expect(decodeServerFrame(frame("views_init", 3)).ok).toBe(false);
    const indexEvent = (init: unknown) => ({
      type: "event",
      topic: INDEX_TOPIC,
      epoch: "i1",
      seq: 4,
      event: { kind: "init_changed", init },
    });
    expect(decodeServerFrame(indexEvent(null)).ok).toBe(true);
    expect(decodeServerFrame(indexEvent({})).ok).toBe(false);
  });

  // spec: PRT-141
  test("the usable models: the index follows the count; snapshots without one leave it unknown", () => {
    const model = new Model();
    expect(model.index.models).toBeNull();
    model.track(INDEX_TOPIC);
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i1", 1, indexState())).ok).toBe(true);
    expect(model.index.models).toBeNull();
    model.takeDirty();
    expect(model.applyEvent(indexFrame("i1", 2, { kind: "models_changed", models: 0 }))).toBe("applied");
    expect(model.index.models).toBe(0);
    expect(model.takeDirty().index).toBe(true);
    expect(model.applyEvent(indexFrame("i1", 3, { kind: "models_changed", models: 3 }))).toBe("applied");
    expect(model.index.models).toBe(3);
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i2", 1, { ...indexState(), models: 2 })).ok).toBe(true);
    expect(model.index.models).toBe(2);
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i3", 1, { ...indexState(), models: -1 })).ok).toBe(false);
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i4", 1, { ...indexState(), models: "2" })).ok).toBe(false);
    const changed = (models: unknown) => ({
      type: "event",
      topic: INDEX_TOPIC,
      epoch: "i1",
      seq: 4,
      event: { kind: "models_changed", models },
    });
    expect(decodeServerFrame(changed(1)).ok).toBe(true);
    expect(decodeServerFrame(changed(null)).ok).toBe(false);
    expect(decodeServerFrame(changed(1.5)).ok).toBe(false);
  });

  // spec: DMN-290, UI-640
  test("the running login: the index follows it; snapshots without one have none, missing parts read as null", () => {
    const model = new Model();
    expect(model.index.login).toBeNull();
    model.track(INDEX_TOPIC);
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i1", 1, indexState())).ok).toBe(true);
    expect(model.index.login).toBeNull();
    const login = { providerId: "kimi-code", name: "Kimi Code", url: null, instructions: null, progress: "Waiting" };
    expect(model.applyEvent(indexFrame("i1", 2, { kind: "login_changed", login }))).toBe("applied");
    expect(model.index.login).toEqual(login);
    expect(model.applyEvent(indexFrame("i1", 3, { kind: "login_changed", login: null }))).toBe("applied");
    expect(model.index.login).toBeNull();
    const partial = { providerId: "kimi-code", name: "Kimi Code" };
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i2", 1, { ...indexState(), login: partial })).ok).toBe(true);
    expect(model.index.login).toEqual({ ...partial, url: null, instructions: null, progress: null });
    expect(model.applySubscribed(INDEX_TOPIC, snapshot("i3", 1, { ...indexState(), login: { name: "x" } })).ok).toBe(
      false,
    );
    const changed = (value: unknown) => ({
      type: "event",
      topic: INDEX_TOPIC,
      epoch: "i1",
      seq: 4,
      event: { kind: "login_changed", login: value },
    });
    expect(decodeServerFrame(changed(login)).ok).toBe(true);
    expect(decodeServerFrame(changed(null)).ok).toBe(true);
    expect(decodeServerFrame(changed({ ...login, url: 3 })).ok).toBe(false);
  });

  // spec: PRT-096
  test("the view files version follows the views topic; snapshots without one have none", () => {
    const model = new Model();
    const topic = viewsTopic("01W");
    model.track(topic);
    const older = { manifest: null, error: null, widgets: [], stored: {} };
    expect(model.applySubscribed(topic, snapshot("v1", 0, older)).ok).toBe(true);
    expect(model.views.get("01W")?.viewFiles).toBeNull();
    const changed: TopicEvent = { kind: "views_files", viewFiles: "0123456789abcdef" };
    expect(model.applyEvent({ type: "event", topic, epoch: "v1", seq: 1, event: changed })).toBe("applied");
    expect(model.views.get("01W")?.viewFiles).toBe("0123456789abcdef");
    const removed: TopicEvent = { kind: "views_files", viewFiles: null };
    expect(model.applyEvent({ type: "event", topic, epoch: "v1", seq: 2, event: removed })).toBe("applied");
    expect(model.views.get("01W")?.viewFiles).toBeNull();
    expect(model.applySubscribed(topic, snapshot("v2", 0, { ...older, viewFiles: 2 })).ok).toBe(false);
    const frame = (viewFiles: unknown) => ({
      type: "event",
      topic,
      epoch: "v1",
      seq: 3,
      event: { kind: "views_files", viewFiles },
    });
    expect(decodeServerFrame(frame("x")).ok).toBe(true);
    expect(decodeServerFrame(frame(3)).ok).toBe(false);
  });

  // spec: PRT-040
  test("a views snapshot that is not a ViewsState fails the topic", () => {
    const model = new Model();
    const topic = viewsTopic("01W");
    model.track(topic);
    const result = model.applySubscribed(topic, snapshot("v1", 0, { widgets: [{ name: "x" }], stored: {} }));
    expect(result.ok === false && result.error.kind).toBe("snapshot");
  });

  // spec: PRT-041
  test("job topics append output lines and record the exit", () => {
    const model = new Model();
    const topic = jobTopic("01JOB");
    model.track(topic);
    const state: JobState = {
      job: { id: "01JOB", workspaceId: "01W", command: "make", startedAt: 5, status: { state: "running" } },
      lines: [{ stream: "stdout", text: "one" }],
      dropped: 2,
    };
    expect(model.applySubscribed(topic, snapshot("j1", 1, state)).ok).toBe(true);
    model.takeDirty();

    const events: TopicEvent[] = [
      { kind: "job_output", stream: "stderr", text: "two" },
      { kind: "job_exited", code: 1, signal: null, endedAt: 9 },
    ];
    for (const [at, event] of events.entries()) {
      expect(model.applyEvent({ type: "event", topic, epoch: "j1", seq: at + 2, event })).toBe("applied");
    }

    expect(model.jobs.get("01JOB")).toEqual({
      job: {
        id: "01JOB",
        workspaceId: "01W",
        command: "make",
        startedAt: 5,
        status: { state: "exited", code: 1, signal: null, endedAt: 9 },
      },
      lines: [
        { stream: "stdout", text: "one" },
        { stream: "stderr", text: "two" },
      ],
      dropped: 2,
    });
    expect([...model.takeDirty().jobs]).toEqual(["01JOB"]);
    expect(model.untrack(topic)).toBe(true);
    expect(model.jobs.has("01JOB")).toBe(false);
    expect([...model.takeDirty().jobs]).toEqual(["01JOB"]);
  });

  // spec: TRM-100, TRM-102
  test("terminal topics append byte chunks and record exit", () => {
    const model = new Model();
    const topic = terminalTopic("01TERMINAL");
    model.track(topic);
    const state: TerminalState = {
      terminal: {
        id: "01TERMINAL",
        workspaceId: "01W",
        command: "sh",
        cwd: "/workspace",
        cols: 80,
        rows: 24,
        startedAt: 5,
        status: { state: "running" },
      },
      scrollback: ["b25l"],
      dropped: 0,
    };
    expect(model.applySubscribed(topic, snapshot("t1", 1, state)).ok).toBe(true);
    model.takeDirty();

    const events: TopicEvent[] = [
      { kind: "output", data: "dHdv" },
      { kind: "exited", code: 0, endedAt: 9 },
    ];
    for (const [at, event] of events.entries()) {
      expect(model.applyEvent({ type: "event", topic, epoch: "t1", seq: at + 2, event })).toBe("applied");
    }

    expect(model.terminals.get("01TERMINAL")).toEqual({
      ...state,
      terminal: { ...state.terminal, status: { state: "exited", code: 0, endedAt: 9 } },
      scrollback: ["b25l", "dHdv"],
    });
    expect([...model.takeDirty().terminals]).toEqual(["01TERMINAL"]);
    expect(model.untrack(topic)).toBe(true);
    expect(model.terminals.has("01TERMINAL")).toBe(false);
  });

  // spec: TRM-103
  test("a terminal topic bounds live byte chunks like its daemon snapshot", () => {
    const model = new Model();
    const topic = terminalTopic("01TERMINAL");
    model.track(topic);
    const half = btoa("x".repeat(TERMINAL_BYTES_LIMIT / 2));
    const state: TerminalState = {
      terminal: {
        id: "01TERMINAL",
        workspaceId: "01W",
        command: "sh",
        cwd: "/workspace",
        cols: 80,
        rows: 24,
        startedAt: 5,
        status: { state: "running" },
      },
      scrollback: [half, half],
      dropped: 1,
    };
    expect(model.applySubscribed(topic, snapshot("t1", 1, state)).ok).toBe(true);
    const event: TopicEvent = { kind: "output", data: "dGFpbA==" };
    expect(model.applyEvent({ type: "event", topic, epoch: "t1", seq: 2, event })).toBe("applied");

    const kept = model.terminals.get("01TERMINAL");
    expect(kept?.scrollback.map((chunk) => chunk.length)).toEqual([half.length, 8]);
    expect(kept?.dropped).toBe(1 + TERMINAL_BYTES_LIMIT / 2);
    for (let seq = 3; seq < TERMINAL_CHUNKS_LIMIT + 2; seq += 1) {
      const oneByte: TopicEvent = { kind: "output", data: "eA==" };
      expect(model.applyEvent({ type: "event", topic, epoch: "t1", seq, event: oneByte })).toBe("applied");
    }
    const chunkBounded = model.terminals.get("01TERMINAL");
    expect(chunkBounded?.scrollback.length).toBe(TERMINAL_CHUNKS_LIMIT);
    expect(chunkBounded?.dropped).toBe(1 + TERMINAL_BYTES_LIMIT);
  });

  // spec: PRT-200
  test("a jobs topic replaces its state on jobs_set, a job by name on job_updated, and keeps run events", () => {
    const model = new Model();
    const topic = jobsTopic("01W");
    model.track(topic);
    const run: RunInfo = {
      id: "01RUN",
      workspaceId: "01W",
      job: "deploy",
      trigger: "manual",
      startedBy: null,
      threadId: null,
      startedAt: 5,
      endedAt: null,
      status: "running",
      reason: null,
      exitCode: null,
      signal: null,
    };
    const job = (name: string, status: JobView["status"] = "idle"): JobView => ({
      name,
      job: { trigger: "manual", missed: "skip", restart: "always", enabled: true, command: "make" },
      status,
      run: null,
      lastRun: null,
      nextAt: null,
    });
    const state: JobsState = { jobs: [job("b"), job("d")], error: null };
    expect(model.applySubscribed(topic, snapshot("j1", 1, { jobs: state.jobs }))).toEqual({
      ok: true,
      mode: "snapshot",
    });
    expect(model.workspaceJobs.get("01W")).toEqual({ state, runs: [] });
    expect([...model.takeDirty().workspaceJobs]).toEqual(["01W"]);

    const events: TopicEvent[] = [
      { kind: "job_updated", job: job("c") },
      { kind: "job_updated", job: job("a") },
      { kind: "run_started", run },
      { kind: "job_updated", job: { ...job("b", "running"), run } },
      { kind: "run_ended", run: { ...run, status: "failed", endedAt: 9, exitCode: 1 } },
    ];
    for (const [at, event] of events.entries()) {
      expect(decodeServerFrame({ type: "event", topic, epoch: "j1", seq: at + 2, event }).ok).toBe(true);
      expect(model.applyEvent({ type: "event", topic, epoch: "j1", seq: at + 2, event })).toBe("applied");
    }
    const followed = model.workspaceJobs.get("01W");
    expect(followed?.state.jobs.map(({ name, status }) => `${name}:${status}`)).toEqual([
      "a:idle",
      "b:running",
      "c:idle",
      "d:idle",
    ]);
    expect(followed?.runs.map(({ seq, kind, run }) => `${seq}:${kind}:${run.status}`)).toEqual([
      "1:run_started:running",
      "2:run_ended:failed",
    ]);

    const refused: TopicEvent = { kind: "jobs_set", jobs: [job("a")], error: "jobs.x: unknown key" };
    expect(model.applyEvent({ type: "event", topic, epoch: "j1", seq: 7, event: refused })).toBe("applied");
    expect(model.workspaceJobs.get("01W")?.state).toEqual({ jobs: [job("a")], error: "jobs.x: unknown key" });

    // A new snapshot announces no run; those applied before stay.
    model.resync(topic);
    expect(model.applySubscribed(topic, snapshot("j2", 3, state)).ok).toBe(true);
    expect(model.workspaceJobs.get("01W")?.state).toEqual(state);
    expect(model.workspaceJobs.get("01W")?.runs.length).toBe(2);

    const bad = { type: "event", topic, epoch: "j2", seq: 4, event: { kind: "run_started", run: { id: 1 } } };
    expect(decodeServerFrame(bad).ok).toBe(false);
    expect(model.untrack(topic)).toBe(true);
    expect(model.workspaceJobs.has("01W")).toBe(false);
  });

  // spec: PRT-200, UI-748
  test("a jobs topic refuses a malformed snapshot and keeps only its newest run events", () => {
    const model = new Model();
    const topic = jobsTopic("01W");
    model.track(topic);
    expect(model.applySubscribed(topic, snapshot("j1", 1, { jobs: [{ name: "a" }] })).ok).toBe(false);
    model.track(topic);
    expect(model.applySubscribed(topic, snapshot("j1", 1, { jobs: [] })).ok).toBe(true);
    const run: RunInfo = { id: "01RUN", workspaceId: "01W", trigger: "schedule", startedAt: 1, status: "running" };
    const heard: string[] = [];
    const stop = model.onRunEvent((workspaceId, event) => heard.push(`${workspaceId} ${event.seq} ${event.run.id}`));
    for (let seq = 2; seq < RUN_EVENTS_KEPT + 12; seq += 1) {
      const event: TopicEvent = { kind: "run_started", run: { ...run, id: `run${seq}` } };
      expect(model.applyEvent({ type: "event", topic, epoch: "j1", seq, event })).toBe("applied");
    }
    const runs = model.workspaceJobs.get("01W")?.runs ?? [];
    expect(runs.length).toBe(RUN_EVENTS_KEPT);
    expect(runs.at(-1)?.run.id).toBe(`run${RUN_EVENTS_KEPT + 11}`);
    expect(runs.at(-1)?.seq).toBe(RUN_EVENTS_KEPT + 10);
    // Listeners hear every live run event as it is applied, beyond what the topic keeps; a snapshot says nothing.
    expect(heard.length).toBe(RUN_EVENTS_KEPT + 10);
    expect(heard.at(0)).toBe("01W 1 run2");
    model.resync(topic);
    model.applySubscribed(topic, snapshot("j2", 1, { jobs: [] }));
    stop();
    const event: TopicEvent = { kind: "run_ended", run };
    expect(model.applyEvent({ type: "event", topic, epoch: "j2", seq: 2, event })).toBe("applied");
    expect(heard.length).toBe(RUN_EVENTS_KEPT + 10);
  });

  // spec: DMN-080, PRT-041
  test("a job topic keeps at most a mebibyte of text, counting the lines it drops", () => {
    const model = new Model();
    const topic = jobTopic("01JOB");
    model.track(topic);
    const half = "x".repeat(JOB_TEXT_LIMIT / 2);
    const state: JobState = {
      job: { id: "01JOB", workspaceId: "01W", command: "make", startedAt: 5, status: { state: "running" } },
      lines: [
        { stream: "stdout", text: half },
        { stream: "stdout", text: half },
      ],
      dropped: 1,
    };
    expect(model.applySubscribed(topic, snapshot("j1", 1, state)).ok).toBe(true);
    const event: TopicEvent = { kind: "job_output", stream: "stdout", text: "tail" };
    expect(model.applyEvent({ type: "event", topic, epoch: "j1", seq: 2, event })).toBe("applied");

    const kept = model.jobs.get("01JOB");
    expect(kept?.lines.map((line) => line.text.length)).toEqual([half.length, 4]);
    expect(kept?.dropped).toBe(2);
  });

  // spec: UI-012
  test("changes accumulate in one dirty set until drained", () => {
    const events = fixture("text_reply");
    const model = following("e1", events.slice(0, 1));
    model.track(INDEX_TOPIC);
    model.takeDirty();
    const revision = model.revision;

    applyAll(model, "e1", 2, events.slice(1));
    const state = indexState();
    model.applySubscribed(INDEX_TOPIC, snapshot("i1", 0, state));

    expect(model.revision).toBeGreaterThan(revision);
    const expected: Dirty = {
      connection: false,
      topics: true,
      index: true,
      threads: new Set([THREAD]),
      workspaces: new Set(),
      views: new Set(),
      jobs: new Set(),
      terminals: new Set(),
      workspaceJobs: new Set(),
    };
    expect(model.takeDirty()).toEqual(expected);
    expect(isDirtyEmpty(model.takeDirty())).toBe(true);
  });

  test("interactions closed by live events are kept with their resolution", () => {
    for (const name of ["approval_granted", "ask_answered"]) {
      const events = fixture(name);
      const model = following("e1", []);
      applyAll(model, "e1", 1, events);

      const thread = view(model);
      const answered = thread.closed.filter((closed) => closed.resolution !== undefined);
      const resolutions = events.filter((event) => event.kind === "interaction_resolved");
      expect({ name, answered: answered.length }).toEqual({ name, answered: resolutions.length });
      const client = resolutions[0]?.kind === "interaction_resolved" ? resolutions[0].resolution.client : undefined;
      expect(client).toBeDefined();
      for (const closed of answered) {
        expect(closed.resolution).toEqual({ by: "client", client: client ?? "" });
      }
      expect(thread.state.interactions).toEqual([]);
    }
  });

  test("a snapshot restarts the closed interaction history", () => {
    const events = fixture("approval_granted");
    const model = following("e1", []);
    applyAll(model, "e1", 1, events);
    expect(view(model).closed).not.toEqual([]);

    model.applySubscribed(TOPIC, snapshot("e2", 1, replay(events)));

    expect(view(model).closed).toEqual([]);
  });
});
