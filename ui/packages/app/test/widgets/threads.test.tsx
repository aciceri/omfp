/** L4: `omfp:threads`, the left column, over a fake client (with the frame's workspace delete dialog). */

import { afterEach, describe, expect, test } from "bun:test";
import type { IndexSnapshot } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { DeleteWorkspaceDialog } from "../../src/frame/dialogs.tsx";
import { DELETE_WORKSPACE, DRAFT_MESSAGE, DRAFT_THREAD, draftKey, SELECTED_THREAD } from "../../src/sdk/index.ts";
import {
  ARCHIVED_OPEN,
  COLLAPSED_WORKSPACES,
  JOBS_OPEN,
  SPAWNED_BY,
  ThreadsWidget,
} from "../../src/widgets/threads/index.ts";
import { repo, thread, workspace } from "../support/fixtures.ts";
import { renderWithHost } from "../support/host.tsx";

afterEach(cleanup);

const INDEX: Partial<IndexSnapshot> = {
  workspaces: [
    { ...workspace("w1", [repo("w1", "fe", "main")]), name: "checkout" },
    { ...workspace("w2"), name: "scratch", path: "/home/me/scratch" },
  ],
  threads: [
    thread("t1", "checkout-v2", "w1", { updatedAt: 5 }),
    thread("t2", "", "w2", { needsAttention: true, updatedAt: 5 }),
    thread("t3", "old", "w2", { archivedAt: 3 }),
  ],
};

/** The column over a fake client, connected, with `index` (`null`: not arrived yet); `before` renders ahead of it. */
function setup(index: Partial<IndexSnapshot> | null = INDEX, before: ReactNode = null) {
  const test = renderWithHost(
    <>
      {before}
      <ThreadsWidget />
      <DeleteWorkspaceDialog />
    </>,
  );
  for (const method of [
    "threads.archive",
    "threads.delete",
    "threads.unarchive",
    "workspaces.remove",
    "workspaces.update",
  ] as const) {
    test.client.answers.set(method, () => ({}));
  }
  test.client.answers.set("fs.complete", () => ({ entries: [] }));
  test.client.answers.set("threads.search", () => ({ results: [] }));
  test.client.connected();
  if (index !== null) {
    test.client.index(index);
  }
  test.flush();
  return { ...test, user: userEvent.setup() };
}

function rows(): string[] {
  return screen.queryAllByTestId("thread-row").map((row) => row.dataset.threadId ?? "");
}

function section(workspaceId: string): HTMLElement {
  const found = screen
    .getAllByTestId("workspace-section")
    .find((candidate) => candidate.dataset.workspaceId === workspaceId);
  if (found === undefined) {
    throw new Error(`no section ${workspaceId}`);
  }
  return found;
}

function sectionIds(): string[] {
  return screen.queryAllByTestId("workspace-section").map((candidate) => candidate.dataset.workspaceId ?? "");
}

