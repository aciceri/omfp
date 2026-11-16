/** L0: the tab selected in each `tabs` container per thread and view (UI-372). */

import { describe, expect, test } from "bun:test";
import {
  adoptDraft,
  chooseTab,
  chosenTab,
  decodeTabChoices,
  dropDraft,
  keepListedTabs,
  NO_TAB_CHOICES,
  replaceTab,
  type TabChoices,
  type TabOwner,
} from "../../src/vm/tab-choices.ts";
import { VIEW_CHOICE_THREADS } from "../../src/vm/view-choices.ts";

const T1: TabOwner = { kind: "thread", threadId: "t1" };
const T2: TabOwner = { kind: "thread", threadId: "t2" };
const DRAFT: TabOwner = { kind: "draft", workspaceId: "w1" };

describe("tab choices", () => {
  // spec: UI-372
  test("each thread keeps its tab per view and container; none chosen is the first tab", () => {
    let choices = chooseTab(NO_TAB_CHOICES, T1, "work", "main", "file:a.rs");
    choices = chooseTab(choices, T1, "work", "0.1", "0.1.1");
    choices = chooseTab(choices, T1, "lean", "main", "widget:omfp:todos");
    choices = chooseTab(choices, T2, "work", "main", "0.0.0");
    expect(chosenTab(choices, T1, "work", "main")).toBe("file:a.rs");
    expect(chosenTab(choices, T1, "work", "0.1")).toBe("0.1.1");
    expect(chosenTab(choices, T1, "lean", "main")).toBe("widget:omfp:todos");
    expect(chosenTab(choices, T2, "work", "main")).toBe("0.0.0");
    expect(chosenTab(choices, T2, "work", "0.1")).toBeUndefined();
    expect(chosenTab(choices, T2, "lean", "main")).toBeUndefined();
    expect(chosenTab(choices, { kind: "thread", threadId: "t3" }, "work", "main")).toBeUndefined();
    expect(chosenTab(choices, T1, "constructor", "main")).toBeUndefined();
    // The same choice again changes nothing; back to the first tab forgets it.
    expect(chooseTab(choices, T1, "work", "main", "file:a.rs")).toBe(choices);
    choices = chooseTab(choices, T1, "work", "main", undefined);
    expect(chosenTab(choices, T1, "work", "main")).toBeUndefined();
    expect(chooseTab(choices, T1, "work", "main", undefined)).toBe(choices);
  });

  // spec: UI-372
  test("at most as many threads as keep a view; the least recently changed goes first", () => {
    let choices: TabChoices = NO_TAB_CHOICES;
    for (let index = 0; index <= VIEW_CHOICE_THREADS; index++) {
      choices = chooseTab(choices, { kind: "thread", threadId: `t${index}` }, "work", "main", "file:a.rs");
    }
    expect(choices.threads.length).toBe(VIEW_CHOICE_THREADS);
    expect(chosenTab(choices, { kind: "thread", threadId: "t0" }, "work", "main")).toBeUndefined();
    choices = chooseTab(choices, { kind: "thread", threadId: "t1" }, "work", "main", "file:b.rs");
    expect(choices.threads.at(-1)?.threadId).toBe("t1");
  });

  // spec: UI-372
  test("a closed tab is forgotten by the threads of its workspace and its draft only", () => {
    let choices = chooseTab(NO_TAB_CHOICES, T1, "work", "main", "file:a.rs");
    choices = chooseTab(choices, T1, "lean", "main", "file:a.rs");
    choices = chooseTab(choices, T1, "work", "0.1", "0.1.1");
    choices = chooseTab(choices, T2, "work", "main", "file:a.rs");
    choices = chooseTab(choices, DRAFT, "work", "main", "file:a.rs");
    const inW1 = (threadId: string) => threadId === "t1";
    const forgotten = replaceTab(choices, "w1", inW1, "file:a.rs", undefined);
    expect(chosenTab(forgotten, T1, "work", "main")).toBeUndefined();
    expect(chosenTab(forgotten, T1, "lean", "main")).toBeUndefined();
    expect(chosenTab(forgotten, T1, "work", "0.1")).toBe("0.1.1");
    expect(chosenTab(forgotten, DRAFT, "work", "main")).toBeUndefined();
    // t2 is another workspace's thread: its `a.rs` is another file.
    expect(chosenTab(forgotten, T2, "work", "main")).toBe("file:a.rs");
    expect(replaceTab(forgotten, "w1", inW1, "file:a.rs", undefined)).toBe(forgotten);
    expect(replaceTab(choices, "w1", inW1, "file:gone.rs", undefined)).toBe(choices);
    // A missing path's tab showing the file it names: the threads that selected it select the file's tab.
    const moved = replaceTab(choices, "w1", inW1, "file:a.rs", "file:src/a.rs");
    expect(chosenTab(moved, T1, "lean", "main")).toBe("file:src/a.rs");
    expect(chosenTab(moved, T1, "work", "0.1")).toBe("0.1.1");
    expect(chosenTab(moved, DRAFT, "work", "main")).toBe("file:src/a.rs");
    expect(chosenTab(moved, T2, "work", "main")).toBe("file:a.rs");
  });

  // spec: UI-372
  test("the draft thread keeps its own tabs per workspace; the thread it becomes takes them, discarding drops them", () => {
    let choices = chooseTab(NO_TAB_CHOICES, DRAFT, "work", "0.1", "0.1.2");
    choices = chooseTab(choices, { kind: "draft", workspaceId: "w2" }, "work", "0.1", "0.1.1");
    expect(chosenTab(choices, DRAFT, "work", "0.1")).toBe("0.1.2");
    const adopted = adoptDraft(choices, "w1", "t9");
    expect(chosenTab(adopted, { kind: "thread", threadId: "t9" }, "work", "0.1")).toBe("0.1.2");
    expect(chosenTab(adopted, DRAFT, "work", "0.1")).toBeUndefined();
    expect(chosenTab(adopted, { kind: "draft", workspaceId: "w2" }, "work", "0.1")).toBe("0.1.1");
    // A thread with tabs of its own keeps them.
    const own = chooseTab(choices, T1, "work", "0.1", "0.1.0");
    expect(chosenTab(adoptDraft(own, "w1", "t1"), T1, "work", "0.1")).toBe("0.1.0");
    expect(adoptDraft(adopted, "w1", "t8")).toBe(adopted);
    const dropped = dropDraft(choices, "w2");
    expect(chosenTab(dropped, { kind: "draft", workspaceId: "w2" }, "work", "0.1")).toBeUndefined();
    expect(dropDraft(dropped, "w2")).toBe(dropped);
  });

  // spec: UI-372
  test("the threads and drafts of workspaces the index no longer lists are dropped", () => {
    let choices = chooseTab(NO_TAB_CHOICES, T1, "work", "main", "file:a.rs");
    choices = chooseTab(choices, T2, "work", "main", "file:b.rs");
    choices = chooseTab(choices, DRAFT, "work", "main", "file:c.rs");
    expect(keepListedTabs(choices, new Set(["t1", "t2"]), new Set(["w1"]))).toBe(choices);
    expect(keepListedTabs(choices, new Set(["t2"]), new Set<string>())).toEqual({
      threads: [{ threadId: "t2", views: { work: { main: "file:b.rs" } } }],
      drafts: {},
    });
  });

  // spec: UI-372
  test("the persisted choices decode leniently", () => {
    expect(
      decodeTabChoices({
        threads: [
          { threadId: "t1", views: { work: { main: "file:a.rs", "0.1": 3 }, lean: "x" } },
          { threadId: 2, views: {} },
          "x",
          { threadId: "t2" },
        ],
        drafts: { w1: { work: { main: "widget:omfp:todos" } }, w2: null },
      }),
    ).toEqual({
      threads: [
        { threadId: "t1", views: { work: { main: "file:a.rs" } } },
        { threadId: "t2", views: {} },
      ],
      drafts: { w1: { work: { main: "widget:omfp:todos" } }, w2: {} },
    });
    expect(decodeTabChoices({ threads: "x", drafts: [] })).toEqual(NO_TAB_CHOICES);
    expect(decodeTabChoices([])).toBeUndefined();
    expect(decodeTabChoices("x")).toBeUndefined();
  });
});
