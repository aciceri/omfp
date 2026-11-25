/**
 * L4: cards in the chat (UI-920..UI-922): the widget of an `omfp_card` call drawn in its tool card's body, `useAnswer`
 * in it, and when the card's body is open.
 */

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import type { ThreadEvent } from "@omfp/core";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { type ReactNode, useState } from "react";
import {
  type Answer,
  CardScope,
  CardWidgetScope,
  SELECTED_THREAD,
  strings,
  useAnswer,
  useWidget,
  type WidgetComponent,
  type WidgetProps,
} from "../../../src/sdk/index.ts";
import { CardWidgetView, WidgetRegistryScope } from "../../../src/view/card.tsx";
import type { WidgetRegistry } from "../../../src/view/resolve.ts";
import type { Turn as CardTurn, ToolCard } from "../../../src/vm/tool.ts";
import { TranscriptWidget } from "../../../src/widgets/chat/index.ts";
import { ToolCardView } from "../../../src/widgets/chat/message.tsx";
import { composingCard, omfpCard } from "../../support/cards.ts";
import { thread, workspace } from "../../support/fixtures.ts";
import { FakeClient, renderWithHost, type TestHost, testHost } from "../../support/host.tsx";
import { fixtureEvents, isFrame } from "./fixtures.ts";
import { renderThread } from "./harness.tsx";
import { fakeLayout } from "./layout.ts";
import "../../../../kit/test/support/mermaid.ts";

let restoreLayout: () => void = () => {};
beforeEach(() => {
  restoreLayout = fakeLayout();
});
afterEach(() => {
  cleanup();
  restoreLayout();
});

const SHOWN = '{"status":"shown"}';

/** The workspace widget `./widgets/chart`. */
function Chart(props: WidgetProps): ReactNode {
  const { instance } = useWidget();
  return <p data-testid="chart">{`${instance} ${JSON.stringify(props.points)}`}</p>;
}

/** What a `code` card's compiled module exports. */
function Compiled(props: WidgetProps): ReactNode {
  const { instance } = useWidget();
  return <p data-testid="compiled">{`${instance} ${String(props.who)}`}</p>;
}

/** Shows what `useAnswer` says. */
function Probe(): ReactNode {
  const { state, value } = useAnswer();
  return <p data-testid="probe">{`${state} ${JSON.stringify(value) ?? ""}`}</p>;
}

/** The last answer a `Capture` rendered. */
let captured: Answer | undefined;

function Capture(): ReactNode {
  captured = useAnswer();
  return null;
}

function Broken(): ReactNode {
  throw new Error("boom");
}

const REGISTRY: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:probe", Probe],
  ["omfp:capture", Capture],
  ["omfp:broken", Broken],
]);

interface Setup extends TestHost {
  /** The URLs the window imported. */
  readonly urls: string[];
}

/** A window on thread `t1` of workspace `w1`, whose views state lists the widget folder `chart`. */
function setup(): Setup {
  const urls: string[] = [];
  const client = new FakeClient();
  client.connected();
  const test = testHost(client, {
    importer: (url) => {
      urls.push(url);
      return Promise.resolve(url.startsWith("blob:") ? { default: Compiled } : { default: Chart });
    },
  });
  client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "cards", "w1")] });
  client.views("w1", { widgets: [{ source: "workspace", name: "chart", version: "v1", entry: "index.tsx" }] });
  test.host.window.set(SELECTED_THREAD, "t1");
  return { ...test, urls };
}

/** Renders the cards of thread `t1`, the built-ins of {@link REGISTRY} in reach. */
function show(test: Setup, cards: readonly ToolCard[], turn: CardTurn = "past"): void {
  renderWithHost(
    <WidgetRegistryScope value={REGISTRY}>
      <CardWidgetScope value={CardWidgetView}>
        {cards.map((card) => (
          <ToolCardView key={card.toolCallId} threadId="t1" cardKey={card.toolCallId} card={card} turn={turn} />
        ))}
      </CardWidgetScope>
    </WidgetRegistryScope>,
    test,
  );
  test.flush();
}

