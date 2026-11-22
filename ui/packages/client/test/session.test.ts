/**
 * L1 tests of the session against an in-memory transport and a manual timer: hello, correlation, subscriptions,
 * reconnect backoff, resubscription positions, resync and the stop conditions.
 */

import { describe, expect, test } from "bun:test";
import { emptyThreadState, type ThreadState } from "@omfp/core";
import { INDEX_TOPIC, jobTopic, PROTOCOL_VERSION, type Topic, threadTopic, viewsTopic } from "@omfp/protocol";
import type { ConnectionStatus, TopicSync } from "../src/index.ts";
import { type CallCell, type Frame, flush, hello, Rig, type ServerEnd, TOKEN } from "./fake.ts";
import { summary } from "./support.ts";

const THREAD = "01THREAD";
const THREAD_TOPIC = threadTopic(THREAD);

function snapshot(epoch: string, seq: number, state: unknown): Frame {
  return { mode: "snapshot", epoch, seq, state };
}

function emptyIndex(): Frame {
  return { workspaces: [], threads: [], theme: { css: null } };
}

function indexEvent(epoch: string, seq: number, id: string): Frame {
  return { type: "event", topic: "index", epoch, seq, event: { kind: "thread_upserted", thread: summary(id, id) } };
}

function threadEvent(epoch: string, seq: number, state: string): Frame {
  return { type: "event", topic: THREAD_TOPIC, epoch, seq, event: { kind: "process", state: { state } } };
}

function subscribes(frames: Frame[]): unknown[] {
  return frames.filter((frame) => frame.method === "subscribe").map((frame) => frame.params);
}

function methods(frames: Frame[]): unknown[] {
  return frames.map((frame) => frame.method);
}

function find(frames: Frame[], topic: string): Frame {
  const frame = frames.find((frame) => {
    const params = frame.params;
    return typeof params === "object" && params !== null && "topic" in params && params.topic === topic;
  });
  if (frame === undefined) {
    throw new Error(`no request for ${topic} in ${JSON.stringify(frames)}`);
  }
  return frame;
}

function sync(rig: Rig, topic: Topic): TopicSync {
  const state = rig.client.model.topic(topic);
  if (state === undefined) {
    throw new Error(`${topic} is not tracked`);
  }
  return state.sync;
}

function reconnecting(rig: Rig): Extract<ConnectionStatus, { kind: "reconnecting" }> {
  const status = rig.status;
  if (status.kind !== "reconnecting") {
    throw new Error(`not reconnecting: ${JSON.stringify(status)}`);
  }
  return status;
}

async function settled(cell: CallCell): Promise<CallCell["settled"]> {
  await flush();
  return cell.settled;
}

function running(): ThreadState {
  return { ...emptyThreadState(), process: { state: "running" } };
}

/** A rig connected and following the index (`i1`/2) and the thread (`e1`/3). */
function following(): [Rig, ServerEnd] {
  const rig = new Rig();
  rig.client.subscribe(INDEX_TOPIC);
  rig.client.subscribe(THREAD_TOPIC);
  const [server, frames] = rig.connect();
  server.respond(find(frames, "index").id, snapshot("i1", 2, emptyIndex()));
  server.respond(find(frames, THREAD_TOPIC).id, snapshot("e1", 3, emptyThreadState()));
  return [rig, server];
}

/** Drops the connection and lets the first retry connect. */
function reconnect(rig: Rig, server: ServerEnd): [ServerEnd, Frame[]] {
  server.close(1001);
  rig.timer.fire();
  return rig.connect();
}

