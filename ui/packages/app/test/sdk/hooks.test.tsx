/** L4: the SDK hooks over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import type { ThreadEvent } from "@omfp/core";
import type { RecentThreads, ThreadSummary } from "@omfp/protocol";
import { threadTopic, viewsTopic } from "@omfp/protocol";
import { act, cleanup, screen } from "@testing-library/react";
import { Activity, useState } from "react";
import { DEFAULT_RECENT_THREADS } from "../../src/sdk/host.tsx";
import {
  CommandRegistry,
  SCOPE_ATTRIBUTE,
  scopesAt,
  useCommand,
  useHost,
  useThread,
  useThreads,
  useViews,
  useWindowState,
  windowKey,
} from "../../src/sdk/index.ts";
import { released, renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

function ThreadProbe({ id }: { id: string }) {
  const thread = useThread(id);
  return <span data-testid={`probe-${id}`}>{thread === undefined ? "none" : thread.state.transcript.length}</span>;
}

function TwoProbes() {
  const [both, setBoth] = useState(true);
  return (
    <>
      <ThreadProbe id="01T" />
      {both && <ThreadProbe id="01T" />}
      <button type="button" data-testid="drop" onClick={() => setBoth(false)} />
    </>
  );
}

const SHOWN = windowKey<string | undefined>("test/shown", undefined);

/** Shows one thread at a time, like the chat follows the selected thread. */
function ShownThread() {
  const [id] = useWindowState(SHOWN);
  return id === undefined ? null : <ThreadProbe id={id} />;
}

function ViewsProbe({ tab }: { tab: string }) {
  const views = useViews("01W");
  return <span data-testid={`views-${tab}`}>{views === undefined ? "none" : views.widgets.length}</span>;
}

/** Two tabs following the same `views/<id>` topic, one shown at a time, the other kept hidden (`<Activity>`) as the
 * view's `tabs` keep theirs. */
function TwoTabs() {
  const [shown, setShown] = useState("a");
  return (
    <>
      {["a", "b"].map((tab) => (
        <Activity key={tab} mode={tab === shown ? "visible" : "hidden"}>
          <ViewsProbe tab={tab} />
        </Activity>
      ))}
      <button type="button" data-testid="switch" onClick={() => setShown((tab) => (tab === "a" ? "b" : "a"))} />
    </>
  );
}

/** A history of `count` user messages. */
function history(count: number): ThreadEvent {
  return {
    kind: "history",
    session: { session_id: "s", resumed: true },
    messages: Array.from({ length: count }, (_, index) => ({ role: "user" as const, content: `message ${index}` })),
  };
}

function ThreadCount() {
  return <span data-testid="count">{useThreads().size}</span>;
}

const COUNTER = windowKey("test/counter", 0);

function Counter({ testid }: { testid: string }) {
  const [count, setCount] = useWindowState(COUNTER);
  return (
    <button type="button" data-testid={testid} onClick={() => setCount((previous) => previous + 1)}>
      {count}
    </button>
  );
}

const ran: string[] = [];

function Scoped() {
  useCommand({
    id: "nav.next",
    title: "Next message",
    scope: "transcript",
    run: () => {
      ran.push("transcript");
    },
  });
  useCommand({
    id: "nav.next",
    title: "Next thread",
    run: () => {
      ran.push("global");
    },
  });
  const { commands } = useHost();
  return (
    <>
      <div {...{ [SCOPE_ATTRIBUTE]: "transcript" }}>
        <button type="button" data-testid="inside" />
      </div>
      <button type="button" data-testid="outside" />
      <button
        type="button"
        data-testid="run"
        onClick={() => commands.run("nav.next", [], scopesAt(document.activeElement))}
      />
    </>
  );
}

function summary(id: string): ThreadSummary {
  return {
    id,
    title: id,
    titleSource: "set",
    workspaceId: "01W",
    process: { state: "stopped" },
    settled: true,
    needsAttention: false,
    provenance: { createdBy: { kind: "user" }, client: "test" },
    createdAt: 1,
    updatedAt: 1,
    position: 0,
  };
}

