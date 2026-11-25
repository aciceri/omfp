/** L4: `omfp:interactions` (and inline in `omfp:chat`) over recorded fixtures. */

import { afterEach, describe, expect, test } from "bun:test";
import type { AskUiRequest, ThreadEvent } from "@omfp/core";
import { act, cleanup, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ChatWidget, InteractionsWidget } from "../../../src/widgets/chat/index.ts";
import { fixtureEvents } from "./fixtures.ts";
import { ManualClock, renderThread, THREAD } from "./harness.tsx";

afterEach(cleanup);

/** `approval_granted` up to (excluding) the daemon resolving the approval select, and that resolution. */
function approval(): { pending: ThreadEvent[]; resolved: ThreadEvent } {
  const events = fixtureEvents("approval_granted");
  const at = events.findIndex((event) => event.kind === "interaction_resolved");
  const resolved = events[at];
  if (resolved === undefined) {
    throw new Error("fixture without a resolution");
  }
  return { pending: events.slice(0, at), resolved };
}

const ASK: AskUiRequest = {
  id: "ask-1",
  timeout: 30000,
  questions: [
    { id: "db", question: "Which database?", multi: false, options: [{ label: "Postgres" }, { label: "SQLite" }] },
    {
      id: "features",
      question: "Which features?",
      multi: true,
      options: [{ label: "Auth" }, { label: "Billing" }, { label: "Search" }],
    },
  ],
};

function request(frame: Record<string, unknown>): ThreadEvent {
  return { kind: "omp", frame: { type: "extension_ui_request", ...frame } } as ThreadEvent;
}

const base = () => fixtureEvents("text_reply");

