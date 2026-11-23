/** L1: the left column's view-model (sections, filter, collapse, unread, attention). */

import { describe, expect, test } from "bun:test";
import type { ThreadSummary, Workspace, WorkspaceRepo } from "@omfp/protocol";
import { type KeyValueStorage, windowKey } from "../../src/sdk/index.ts";
import { WindowStore } from "../../src/sdk/window-store.ts";
import {
  adjacentThread,
  ConversationSearch,
  decodeIds,
  displayTitle,
  forkLineage,
  isUnread,
  isWorkspacePath,
  moveSelection,
  newWorkspaceParams,
  nextNeedingAttention,
  SEARCH_DELAY_MS,
  type SearchHit,
  sidebarSections,
  spawnedCounts,
  spawnedTree,
  spawnerOf,
  type ThreadFilter,
  threadStatus,
  toggled,
  updateSeen,
  visibleOrder,
  waitingCount,
  workspaceChip,
} from "../../src/vm/threads.ts";
import { NO_FILTER, repo, thread, workspace } from "../support/fixtures.ts";

function index(): { workspaces: Workspace[]; threads: ThreadSummary[] } {
  const linked: WorkspaceRepo = {
    ...repo("w-checkout", "be", "main"),
    kind: "link",
    target: "/src/be",
    canonicalPath: "/src/be",
  };
  const workspaces = [
    { ...workspace("w-scratch"), name: "scratch" },
    { ...workspace("w-checkout", [repo("w-checkout", "fe", "feat/checkout"), linked]), name: "checkout" },
    { ...workspace("w-flaky", [repo("w-flaky", "be", "flaky")]), name: "Flaky" },
    { ...workspace("w-empty"), name: "empty" },
  ];
  const threads = [
    thread("t-checkout", "checkout-v2", "w-checkout"),
    thread("t-flaky", "flaky-tests", "w-flaky"),
    thread("t-old", "old flake", "w-flaky", { archivedAt: 3 }),
    thread("t-scratch", "scratch", "w-scratch"),
    thread("t-orphan", "orphan", "w-gone"),
  ];
  return { workspaces, threads };
}

function sections(filter: ThreadFilter = NO_FILTER): Array<[string, readonly string[], readonly string[]]> {
  const { workspaces, threads } = index();
  return sidebarSections(threads, workspaces, filter).map((section) => [
    section.workspace.name,
    section.threadIds,
    section.archivedIds,
  ]);
}

function filter(query: string, conversation: Record<string, string> = {}): ThreadFilter {
  return { query, conversation: new Map(Object.entries(conversation)) };
}

/** A `localStorage` stand-in. */
function memoryStorage(): KeyValueStorage & { readonly items: Map<string, string> } {
  const items = new Map<string, string>();
  return { items, getItem: (key) => items.get(key) ?? null, setItem: (key, value) => void items.set(key, value) };
}

/** Timers run by the test. */
function manualTimers() {
  const pending: Array<{ run: () => void; ms: number; cancelled: boolean }> = [];
  return {
    pending,
    schedule: (run: () => void, ms: number) => {
      const timer = { run, ms, cancelled: false };
      pending.push(timer);
      return () => {
        timer.cancelled = true;
      };
    },
    fire: () => {
      for (const timer of pending.splice(0)) {
        if (!timer.cancelled) {
          timer.run();
        }
      }
    },
  };
}

