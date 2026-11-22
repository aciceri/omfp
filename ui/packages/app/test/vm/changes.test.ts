/** L0: the view model of `omfp:changes` (UI-080): changed files per repository and when to refresh. */

import { describe, expect, test } from "bun:test";
import type { GitStatus, WorkspaceRepo } from "@omfp/protocol";
import { changeRunsKey, changesSections, changesView } from "../../src/vm/changes.ts";
import { repo, workspace } from "../support/fixtures.ts";
import { fixture, fixtureUntil, isFrame } from "../widgets/chat/fixtures.ts";

const status: GitStatus = {
  branch: "feat",
  baseRef: "origin/feat",
  ahead: 2,
  behind: 1,
  dirty: true,
  files: [
    { path: "src/lib.rs", oldPath: null, status: "modified", additions: 3, deletions: 1 },
    { path: "docs/new.md", oldPath: "docs/old.md", status: "renamed", additions: 0, deletions: 0 },
    { path: "notes.txt", oldPath: null, status: "untracked", additions: 7, deletions: 0 },
  ],
};

describe("changes", () => {
  // spec: UI-080, UI-011
  test("each repository lists its changed files with +/− counts, totals, branch and ahead/behind", () => {
    const linked: WorkspaceRepo = { ...repo("w1", "be", "main"), kind: "link", target: "/src/be" };
    const view = changesView(workspace("w1", [repo("w1", "fe", "old"), linked]), [{ name: "fe", status }]);
    expect(view).toEqual([
      {
        name: "fe",
        isWorkspace: false,
        branch: "feat",
        aheadBehind: "↑2 ↓1",
        summary: "3 files +10 −1",
        clean: false,
        files: [
          { path: "src/lib.rs", from: undefined, status: "modified", additions: 3, deletions: 1 },
          { path: "docs/new.md", from: "docs/old.md", status: "renamed", additions: 0, deletions: 0 },
          { path: "notes.txt", from: undefined, status: "untracked", additions: 7, deletions: 0 },
        ],
        error: undefined,
      },
      // No status yet: the files are unknown, not empty.
      {
        name: "be",
        isWorkspace: false,
        branch: "main",
        aheadBehind: undefined,
        summary: undefined,
        clean: false,
        files: undefined,
        error: undefined,
      },
    ]);
  });

  // spec: UI-080, DMN-383
  test("a repository whose status the daemon could not read carries the error and no changes, not a clean status", () => {
    const clean: GitStatus = { branch: null, ahead: 0, behind: 0, dirty: false, files: [] };
    const view = changesView(workspace("w1", [repo("w1", "fe", "main"), repo("w1", "be", "main")]), [
      { name: "fe", status: clean, error: "git diff failed: fatal: bad object HEAD" },
      { name: "be", status: { ...clean, branch: "main" } },
    ]);
    expect(view.map((row) => [row.name, row.branch, row.summary, row.clean, row.files, row.error])).toEqual([
      ["fe", "main", undefined, false, undefined, "git diff failed: fatal: bad object HEAD"],
      ["be", "main", "clean", true, [], undefined],
    ]);
    // Not clean: it stays listed rather than going to the clean group (UI-342).
    expect(changesSections(view, undefined).listed.map((row) => row.name)).toEqual(["fe"]);
  });

  // spec: UI-341
  test("the workspace folder's own repository is marked as the workspace; the others are not", () => {
    const root: WorkspaceRepo = { ...repo("w1", "geo", "main"), isRoot: true };
    const view = changesView(workspace("w1", [root, repo("w1", "fe", "main")]), undefined);
    expect(view.map((row) => [row.name, row.isWorkspace])).toEqual([
      ["geo", true],
      ["fe", false],
    ]);
  });

  // spec: UI-080
  test("a clean repository says so; a detached HEAD has no branch", () => {
    const clean: GitStatus = { branch: null, ahead: 0, behind: 0, dirty: false, files: [] };
    const [row] = changesView(workspace("w1", [repo("w1", "fe", null)]), [{ name: "fe", status: clean }]);
    expect([row?.branch, row?.aheadBehind, row?.summary, row?.clean, row?.files]).toEqual([
      "detached",
      undefined,
      "clean",
      true,
      [],
    ]);
  });

  // spec: UI-342, UI-341
  test("the clean repositories go to one group at the bottom; the workspace folder's own stays first, clean or not", () => {
    const clean: GitStatus = { branch: "main", ahead: 0, behind: 0, dirty: false, files: [] };
    const ahead: GitStatus = { ...clean, ahead: 3 };
    const repos = [repo("w1", "api", "main"), repo("w1", "fe", "main"), repo("w1", "be", "main")];
    // The workspace folder's own repository comes last in the workspace's order here: it is still listed first.
    const root: WorkspaceRepo = { ...repo("w1", "geo", "main"), isRoot: true };
    const view = changesView(workspace("w1", [...repos, root, repo("w1", "docs", "main")]), [
      { name: "api", status: clean },
      { name: "fe", status },
      { name: "be", status: ahead },
      { name: "geo", status: clean },
    ]);
    const sections = changesSections(view, undefined);
    // `docs` has no status yet: listed (reading), not counted as clean.
    expect([sections.listed.map((row) => row.name), sections.clean.map((row) => row.name)]).toEqual([
      ["geo", "fe", "docs"],
      ["api", "be"],
    ]);
    expect(sections.clean.map((row) => row.aheadBehind)).toEqual([undefined, "↑3 ↓0"]);
  });

  // spec: UI-342, UI-080
  test("with a repo prop only that repository is listed, clean or not, and nothing is grouped", () => {
    const clean: GitStatus = { branch: "main", ahead: 0, behind: 0, dirty: false, files: [] };
    const view = changesView(workspace("w1", [repo("w1", "fe", "main"), repo("w1", "be", "main")]), [
      { name: "fe", status },
      { name: "be", status: clean },
    ]);
    const sections = changesSections(view, "be");
    expect([sections.listed.map((row) => row.name), sections.clean.length]).toEqual([["be"], 0]);
    expect(changesSections(view, "gone")).toEqual({ listed: [], clean: [] });
  });

  // spec: UI-342
  test("every repository clean and no workspace folder repository: only the group", () => {
    const clean: GitStatus = { branch: "main", ahead: 0, behind: 0, dirty: false, files: [] };
    const view = changesView(workspace("w1", [repo("w1", "fe", "main"), repo("w1", "be", "main")]), [
      { name: "fe", status: clean },
      { name: "be", status: clean },
    ]);
    const sections = changesSections(view, undefined);
    expect([sections.listed.length, sections.clean.map((row) => row.name)]).toEqual([0, ["fe", "be"]]);
  });

  // spec: UI-080
  test("the refresh key changes when an edit, write or bash run ends, not while it runs", () => {
    const running = fixtureUntil("bash_tool_call", isFrame("tool_execution_end"));
    expect(running.tools.map((run) => run.status)).toEqual(["running"]);
    expect(changeRunsKey(running.tools)).toBe("");
    expect(changeRunsKey(fixture("bash_tool_call").tools)).toBe("call_bash_1");
    expect(changeRunsKey(undefined)).toBe("");
    // Reads never refresh the changes; failed edits do (a partial edit may have happened).
    expect(changeRunsKey(fixture("read_tool_calls").tools)).toBe("");
    expect(changeRunsKey(fixture("edit_tool_calls").tools)).toBe("call_edit_1 call_edit_2");
    expect(changeRunsKey(fixture("write_tool_calls").tools)).toBe("call_write_1 call_write_2");
  });
});