describe("session", () => {
  // spec: PRT-001, PRT-002
  test("the first frame is hello with the token and its answer connects", () => {
    const rig = new Rig();
    const server = rig.accept();

    const frames = server.frames();

    expect(frames).toEqual([
      {
        type: "request",
        id: frames[0]?.id,
        method: "system.hello",
        params: { token: TOKEN, protocol: PROTOCOL_VERSION, client: "test/l1" },
      },
    ]);
    expect(rig.status).toEqual({ kind: "connecting" });
    server.respond(frames[0]?.id, hello());
    expect(rig.status).toEqual({ kind: "connected", hello: hello() });
  });

  // spec: UI-003
  test("reconnect attempts back off exponentially up to ten seconds", () => {
    const rig = new Rig();
    const expected = [500, 1000, 2000, 4000, 8000, 10_000, 10_000, 10_000];

    for (const [index, retryInMs] of expected.entries()) {
      const attempt = index + 1;
      rig.net.takeAttempt().refuse("connection refused");
      expect(rig.status).toEqual({ kind: "reconnecting", attempt, retryInMs, error: "connection refused" });
      rig.timer.advance(retryInMs - 1);
      expect(rig.net.attempts).toBe(attempt); // no attempt before the delay
      rig.timer.advance(1);
    }

    expect(rig.timer.requested).toEqual(expected);
    expect(rig.net.attempts).toBe(expected.length + 1);
  });

  // spec: UI-003
  test("the backoff starts over after a successful connection", () => {
    const rig = new Rig();
    for (let failures = 0; failures < 2; failures += 1) {
      rig.net.takeAttempt().refuse("connection refused");
      rig.timer.fire();
    }
    const [server] = rig.connect();

    server.close();

    const { attempt, retryInMs } = reconnecting(rig);
    expect([attempt, retryInMs]).toEqual([1, 500]);
  });

  // spec: UI-003, PRT-020
  test("after a reconnect every topic is resubscribed from its last position", () => {
    const [rig, first] = following();
    first.send(indexEvent("i1", 3, "01NEW"));
    first.send(threadEvent("e1", 4, "starting"));

    const [server, frames] = reconnect(rig, first);

    expect(subscribes(frames)).toEqual([
      { topic: "index", since: { epoch: "i1", seq: 3 } },
      { topic: THREAD_TOPIC, since: { epoch: "e1", seq: 4 } },
    ]);
    server.respond(find(frames, "index").id, { mode: "replay", epoch: "i1", fromSeq: 4 });
    server.respond(find(frames, THREAD_TOPIC).id, snapshot("e2", 1, emptyThreadState()));
    server.send(indexEvent("i1", 4, "01NEWER"));
    expect(sync(rig, INDEX_TOPIC)).toEqual({ kind: "live", mode: "replay" });
    expect(sync(rig, THREAD_TOPIC)).toEqual({ kind: "live", mode: "snapshot" });
    expect([...rig.client.model.index.threads.keys()]).toEqual(["01NEW", "01NEWER"]);
    expect(rig.client.model.topic(THREAD_TOPIC)?.position).toEqual({ epoch: "e2", seq: 1 });
  });

  // spec: PRT-222, PRT-223, UI-997
  test("a kept thread switches watch on its live subscription and is resubscribed with its flag after a reconnect", () => {
    const [rig, first] = following();
    const kept = threadTopic("01KEPT");
    rig.client.subscribe(kept);
    first.respond(find(first.frames(), kept).id, snapshot("k1", 5, emptyThreadState()));

    rig.client.watch(kept, false);
    rig.client.watch(kept, false);
    rig.client.watch(THREAD_TOPIC, true);
    rig.client.watch(threadTopic("01NOTFOLLOWED"), false);
    expect(first.frames().map((frame) => [frame.method, frame.params])).toEqual([
      ["watch", { topic: kept, watch: false }],
    ]);

    const [second, frames] = reconnect(rig, first);
    expect(subscribes(frames)).toEqual([
      { topic: "index", since: { epoch: "i1", seq: 2 } },
      { topic: THREAD_TOPIC, since: { epoch: "e1", seq: 3 } },
      { topic: kept, since: { epoch: "k1", seq: 5 }, watch: false },
    ]);
    second.respond(find(frames, kept).id, { mode: "replay", epoch: "k1", fromSeq: 6 });

    // Shown again while disconnected: the next subscription watches it.
    second.close(1001);
    rig.client.watch(kept, true);
    rig.timer.fire();
    const [third, again] = rig.connect();
    expect(find(again, kept).params).toEqual({ topic: kept, since: { epoch: "k1", seq: 5 } });
    expect(methods(again).filter((method) => method === "watch")).toEqual([]);

    // Unsubscribing forgets the flag.
    rig.client.watch(kept, false);
    rig.client.unsubscribe(kept);
    rig.client.subscribe(kept);
    expect(
      find(
        third.frames().filter((frame) => frame.method === "subscribe"),
        kept,
      ).params,
    ).toEqual({
      topic: kept,
    });
  });

  // spec: UI-003
  test("events between the loss and the resubscription are not applied", () => {
    const [rig, first] = following();
    first.close();
    expect(sync(rig, INDEX_TOPIC)).toEqual({ kind: "subscribing" });
    rig.timer.fire();
    const [server, frames] = rig.connect();

    // Arrives before the subscribe answer: not part of a known stream yet.
    server.send(indexEvent("i1", 3, "01EARLY"));

    expect(rig.client.model.index.threads.size).toBe(0);
    expect(subscribes(frames)).toHaveLength(2);
  });

  // spec: PRT-023
  test("resync_required resubscribes without position and takes the snapshot", () => {
    const [rig, server] = following();

    server.send({ type: "resync_required", topic: THREAD_TOPIC });

    const frames = server.frames();
    expect(subscribes(frames)).toEqual([{ topic: THREAD_TOPIC }]);
    const state = running();
    server.respond(frames[0]?.id, snapshot("e1", 40, state));
    expect(rig.client.model.thread(THREAD)?.state).toEqual(state);
  });

  // spec: PRT-054, DMN-110
  test("transient frames of a followed topic reach their listeners and never the model", () => {
    const [rig, server] = following();
    const seen: unknown[] = [];
    const stop = rig.client.onTransient((frame) => seen.push(frame));
    const revision = rig.client.model.revision;
    const levels = (topic: string, input: unknown) => ({
      type: "transient",
      topic,
      event: { kind: "live_levels", input, output: 0.5 },
    });

    server.send(levels(THREAD_TOPIC, 0.25));
    server.send(levels(threadTopic("01OTHER"), 0.25));
    server.send(levels(THREAD_TOPIC, 3));
    server.send({ type: "transient", topic: INDEX_TOPIC, event: { kind: "live_levels", input: 0, output: 0 } });
    stop();
    server.send(levels(THREAD_TOPIC, 0.75));

    expect(seen).toEqual([levels(THREAD_TOPIC, 0.25)]);
    expect(rig.client.model.revision).toBe(revision);
    expect(subscribes(server.frames())).toEqual([]);
  });

  // spec: PRT-124, UI-354
  test("window requests reach their listener, and without one the client answers that it takes none", () => {
    const [rig, server] = following();
    const run = { type: "window_request", id: "w1", request: { kind: "run_command", command: "view", args: ["ops"] } };
    server.send(run);
    const answered = server.frames();
    expect(answered).toEqual([
      {
        type: "request",
        id: answered[0]?.id,
        method: "windows.answer",
        params: {
          requestId: "w1",
          answer: { kind: "not_run", reason: "failed", message: "this client takes no window requests" },
        },
      },
    ]);

    const seen: unknown[] = [];
    const stop = rig.client.onWindowRequest((frame) => seen.push(frame));
    const check = {
      type: "window_request",
      id: "w2",
      request: { kind: "check_widget", reference: "./widgets/tests", version: "3f2a", timeoutMs: 4000 },
    };
    server.send(run);
    server.send(check);
    // An invalid request is dropped.
    server.send({ type: "window_request", id: "w3", request: { kind: "eval", code: "alert(1)" } });
    stop();
    expect(seen).toEqual([run, check]);
    expect(methods(server.frames())).toEqual([]);
  });

  // spec: PRT-131, UI-380
  test("a login's window requests reach the listener, their optional fields null when missing", () => {
    const [rig, server] = following();
    const seen: unknown[] = [];
    const stop = rig.client.onWindowRequest((frame) => seen.push(frame));
    const login = { providerId: "anthropic", name: "Anthropic (Claude Pro/Max)" };
    const url = "https://claude.ai/oauth/authorize?code=true";
    server.send({ type: "window_request", id: "l1", request: { kind: "login_url", ...login, url } });
    server.send({
      type: "window_request",
      id: "l2",
      request: { kind: "login_url", ...login, url, launchUrl: "http://127.0.0.1:7420/l/1", instructions: "Paste it" },
    });
    server.send({
      type: "window_request",
      id: "l3",
      request: { kind: "login_progress", ...login, message: "Waiting" },
    });
    server.send({ type: "window_request", id: "l4", request: { kind: "login_input", ...login, message: "Code" } });
    server.send({
      type: "window_request",
      id: "l5",
      request: { kind: "login_input", ...login, message: "Code", placeholder: "code#state" },
    });
    // Invalid: no url, a number message.
    server.send({ type: "window_request", id: "l6", request: { kind: "login_url", ...login } });
    server.send({ type: "window_request", id: "l7", request: { kind: "login_progress", ...login, message: 3 } });
    stop();
    expect(seen).toEqual([
      {
        type: "window_request",
        id: "l1",
        request: { kind: "login_url", ...login, url, launchUrl: null, instructions: null },
      },
      {
        type: "window_request",
        id: "l2",
        request: { kind: "login_url", ...login, url, launchUrl: "http://127.0.0.1:7420/l/1", instructions: "Paste it" },
      },
      { type: "window_request", id: "l3", request: { kind: "login_progress", ...login, message: "Waiting" } },
      {
        type: "window_request",
        id: "l4",
        request: { kind: "login_input", ...login, message: "Code", placeholder: null },
      },
      {
        type: "window_request",
        id: "l5",
        request: { kind: "login_input", ...login, message: "Code", placeholder: "code#state" },
      },
    ]);
    expect(methods(server.frames())).toEqual([]);
  });

  // spec: DMN-375
  test("an agent's notify window request reaches the listener; one missing a field is dropped", () => {
    const [rig, server] = following();
    const seen: unknown[] = [];
    const stop = rig.client.onWindowRequest((frame) => seen.push(frame));
    const request = { kind: "notify", threadId: "t9", title: "Key", body: "Load it" };
    server.send({ type: "window_request", id: "n1", request });
    server.send({ type: "window_request", id: "n2", request: { kind: "notify", threadId: "t9", title: "Key" } });
    stop();
    expect(seen).toEqual([{ type: "window_request", id: "n1", request }]);
  });

  // spec: PRT-022
  test("a gap in the stream resubscribes without position", () => {
    const [rig, server] = following();

    server.send(threadEvent("e1", 6, "starting"));

    expect(subscribes(server.frames())).toEqual([{ topic: THREAD_TOPIC }]);
    expect(sync(rig, THREAD_TOPIC)).toEqual({ kind: "subscribing" });
  });

  // spec: PRT-001, UI-003
  test("a refused token stops the client without retrying", async () => {
    for (const answerFirst of [true, false]) {
      const rig = new Rig();
      const pending = rig.call("repos.suggest", {});
      const server = rig.accept();
      const [helloRequest] = server.frames();

      if (answerFirst) {
        server.fail(helloRequest?.id, { code: "unauthorized", message: "invalid token" });
      }
      server.close(4401);
      await flush();

      expect(rig.status).toEqual({ kind: "unauthenticated" });
      expect(rig.done).toBe(true);
      expect(rig.timer.requested).toEqual([]);
      expect(rig.net.attempts).toBe(1);
      expect(pending.settled?.ok === false && pending.settled.error.failure).toEqual({ kind: "closed" });
    }
  });

  // spec: PRT-004
  test("a refused protocol version is reported and not retried", async () => {
    const rig = new Rig();
    const server = rig.accept();
    const [helloRequest] = server.frames();

    server.fail(helloRequest?.id, { code: "version_mismatch", message: "unsupported", details: { supported: [1] } });
    server.close(4426);
    await flush();

    expect(rig.status).toEqual({ kind: "version_mismatch", supported: [1], message: "unsupported" });
    expect(rig.done).toBe(true);
    expect(rig.net.attempts).toBe(1);
  });

  // spec: PRT-010
  test("responses are matched to calls by id in any order", async () => {
    const rig = new Rig();
    const [server] = rig.connect();
    const first = rig.call("repos.suggest", {});
    const second = rig.call("thread.state", { threadId: THREAD });

    const frames = server.frames();
    expect(frames).toHaveLength(2);
    expect(frames[0]?.id).not.toEqual(frames[1]?.id);
    server.respond(frames[1]?.id, "state");
    server.fail(frames[0]?.id, { code: "internal", message: "boom" });

    expect(await settled(second)).toEqual({ ok: true, value: "state" });
    const failure = first.settled?.ok === false ? first.settled.error.failure : undefined;
    expect(failure).toEqual({ kind: "daemon", error: { code: "internal", message: "boom" } });
  });

  test("calls made offline are sent once connected and in-flight calls fail on loss", async () => {
    const rig = new Rig();
    const queued = rig.call("repos.suggest", {});
    const [server, frames] = rig.connect();
    expect(methods(frames)).toEqual(["repos.suggest"]);
    server.respond(frames[0]?.id, { repos: [] });
    const inFlight = rig.call("thread.state", { threadId: THREAD });
    expect(server.frames()).toHaveLength(1);

    server.close();

    expect(await settled(queued)).toEqual({ ok: true, value: { repos: [] } });
    expect(inFlight.settled?.ok === false && inFlight.settled.error.failure).toEqual({ kind: "connection_lost" });
  });

  test("typed requests resolve to their result", async () => {
    const rig = new Rig();
    const [server] = rig.connect();
    const request = rig.client.call("repos.suggest", {});
    const frames = server.frames();

    server.respond(frames[0]?.id, { repos: [] });

    const list = await request;
    expect(list.repos).toEqual([]);
  });

  test("session methods cannot be called directly", async () => {
    const rig = new Rig();

    const result = rig.callAny("unsubscribe", { topic: INDEX_TOPIC });

    const outcome = await settled(result);
    expect(outcome?.ok === false && outcome.error.failure).toEqual({ kind: "reserved", method: "unsubscribe" });
  });

  // spec: PRT-024
  test("unsubscribing tells the daemon and drops the topic", () => {
    const [rig, server] = following();

    rig.client.unsubscribe(THREAD_TOPIC);

    const frames = server.frames();
    expect(frames[0]?.method).toBe("unsubscribe");
    expect(frames[0]?.params).toEqual({ topic: THREAD_TOPIC });
    expect(rig.client.model.thread(THREAD)).toBeUndefined();
    const [, again] = reconnect(rig, server);
    expect(subscribes(again)).toEqual([{ topic: "index", since: { epoch: "i1", seq: 2 } }]);
  });

  test("closing the client closes the connection and stops retrying", async () => {
    const [rig, server] = following();

    rig.client.close();
    await flush();

    expect(rig.status).toEqual({ kind: "closed" });
    expect(rig.done).toBe(true);
    expect(server.closedByClient).toBe(true);
    expect(rig.timer.requested).toEqual([]);
    const late = await settled(rig.call("repos.suggest", {}));
    expect(late?.ok === false && late.error.failure).toEqual({ kind: "closed" });
  });

  // spec: UI-012
  test("many changes wake the host once", async () => {
    const [rig, server] = following();
    let wakeups = 0;
    const changes = rig.client.changes();
    void (async () => {
      for (;;) {
        await changes.next();
        wakeups += 1;
      }
    })();
    await flush();

    for (let seq = 3; seq < 23; seq += 1) {
      server.send(indexEvent("i1", seq, `01T${String(seq).padStart(2, "0")}`));
    }
    await flush();

    expect(wakeups).toBe(1);
    expect(rig.client.takeDirty().index).toBe(true);
    expect(rig.client.takeDirty()).toEqual({
      connection: false,
      topics: false,
      index: false,
      threads: new Set(),
      workspaces: new Set(),
      views: new Set(),
      jobs: new Set(),
      terminals: new Set(),
      workspaceJobs: new Set(),
    });
  });

  // spec: UI-003
  test("queued calls a failed send left behind go out on the next connection", async () => {
    const rig = new Rig();
    const first = rig.accept();
    const [helloRequest] = first.frames();
    const one = rig.call("repos.suggest", {});
    const two = rig.call("thread.state", { threadId: THREAD });
    const three = rig.call("thread.restart", { threadId: THREAD });
    first.failSendsAfter(1);

    first.respond(helloRequest?.id, hello());

    expect(methods(first.frames())).toEqual(["repos.suggest"]);
    expect(reconnecting(rig).attempt).toBe(1);
    const lost = await settled(one);
    expect(lost?.ok === false && lost.error.failure).toEqual({ kind: "connection_lost" });
    rig.timer.fire();
    const [server, frames] = rig.connect();
    expect(methods(frames)).toEqual(["thread.state", "thread.restart"]);
    for (const frame of frames) {
      server.respond(frame.id, frame.method);
    }
    await flush();
    expect(two.settled).toEqual({ ok: true, value: "thread.state" });
    expect(three.settled).toEqual({ ok: true, value: "thread.restart" });
  });

  // spec: UI-003
  test("a call whose send fails mid-session is sent after reconnecting", async () => {
    const rig = new Rig();
    const [first] = rig.connect();
    first.failSendsAfter(0);

    const call = rig.call("repos.suggest", {});

    expect(reconnecting(rig).attempt).toBe(1);
    expect(await settled(call)).toBeUndefined();
    rig.timer.fire();
    const [server, frames] = rig.connect();
    expect(methods(frames)).toEqual(["repos.suggest"]);
    server.respond(frames[0]?.id, { repos: [] });
    expect(await settled(call)).toEqual({ ok: true, value: { repos: [] } });
  });

  test("a malformed response fails its call", async () => {
    const rig = new Rig();
    const [server] = rig.connect();
    const call = rig.call("repos.suggest", {});
    const id = server.frames()[0]?.id;

    server.send({ type: "response", id, ok: true, error: { code: "internal", message: "both" } });

    const outcome = await settled(call);
    expect(outcome?.ok === false && outcome.error.failure.kind).toBe("codec");
    expect(rig.status.kind).toBe("connected");
  });

  // spec: UI-003
  test("a malformed hello answer reconnects with backoff", async () => {
    const rig = new Rig();
    const server = rig.accept();
    const [helloRequest] = server.frames();

    server.send({ type: "response", id: helloRequest?.id, ok: false });
    await flush();

    const { attempt, retryInMs } = reconnecting(rig);
    expect([attempt, retryInMs]).toEqual([1, 500]);
    expect(rig.done).toBe(false);
  });

  // spec: PRT-022
  test("a malformed event of a followed topic resubscribes it at once", () => {
    const [rig, server] = following();

    server.send({ type: "event", topic: THREAD_TOPIC, epoch: "e1", seq: 4, event: { kind: "not_a_thread_event" } });

    expect(subscribes(server.frames())).toEqual([{ topic: THREAD_TOPIC }]);
    expect(sync(rig, THREAD_TOPIC)).toEqual({ kind: "subscribing" });
  });

  // spec: PRT-040, PRT-041
  test("views and job topics decode their snapshots and events from the wire", () => {
    const rig = new Rig();
    const views = viewsTopic("01W");
    const job = jobTopic("01JOB");
    rig.client.subscribe(views);
    rig.client.subscribe(job);
    const [server, frames] = rig.connect();
    server.respond(find(frames, views).id, snapshot("v1", 0, { manifest: null, error: null, widgets: [], stored: {} }));
    const info = { id: "01JOB", workspaceId: "01W", command: "make", startedAt: 1, status: { state: "running" } };
    server.respond(find(frames, job).id, snapshot("j1", 0, { job: info, lines: [], dropped: 0 }));

    server.send({
      type: "event",
      topic: views,
      epoch: "v1",
      seq: 1,
      event: { kind: "widget_stored", instance: "w", key: "k", value: true },
    });
    server.send({
      type: "event",
      topic: job,
      epoch: "j1",
      seq: 1,
      event: { kind: "job_output", stream: "stdout", text: "hi" },
    });

    expect(rig.client.model.views.get("01W")?.stored).toEqual({ w: { k: true } });
    expect(rig.client.model.jobs.get("01JOB")?.lines).toEqual([{ stream: "stdout", text: "hi" }]);

    // A job event on a views topic does not decode: the topic is resubscribed.
    server.send({
      type: "event",
      topic: views,
      epoch: "v1",
      seq: 2,
      event: { kind: "job_output", stream: "stdout", text: "x" },
    });
    expect(subscribes(server.frames())).toEqual([{ topic: views }]);
  });

  test("commands given during the hello are sent once after it", async () => {
    const rig = new Rig();
    const server = rig.accept();
    const [helloRequest] = server.frames();

    rig.client.subscribe(INDEX_TOPIC);
    rig.client.subscribe(THREAD_TOPIC);
    rig.client.unsubscribe(THREAD_TOPIC);
    const call = rig.call("repos.suggest", {});
    expect(server.frames()).toEqual([]);
    server.respond(helloRequest?.id, hello());

    const frames = server.frames();
    expect(methods(frames)).toEqual(["subscribe", "repos.suggest"]);
    expect(frames[0]?.params).toEqual({ topic: "index" });
    server.respond(frames[1]?.id, { repos: [] });
    expect(await settled(call)).toEqual({ ok: true, value: { repos: [] } });
    expect(server.frames()).toEqual([]);
  });

  // spec: PRT-020
  test("a replay that does not continue the position resubscribes without it", () => {
    const [rig, first] = following();
    const [server, frames] = reconnect(rig, first);

    server.respond(find(frames, "index").id, { mode: "replay", epoch: "i1", fromSeq: 9 });

    expect(subscribes(server.frames())).toEqual([{ topic: "index" }]);
    expect(sync(rig, INDEX_TOPIC)).toEqual({ kind: "subscribing" });
  });

  test("an answer to a superseded subscribe is discarded", () => {
    const rig = new Rig();
    const [server] = rig.connect();
    rig.client.subscribe(THREAD_TOPIC);
    rig.client.unsubscribe(THREAD_TOPIC);
    rig.client.subscribe(THREAD_TOPIC);
    const frames = server.frames();
    expect(methods(frames)).toEqual(["subscribe", "unsubscribe", "subscribe"]);

    server.respond(frames[0]?.id, snapshot("e1", 5, running()));
    server.respond(frames[1]?.id, {});
    expect(sync(rig, THREAD_TOPIC)).toEqual({ kind: "subscribing" });
    expect(rig.client.model.thread(THREAD)).toBeUndefined();

    server.respond(frames[2]?.id, snapshot("e1", 6, emptyThreadState()));
    expect(rig.client.model.thread(THREAD)?.state).toEqual(emptyThreadState());
  });

  test("a refused subscription is retried on request and on the next connection", () => {
    const rig = new Rig();
    const [first] = rig.connect();
    const notFound = { code: "not_found", message: "no such thread" };
    rig.client.subscribe(THREAD_TOPIC);
    first.fail(first.frames()[0]?.id, notFound);
    expect(sync(rig, THREAD_TOPIC).kind).toBe("failed");

    rig.client.subscribe(THREAD_TOPIC);
    const frames = first.frames();
    expect(subscribes(frames)).toEqual([{ topic: THREAD_TOPIC }]);
    first.fail(frames[0]?.id, notFound);

    const [, again] = reconnect(rig, first);
    expect(subscribes(again)).toEqual([{ topic: THREAD_TOPIC }]);
  });

  // spec: PRT-023
  test("resync_required while resubscribing waits for the pending answer", () => {
    const [rig, server] = following();
    server.send(threadEvent("e1", 6, "starting"));
    const frames = server.frames();
    expect(subscribes(frames)).toHaveLength(1);

    server.send({ type: "resync_required", topic: THREAD_TOPIC });

    expect(server.frames()).toEqual([]);
    server.respond(frames[0]?.id, snapshot("e1", 9, emptyThreadState()));
    expect(sync(rig, THREAD_TOPIC)).toEqual({ kind: "live", mode: "snapshot" });
  });

  // spec: PRT-004
  test("close 4426 without an answer reports a version mismatch", async () => {
    const rig = new Rig();
    const server = rig.accept();

    server.close(4426);
    await flush();

    expect(rig.status).toEqual({ kind: "version_mismatch", supported: [], message: "" });
    expect(rig.done).toBe(true);
    expect(rig.net.attempts).toBe(1);
  });

  // spec: UI-003
  test("a hello refused for another reason is retried with backoff", async () => {
    const rig = new Rig();
    const server = rig.accept();
    const [helloRequest] = server.frames();

    server.fail(helloRequest?.id, { code: "internal", message: "starting up" });
    await flush();

    expect(rig.status).toEqual({ kind: "reconnecting", attempt: 1, retryInMs: 500, error: "starting up" });
    expect(rig.done).toBe(false);
    rig.timer.fire();
    expect(rig.net.attempts).toBe(2);
  });
});