describe("thread list", () => {
  // spec: UI-020, UI-011
  test("status puts attention before the process and the settled flag", () => {
    const summary = thread("t", "t", "w");
    expect(threadStatus(summary)).toBe("idle");
    expect(threadStatus({ ...summary, settled: false })).toBe("running");
    expect(threadStatus({ ...summary, settled: false, needsAttention: true })).toBe("awaiting");
    expect(threadStatus({ ...summary, process: { state: "failed", message: "boom" } })).toBe("failed");
    expect(threadStatus({ ...summary, process: { state: "stopped" } })).toBe("stopped");
  });

  // spec: UI-020
  test("a thread without a title yet shows the placeholder title", () => {
    expect(displayTitle(thread("t", "", "w"))).toBe("New thread");
    expect(displayTitle(thread("t", "checkout", "w"))).toBe("checkout");
  });

  // spec: UI-340, UI-011
  test("the workspace chip is the name and repository count, the repositories with branch and kind its tooltip", () => {
    const { workspaces, threads } = index();
    const workspaceId = threads[0]?.workspaceId ?? "";
    expect(workspaceChip(workspaceId, workspaces)).toEqual({
      workspaceId,
      label: "checkout · 2 repos",
      tooltip: "fe:feat/checkout (worktree)\nbe:main (link)",
      repoCount: 2,
    });
    expect(workspaceChip("w-unknown", workspaces)).toBeUndefined();
  });

  // spec: UI-020, UI-026
  test("every workspace is a section by name with its threads; archived ones apart; no orphan", () => {
    expect(sections()).toEqual([
      ["checkout", ["t-checkout"], []],
      ["empty", [], []],
      ["Flaky", ["t-flaky"], ["t-old"]],
      ["scratch", ["t-scratch"], []],
    ]);
  });

  // spec: UI-095
  test("j/k, gg/G and half pages move through the list order, clamped at both ends", () => {
    const order = ["a", "b", "c"];
    expect(moveSelection(order, undefined, 1)).toBe("a");
    expect(moveSelection(order, undefined, -1)).toBe("c");
    expect(moveSelection(order, "a", 1)).toBe("b");
    expect(moveSelection(order, "a", -1)).toBe("a");
    expect(moveSelection(order, "b", 10)).toBe("c");
    expect(moveSelection(order, "c", "top")).toBe("a");
    expect(moveSelection(order, "a", "bottom")).toBe("c");
    expect(moveSelection(order, "gone", 1)).toBe("a");
    expect(moveSelection([], "a", 1)).toBeUndefined();
  });

  // spec: UI-022
  test("the filter matches title, workspace and repository names, case-insensitively, and hides other workspaces", () => {
    expect(sections(filter("V2"))).toEqual([["checkout", ["t-checkout"], []]]);
    expect(sections(filter("flaky"))).toEqual([["Flaky", ["t-flaky"], ["t-old"]]]);
    expect(sections(filter("FE"))).toEqual([["checkout", ["t-checkout"], []]]);
    expect(sections(filter("be"))).toEqual([
      ["checkout", ["t-checkout"], []],
      ["Flaky", ["t-flaky"], ["t-old"]],
    ]);
    expect(sections(filter("nothing like it"))).toEqual([]);
  });

  // spec: UI-023
  test("+ workspace input starting with / or ~ links that repository, anything else is the name", () => {
    expect(isWorkspacePath("/src/be")).toBe(true);
    expect(isWorkspacePath("~/src")).toBe(true);
    expect(isWorkspacePath("checkout")).toBe(false);
    expect(newWorkspaceParams(" /src/be/ ")).toEqual({ link: "/src/be" });
    expect(newWorkspaceParams("~/src/fe")).toEqual({ link: "~/src/fe" });
    expect(newWorkspaceParams("/")).toEqual({ link: "/" });
    expect(newWorkspaceParams(" checkout ")).toEqual({ name: "checkout" });
    expect(newWorkspaceParams("  ")).toEqual({});
  });

  // spec: UI-022
  test("conversation matches from threads.search join the local ones while a query is typed", () => {
    expect(sections(filter("retry loop", { "t-scratch": "…the retry loop…", "t-old": "a retry loop" }))).toEqual([
      ["Flaky", [], ["t-old"]],
      ["scratch", ["t-scratch"], []],
    ]);
    expect(sections(filter("v2", { "t-scratch": "v2" }))).toEqual([
      ["checkout", ["t-checkout"], []],
      ["scratch", ["t-scratch"], []],
    ]);
    // Without a query, search results do not narrow the list.
    expect(sections(filter("", { "t-scratch": "x" }))).toHaveLength(4);
  });

  // spec: UI-022, UI-026
  test("rows shown skip collapsed sections (unless filtering) and list archived ones only when opened", () => {
    const { workspaces, threads } = index();
    const all = sidebarSections(threads, workspaces, NO_FILTER);
    expect(visibleOrder(all, [], [], NO_FILTER)).toEqual(["t-checkout", "t-flaky", "t-scratch"]);
    expect(visibleOrder(all, ["w-checkout"], ["w-flaky"], NO_FILTER)).toEqual(["t-flaky", "t-old", "t-scratch"]);
    const typed = filter("t");
    expect(visibleOrder(sidebarSections(threads, workspaces, typed), ["w-checkout"], [], typed)).toEqual([
      "t-checkout",
      "t-flaky",
      "t-scratch",
    ]);
  });

  // spec: UI-022
  test("the collapsed set toggles and survives a reload through storage", () => {
    expect(toggled([], "w1")).toEqual(["w1"]);
    expect(toggled(["w1", "w2"], "w1")).toEqual(["w2"]);
    const storage = memoryStorage();
    const collapsed = windowKey<readonly string[]>("threads/collapsed", [], decodeIds);
    new WindowStore(storage).set(collapsed, (previous) => toggled(previous, "w-checkout"));
    expect(new WindowStore(storage).get(collapsed)).toEqual(["w-checkout"]);
    storage.setItem("omfp.threads/collapsed", '{"not":"a list"}');
    expect(new WindowStore(storage).get(collapsed)).toEqual([]);
    expect(decodeIds(["a", 1])).toBeUndefined();
  });

  // spec: UI-022
  test("threads.search is called once typing pauses, not for short queries, and a late older answer is dropped", async () => {
    const timers = manualTimers();
    const asked: string[] = [];
    const answers = new Map<string, (hits: readonly SearchHit[]) => void>();
    const reported: Array<[string, string[]]> = [];
    const search = new ConversationSearch(
      (query) => {
        asked.push(query);
        const { promise, resolve } = Promise.withResolvers<readonly SearchHit[]>();
        answers.set(query, resolve);
        return promise;
      },
      timers.schedule,
      (query, snippets) => reported.push([query, [...snippets.keys()]]),
    );
    search.update("r");
    expect(reported).toEqual([["r", []]]);
    search.update("re");
    search.update("ret");
    expect(timers.pending.map((timer) => timer.ms)).toEqual([SEARCH_DELAY_MS, SEARCH_DELAY_MS]);
    timers.fire();
    expect(asked).toEqual(["ret"]);
    search.update("retr");
    timers.fire();
    expect(asked).toEqual(["ret", "retr"]);
    answers.get("retr")?.([{ threadId: "t2", snippet: "retry" }]);
    answers.get("ret")?.([{ threadId: "t1", snippet: "ret" }]);
    await Promise.resolve();
    await Promise.resolve();
    expect(reported).toEqual([
      ["r", []],
      ["retr", ["t2"]],
    ]);
    search.update("retry");
    timers.fire();
    search.dispose();
    answers.get("retry")?.([{ threadId: "t3", snippet: "retry" }]);
    await Promise.resolve();
    expect(reported).toHaveLength(2);
  });

  /** Workspaces a, b, c (in that order) and an empty d: a1, a2 and the archived a0 in a; b1; c1, c2. */
  function column(typed: ThreadFilter = NO_FILTER) {
    const workspaces = ["a", "b", "c", "d"].map((id, position) => ({ ...workspace(id), position }));
    const threads = [
      thread("a0", "zero", "a", { archivedAt: 3, position: 0 }),
      thread("a1", "one", "a", { position: 1 }),
      thread("a2", "two", "a", { position: 2 }),
      thread("b1", "bee", "b"),
      thread("c1", "one", "c", { position: 1 }),
      thread("c2", "two", "c", { position: 2 }),
    ];
    return sidebarSections(threads, workspaces, typed);
  }

  // spec: UI-989
  test("next and previous thread follow the column across workspaces and wrap around", () => {
    const all = column();
    const next = (current: string) => adjacentThread(all, [], NO_FILTER, current, undefined, 1);
    const prev = (current: string) => adjacentThread(all, [], NO_FILTER, current, undefined, -1);
    expect(["a1", "a2", "b1", "c1", "c2"].map(next)).toEqual(["a2", "b1", "c1", "c2", "a1"]);
    expect(["a1", "a2", "b1", "c1", "c2"].map(prev)).toEqual(["c2", "a1", "a2", "b1", "c1"]);
    // Nothing selected: next starts at the top, previous at the bottom.
    expect(adjacentThread(all, [], NO_FILTER, undefined, undefined, 1)).toBe("a1");
    expect(adjacentThread(all, [], NO_FILTER, undefined, undefined, -1)).toBe("c2");
    // The only thread wraps onto itself; an empty column has none.
    const one = column(filter("bee"));
    expect(adjacentThread(one, [], filter("bee"), "b1", "b", 1)).toBe("b1");
    expect(adjacentThread([], [], NO_FILTER, undefined, undefined, 1)).toBeUndefined();
  });

  // spec: UI-990
  test("next and previous thread skip archived threads, collapsed sections, unmatched threads and drafts", () => {
    const all = column();
    // The archived a0 is never chosen, but an archived selection starts from its place.
    expect(adjacentThread(all, [], NO_FILTER, "a1", "a", -1)).toBe("c2");
    expect(adjacentThread(all, [], NO_FILTER, "a0", "a", 1)).toBe("b1");
    // b collapsed: a2 → c1 and back; a thread of a collapsed section moves out of it.
    expect(adjacentThread(all, ["b"], NO_FILTER, "a2", "a", 1)).toBe("c1");
    expect(adjacentThread(all, ["b"], NO_FILTER, "c1", "c", -1)).toBe("a2");
    expect(adjacentThread(all, ["b"], NO_FILTER, "b1", "b", 1)).toBe("c1");
    expect(adjacentThread(all, ["a", "b", "c"], NO_FILTER, "a1", "a", 1)).toBeUndefined();
    // A draft thread in a sits after a's threads: next is b's first, previous a's last.
    expect(adjacentThread(all, [], NO_FILTER, undefined, "a", 1)).toBe("b1");
    expect(adjacentThread(all, [], NO_FILTER, undefined, "a", -1)).toBe("a2");
    // Filtering: only the matching threads, collapsed sections included (they show while filtering); a selection the
    // filter hides starts from the end of its workspace, or from an end of the column when no thread of it matches.
    const typed = filter("two");
    const matching = column(typed);
    expect(adjacentThread(matching, ["a"], typed, "a2", "a", 1)).toBe("c2");
    expect(adjacentThread(matching, ["a"], typed, "c2", "c", 1)).toBe("a2");
    expect(adjacentThread(matching, [], typed, "a1", "a", 1)).toBe("c2");
    expect(adjacentThread(matching, [], typed, "b1", "b", -1)).toBe("c2");
    expect(adjacentThread(matching, [], typed, "b1", "b", 1)).toBe("a2");
  });

  // spec: UI-021
  test("next needing attention wraps around after the current thread", () => {
    const threads = index().threads.filter((summary) => ["t-checkout", "t-flaky", "t-scratch"].includes(summary.id));
    expect(nextNeedingAttention(threads, undefined)).toBeUndefined();
    const waiting = threads.map((summary) =>
      summary.id === "t-flaky" ? summary : { ...summary, needsAttention: true },
    );
    expect(waitingCount(waiting)).toBe(2);
    expect(nextNeedingAttention(waiting, undefined)).toBe("t-checkout");
    expect(nextNeedingAttention(waiting, "t-checkout")).toBe("t-scratch");
    expect(nextNeedingAttention(waiting, "t-scratch")).toBe("t-checkout");
    const one = waiting.map((summary) =>
      summary.id === "t-scratch" ? { ...summary, needsAttention: false } : summary,
    );
    expect(nextNeedingAttention(one, "t-checkout")).toBe("t-checkout");
  });

  // spec: UI-020
  test("a thread is unread when it changed since it was last seen and is not selected", () => {
    const summary = thread("t", "t", "w", { updatedAt: 10 });
    expect(isUnread(summary, new Map(), undefined)).toBe(false);
    const seen = new Map([["t", 10]]);
    expect(isUnread(summary, seen, undefined)).toBe(false);
    expect(isUnread({ ...summary, updatedAt: 11 }, seen, undefined)).toBe(true);
    expect(isUnread({ ...summary, updatedAt: 11 }, seen, "t")).toBe(false);
  });

  // spec: UI-020
  test("the index sets unread baselines and the selected thread stays seen", () => {
    const first = thread("t1", "one", "w", { updatedAt: 5 });
    const second = thread("t2", "two", "w", { updatedAt: 7 });
    const seen = updateSeen(new Map(), [first, second], undefined);
    expect([...seen]).toEqual([
      ["t1", 5],
      ["t2", 7],
    ]);
    expect(updateSeen(seen, [first, second], undefined)).toBe(seen);
    const changed = [
      { ...first, updatedAt: 9 },
      { ...second, updatedAt: 9 },
    ];
    const selected = updateSeen(seen, changed, "t1");
    expect([...selected]).toEqual([
      ["t1", 9],
      ["t2", 7],
    ]);
  });
});