describe("hooks", () => {
  // spec: UI-010, UI-335
  test("useThread follows thread/<id> while any widget shows it and renders the reducer state", () => {
    const topic = threadTopic("01T");
    const { client, flush, view } = renderWithHost(<TwoProbes />, testHost());
    expect(client.subscribed).toEqual([topic]);

    client.thread("01T", []);
    flush();
    expect(screen.getAllByTestId("probe-01T").map((node) => node.textContent)).toEqual(["0", "0"]);

    act(() => screen.getByTestId("drop").click());
    expect(client.unsubscribed).toEqual([]);
    view.unmount();
    // Kept among the recent threads.
    expect(client.unsubscribed).toEqual([]);
    expect(client.model.thread("01T")?.state.transcript.length).toBe(0);
  });

  // spec: UI-335
  test("a thread shown again among the recent ones needs no subscribe and renders its live state at once", () => {
    const { client, host, flush } = renderWithHost(<ShownThread />, testHost());
    act(() => host.window.set(SHOWN, "01A"));
    client.thread("01A", [history(1)]);
    flush();
    expect(screen.getByTestId("probe-01A").textContent).toBe("1");

    act(() => host.window.set(SHOWN, "01B"));
    client.thread("01B", [history(2)]);
    flush();
    expect(screen.getByTestId("probe-01B").textContent).toBe("2");
    // The hidden thread stays followed: its events still apply.
    client.threadEvents("01A", history(3));
    flush();

    act(() => host.window.set(SHOWN, "01A"));
    // From the kept state, before any frame or snapshot.
    expect(screen.getByTestId("probe-01A").textContent).toBe("3");
    expect(client.subscribed).toEqual([threadTopic("01A"), threadTopic("01B")]);
    expect(client.unsubscribed).toEqual([]);
  });

  // spec: UI-997, PRT-223
  test("a hidden thread is kept unwatched without resubscribing and watched again once shown", () => {
    const { client, host } = renderWithHost(<ShownThread />, testHost());
    act(() => host.window.set(SHOWN, "01A"));
    expect(client.watches).toEqual([]);

    act(() => host.window.set(SHOWN, "01B"));
    expect(client.watches).toEqual([[threadTopic("01A"), false]]);

    act(() => host.window.set(SHOWN, "01A"));
    expect(client.watches).toEqual([
      [threadTopic("01A"), false],
      [threadTopic("01B"), false],
      [threadTopic("01A"), true],
    ]);
    expect(client.subscribed).toEqual([threadTopic("01A"), threadTopic("01B")]);
    expect(client.unsubscribed).toEqual([]);
  });

  // spec: UI-996
  test("the number of kept threads follows the recent_threads setting", () => {
    const { client, host, flush } = renderWithHost(<ShownThread />, testHost());
    const settings = (recentThreads: RecentThreads) => ({ ui: { recentThreads }, keymap: [], error: null });
    client.index({ settings: settings(2) });
    flush();
    for (const id of ["01T0", "01T1", "01T2", "01T3"]) {
      act(() => host.window.set(SHOWN, id));
    }
    // Three hidden, two kept.
    expect(client.unsubscribed).toEqual([threadTopic("01T0")]);

    client.indexEvents({ kind: "settings_changed", settings: settings(0) });
    flush();
    expect(client.unsubscribed).toEqual(["01T0", "01T1", "01T2"].map(threadTopic));
    act(() => host.window.set(SHOWN, "01T1"));
    expect(client.unsubscribed).toEqual(["01T0", "01T1", "01T2", "01T3"].map(threadTopic));
    expect(
      client.watches.filter(([topic]) => topic === threadTopic("01T3")),
      "released at once, never kept",
    ).toEqual([]);
  });

  // spec: UI-996
  test("with recent_threads = all no kept thread is released, but a deleted one still is", () => {
    const { client, host, flush } = renderWithHost(<ShownThread />, testHost());
    const ids = Array.from({ length: 3 * DEFAULT_RECENT_THREADS }, (_, index) => `01T${index}`);
    client.index({ threads: ids.map(summary), settings: { ui: { recentThreads: "all" }, keymap: [], error: null } });
    flush();
    for (const id of ids) {
      act(() => host.window.set(SHOWN, id));
    }
    expect(client.unsubscribed).toEqual([]);
    expect(client.watches.filter(([, watch]) => !watch)).toHaveLength(ids.length - 1);

    client.indexEvents({ kind: "thread_removed", threadId: "01T0" });
    flush();
    expect(client.unsubscribed).toEqual([threadTopic("01T0")]);
  });

  // spec: UI-335
  test("the recent threads are bounded: the least recently shown is released first", () => {
    const { client, host } = renderWithHost(<ShownThread />, testHost());
    const ids = Array.from({ length: DEFAULT_RECENT_THREADS + 2 }, (_, index) => `01T${index}`);
    for (const id of ids) {
      act(() => host.window.set(SHOWN, id));
    }
    // The last one is shown, the others hidden: one too many is kept.
    expect(client.unsubscribed).toEqual([threadTopic("01T0")]);

    // Shown again, 01T2 is the most recent once hidden; 01T0 needs a new subscribe and 01T1 goes.
    act(() => host.window.set(SHOWN, "01T2"));
    act(() => host.window.set(SHOWN, "01T0"));
    expect(client.subscribed).toEqual([...ids, "01T0"].map(threadTopic));
    expect(client.unsubscribed).toEqual([threadTopic("01T0"), threadTopic("01T1")]);
  });

  // spec: UI-335
  test("a recent thread deleted from the index is released", () => {
    const { client, host, flush } = renderWithHost(<ShownThread />, testHost());
    client.index({ threads: [summary("01A"), summary("01B")] });
    act(() => host.window.set(SHOWN, "01A"));
    act(() => host.window.set(SHOWN, "01B"));
    flush();
    expect(client.unsubscribed).toEqual([]);

    client.indexEvents({ kind: "thread_removed", threadId: "01A" });
    flush();
    expect(client.unsubscribed).toEqual([threadTopic("01A")]);
  });

  // spec: UI-335
  test("a topic the hidden tab releases as the shown one follows it stays subscribed, its state kept", async () => {
    const topic = viewsTopic("01W");
    const { client, flush } = renderWithHost(<TwoTabs />, testHost());
    client.views("01W", { widgets: [{ source: "user", name: "clock", version: "1", entry: "index.tsx" }] });
    flush();
    expect(screen.getByTestId("views-a").textContent).toBe("1");

    act(() => screen.getByTestId("switch").click());
    await released();
    flush();
    expect(screen.getByTestId("views-b").textContent).toBe("1");
    expect(client.subscribed).toEqual([topic]);
    expect(client.unsubscribed).toEqual([]);
  });

  // spec: UI-012
  test("useThreads renders the index of the last committed frame only", () => {
    const { client, flush } = renderWithHost(<ThreadCount />);
    client.index({ threads: [summary("01A")] });
    expect(screen.getByTestId("count").textContent).toBe("0");
    flush();
    expect(screen.getByTestId("count").textContent).toBe("1");
  });

  test("useWindowState shares a slot between widgets", () => {
    renderWithHost(
      <>
        <Counter testid="a" />
        <Counter testid="b" />
      </>,
    );
    act(() => screen.getByTestId("a").click());
    expect(screen.getByTestId("b").textContent).toBe("1");
  });

  // spec: UI-095
  test("a command resolves in the scope around the focused element first, then globally", () => {
    ran.length = 0;
    renderWithHost(<Scoped />);
    const press = () => {
      // Keeps the focus where it is, as a key binding would.
      const run = screen.getByTestId("run");
      act(() => {
        run.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });
    };
    act(() => screen.getByTestId("inside").focus());
    press();
    act(() => screen.getByTestId("outside").focus());
    press();
    expect(ran).toEqual(["transcript", "global"]);
  });
});