function toolCard(): HTMLElement {
  return screen.getByTestId("tool-card");
}

describe("cards in the chat", () => {
  // spec: UI-920
  test("a reference card draws the workspace's widget with its props in its body, under the instance card:<id>", async () => {
    const test = setup();
    show(test, [omfpCard({ widget: "chart", props: { points: [1, 2] }, title: "Load" }, "succeeded", SHOWN)], "open");
    const card = toolCard();
    expect(within(card).getByTestId("tool-subject").textContent).toBe("Load");
    const slot = within(card).getByTestId("widget");
    expect([slot.dataset.widget, slot.dataset.instance, slot.dataset.path]).toEqual([
      "./widgets/chart",
      "card:call_card",
      "card/call_card",
    ]);
    expect((await within(card).findByTestId("chart")).textContent).toBe("card:call_card [1,2]");
    expect(test.urls).toEqual(["/widgets/test/w1/chart/v1/index.tsx"]);
    expect(within(card).getByTestId("card-widget").dataset.state).toBe("none");
  });

  // spec: UI-920
  test("the card's subject is its title, else its reference, else card", () => {
    const subjects = [{ widget: "chart" }, { code: "export default () => null;" }].map((args) => {
      show(setup(), [omfpCard(args, "succeeded", SHOWN)]);
      const subject = within(toolCard()).getByTestId("tool-subject").textContent;
      cleanup();
      return subject;
    });
    expect(subjects).toEqual(["./widgets/chart", strings.chat.tool.card]);
  });

  // spec: UI-920
  test("a failing widget stays inside its card: an unknown reference, a render error", () => {
    const test = setup();
    const missing = omfpCard({ widget: "nope" }, "running", undefined, "call_missing");
    const broken = omfpCard({ widget: "omfp:broken" }, "running", undefined, "call_broken");
    show(test, [missing, broken]);
    const kinds = screen
      .getAllByTestId("tool-card")
      .map((card) => [card.dataset.call, within(card).getByTestId("widget-error").dataset.errorKind]);
    expect(kinds).toEqual([
      ["call_missing", "missing"],
      ["call_broken", "render"],
    ]);
  });

  // spec: UI-920, PRT-225
  test("a code card loads the module widgets.compile makes of its source, once per source", async () => {
    const test = setup();
    test.client.answers.set("widgets.compile", () => ({ module: "export default function Card() { return null; }" }));
    const code = "export default function Card(props) { return props.who; }";
    show(test, [
      omfpCard({ code, props: { who: "a" } }, "running", undefined, "call_a"),
      omfpCard({ code, props: { who: "b" } }, "running", undefined, "call_b"),
    ]);
    await waitFor(() =>
      expect(screen.getAllByTestId("compiled").map((node) => node.textContent)).toEqual([
        "card:call_a a",
        "card:call_b b",
      ]),
    );
    expect(test.client.callsTo("widgets.compile").map((call) => call.params)).toEqual([{ code }]);
    expect(test.urls.map((url) => url.startsWith("blob:"))).toEqual([true]);
  });

  // spec: UI-920, PRT-225
  test("a refused compile shows the daemon's error in the card", async () => {
    const test = setup();
    show(test, [omfpCard({ code: "export default (" }, "running")]);
    const [compile] = test.client.callsTo("widgets.compile");
    const refusal = { code: "bad_request" as const, message: "card.tsx:1:17: Unexpected end of file" };
    await act(async () => compile?.reject(new CallError({ kind: "daemon", error: refusal })));
    const error = await within(toolCard()).findByTestId("widget-error");
    expect([error.dataset.errorKind, within(error).getByTestId("widget-error-location").textContent]).toEqual([
      "compile",
      "card.tsx:1:17",
    ]);
    expect(error.textContent).toContain("Unexpected end of file");
    expect(test.urls).toEqual([]);
  });

  // spec: UI-054, UI-056, UI-920
  test("while the model writes a card's arguments, its code shows as written and no widget compiles or mounts", () => {
    const test = setup();
    const code = "export default function Card() { return";
    show(test, [composingCard("omfp_card", `{"title": "Greeting", "code": "${code}`)]);
    expect([toolCard().dataset.status, within(toolCard()).getByTestId("tool-subject").textContent]).toEqual([
      "composing",
      "Greeting",
    ]);
    expect(within(toolCard()).getByTestId("tool-content").textContent).toBe(code);
    expect(within(toolCard()).queryByTestId("card-widget")).toBeNull();
    expect(test.client.callsTo("widgets.compile")).toEqual([]);
  });
});

