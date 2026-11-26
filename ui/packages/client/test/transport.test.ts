/**
 * The transport suite (TST-030, UI-001): the client over Bun's `WebSocket` against a real daemon (the test daemon
 * `just test-ts` starts, with the fake omp launcher and fake git; its URL and token come from `OMFP_TEST_DAEMON_URL`
 * and `OMFP_TEST_DAEMON_TOKEN`). The daemon is shared by every test and keeps running: a {@link Switch} between the
 * client and its WebSocket drops the live connection from the client side and holds reconnect attempts.
 */

import { describe, expect, test } from "bun:test";
import { OMP_VERSION } from "@omfp/core";
import { INDEX_TOPIC, PROTOCOL_VERSION, type Topic, type TransientFrame, threadTopic } from "@omfp/protocol";
import {
  CallError,
  type Client,
  type Connector,
  connect,
  type ModelView,
  type StartMode,
  systemTimer,
  type Transport,
  TransportError,
  webSocketConnector,
} from "../src/index.ts";

/** How long a scenario waits for a condition before failing. */
const WAIT_MS = 20_000;
/** Per-test limit of bun:test, above every wait of a scenario. */
const TEST_MS = 60_000;

const BACKOFF = { initialMs: 20, maxMs: 200 };

function daemon(): { url: string; token: string } {
  const url = process.env.OMFP_TEST_DAEMON_URL;
  const token = process.env.OMFP_TEST_DAEMON_TOKEN;
  if (url === undefined || url === "" || token === undefined || token === "") {
    throw new Error(
      "OMFP_TEST_DAEMON_URL and OMFP_TEST_DAEMON_TOKEN are not set: run the transport suite through `just test-ts`, " +
        "which starts the test daemon",
    );
  }
  return { url, token };
}

/** Lets a scenario drop the live connection and hold reconnect attempts. */
class Switch {
  /** Connection attempts so far. */
  attempts = 0;
  #held = false;
  #waiting: Array<() => void> = [];
  #dropLive: (() => void) | undefined;

  /** Later attempts wait until {@link Switch.release}. */
  hold(): void {
    this.#held = true;
  }

  release(): void {
    this.#held = false;
    const waiting = this.#waiting;
    this.#waiting = [];
    for (const start of waiting) {
      start();
    }
  }

  /** Closes the live connection from the client side, as a network failure would. */
  dropConnection(): void {
    const drop = this.#dropLive;
    this.#dropLive = undefined;
    drop?.();
  }

  /** `inner` through this switch. */
  connector(inner: Connector): Connector {
    return (listener) => {
      let transport: Transport | undefined;
      let gone = false;
      const drop = () => {
        gone = true;
        transport?.close();
        listener.closed({ code: undefined, reason: "dropped by the test" });
      };
      const start = () => {
        if (gone) {
          return;
        }
        this.attempts += 1;
        transport = inner({
          opened: () => {
            if (!gone) {
              this.#dropLive = drop;
              listener.opened();
            }
          },
          received: (text) => {
            if (!gone) {
              listener.received(text);
            }
          },
          closed: (closed) => {
            if (!gone) {
              listener.closed(closed);
            }
          },
          failed: (error) => {
            if (!gone) {
              listener.failed(error);
            }
          },
        });
      };
      if (this.#held) {
        this.#waiting.push(start);
      } else {
        start();
      }
      return {
        send(text) {
          if (transport === undefined || gone) {
            throw new TransportError("not open");
          }
          transport.send(text);
        },
        close() {
          gone = true;
          transport?.close();
        },
      };
    };
  }
}

function client(name: string, options: { switch?: Switch; token?: string } = {}): Client {
  const { url, token } = daemon();
  const direct = webSocketConnector(url);
  return connect({
    connector: options.switch?.connector(direct) ?? direct,
    token: options.token ?? token,
    client: name,
    backoff: BACKOFF,
  });
}

function describeModel(model: ModelView): string {
  return JSON.stringify({
    connection: model.connection,
    topics: model.topics,
    threads: [...model.index.threads.keys()],
  });
}

/** `work`, or a failure naming `what` after {@link WAIT_MS}. A guard against a hung real daemon, never a wait: every
 * scenario awaits model changes. */
async function within<T>(what: string, state: () => string, work: Promise<T>): Promise<T> {
  let cancel = () => {};
  const timeout = new Promise<never>((_, reject) => {
    cancel = systemTimer.schedule(WAIT_MS, () => reject(new Error(`timed out waiting for ${what}: ${state()}`)));
  });
  try {
    return await Promise.race([work, timeout]);
  } finally {
    cancel();
  }
}

/** Waits until `check` holds for the client's model. */
async function until(client: Client, what: string, check: (model: ModelView) => boolean): Promise<void> {
  const changes = client.changes();
  const holds = async () => {
    while (!check(client.model)) {
      await changes.next();
    }
  };
  await within(what, () => describeModel(client.model), holds());
}