describe("omfp:threads", () => {
  // spec: UI-020
  test("each workspace is a section with its threads (state badge, title), and no grouping, filter keys or settings", () => {
    setup();
    expect(sectionIds()).toEqual(["w1", "w2"]);
    expect(within(section("w1")).getByTestId("workspace-section-name").textContent).toBe("checkout");
    expect(rows()).toEqual(["t1", "t2"]);
    const [first, second] = screen.getAllByTestId("thread-row");
    expect(first?.dataset.status).toBe("idle");
    expect(within(first ?? document.body).getByTestId("thread-status")).toBeDefined();
    expect(second?.dataset.status).toBe("awaiting");
    expect(second?.textContent).toContain("New thread");
    expect(second?.dataset.titleSource).toBe("placeholder");
    for (const gone of ["grouping", "filter-key", "next-attention", "theme-mode", "density-mode", "workspace-chip"]) {
      expect(screen.queryByTestId(gone)).toBeNull();
    }
  });

  // spec: UI-339
  test("a section header shows the workspace's name; its path only as the name's tooltip and description", () => {
    setup();
    const name = within(section("w2")).getByTestId("workspace-section-name");
    expect(name.textContent).toBe("scratch");
    expect([name.getAttribute("title"), name.getAttribute("aria-description")]).toEqual([
      "/home/me/scratch",
      "/home/me/scratch",
    ]);
    expect(section("w2").textContent).not.toContain("/home/me/scratch");
  });

  // spec: UI-020
  test("selecting a row highlights it and its workspace and marks it seen; a change elsewhere marks the other unread", async () => {
    const { client, flush, user } = setup();
    await user.click(screen.getAllByTestId("thread-row")[0] ?? document.body);
    expect(screen.getAllByTestId("thread-row")[0]?.getAttribute("aria-current")).toBe("true");
    expect(section("w1").dataset.current).toBe("true");
    expect(section("w2").dataset.current).toBeUndefined();
    client.indexEvents(
      { kind: "thread_upserted", thread: thread("t1", "checkout-v2", "w1", { updatedAt: 9 }) },
      { kind: "thread_upserted", thread: thread("t2", "", "w2", { needsAttention: true, updatedAt: 9 }) },
    );
    flush();
    const [first, second] = screen.getAllByTestId("thread-row");
    expect(within(first ?? document.body).queryByTestId("unread")).toBeNull();
    expect(within(second ?? document.body).getByTestId("unread")).toBeDefined();
    await user.click(second ?? document.body);
    expect(within(second ?? document.body).queryByTestId("unread")).toBeNull();
  });

  // spec: UI-122, UI-169, UI-170
  test("rows show the state mark, the selected bar and the attention flash; the current workspace's name flows", async () => {
    const { client, flush, user } = setup();
    const item = (index: number) => screen.getAllByTestId("thread-row")[index]?.closest("li") ?? document.body;
    expect([item(0).dataset.state, item(1).dataset.state]).toEqual(["idle", "awaiting"]);
    expect(within(item(1)).getByTestId("thread-status").getAttribute("aria-label")).toBe("awaiting input");
    expect(within(section("w1")).getByTestId("workspace-collapse").textContent).toBe("▾");
    await user.click(screen.getAllByTestId("thread-row")[0] ?? document.body);
    expect(item(0).dataset.selected).toBe("true");
    expect(within(section("w1")).getByTestId("workspace-section-name").classList).toContain("omfp-grad-text");
    expect(within(section("w2")).getByTestId("workspace-section-name").classList).not.toContain("omfp-grad-text");
    // Already awaiting when first shown: no flash; turning failed or awaiting while shown flashes.
    expect([item(0).dataset.fresh, item(1).dataset.fresh]).toEqual([undefined, undefined]);
    client.indexEvents({ kind: "thread_upserted", thread: thread("t1", "checkout", "w1", { needsAttention: true }) });
    flush();
    expect([item(0).dataset.state, item(0).dataset.fresh]).toEqual(["awaiting", "true"]);
  });

  // spec: UI-022
  test("the filter matches title, workspace and repository names, hiding workspaces without a match", async () => {
    const { user } = setup();
    await user.type(screen.getByTestId("thread-filter"), "FE");
    expect(sectionIds()).toEqual(["w1"]);
    expect(rows()).toEqual(["t1"]);
    await user.clear(screen.getByTestId("thread-filter"));
    await user.type(screen.getByTestId("thread-filter"), "scratch");
    expect(sectionIds()).toEqual(["w2"]);
    expect(rows()).toEqual(["t2"]);
    await user.clear(screen.getByTestId("thread-filter"));
    await user.type(screen.getByTestId("thread-filter"), "nothing");
    expect(sectionIds()).toEqual([]);
    expect(screen.getByTestId("no-threads")).toBeDefined();
  });

  // spec: UI-022
  test("conversation matches come from threads.search once typing pauses, with their snippet", async () => {
    const { client, user } = setup();
    client.answers.set("threads.search", () => ({ results: [{ threadId: "t2", snippet: "…the retry loop…" }] }));
    await user.type(screen.getByTestId("thread-filter"), "retry");
    const snippet = await screen.findByTestId("thread-snippet");
    expect(snippet.textContent).toBe("…the retry loop…");
    expect(rows()).toEqual(["t2"]);
    expect(sectionIds()).toEqual(["w2"]);
    expect(client.callsTo("threads.search").map((call) => call.params)).toEqual([{ query: "retry" }]);
  });

  // spec: UI-022
  test("a section collapses with its chevron or `za`; the collapsed set is kept in window state", async () => {
    const { host, user } = setup();
    const chevron = within(section("w1")).getByTestId("workspace-collapse");
    expect(chevron.getAttribute("aria-expanded")).toBe("true");
    await user.click(chevron);
    expect(rows()).toEqual(["t2"]);
    expect(within(section("w1")).queryByTestId("add-thread")).toBeNull();
    expect(host.window.get(COLLAPSED_WORKSPACES)).toEqual(["w1"]);
    expect(within(section("w1")).getByTestId("workspace-collapse").getAttribute("aria-expanded")).toBe("false");
    act(() => within(section("w2")).getByTestId("workspace-menu").focus());
    act(() => {
      host.commands.run("nav.toggleSection", [], ["threads"]);
    });
    expect(host.window.get(COLLAPSED_WORKSPACES)).toEqual(["w1", "w2"]);
    expect(rows()).toEqual([]);
    // While filtering, collapsed sections show their matches.
    await user.type(screen.getByTestId("thread-filter"), "checkout");
    expect(rows()).toEqual(["t1"]);
  });

  // spec: UI-026
  test("archived threads sit behind a collapsed Archived (N) row: reopen, unarchive or delete them there", async () => {
    const { client, host, user } = setup();
    expect(within(section("w1")).queryByTestId("archived-toggle")).toBeNull();
    const toggle = within(section("w2")).getByTestId("archived-toggle");
    expect(toggle.querySelector(".omfp-row__title")?.textContent).toBe("Archived (1)");
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(rows()).toEqual(["t1", "t2"]);
    await user.click(toggle);
    expect(rows()).toEqual(["t1", "t2", "t3"]);
    const archived = within(screen.getByTestId("archived-list"));
    expect(archived.getByTestId("thread-row").dataset.archived).toBe("true");
    await user.click(archived.getByTestId("thread-row"));
    expect(host.window.get(SELECTED_THREAD)).toBe("t3");
    await user.click(archived.getByTestId("unarchive-thread"));
    expect(client.callsTo("threads.unarchive").map((call) => call.params)).toEqual([{ threadId: "t3" }]);
    await user.click(archived.getByTestId("row-delete-thread"));
    expect(client.callsTo("threads.delete").map((call) => call.params)).toEqual([{ threadId: "t3" }]);
  });

  // spec: UI-020, UI-026, UI-740
  test("a section's rows: threads, the draft, + thread, then Jobs (N) and Archived (N), collapsed and toggled apart", async () => {
    const jobs = { total: 2, manual: 2, schedule: 0, service: 0, running: 0, failing: 0, error: false };
    const { host, user } = setup({
      ...INDEX,
      workspaces: (INDEX.workspaces ?? []).map((candidate) =>
        candidate.id === "w2" ? { ...candidate, jobs } : candidate,
      ),
    });
    act(() => host.window.set(DRAFT_THREAD, { workspaceId: "w2" }));
    const order = () =>
      [
        ...section("w2").querySelectorAll(
          "[data-testid=thread-row], [data-testid=draft-thread], [data-testid=add-thread], [data-testid=jobs-toggle], [data-testid=archived-toggle]",
        ),
      ].map((element) => element.getAttribute("data-testid"));
    expect(order()).toEqual(["thread-row", "draft-thread", "add-thread", "jobs-toggle", "archived-toggle"]);
    expect(within(section("w1")).queryByTestId("jobs-toggle")).toBeNull();

    const jobsToggle = within(section("w2")).getByTestId("jobs-toggle");
    const archivedToggle = within(section("w2")).getByTestId("archived-toggle");
    // Alike rows: the ▸/▾ glyph in place of an icon, collapsed by default.
    for (const toggle of [jobsToggle, archivedToggle]) {
      expect(toggle.classList.contains("omfp-toggle-row")).toBe(true);
      expect(toggle.querySelector(".omfp-ic")).toBeNull();
      expect([
        toggle.querySelector(".omfp-toggle-row__glyph")?.textContent,
        toggle.getAttribute("aria-expanded"),
      ]).toEqual(["▸", "false"]);
    }
    expect(jobsToggle.querySelector(".omfp-row__title")?.textContent).toBe("Jobs (2)");
    // Nothing runs, failed or is refused: no marks, the tooltip still spells the jobs out.
    expect(within(jobsToggle).queryByTestId("job-counts")).toBeNull();
    expect(jobsToggle.querySelector(".omfp-row__title [title]")?.getAttribute("title")).toBe("2 jobs: 2 manual");

    await user.click(jobsToggle);
    expect([host.window.get(JOBS_OPEN), host.window.get(ARCHIVED_OPEN)]).toEqual([["w2"], []]);
    expect([jobsToggle.getAttribute("aria-expanded"), archivedToggle.getAttribute("aria-expanded")]).toEqual([
      "true",
      "false",
    ]);
    expect(jobsToggle.querySelector(".omfp-toggle-row__glyph")?.textContent).toBe("▾");
    expect(within(section("w2")).getAllByTestId("jobs-list").length).toBe(1);
    expect(screen.queryAllByTestId("archived-list").length).toBe(0);
    await user.click(archivedToggle);
    await user.click(jobsToggle);
    expect([host.window.get(JOBS_OPEN), host.window.get(ARCHIVED_OPEN)]).toEqual([[], ["w2"]]);
    expect(screen.queryAllByTestId("jobs-list").length).toBe(0);
    // j/k still move through threads only.
    await user.click(jobsToggle);
    act(() => {
      host.commands.run("nav.bottom", [], ["threads"]);
    });
    expect(host.window.get(SELECTED_THREAD)).toBe("t3");
  });

  // spec: UI-020, UI-442
  test("a section's head: no badges after the name, the count last, the menu and delete after it to take its place", () => {
    const jobs = { total: 1, manual: 1, schedule: 0, service: 0, running: 1, failing: 0, error: false };
    setup({ ...INDEX, workspaces: (INDEX.workspaces ?? []).map((candidate) => ({ ...candidate, jobs })) });
    const head = section("w1").querySelector(".omfp-wsection__head");
    expect([...(head?.children ?? [])].map((child) => child.classList[0])).toEqual([
      "omfp-wsection__toggle",
      "omfp-wsection__name",
      "omfp-wsection__count",
      "omfp-wsection__actions",
    ]);
    expect(head?.querySelector(".omfp-wsection__count")?.textContent).toBe("1");
    expect(
      [...(head?.querySelectorAll(".omfp-wsection__actions [data-testid]") ?? [])].map((button) =>
        button.getAttribute("data-testid"),
      ),
    ).toEqual(["workspace-menu", "delete-workspace"]);
    expect(head?.querySelector("[data-testid=job-counts]")).toBeNull();
    expect(within(section("w1")).getAllByTestId("job-counts").length).toBe(1);
  });

  // spec: UI-023
  test("+ thread opens a draft thread in its workspace at once; leaving it empty discards it", async () => {
    const { client, host, user } = setup();
    await user.click(screen.getAllByTestId("thread-row")[0] ?? document.body);
    await user.click(within(section("w2")).getByTestId("add-thread"));
    expect(host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "w2" });
    expect(host.window.get(SELECTED_THREAD)).toBeUndefined();
    expect(section("w2").dataset.current).toBe("true");
    const draft = within(section("w2")).getByTestId("draft-thread");
    expect(draft.getAttribute("aria-current")).toBe("true");
    expect(client.callsTo("threads.create")).toHaveLength(0);
    await user.click(screen.getAllByTestId("thread-row")[0] ?? document.body);
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
    expect(screen.queryByTestId("draft-thread")).toBeNull();
    // A draft with text stays listed while another thread is shown, and reopens from its row.
    await user.click(within(section("w1")).getByTestId("add-thread"));
    act(() => host.window.set(DRAFT_MESSAGE, { text: "fix it", images: [], pastes: [] }));
    await user.click(screen.getAllByTestId("thread-row")[1] ?? document.body);
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    await user.click(within(section("w1")).getByTestId("draft-thread"));
    expect(host.window.get(SELECTED_THREAD)).toBeUndefined();
    expect(host.window.get(DRAFT_THREAD)?.workspaceId).toBe("w1");
  });

  // spec: UI-023
  test("+ workspace takes a name, or a path (/ or ~) completed on the daemon host that it links; Enter opens a draft", async () => {
    const { client, host, user } = setup();
    client.answers.set("workspaces.create", (params) => ({
      ...workspace("w9"),
      name: typeof params === "object" && params !== null && "name" in params ? String(params.name) : "workspace",
    }));
    client.answers.set("fs.complete", () => ({ entries: [{ name: "app", path: "/srv/app", isGitRoot: true }] }));
    const field = screen.getByTestId("new-workspace-input");
    await user.type(field, "billing{Enter}");
    expect(client.callsTo("workspaces.create").map((call) => call.params)).toEqual([{ name: "billing" }]);
    await waitFor(() => expect(host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "w9" }));
    expect(field).toHaveProperty("value", "");
    expect(screen.getByTestId("new-workspace").dataset.mode).toBe("name");
    expect(screen.queryByTestId("new-workspace-existing")).toBeNull();
    act(() => host.window.set(DRAFT_THREAD, undefined));
    await user.type(screen.getByTestId("new-workspace-input"), "/srv/a");
    expect(screen.getByTestId("new-workspace").dataset.mode).toBe("path");
    const option = await screen.findByTestId("path-option");
    expect(option.dataset.path).toBe("/srv/app");
    expect(client.callsTo("fs.complete").at(-1)?.params).toEqual({ path: "/srv/a" });
    await user.keyboard("{ArrowDown}{Enter}");
    expect(screen.getByTestId("new-workspace-input")).toHaveProperty("value", "/srv/app");
    await user.keyboard("{Enter}");
    expect(client.callsTo("workspaces.create").map((call) => call.params)).toEqual([
      { name: "billing" },
      { link: "/srv/app" },
    ]);
    await waitFor(() => expect(host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "w9" }));
    expect(screen.getByTestId("new-workspace").dataset.mode).toBe("name");
  });

  // spec: UI-023
  test("a failed workspace creation shows the error and keeps the name", async () => {
    const { client, user } = setup();
    await user.type(screen.getByTestId("new-workspace-input"), "dup{Enter}");
    act(() => client.callsTo("workspaces.create")[0]?.reject(new Error("name taken")));
    expect((await screen.findByTestId("new-workspace-error")).textContent).toBe("name taken");
    expect(screen.getByTestId("new-workspace-input")).toHaveProperty("value", "dup");
  });

  // spec: UI-023
  test("with no workspace yet, the column shows only the focused + workspace field and a hint", async () => {
    setup({ workspaces: [], threads: [] });
    expect(screen.getByTestId("no-workspaces-hint").textContent).toContain("No workspace yet");
    expect(screen.queryByTestId("thread-filter")).toBeNull();
    expect(screen.queryByTestId("workspace-section")).toBeNull();
    await waitFor(() => expect(document.activeElement === screen.getByTestId("new-workspace-input")).toBe(true));
  });

  // spec: UI-023, UI-092
  test("the column is busy until the index arrives; the + workspace field then takes only a focus nobody has", () => {
    const { client, flush } = setup(null, <input data-testid="elsewhere" />);
    expect(screen.getByTestId("sidebar").getAttribute("aria-busy")).toBe("true");
    act(() => screen.getByTestId("elsewhere").focus());
    client.index({ workspaces: [], threads: [] });
    flush();
    expect(screen.getByTestId("sidebar").getAttribute("aria-busy")).toBe("false");
    expect(screen.getByTestId("no-workspaces-hint")).toBeDefined();
    expect(document.activeElement?.getAttribute("data-testid")).toBe("elsewhere");
  });

  // spec: UI-020
  test("the section menu renames the workspace in place and opens its removal", async () => {
    const { client, host, user } = setup();
    await user.click(within(section("w1")).getByTestId("workspace-menu"));
    await user.click(await screen.findByTestId("workspace-rename"));
    const input = await screen.findByTestId("workspace-name-input");
    await user.clear(input);
    await user.type(input, "checkout-2{Enter}");
    expect(client.callsTo("workspaces.update").map((call) => call.params)).toEqual([
      { workspaceId: "w1", name: "checkout-2" },
    ]);
    await user.click(within(section("w2")).getByTestId("workspace-menu"));
    await user.click(await screen.findByTestId("workspace-remove"));
    expect(host.window.get(DELETE_WORKSPACE)).toBe("w2");
  });

  // spec: UI-020, UI-026
  test("a live row's close button immediately archives the thread", async () => {
    const { client, host, user } = setup();
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    const row = within(section("w1")).getByTestId("row-archive-thread");
    await user.click(row);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(client.callsTo("threads.archive").map((call) => call.params)).toEqual([{ threadId: "t1" }]);
    expect(client.callsTo("threads.delete")).toHaveLength(0);
    expect(host.window.get(SELECTED_THREAD)).toBeUndefined();
  });

  // spec: UI-026
  test.each(["t1", "t2"])("middle-click deletes %s without selecting it and clears only its draft", async (id) => {
    const { client, host, user } = setup();
    act(() => {
      host.window.set(SELECTED_THREAD, "t1");
      host.window.set(draftKey("t1"), { text: "first draft", images: [], pastes: [] });
      host.window.set(draftKey("t2"), { text: "second draft", images: [], pastes: [] });
    });
    const selection: (string | undefined)[] = [];
    const unsubscribe = host.window.subscribe(SELECTED_THREAD, () => selection.push(host.window.get(SELECTED_THREAD)));
    const target = screen.getAllByTestId("thread-row").find((row) => row.dataset.threadId === id);
    if (target === undefined) {
      throw new Error(`missing thread ${id}`);
    }
    await user.pointer({ target, keys: "[MouseMiddle]" });
    unsubscribe();
    expect(client.callsTo("threads.delete").map((call) => call.params)).toEqual([{ threadId: id }]);
    expect(host.window.get(SELECTED_THREAD)).toBe(id === "t1" ? undefined : "t1");
    expect(selection).toEqual(id === "t1" ? [undefined] : []);
    expect(host.window.get(draftKey(id)).text).toBe("");
    expect(host.window.get(draftKey(id === "t1" ? "t2" : "t1")).text).toBe(
      id === "t1" ? "second draft" : "first draft",
    );
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(client.callsTo("thread.abort")).toHaveLength(0);
    expect(client.callsTo("thread.stop")).toHaveLength(0);
    expect(client.callsTo("threads.archive")).toHaveLength(0);
  });

  // spec: UI-026
  test("only middle-click is handled: its press prevents autoscroll, its release deletes once", async () => {
    const { client, host, user } = setup();
    const [first, second] = screen.getAllByTestId("thread-row");
    if (first === undefined || second === undefined) {
      throw new Error("missing threads");
    }
    await user.click(first);
    await user.pointer({ target: second, keys: "[MouseRight]" });
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
    expect(client.callsTo("threads.delete")).toHaveLength(0);
    for (const button of [0, 2]) {
      expect(fireEvent.mouseDown(second, { button, cancelable: true })).toBe(true);
      expect(fireEvent(second, new MouseEvent("auxclick", { button, bubbles: true, cancelable: true }))).toBe(true);
    }
    const title = second.querySelector(".omfp-row__title") ?? second;
    expect(fireEvent.mouseDown(title, { button: 1, cancelable: true })).toBe(false);
    expect(client.callsTo("threads.delete")).toHaveLength(0);
    expect(fireEvent(title, new MouseEvent("auxclick", { button: 1, bubbles: true, cancelable: true }))).toBe(false);
    expect(client.callsTo("threads.delete").map((call) => call.params)).toEqual([{ threadId: "t2" }]);
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
  });

  // spec: UI-020, DMN-028
  test("deleting a workspace without threads asks, saying its folder goes and linked projects stay", async () => {
    const { client, host, user } = setup({
      ...INDEX,
      threads: [thread("t2", "", "w2")],
    });
    act(() => host.window.set(DRAFT_THREAD, { workspaceId: "w1" }));
    await user.click(within(section("w1")).getByTestId("delete-workspace"));
    const dialog = await screen.findByTestId("delete-workspace-dialog");
    expect(within(dialog).queryByTestId("delete-workspace-threads")).toBeNull();
    expect(within(dialog).queryByTestId("delete-workspace-folder")).toBeNull();
    const message = within(dialog).getByTestId("delete-workspace-message").textContent ?? "";
    expect(message).toContain("/work/w1 is deleted");
    expect(message).toContain("linked projects are kept");
    await user.click(within(dialog).getByTestId("confirm-delete-workspace"));
    await waitFor(() =>
      expect(client.callsTo("workspaces.remove").map((call) => call.params)).toEqual([{ workspaceId: "w1" }]),
    );
    expect(client.callsTo("threads.delete")).toHaveLength(0);
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
  });

  // spec: UI-020, DMN-028
  test("deleting a workspace with threads lists them and deletes them first once asked to", async () => {
    const { client, host, user } = setup();
    act(() => within(section("w2")).getByTestId("workspace-collapse").focus());
    act(() => {
      host.commands.run("nav.delete", [], ["threads"]);
    });
    const dialog = await screen.findByTestId("delete-workspace-dialog");
    const listed = within(within(dialog).getByTestId("delete-workspace-thread-list")).getAllByRole("listitem");
    expect(listed.map((item) => item.dataset.threadId)).toEqual(["t2", "t3"]);
    const confirm = within(dialog).getByTestId("confirm-delete-workspace");
    expect(confirm.hasAttribute("disabled") || confirm.getAttribute("aria-disabled") === "true").toBe(true);
    await user.click(within(dialog).getByTestId("delete-workspace-threads"));
    await user.click(within(dialog).getByTestId("confirm-delete-workspace"));
    await waitFor(() => expect(client.callsTo("workspaces.remove")).toHaveLength(1));
    expect(client.calls.map((call) => [call.method, call.params])).toEqual([
      ["threads.delete", { threadId: "t2", jobs: "kill" }],
      ["threads.delete", { threadId: "t3", jobs: "kill" }],
      ["workspaces.remove", { workspaceId: "w2" }],
    ]);
  });

  // spec: UI-020
  test("`dd` on a focused row immediately deletes that thread without selecting it", () => {
    const { client, host } = setup();
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    act(() => screen.getAllByTestId("thread-row")[1]?.focus());
    act(() => {
      host.commands.run("nav.delete", [], ["threads"]);
    });
    expect(client.callsTo("threads.delete").map((call) => call.params)).toEqual([{ threadId: "t2" }]);
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
  });

  // spec: UI-287
  test("each workspace's name is an h2, under the thread title's h1", () => {
    setup();
    expect(screen.getAllByRole("heading").map((heading) => [heading.tagName, heading.textContent])).toEqual([
      ["H2", "checkout"],
      ["H2", "scratch"],
    ]);
  });

  /** The focused element's test id and its thread or workspace id. */
  function focusedItem(): string {
    const element = document.activeElement as HTMLElement | null;
    const testId = element?.getAttribute("data-testid") ?? element?.tagName ?? "none";
    const owner =
      element?.dataset.threadId ?? element?.closest<HTMLElement>("[data-workspace-id]")?.dataset.workspaceId;
    return owner === undefined ? testId : `${testId} ${owner}`;
  }

  function row(threadId: string): HTMLElement {
    const found = screen.getAllByTestId("thread-row").find((candidate) => candidate.dataset.threadId === threadId);
    if (found === undefined) {
      throw new Error(`no row ${threadId}`);
    }
    return found;
  }

  // spec: UI-092, UI-285
  test("when the focused thread leaves its list, the focus goes to the next row, else the previous one, else + thread", () => {
    const [a, b, c] = [
      thread("a", "alpha", "w1", { updatedAt: 9 }),
      thread("b", "beta", "w1", { updatedAt: 8 }),
      thread("c", "gamma", "w1", { updatedAt: 7 }),
    ];
    const { client, flush } = setup({ ...INDEX, threads: [a, b, c] });
    expect(rows()).toEqual(["a", "b", "c"]);
    // Archived: the next row.
    act(() => row("b").focus());
    client.indexEvents({ kind: "thread_upserted", thread: { ...b, archivedAt: 10 } });
    flush();
    expect(focusedItem()).toBe("thread-row c");
    // Deleted, the last one: the previous row.
    client.indexEvents({ kind: "thread_removed", threadId: "c" });
    flush();
    expect(focusedItem()).toBe("thread-row a");
    // The workspace's last thread: its + thread.
    client.indexEvents({ kind: "thread_removed", threadId: "a" });
    flush();
    expect(focusedItem()).toBe("add-thread w1");
  });

  // spec: UI-092, UI-285
  test("a thread removed while the focus is elsewhere leaves the focus where it is", async () => {
    const { client, flush, user } = setup();
    await user.click(screen.getByTestId("thread-filter"));
    client.indexEvents({ kind: "thread_removed", threadId: "t1" });
    flush();
    expect(focusedItem()).toBe("thread-filter");
  });

  // spec: UI-092, UI-285
  test("when the focused workspace goes, the focus goes to the next workspace head, else the previous, else + workspace", () => {
    // By name: checkout (w1), extra (w3), scratch (w2).
    const extra = { ...workspace("w3"), name: "extra", path: "/home/me/extra" };
    const { client, flush } = setup({ ...INDEX, workspaces: [...(INDEX.workspaces ?? []), extra] });
    act(() => within(section("w1")).getByTestId("delete-workspace").focus());
    client.indexEvents({ kind: "thread_removed", threadId: "t1" }, { kind: "workspace_removed", workspaceId: "w1" });
    flush();
    expect(focusedItem()).toBe("workspace-collapse w3");
    act(() => within(section("w2")).getByTestId("workspace-menu").focus());
    client.indexEvents(
      { kind: "thread_removed", threadId: "t2" },
      { kind: "thread_removed", threadId: "t3" },
      { kind: "workspace_removed", workspaceId: "w2" },
    );
    flush();
    expect(focusedItem()).toBe("workspace-collapse w3");
    client.indexEvents({ kind: "workspace_removed", workspaceId: "w3" });
    flush();
    expect(focusedItem()).toBe("new-workspace-input");
  });

  // spec: UI-092, UI-286
  test("renaming a workspace with Enter leaves the focus on its head", async () => {
    const { user } = setup();
    await user.click(within(section("w1")).getByTestId("workspace-menu"));
    await user.click(await screen.findByTestId("workspace-rename"));
    const input = await screen.findByTestId("workspace-name-input");
    await user.clear(input);
    await user.type(input, "checkout-2{Enter}");
    await waitFor(() => expect(focusedItem()).toBe("workspace-collapse w1"));
  });

  // spec: UI-095, UI-022, UI-023
  test("nav commands move through visible rows and Space w focuses the + workspace field", () => {
    const { host } = setup();
    const run = (name: string, scopes: string[] = []) =>
      act(() => {
        host.commands.run(name, [], scopes);
      });
    run("nav.next", ["threads"]);
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
    run("nav.next");
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    run("nav.top");
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
    run("nav.bottom", ["threads"]);
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    run("nav.halfUp");
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
    run("workspace.new");
    expect(document.activeElement === screen.getByTestId("new-workspace-input")).toBe(true);
  });

  // spec: UI-092
  test("every control and row is reachable with Tab", async () => {
    const jobs = { total: 1, manual: 1, schedule: 0, service: 0, running: 0, failing: 0, error: false };
    const { user } = setup({
      ...INDEX,
      workspaces: (INDEX.workspaces ?? []).map((candidate) =>
        candidate.id === "w1" ? { ...candidate, jobs } : candidate,
      ),
    });
    const reached = new Set<string>();
    for (let step = 0; step < 24; step += 1) {
      await user.tab();
      const testid = document.activeElement?.closest("[data-testid]")?.getAttribute("data-testid");
      if (testid !== undefined && testid !== null) {
        reached.add(testid);
      }
    }
    for (const testid of [
      "thread-filter",
      "new-workspace-input",
      "workspace-collapse",
      "workspace-menu",
      "delete-workspace",
      "thread-row",
      "row-archive-thread",
      "archived-toggle",
      "add-thread",
      "jobs-toggle",
    ]) {
      expect(reached).toContain(testid);
    }
  });

  // spec: UI-240, AGT-070
  test("a thread that spawned threads filters the column to its spawned tree; the bar names it and clears it", async () => {
    const by = (threadId: string) => ({ provenance: { createdBy: { kind: "thread" as const, threadId } } });
    const workspaces = INDEX.workspaces ?? [];
    const threads = [
      thread("t1", "orchestrator", "w1"),
      thread("t2", "fe#412", "w2", by("t1")),
      thread("t3", "be#88", "w2", by("t2")),
      thread("t4", "unrelated", "w2"),
    ];
    const { client, flush, host, user } = setup({ workspaces, threads });
    const filter = (threadId: string) =>
      screen.getAllByTestId("spawned-filter").find((button) => button.dataset.threadId === threadId) ?? document.body;
    expect(screen.getAllByTestId("spawned-filter").map((button) => button.dataset.threadId)).toEqual(["t1", "t2"]);
    expect(filter("t1").textContent).toBe("2");
    expect(filter("t1").getAttribute("aria-label")).toBe("Show the 2 threads \u201corchestrator\u201d spawned");
    expect(rows()).toEqual(["t1", "t2", "t3", "t4"]);

    await user.click(filter("t1"));
    expect(rows()).toEqual(["t1", "t2", "t3"]);
    expect(host.window.get(SPAWNED_BY)).toBe("t1");
    expect(screen.getByTestId("spawned-by").textContent).toContain("Spawned by orchestrator");
    expect(filter("t1").getAttribute("aria-pressed")).toBe("true");
    // Combined with a typed query.
    await user.type(screen.getByTestId("thread-filter"), "be#");
    expect(rows()).toEqual(["t3"]);
    await user.clear(screen.getByTestId("thread-filter"));
    await user.click(screen.getByTestId("clear-spawned-by"));
    expect(rows()).toEqual(["t1", "t2", "t3", "t4"]);
    expect(screen.queryAllByTestId("spawned-by").length).toBe(0);

    // `:spawned` toggles the filter on the selected thread.
    const spawned = () =>
      act(() => {
        host.commands.run("threads.spawnedBy", [], []);
      });
    act(() => host.window.set(SELECTED_THREAD, "t2"));
    spawned();
    expect(rows()).toEqual(["t2", "t3"]);
    spawned();
    expect(rows()).toEqual(["t1", "t2", "t3", "t4"]);

    // Deleting the thread drops its filter.
    spawned();
    expect(host.window.get(SPAWNED_BY)).toBe("t2");
    client.index({ workspaces, threads: threads.filter((candidate) => candidate.id !== "t2") });
    flush();
    expect(host.window.get(SPAWNED_BY)).toBeUndefined();
    expect(rows()).toEqual(["t1", "t3", "t4"]);
  });
});

describe("a fork's row", () => {
  // spec: UI-665
  test("marks the fork before its title, naming its source", () => {
    setup({
      ...INDEX,
      threads: [
        ...(INDEX.threads ?? []),
        thread("t4", "checkout-v2 (fork)", "w1", {
          titleSource: "placeholder",
          forkedFrom: { threadId: "t1", title: "checkout-v2", entryId: "e1", message: "Fix it" },
        }),
      ],
    });
    const marks = screen.getAllByTestId("thread-forked");
    expect(marks.length).toBe(1);
    const row = marks[0]?.closest<HTMLElement>("[data-testid=thread-row]");
    expect(row?.dataset.threadId).toBe("t4");
    expect(marks[0]?.textContent).toBe("Forked from checkout-v2 at \u201cFix it\u201d");
  });
});