describe("useAnswer", () => {
  // spec: UI-921
  test("the state and value of the card's call, none outside a card", () => {
    const test = setup();
    const waits = { widget: "omfp:probe", wait: true };
    show(
      test,
      [
        omfpCard(waits, "running", undefined, "c1"),
        omfpCard(waits, "succeeded", '{"status":"submitted","value":{"pick":2}}', "c2"),
        omfpCard(waits, "succeeded", '{"status":"cancelled"}', "c3"),
        omfpCard(waits, "succeeded", '{"status":"timeout"}', "c4"),
        omfpCard(waits, "failed", "cancelled", "c5"),
        omfpCard({ widget: "omfp:probe" }, "succeeded", SHOWN, "c6"),
      ],
      "open",
    );
    expect(screen.getAllByTestId("probe").map((probe) => probe.textContent)).toEqual([
      "waiting ",
      'submitted {"pick":2}',
      "cancelled ",
      "timeout ",
      "aborted ",
      "none ",
    ]);
    expect(screen.getAllByTestId("card-widget").map((widget) => widget.dataset.state)).toEqual([
      "waiting",
      "submitted",
      "cancelled",
      "timeout",
      "aborted",
      "none",
    ]);
    cleanup();
    renderWithHost(<Probe />, setup());
    expect(screen.getByTestId("probe").textContent).toBe("none ");
  });

  // spec: UI-921, PRT-224
  test("submit and cancel of a waiting card send card.answer and reject with the daemon's error", async () => {
    const test = setup();
    show(test, [omfpCard({ widget: "omfp:capture", wait: true }, "running", undefined, "c1")]);
    const submitted = captured?.submit({ pick: 2 });
    const cancelled = captured?.cancel();
    const [submit, cancel] = test.client.callsTo("card.answer");
    expect([submit?.params, cancel?.params]).toEqual([
      { threadId: "t1", toolCallId: "c1", answer: { kind: "submit", value: { pick: 2 } } },
      { threadId: "t1", toolCallId: "c1", answer: { kind: "cancel" } },
    ]);
    submit?.resolve({});
    const resolved = { code: "already_resolved" as const, message: "the card does not wait" };
    cancel?.reject(new CallError({ kind: "daemon", error: resolved }));
    expect(await submitted).toBeUndefined();
    await expect(cancelled).rejects.toThrow("the card does not wait");
  });

  // spec: UI-921
  test("submit and cancel reject at once unless the card waits, without a call", async () => {
    const test = setup();
    show(test, [omfpCard({ widget: "omfp:capture" }, "running", undefined, "c1")], "open");
    expect(captured?.state).toBe("none");
    await expect(captured?.submit(1)).rejects.toThrow(strings.widgets.notWaiting);
    await expect(captured?.cancel()).rejects.toThrow(strings.widgets.notWaiting);
    cleanup();
    renderWithHost(
      <CardScope value={{ threadId: "t1", toolCallId: "c2", state: "submitted", value: 1 }}>
        <Capture />
      </CardScope>,
      test,
    );
    await expect(captured?.submit(2)).rejects.toThrow(strings.widgets.notWaiting);
    expect(test.client.callsTo("card.answer")).toEqual([]);
  });
});