describe("threads made by jobs", () => {
  const COUNTS = { total: 2, manual: 2, schedule: 0, service: 0, running: 0, failing: 0, error: false };
  const byJob = (job: string) => ({ provenance: { createdBy: { kind: "job" as const, job }, client: "omfp-daemon" } });
  const jobIndex = () => {
    const workspaces = [
      { ...workspace("w-jobs"), name: "jobs", jobs: COUNTS },
      { ...workspace("w-plain"), name: "plain" },
    ];
    const threads = [
      thread("t-own", "own", "w-jobs"),
      thread("t-mail-1", "mail · 07:00", "w-jobs", { ...byJob("mail"), position: 1 }),
      thread("t-mail-old", "mail · yesterday", "w-jobs", { ...byJob("mail"), position: 2, archivedAt: 5 }),
      thread("t-gone", "gone · 06:00", "w-jobs", { ...byJob("gone"), position: 3 }),
      thread("t-moved", "moved", "w-plain", byJob("mail")),
    ];
    return { workspaces, threads };
  };
  const ids = (filter: ThreadFilter, declared?: ReadonlyMap<string, ReadonlySet<string>>) => {
    const { workspaces, threads } = jobIndex();
    return sidebarSections(threads, workspaces, filter, declared).map((section) => [
      section.workspace.name,
      section.threadIds,
      section.archivedIds,
      section.jobThreadIds,
    ]);
  };

  // spec: UI-992, UI-026
  test("a declared job's non-archived threads leave the section's list for its job; archived ones stay under Archived", () => {
    const declared = new Map([["w-jobs", new Set(["mail"])]]);
    expect(ids(NO_FILTER, declared)).toEqual([
      ["jobs", ["t-own", "t-gone"], ["t-mail-old"], ["t-mail-1"]],
      ["plain", ["t-moved"], [], []],
    ]);
    // Before the workspace's jobs are read, every job thread waits under the jobs.
    expect(ids(NO_FILTER)).toEqual([
      ["jobs", ["t-own"], ["t-mail-old"], ["t-mail-1", "t-gone"]],
      ["plain", ["t-moved"], [], []],
    ]);
    const { workspaces, threads } = jobIndex();
    const all = sidebarSections(threads, workspaces, NO_FILTER, declared);
    expect(visibleOrder(all, [], ["w-jobs"], NO_FILTER)).toEqual(["t-own", "t-gone", "t-mail-old", "t-moved"]);
  });

  // spec: UI-992, UI-995, UI-022
  test("the column filter lists a job's matching threads among the section's", () => {
    const declared = new Map([["w-jobs", new Set(["mail", "gone"])]]);
    expect(ids(filter("mail"), declared)).toEqual([["jobs", ["t-mail-1"], ["t-mail-old"], ["t-mail-1", "t-gone"]]]);
  });

  // spec: UI-995, UI-021
  test("next needing attention reaches a job's thread though the column lists it under its job", () => {
    const { threads } = jobIndex();
    const waiting = threads.map((summary) =>
      summary.id === "t-mail-1" ? { ...summary, needsAttention: true } : summary,
    );
    expect(nextNeedingAttention(waiting, "t-own")).toBe("t-mail-1");
  });

  // spec: UI-992, UI-990
  test("next and previous thread skip a job's threads; from one they start at the end of its workspace", () => {
    const { workspaces, threads } = jobIndex();
    const all = sidebarSections(threads, workspaces, NO_FILTER, new Map([["w-jobs", new Set(["mail", "gone"])]]));
    expect(adjacentThread(all, [], NO_FILTER, "t-own", "w-jobs", 1)).toBe("t-moved");
    expect(adjacentThread(all, [], NO_FILTER, "t-moved", "w-plain", -1)).toBe("t-own");
    expect(adjacentThread(all, [], NO_FILTER, "t-mail-1", "w-jobs", 1)).toBe("t-moved");
    expect(adjacentThread(all, [], NO_FILTER, "t-mail-1", "w-jobs", -1)).toBe("t-own");
  });
});

