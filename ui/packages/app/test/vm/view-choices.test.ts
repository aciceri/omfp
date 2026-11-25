/** L0: the views a window chose per thread and on the draft thread (UI-367). */

import { describe, expect, test } from "bun:test";
import {
  activeView,
  adoptDraftView,
  chooseView,
  decodeViewChoices,
  dropDraftView,
  keepListed,
  NO_VIEW_CHOICES,
  VIEW_CHOICE_THREADS,
  type ViewChoices,
} from "../../src/vm/view-choices.ts";

const VIEWS = ["work", "review", "lean"];

describe("view choices", () => {
  // spec: UI-367
  test("a thread's own view, else the default; another thread's choice never leaks; views gone are skipped", () => {
    let choices = chooseView(NO_VIEW_CHOICES, "t1", "review");
    choices = chooseView(choices, "t2", "lean");
    expect(activeView(choices, "t1", VIEWS)).toBe("review");
    expect(activeView(choices, "t2", VIEWS)).toBe("lean");
    expect(activeView(choices, "t3", VIEWS)).toBeUndefined();
    expect(activeView(choices, undefined, VIEWS)).toBeUndefined();
    expect(activeView(choices, "t1", ["work", "lean"])).toBeUndefined();
  });

  // spec: UI-367
  test("the draft opens on the default view, keeps its own choice, and hands it to the thread created from it", () => {
    let choices = chooseView(NO_VIEW_CHOICES, "t1", "review");
    expect(activeView(choices, undefined, VIEWS)).toBeUndefined();
    choices = chooseView(choices, undefined, "lean");
    expect(chooseView(choices, undefined, "lean")).toBe(choices);
    expect(activeView(choices, undefined, VIEWS)).toBe("lean");
    expect(activeView(choices, "t1", VIEWS)).toBe("review");
    expect(activeView(choices, undefined, ["work"])).toBeUndefined();
    const created = adoptDraftView(choices, "t2", choices.draft);
    expect(activeView(created, "t2", VIEWS)).toBe("lean");
    expect(adoptDraftView(choices, "t2", undefined)).toBe(choices);
    const reset = dropDraftView(created);
    expect(reset).toEqual({ threads: created.threads });
    expect(activeView(reset, undefined, VIEWS)).toBeUndefined();
    expect(dropDraftView(reset)).toBe(reset);
  });

  // spec: UI-367
  test("choosing again moves the thread last; the oldest threads go over the bound; no change keeps the object", () => {
    let choices: ViewChoices = NO_VIEW_CHOICES;
    for (let index = 0; index <= VIEW_CHOICE_THREADS; index += 1) {
      choices = chooseView(choices, `t${index}`, "work");
    }
    expect(choices.threads.length).toBe(VIEW_CHOICE_THREADS);
    expect(choices.threads[0]?.threadId).toBe("t1");
    choices = chooseView(choices, "t1", "lean");
    expect(choices.threads.at(-1)).toEqual({ threadId: "t1", viewId: "lean" });
    expect(choices.threads.length).toBe(VIEW_CHOICE_THREADS);
    expect(chooseView(choices, "t1", "lean")).toBe(choices);
  });

  // spec: UI-367
  test("threads the index no longer lists are dropped; the draft's view stays", () => {
    let choices = chooseView(NO_VIEW_CHOICES, "t1", "review");
    choices = chooseView(choices, "t2", "lean");
    choices = chooseView(choices, undefined, "work");
    expect(keepListed(choices, new Set(["t1", "t2"]))).toBe(choices);
    expect(keepListed(choices, new Set(["t2"]))).toEqual({
      threads: [{ threadId: "t2", viewId: "lean" }],
      draft: "work",
    });
  });

  // spec: UI-367
  test("decoding skips malformed entries and the per-workspace choices of earlier versions, keeping the newest threads", () => {
    expect(decodeViewChoices("review")).toBeUndefined();
    expect(decodeViewChoices(null)).toBeUndefined();
    expect(decodeViewChoices([])).toBeUndefined();
    expect(
      decodeViewChoices({
        threads: [{ threadId: "t1", viewId: "review" }, { threadId: 2, viewId: "x" }, null, "t3"],
        workspaces: { w1: "lean" },
        draft: "lean",
      }),
    ).toEqual({ threads: [{ threadId: "t1", viewId: "review" }], draft: "lean" });
    expect(decodeViewChoices({ threads: "x", draft: 3 })).toEqual(NO_VIEW_CHOICES);
    const many = Array.from({ length: VIEW_CHOICE_THREADS + 5 }, (_, index) => ({
      threadId: `t${index}`,
      viewId: "v",
    }));
    const decoded = decodeViewChoices({ threads: many });
    expect([decoded?.threads.length, decoded?.threads[0]?.threadId]).toEqual([VIEW_CHOICE_THREADS, "t5"]);
  });
});