describe("interactions", () => {
  // spec: UI-070, UI-164, UI-187
  test("an approval select renders inline at the bottom of the transcript and takes the focus", async () => {
    const { client } = renderThread(<ChatWidget />, approval().pending);
    const card = screen.getByTestId("interaction");
    expect(card.dataset.kind).toBe("select");
    expect(card.closest('[data-testid="transcript"]')).not.toBeNull();
    expect(card.querySelector(".omfp-bbox__head > .omfp-bbox__label")?.textContent).toBe("◆ Allow tool: bash");
    expect(within(card).getByTestId("interaction-detail").textContent).toBe("Command: echo approved-run");
    const options = within(card).getAllByTestId("interaction-option");
    expect(options.map((option) => option.textContent)).toEqual(["Approve", "Approve bash for this session", "Deny"]);
    expect(document.activeElement === options[0]).toBe(true);
    await userEvent.click(options[0] as HTMLElement);
    expect(client.callsTo("interaction.respond").map((call) => call.params)).toEqual([
      { threadId: THREAD, interactionId: "<id>", response: { value: "Approve" } },
    ]);
    expect(card.getAttribute("aria-busy")).toBe("true");
  });

  // spec: UI-070, UI-092
  test("the arrow keys move between a select's options; Tab leaves them for Dismiss", async () => {
    renderThread(<ChatWidget />, approval().pending);
    const focused = () => document.activeElement?.textContent;
    expect(focused()).toBe("Approve");
    await userEvent.keyboard("{ArrowRight}");
    expect(focused()).toBe("Approve bash for this session");
    // happy-dom's TreeWalker cannot walk backwards: E2E-02 checks ArrowLeft in Chromium.
    await userEvent.keyboard("{Tab}");
    expect(document.activeElement?.getAttribute("data-testid")).toBe("interaction-dismiss");
  });

  // spec: UI-092
  test("a new request is announced", () => {
    const { pending } = approval();
    const { client, flush, host } = renderThread(<InteractionsWidget />, pending.slice(0, -1));
    client.threadEvents(THREAD, ...pending.slice(-1));
    flush();
    expect(host.announcer.getSnapshot().message).toBe("Request: Allow tool: bash\nCommand: echo approved-run");
  });

  // spec: UI-450
  test("an ask's select highlights no option; an approval keeps Approve primary", () => {
    const options = () => screen.getAllByTestId("interaction-option").map((option) => option.dataset.variant);
    const ask = request({ method: "select", id: "s", title: "Which database?", options: ["Postgres", "SQLite"] });
    renderThread(<InteractionsWidget />, [...base(), ask]);
    expect(options()).toEqual(["secondary", "secondary"]);
    cleanup();
    renderThread(<InteractionsWidget />, approval().pending);
    expect(options()).toEqual(["primary", "secondary", "secondary"]);
  });

  // spec: UI-072
  test("a request answered by another client closes with who resolved it", async () => {
    const { pending, resolved } = approval();
    const { client, flush } = renderThread(<InteractionsWidget />, pending);
    client.threadEvents(THREAD, resolved);
    flush();
    expect(screen.queryByTestId("interaction")).toBeNull();
    const closed = screen.getByTestId("interaction-closed");
    expect(closed.textContent).toContain("Answered elsewhere by contract-test");
    await userEvent.click(within(closed).getByTestId("interaction-closed-dismiss"));
    expect(screen.queryByTestId("interaction-closed")).toBeNull();
  });

  // spec: UI-072
  test("a request answered here closes without an 'answered elsewhere' row; a refused answer can be retried", async () => {
    const { pending, resolved } = approval();
    const { client, flush } = renderThread(<InteractionsWidget />, pending);
    await userEvent.click(screen.getAllByTestId("interaction-option")[2] as HTMLElement);
    const [first] = client.callsTo("interaction.respond");
    await act(async () => first?.reject(new Error("bad_request")));
    expect(screen.getByTestId("interaction").getAttribute("aria-busy")).toBe("false");
    await userEvent.click(screen.getAllByTestId("interaction-option")[2] as HTMLElement);
    client.threadEvents(THREAD, resolved);
    flush();
    expect(screen.queryByTestId("interaction")).toBeNull();
    expect(screen.queryByTestId("interaction-closed")).toBeNull();
  });

  // spec: UI-071
  test("ask offers free text for every question and enforces single/multi rules before sending", async () => {
    const { client } = renderThread(<InteractionsWidget />, [...base(), request({ method: "ask", ...ASK })]);
    const form = screen.getByTestId("ask-form");
    const questions = within(form).getAllByTestId("ask-question");
    expect(questions.map((question) => question.dataset.multi)).toEqual(["false", "true"]);
    expect(within(form).getAllByTestId("ask-custom").length).toBe(2);
    const submit = within(form).getByTestId("interaction-submit");
    expect(submit.hasAttribute("disabled") || submit.getAttribute("aria-disabled") === "true").toBe(true);
    expect(within(form).getByTestId("ask-error").textContent).toBe("Answer \u201cWhich database?\u201d");

    const [single, multi] = questions as [HTMLElement, HTMLElement];
    await userEvent.click(within(single).getAllByTestId("ask-option")[0] as HTMLElement);
    await userEvent.click(within(single).getAllByTestId("ask-option")[1] as HTMLElement);
    await userEvent.click(within(multi).getAllByTestId("ask-option")[2] as HTMLElement);
    await userEvent.click(within(multi).getAllByTestId("ask-option")[0] as HTMLElement);
    await userEvent.type(within(multi).getByTestId("ask-custom"), "Audit");
    expect(within(form).queryByTestId("ask-error")).toBeNull();
    await userEvent.click(submit);
    expect(client.callsTo("interaction.respond")[0]?.params).toEqual({
      threadId: THREAD,
      interactionId: "ask-1",
      response: {
        answers: [
          { id: "db", selectedOptions: ["SQLite"] },
          { id: "features", selectedOptions: ["Auth", "Search"], customInput: "Audit" },
        ],
      },
    });
  });

  // spec: UI-071
  test("free text replaces the choice of a single-select question", async () => {
    renderThread(<InteractionsWidget />, [...base(), request({ method: "ask", ...ASK })]);
    const [single] = screen.getAllByTestId("ask-question") as [HTMLElement];
    const radio = within(single).getAllByTestId("ask-option")[0] as HTMLElement;
    await userEvent.click(radio);
    expect(radio.dataset.selected).toBe("true");
    await userEvent.type(within(single).getByTestId("ask-custom"), "MySQL");
    expect(radio.dataset.selected).toBeUndefined();
  });

  // spec: UI-073
  test("an announced timeout shows a countdown from when the request was shown", () => {
    const clock = new ManualClock(1_000);
    renderThread(<InteractionsWidget />, [...base(), request({ method: "ask", ...ASK })], clock);
    const countdown = screen.getByTestId("countdown");
    expect(countdown.textContent).toBe("0:30");
    act(() => clock.advance(1_500));
    expect(countdown.textContent).toBe("0:29");
    act(() => clock.advance(40_000));
    expect(countdown.textContent).toBe("0:00");
    expect(countdown.dataset.expired).toBe("true");
  });

  // spec: UI-070
  test("confirm, input and editor requests answer with their own shapes", async () => {
    const { client } = renderThread(<InteractionsWidget />, [
      ...base(),
      request({ method: "confirm", id: "c", title: "Proceed?", message: "Really?" }),
      request({ method: "input", id: "i", title: "Name?", placeholder: "name" }),
      request({ method: "editor", id: "e", title: "Edit", prefill: "draft" }),
    ]);
    expect(screen.queryByTestId("countdown")).toBeNull();
    expect(document.activeElement === screen.getByTestId("interaction-confirm")).toBe(true);
    await userEvent.click(screen.getByTestId("interaction-confirm"));
    await userEvent.type(screen.getByTestId("interaction-input"), "omfp{Enter}");
    const editor = screen.getByTestId("interaction-editor") as HTMLTextAreaElement;
    expect(editor.value).toBe("draft");
    await userEvent.type(editor, "!");
    const cards = screen.getAllByTestId("interaction");
    await userEvent.click(within(cards[2] as HTMLElement).getByTestId("interaction-submit"));
    expect(client.callsTo("interaction.respond").map((call) => call.params)).toEqual([
      { threadId: THREAD, interactionId: "c", response: { confirmed: true } },
      { threadId: THREAD, interactionId: "i", response: { value: "omfp" } },
      { threadId: THREAD, interactionId: "e", response: { value: "draft!" } },
    ]);
  });

  test("dismissing a request cancels it", async () => {
    const { client } = renderThread(<InteractionsWidget />, approval().pending);
    await userEvent.click(screen.getByTestId("interaction-dismiss"));
    expect(client.callsTo("interaction.respond")[0]?.params).toEqual({
      threadId: THREAD,
      interactionId: "<id>",
      response: { cancelled: true },
    });
  });

  const FORM = `<form>
    <fieldset><legend>Deploy</legend>
      <p>Pick a <strong>region</strong><br>for the <code>api</code></p>
      <label><input type="radio" name="region" value="eu" required> Europe</label>
      <label><input type="radio" name="region" value="us"> US</label>
      <ul><li>Lint is fast</li></ul>
      <label><input type="checkbox" name="checks" value="lint"> Lint</label>
      <label><input type="checkbox" name="checks" value="test" disabled> Test</label>
      <table><tr><th>Tier</th></tr><tr><td>free</td></tr></table>
      <img src="/tmp/plot.png" alt="Latency plot">
      <label>Replicas <input type="number" name="replicas" min="1" required></label>
      <input type="hidden" name="plan" value="p-7">
    </fieldset>
  </form>`;

  // spec: UI-074, UI-070, UI-073
  test("a form renders omp's whole HTML form, flags broken rules, holds back what omp refuses and sends", async () => {
    const { client } = renderThread(<InteractionsWidget />, [
      ...base(),
      request({ method: "form", id: "form-1", html: FORM, timeout: 30000 }),
    ]);
    const card = screen.getByTestId("interaction");
    expect(card.dataset.kind).toBe("form");
    expect(card.getAttribute("aria-label")).toBe("Deploy");
    expect(screen.getByTestId("countdown").textContent).toBe("0:30");
    const form = within(card).getByTestId("form-form");
    const intro = within(form).getAllByTestId("form-text")[0];
    expect(intro?.innerHTML).toBe(
      "<span>Pick a </span><strong>region</strong><span><br>for the </span><code>api</code>",
    );
    expect(within(form).getByTestId("form-list").textContent).toBe("Lint is fast");
    expect(within(form).getByTestId("form-table").querySelectorAll("thead th").length).toBe(1);
    // A local picture is never loaded: its alt text stands in.
    expect(within(form).queryByTestId("form-image")).toBeNull();
    expect(within(form).getByTestId("form-image-text").textContent).toBe("Image: Latency plot");

    const [region, checks] = within(form).getAllByTestId("form-field") as [HTMLElement, HTMLElement];
    const regionOptions = within(region).getAllByTestId("form-option");
    expect(regionOptions.map((option) => option.textContent)).toEqual(["Europe", "US"]);
    expect(within(region).getByTestId("form-other-choice").textContent).toBe("Other");
    // The first control takes the focus (UI-070).
    expect(document.activeElement === regionOptions[0]?.querySelector("input")).toBe(true);
    const checkOptions = within(checks).getAllByTestId("form-option");
    expect(checkOptions.map((option) => option.dataset.disabled)).toEqual([undefined, "true"]);

    // An unanswered required question is only marked; a broken rule is flagged as the user types.
    expect(within(form).queryByTestId("form-field-error")).toBeNull();
    await userEvent.click(regionOptions[0] as HTMLElement);
    await userEvent.click(checkOptions[0] as HTMLElement);
    const replicas = within(form).getAllByTestId("form-input")[0] as HTMLElement;
    await userEvent.type(replicas, "0");
    expect(
      within(form)
        .getAllByTestId("form-field-error")
        .map((error) => error.textContent),
    ).toEqual(["Must be at least 1"]);
    expect(within(form).queryByTestId("form-error")).toBeNull();

    // An Other answer naming the disabled option is one omp would refuse: Submit holds it back and says why.
    await userEvent.type(within(checks).getByTestId("form-other"), "test");
    const submit = within(form).getByTestId("interaction-submit");
    await userEvent.click(submit);
    expect(client.callsTo("interaction.respond").length).toBe(0);
    expect(
      within(form)
        .getAllByTestId("form-field-error")
        .map((error) => error.textContent),
    ).toEqual(["\u201ctest\u201d is not available", "Must be at least 1"]);
    expect(within(form).getByTestId("form-error").textContent).toBe("Fix the answers marked above");
    expect(checks.contains(document.activeElement)).toBe(true);

    // The broken rule goes with the answer, as omp 19.2 takes it.
    await userEvent.clear(within(checks).getByTestId("form-other"));
    await userEvent.type(within(checks).getByTestId("form-other"), "docs");
    expect(within(form).queryByTestId("form-error")).toBeNull();
    await userEvent.click(submit);
    expect(client.callsTo("interaction.respond").map((call) => call.params)).toEqual([
      {
        threadId: THREAD,
        interactionId: "form-1",
        response: { data: { plan: "p-7", region: "eu", checks: ["lint", "docs"], replicas: 0 } },
      },
    ]);
    expect(card.getAttribute("aria-busy")).toBe("true");
  });

  // spec: UI-074
  test("a form's Other answer replaces the radio choice; Dismiss cancels the form", async () => {
    const { client } = renderThread(<InteractionsWidget />, [
      ...base(),
      request({ method: "form", id: "form-2", html: FORM }),
    ]);
    expect(screen.queryByTestId("countdown")).toBeNull();
    const [region] = screen.getAllByTestId("form-field") as [HTMLElement];
    const europe = within(region).getAllByTestId("form-option")[0] as HTMLElement;
    await userEvent.click(europe);
    expect(europe.dataset.selected).toBe("true");
    await userEvent.type(within(region).getByTestId("form-other"), "Asia");
    expect(europe.dataset.selected).toBeUndefined();
    expect(within(region).getByTestId("form-other-choice").dataset.selected).toBe("true");
    await userEvent.click(screen.getByTestId("interaction-dismiss"));
    expect(client.callsTo("interaction.respond").map((call) => call.params)).toEqual([
      { threadId: THREAD, interactionId: "form-2", response: { cancelled: true } },
    ]);
  });

  // spec: UI-074
  test("a multi-select shows its own label as its question; a checkbox group's question is the text around it", () => {
    renderThread(<InteractionsWidget />, [
      ...base(),
      request({
        method: "form",
        id: "form-3",
        html: `<form>
          <label>Languages to enable <select name="langs" multiple><option value="rs">Rust</option><option value="ts">TS</option></select></label>
          <p>Extras</p>
          <label><input type="checkbox" name="extras" value="a"> A</label>
          <label><input type="checkbox" name="extras" value="b"> B</label>
        </form>`,
      }),
    ]);
    const [langs, extras] = screen.getAllByTestId("form-field") as [HTMLElement, HTMLElement];
    expect(within(langs).getByTestId("form-field-label").textContent).toBe("Languages to enable");
    expect(within(extras).queryByTestId("form-field-label")).toBeNull();
  });
});