describe("spawned tree (UI-240)", () => {
  const by = (threadId: string) => ({ provenance: { createdBy: { kind: "thread" as const, threadId } } });
  const root = thread("root", "orchestrator", "w1");
  const child = thread("a", "fe", "w1", by("root"));
  const threads = [
    root,
    child,
    thread("b", "be", "w1", { ...by("a"), archivedAt: 4 }),
    thread("c", "docs", "w1", by("root")),
    thread("other", "other", "w1"),
  ];

  // spec: AGT-070, UI-240
  test("a thread's spawned tree follows provenance transitively, archived threads included", () => {
    expect(spawnerOf(child)).toBe("root");
    expect(spawnerOf(root)).toBeUndefined();
    expect([...spawnedTree(threads, "root")].sort()).toEqual(["a", "b", "c", "root"]);
    expect([...spawnedTree(threads, "a")].sort()).toEqual(["a", "b"]);
    expect([...spawnedTree(threads, "other")]).toEqual(["other"]);
    expect(spawnedCounts(threads)).toEqual(
      new Map([
        ["root", 3],
        ["a", 1],
      ]),
    );
  });

  // spec: UI-240, UI-022
  test("the spawned filter keeps its tree, opens collapsed sections and combines with the query", () => {
    const workspaces = [workspace("w1"), workspace("w2")];
    const filter: ThreadFilter = { ...NO_FILTER, spawned: spawnedTree(threads, "a") };
    const sections = sidebarSections(threads, workspaces, filter);
    expect(sections.map((section) => [section.threadIds, section.archivedIds])).toEqual([[["a"], ["b"]]]);
    expect(visibleOrder(sections, ["w1"], [], filter)).toEqual(["a"]);
    const typed: ThreadFilter = { ...filter, query: "be" };
    expect(sidebarSections(threads, workspaces, typed).map((section) => section.archivedIds)).toEqual([["b"]]);
  });
});

describe("a fork's lineage", () => {
  const fork = { forkedFrom: { threadId: "01S", title: "Old title", entryId: "e1", message: "Fix the cart" } };

  // spec: UI-664, UI-665
  test("names the source by its title now, else the one it had, and the message", () => {
    expect(forkLineage(fork, new Map([["01S", { title: "New title" }]]))).toEqual({
      sourceId: "01S",
      sourceTitle: "New title",
      sourceExists: true,
      message: "Fix the cart",
      label: "Forked from New title at \u201cFix the cart\u201d",
    });
    const deleted = forkLineage({ forkedFrom: { threadId: "01S", title: "Old title" } }, new Map());
    expect(deleted?.sourceExists).toBe(false);
    expect(deleted?.label).toBe("Forked from Old title");
    expect(forkLineage({}, new Map())).toBeUndefined();
    expect(forkLineage({ forkedFrom: null }, new Map())).toBeUndefined();
  });
});