describe("command registry", () => {
  test("aliases, disabled commands and unregistering", () => {
    const registry = new CommandRegistry();
    const args: string[][] = [];
    const off = registry.register({
      id: "window.theme",
      title: "Theme",
      aliases: ["theme"],
      run: (given) => {
        args.push([...given]);
      },
    });
    registry.register({ id: "thread.archive", title: "Archive", enabled: () => false, run: () => {} });

    expect(registry.run("theme", ["dark"])).toBe(true);
    expect(registry.run("thread.archive")).toBe(false);
    off();
    expect(registry.run("theme", ["light"])).toBe(false);
    expect(args).toEqual([["dark"]]);
  });

  // spec: UI-527
  test("a former built-in id runs the command it was renamed to, unless a command answers to it", () => {
    const registry = new CommandRegistry();
    const ran: string[] = [];
    registry.register({ id: "window.theme", title: "Theme", aliases: ["theme"], run: () => void ran.push("theme") });
    registry.register({ id: "widget.manage", title: "Widget", aliases: ["widget"], run: () => void ran.push("w") });
    expect(registry.resolve("theme.set")?.id).toBe("window.theme");
    expect(registry.run("theme.set", ["dark"])).toBe(true);
    const off = registry.register({ id: "theme.set", title: "Mine", run: () => void ran.push("mine") });
    expect(registry.run("theme.set")).toBe(true);
    off();
    expect(registry.resolve("widget")?.id).toBe("widget.manage");
    expect(ran).toEqual(["theme", "mine"]);
  });
});