/** A settled card whose turn the test opens and closes. */
function Turn({ card }: { card: ToolCard }): ReactNode {
  const [turn, setTurn] = useState<CardTurn>("open");
  return (
    <>
      <button type="button" data-testid="end-turn" onClick={() => setTurn((now) => (now === "open" ? "past" : "open"))}>
        turn
      </button>
      <ToolCardView threadId="t1" cardKey={card.toolCallId} card={card} turn={turn} />
    </>
  );
}

describe("cards make way", () => {
  const expanded = () => within(toolCard()).getByTestId("toggle").getAttribute("aria-expanded");

  // spec: UI-922, UI-052
  test("a card is open while it runs or its turn is open, collapsed after; the user's toggle wins", async () => {
    show(setup(), [omfpCard({ widget: "chart" }, "running")]);
    expect(expanded()).toBe("true");
    cleanup();
    renderWithHost(<Turn card={omfpCard({ widget: "chart" }, "succeeded", SHOWN)} />, setup());
    expect(expanded()).toBe("true");
    await userEvent.click(screen.getByTestId("end-turn"));
    expect(expanded()).toBe("false");
    await userEvent.click(within(toolCard()).getByTestId("toggle"));
    await userEvent.click(screen.getByTestId("end-turn"));
    await userEvent.click(screen.getByTestId("end-turn"));
    expect(expanded()).toBe("true");
    await userEvent.click(within(toolCard()).getByTestId("toggle"));
    await userEvent.click(screen.getByTestId("end-turn"));
    expect(expanded()).toBe("false");
  });

  /** `events` with every string `from` replaced by `to`. */
  function replaced(events: readonly ThreadEvent[], from: string, to: string): ThreadEvent[] {
    return JSON.parse(JSON.stringify(events).replaceAll(JSON.stringify(from), JSON.stringify(to)));
  }

  /** The bash fixture's call as an `omfp_card` call: the transcript's last turn, the thread working or not. */
  function cardTurn(settled: boolean, userAfter = false): ThreadEvent[] {
    const events = replaced(fixtureEvents("bash_tool_call"), "bash", "omfp_card");
    const end = events.findIndex(isFrame("session_settled"));
    // The user's message of the fixture's turn (its first `message_start`/`message_end`), sent again after the card.
    const first = events.slice(
      events.findIndex(isFrame("message_start")),
      events.findIndex(isFrame("message_end")) + 1,
    );
    const user = userAfter ? replaced(first, "msg-1", "msg-again") : [];
    return [...events.slice(0, end), ...user, ...(settled ? events.slice(end) : [])];
  }

  // spec: UI-922
  test("the transcript keeps a card open while no user message follows it, settled or not; it folds after one", () => {
    renderThread(<TranscriptWidget />, cardTurn(false));
    expect([toolCard().dataset.view, expanded()]).toEqual(["card", "true"]);
    cleanup();
    renderThread(<TranscriptWidget />, cardTurn(true));
    expect(expanded()).toBe("true");
    cleanup();
    renderThread(<TranscriptWidget />, cardTurn(true, true));
    expect(expanded()).toBe("false");
    cleanup();
    renderThread(<TranscriptWidget />, cardTurn(false, true));
    expect(expanded()).toBe("false");
  });

  // spec: UI-922, PRT-224
  test("a waiting card offers Cancel outside its widget, which sends card.answer cancel", async () => {
    const test = setup();
    show(test, [
      omfpCard({ widget: "chart", wait: true }, "running", undefined, "c1"),
      omfpCard({ widget: "chart" }, "running", undefined, "c2"),
    ]);
    const [waiting, shown] = screen.getAllByTestId("tool-card");
    if (waiting === undefined || shown === undefined) {
      throw new Error("two cards");
    }
    expect(within(shown).queryAllByTestId("card-cancel").length).toBe(0);
    const cancel = within(waiting).getByTestId("card-cancel");
    expect(within(within(waiting).getByTestId("card-widget")).queryAllByTestId("card-cancel").length).toBe(0);
    await userEvent.click(cancel);
    expect(test.client.callsTo("card.answer").map((call) => call.params)).toEqual([
      { threadId: "t1", toolCallId: "c1", answer: { kind: "cancel" } },
    ]);
  });
});