function connected(model: ModelView): boolean {
  return model.connection.kind === "connected";
}

function live(model: ModelView, topic: Topic, mode: StartMode): boolean {
  const sync = model.topic(topic)?.sync;
  return sync?.kind === "live" && sync.mode === mode;
}

async function createThread(client: Client, title: string): Promise<string> {
  const thread = await client.call("threads.create", { workspace: { new: {} }, title });
  return thread.id;
}

/** Waits until both clients applied `topic` up to the same position. */
async function samePosition(a: Client, b: Client, topic: Topic): Promise<void> {
  const position = (client: Client) => JSON.stringify(client.model.topic(topic)?.position);
  const aChanges = a.changes();
  const bChanges = b.changes();
  const converged = async () => {
    while (position(a) !== position(b)) {
      await Promise.race([aChanges.next(), bChanges.next()]);
    }
  };
  await within(`the same position of ${topic}`, () => `${position(a)} != ${position(b)}`, converged());
}

describe("transport suite", () => {
  // spec: TST-030, UI-001, PRT-001, PRT-005
  test(
    "hello",
    async () => {
      const links = new Switch();
      const hello = client("suite/hello", { switch: links });
      try {
        await until(hello, "hello", connected);
        const status = hello.model.connection;
        expect(status.kind === "connected" && [status.hello.protocol, status.hello.omp]).toEqual([
          PROTOCOL_VERSION,
          OMP_VERSION,
        ]);
        hello.close();
        await until(hello, "closed", (model) => model.connection.kind === "closed");
      } finally {
        hello.close();
      }
      expect(links.attempts).toBe(1);
    },
    TEST_MS,
  );

  // spec: TST-030, UI-001, PRT-002
  test(
    "auth failure",
    async () => {
      const links = new Switch();
      const refused = client("suite/auth", { switch: links, token: "not-the-token" });
      try {
        const pending = refused.call("repos.suggest", {}).then(
          () => "resolved",
          (error: unknown) => error,
        );

        // The client stops by itself.
        await within("the client to stop", () => describeModel(refused.model), refused.done);

        expect(refused.model.connection).toEqual({ kind: "unauthenticated" });
        expect(links.attempts).toBe(1);
        const error = await pending;
        expect(error instanceof Error && "failure" in error && error.failure).toEqual({ kind: "closed" });
      } finally {
        refused.close();
      }
    },
    TEST_MS,
  );

  // spec: TST-030, UI-001, PRT-020, PRT-021
  test(
    "subscribe snapshot",
    async () => {
      const follower = client("suite/snapshot");
      const observer = client("suite/observer");
      try {
        const thread = await createThread(follower, "snapshot");
        const topic = threadTopic(thread);
        follower.subscribe(INDEX_TOPIC);
        follower.subscribe(topic);
        await until(
          follower,
          "snapshots",
          (model) => live(model, INDEX_TOPIC, "snapshot") && live(model, topic, "snapshot"),
        );
        expect(follower.model.index.threads.get(thread)?.title).toBe("snapshot");
        expect(follower.model.thread(thread)?.state.process).toEqual({ state: "stopped" });

        // Starts the (fake) omp process: its events follow the snapshot. (The fake answers `get_state` with `{}`.)
        await follower.call("thread.state", { threadId: thread });
        await until(follower, "a running thread", (model) => {
          const state = model.thread(thread)?.state;
          return state?.process.state === "running" && state.session !== null;
        });

        observer.subscribe(topic);
        await until(observer, "the observer's snapshot", (model) => live(model, topic, "snapshot"));
        await samePosition(follower, observer, topic);
        expect(follower.model.thread(thread)?.state).toEqual(observer.model.thread(thread)?.state);
      } finally {
        follower.close();
        observer.close();
      }
    },
    TEST_MS,
  );

  // spec: TST-030, UI-001, UI-003, PRT-020
  test(
    "replay",
    async () => {
      const links = new Switch();
      const follower = client("suite/replay", { switch: links });
      const other = client("suite/other");
      try {
        follower.subscribe(INDEX_TOPIC);
        await until(follower, "the index snapshot", (model) => live(model, INDEX_TOPIC, "snapshot"));

        links.hold();
        links.dropConnection();
        await until(follower, "reconnecting", (model) => model.connection.kind === "reconnecting");
        const missed = await createThread(other, "missed while offline");
        links.release();

        await until(follower, "the missed thread", (model) => model.index.threads.has(missed));
        expect(connected(follower.model)).toBe(true);
        expect(live(follower.model, INDEX_TOPIC, "replay")).toBe(true);
        expect(links.attempts).toBe(2);
      } finally {
        follower.close();
        other.close();
      }
    },
    TEST_MS,
  );

  // spec: TST-030, UI-001, UI-003
  test(
    "reconnect",
    async () => {
      const links = new Switch();
      const follower = client("suite/reconnect", { switch: links });
      try {
        follower.subscribe(INDEX_TOPIC);
        await until(
          follower,
          "the index snapshot",
          (model) => connected(model) && live(model, INDEX_TOPIC, "snapshot"),
        );

        links.dropConnection();
        await until(
          follower,
          "the second connection",
          (model) => links.attempts === 2 && connected(model) && model.topic(INDEX_TOPIC)?.sync.kind === "live",
        );
        const created = await createThread(follower, "after reconnect");

        await until(follower, "the new thread", (model) => model.index.threads.has(created));
      } finally {
        follower.close();
      }
    },
    TEST_MS,
  );

  // spec: TST-030, PRT-050, PRT-051, DMN-100, DMN-101, DMN-102, DMN-103, DMN-118
  test(
    "live voice",
    async () => {
      const owner = client("suite/live");
      const watcher = client("suite/live-watcher");
      try {
        const thread = await createThread(owner, "live");
        const topic = threadTopic(thread);
        watcher.subscribe(topic);
        await until(owner, "hello", connected);
        const status = owner.model.connection;
        const connection = status.kind === "connected" ? status.hello.connection : -1;
        const failure = (error: unknown) => (error instanceof CallError ? error.failure : error);
        const offer = "v=0\r\na=setup:actpass\r\na=mid:0\r\n";

        const blank = await owner.call("thread.live_connect", { threadId: thread, sdpOffer: "" }).catch(failure);
        expect(blank).toMatchObject({ kind: "daemon", error: { code: "bad_request" } });
        const answer = await owner.call("thread.live_connect", { threadId: thread, sdpOffer: offer });
        expect(answer.sdpAnswer).toBe("v=0\r\na=setup:active\r\na=mid:0\r\n");
        expect(answer.voice).toBe("sol");
        // omfp's prompt and `<about>` section for the window to append, never the thread's conversation (DMN-118).
        const context = (answer.context ?? []).join("");
        expect(context.includes("You: omfp's voice")).toBe(true);
        expect(context.includes("</about>")).toBe(true);
        expect(context.includes("<thread-context>")).toBe(false);
        await until(
          watcher,
          "the listening session",
          (model) => model.thread(thread)?.state.live?.phase === "listening",
        );
        expect(watcher.model.thread(thread)?.state.live?.owner).toEqual({ client: "suite/live", connection });

        const second = await watcher.call("thread.live_connect", { threadId: thread, sdpOffer: offer }).catch(failure);
        expect(second).toMatchObject({ kind: "daemon", error: { code: "conflict" } });
        expect(await owner.call("thread.live_mute", { threadId: thread, muted: true })).toEqual({ muted: true });
        await until(watcher, "the muted phase", (model) => model.thread(thread)?.state.live?.phase === "muted");
        expect(await owner.call("thread.live_stop", { threadId: thread })).toEqual({});
        await until(watcher, "the end of the session", (model) => model.thread(thread)?.state.live === null);
        const mute = await owner.call("thread.live_mute", { threadId: thread, muted: false }).catch(failure);
        expect(mute).toMatchObject({ kind: "daemon", error: { code: "conflict" } });

        // The owner's connection goes: the daemon stops its session.
        await owner.call("thread.live_connect", { threadId: thread, sdpOffer: offer });
        await until(watcher, "the second session", (model) => model.thread(thread)?.state.live?.phase === "listening");
        owner.close();
        await until(watcher, "the stop on disconnect", (model) => model.thread(thread)?.state.live === null);
      } finally {
        owner.close();
        watcher.close();
      }
    },
    TEST_MS,
  );

  // spec: TST-030, PRT-053, PRT-054, DMN-109, DMN-110
  test(
    "live voice on the daemon's machine",
    async () => {
      const owner = client("suite/live-start");
      const levels = new Promise<TransientFrame>((resolve) => owner.onTransient(resolve));
      try {
        const thread = await createThread(owner, "live start");
        const topic = threadTopic(thread);
        owner.subscribe(topic);
        await until(owner, "the thread", (model) => live(model, topic, "snapshot"));

        expect(await owner.call("thread.live_start", { threadId: thread })).toEqual({ voice: "sol" });
        await until(owner, "the listening session", (model) => model.thread(thread)?.state.live?.phase === "listening");
        expect(await within("the levels", () => describeModel(owner.model), levels)).toEqual({
          type: "transient",
          topic,
          event: { kind: "live_levels", input: 0.25, output: 0.5 },
        });
        expect(await owner.call("thread.live_stop", { threadId: thread })).toEqual({});
      } finally {
        owner.close();
      }
    },
    TEST_MS,
  );
});
