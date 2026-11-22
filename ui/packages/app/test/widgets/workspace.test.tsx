/** L4: `omfp:workspace` over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import type { WorkspaceRepo } from "@omfp/protocol";
import { workspaceTopic } from "@omfp/protocol";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DRAFT_THREAD, ERRORS, type OmfpHost, SELECTED_THREAD, SUCCESSES } from "../../src/sdk/index.ts";
import { WorkspaceWidget } from "../../src/widgets/workspace/index.ts";
import { repo, thread, workspace } from "../support/fixtures.ts";
import { type CallRecord, renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

const linked: WorkspaceRepo = { ...repo("w1", "be", "main"), kind: "link", target: "/src/be" };
const shared: WorkspaceRepo = { ...repo("w1", "fe", "feat"), sharedWith: ["w2"] };
const cloned: WorkspaceRepo = { ...repo("w1", "api", "main"), kind: "clone" };

/** Renders the widget after `select` picked what the window shows (the thread `t1` by default). */
function setup(select: (host: OmfpHost) => void = (host) => host.window.set(SELECTED_THREAD, "t1")) {
  const base = testHost();
  for (const method of [
    "workspaces.add_repo",
    "workspaces.remove_repo",
    "workspaces.update",
  ] as const satisfies readonly CallMethod[]) {
    base.client.answers.set(method, () => ({}));
  }
  base.client.answers.set("repos.suggest", () => ({ repos: [{ path: "/src/fe", name: "fe", lastSeenAt: 1 }] }));
  base.client.answers.set("fs.complete", () => ({ entries: [] }));
  select(base.host);
  const test = renderWithHost(<WorkspaceWidget />, base);
  test.client.connected();
  test.client.index({
    workspaces: [
      { ...workspace("w1", [shared, linked, cloned]), name: "checkout" },
      { ...workspace("w2"), name: "other" },
    ],
    threads: [thread("t1", "one", "w1"), thread("t2", "two", "w1"), thread("t3", "three", "w2")],
  });
  test.flush();
  return { ...test, user: userEvent.setup() };
}

function rows(): HTMLElement[] {
  return screen.getAllByTestId("workspace-repo");
}

function row(name: string): HTMLElement {
  const found = rows().find((element) => element.dataset.name === name);
  if (found === undefined) {
    throw new Error(`no repository row ${name}`);
  }
  return found;
}

function pathInput(): HTMLInputElement {
  return screen.getByTestId("add-repo-path") as HTMLInputElement;
}

/** Answers `call` with success inside an awaited `act`, which settles the widget's promise chain and its render. */
async function answer(call: CallRecord | undefined): Promise<void> {
  await act(async () => {
    call?.resolve({});
  });
}

describe("omfp:workspace", () => {
  // spec: UI-025
  test("the header shows the selected thread's workspace: name, folder and a shared warning; no Rescan button", () => {
    const { client } = setup();
    expect(client.subscribed).toContain(workspaceTopic("w1"));
    expect(screen.getByTestId("workspace-name").textContent).toBe("checkout");
    expect(screen.getByTestId("workspace-path").textContent).toBe("/work/w1");
    expect(screen.getByTestId("shared-warning").textContent).toBe("shared by 2 threads");
    expect(screen.queryByTestId("workspace-row")).toBeNull();
    expect(screen.queryByTestId("repositories")).toBeNull();
    expect(screen.queryByTestId("refresh-workspace")).toBeNull();
  });

  // spec: UI-025, UI-187
  test("the side boxes are titled on their top border: Repositories and Add repository", () => {
    setup();
    const repos = screen.getByRole("region", { name: "Repositories" });
    const add = screen.getByRole("region", { name: "Add repository" });
    expect(add === screen.getByTestId("add-repo")).toBe(true);
    for (const box of [repos, add]) {
      expect(box.classList.contains("omfp-bbox")).toBe(true);
      expect(box.querySelector(".omfp-bbox__head > h3.omfp-bbox__label")?.textContent).toBe(
        box === repos ? "Repositories" : "Add repository",
      );
    }
    expect(within(repos).getAllByTestId("workspace-repo").length).toBe(3);
  });

  // spec: UI-025, UI-023
  test("without a selected thread it shows the draft thread's workspace, else none", () => {
    setup((host) => host.window.set(DRAFT_THREAD, { workspaceId: "w2" }));
    expect(screen.getByTestId("workspace-name").textContent).toBe("other");
    cleanup();
    setup(() => {});
    expect(screen.getByTestId("no-workspace")).toBeDefined();
  });

  // spec: UI-025, UI-520
  test("the folder path is copied to the clipboard, with a success toast", async () => {
    const { host, user } = setup();
    await user.click(screen.getByTestId("copy-workspace-path"));
    expect(await navigator.clipboard.readText()).toBe("/work/w1");
    await waitFor(() =>
      expect(host.window.get(SUCCESSES).map((success) => success.text)).toEqual(["Folder path copied"]),
    );
  });

  // spec: UI-025
  test("the name is edited in place", async () => {
    const { client, user } = setup();
    await user.click(screen.getByTestId("rename-workspace"));
    const input = screen.getByTestId("workspace-name-input");
    await user.clear(input);
    await user.type(input, "checkout-v2{Enter}");
    expect(client.callsTo("workspaces.update").map((call) => call.params)).toEqual([
      { workspaceId: "w1", name: "checkout-v2" },
    ]);
    expect(screen.getByTestId("workspace-name")).toBeDefined();
  });

  // spec: UI-025, DMN-027, UI-157
  test("each repository shows its kind, origin, branch and live git status, and a shared badge", () => {
    const { client, flush } = setup();
    expect(rows().map((element) => [element.dataset.name, element.dataset.kind])).toEqual([
      ["fe", "worktree"],
      ["be", "link"],
      ["api", "clone"],
    ]);
    expect(rows().map((element) => within(element).getByTestId("repo-kind").getAttribute("aria-label"))).toEqual([
      "Worktree",
      "Link",
      "Clone",
    ]);
    // Each kind is the kit's kind mark, colored by kind (UI-157).
    expect(rows().map((element) => within(element).getByTestId("repo-kind").dataset.kind)).toEqual([
      "worktree",
      "link",
      "clone",
    ]);
    expect(within(row("fe")).getByTestId("repo-origin").textContent).toBe("of /src/fe");
    expect(within(row("be")).getByTestId("repo-origin").textContent).toBe("→ /src/be");
    expect(within(row("api")).queryByTestId("repo-origin")).toBeNull();
    expect(within(row("fe")).getByTestId("repo-branch").textContent).toBe("feat");
    expect(within(row("fe")).getByTestId("shared-repo").textContent).toBe("also in other");
    expect(within(row("be")).queryByTestId("shared-repo")).toBeNull();
    expect(screen.queryByTestId("git-status")).toBeNull();
    client.model.applySubscribed(workspaceTopic("w1"), {
      mode: "snapshot",
      epoch: "e1",
      seq: 0,
      state: [{ name: "fe", status: { branch: "feat2", ahead: 1, behind: 0, dirty: true, files: [] } }],
    });
    client.notify();
    flush();
    expect(within(row("fe")).getByTestId("git-status").textContent).toBe("↑1 ↓0 · dirty");
    expect(within(row("fe")).getByTestId("repo-branch").textContent).toBe("feat2");
    expect(row("fe").dataset.dirty).toBe("true");
  });

  // spec: UI-025, DMN-025
  test("every kind can be removed: a link at once, a worktree or a clone after a confirmation", async () => {
    const { client, user } = setup();
    await user.click(within(row("be")).getByTestId("remove-repo"));
    expect(client.callsTo("workspaces.remove_repo").map((call) => call.params)).toEqual([
      { workspaceId: "w1", name: "be" },
    ]);
    await user.click(within(row("fe")).getByTestId("remove-repo"));
    expect(screen.getByTestId("remove-repo-dialog")).toBeDefined();
    await user.click(screen.getByTestId("confirm-remove-repo"));
    await user.click(within(row("api")).getByTestId("remove-repo"));
    await user.click(await screen.findByTestId("confirm-remove-repo"));
    expect(client.callsTo("workspaces.remove_repo").map((call) => call.params)).toEqual([
      { workspaceId: "w1", name: "be" },
      { workspaceId: "w1", name: "fe" },
      { workspaceId: "w1", name: "api" },
    ]);
  });

  // spec: UI-025, DMN-021
  test("add a link: a typed folder, completed by the daemon, submitted with Enter", async () => {
    const { client, user } = setup();
    const group = screen.getByTestId("add-repo-kind");
    expect(
      within(group)
        .getAllByRole("radio")
        .map((radio) => (radio as HTMLInputElement).value),
    ).toEqual(["link", "clone"]);
    expect((within(group).getByRole("radio", { name: "Link" }) as HTMLInputElement).checked).toBe(true);
    expect(screen.queryByTestId("add-repo-url")).toBeNull();
    await user.type(pathInput(), "/tmp/x");
    await waitFor(() => expect(client.callsTo("fs.complete").length).toBeGreaterThan(0));
    await user.keyboard("{Enter}");
    expect(client.callsTo("workspaces.add_repo").map((call) => call.params)).toEqual([
      { workspaceId: "w1", source: { kind: "link", path: "/tmp/x" } },
    ]);
    await waitFor(() => expect(pathInput().value).toBe(""));
  });

  // spec: UI-025, DMN-021
  test("add a link to a suggested repository with the button", async () => {
    const { client, user } = setup();
    expect(screen.getByTestId("add-repo-submit").hasAttribute("disabled")).toBe(true);
    await user.type(pathInput(), "fe");
    const option = (await screen.findAllByTestId("path-option")).find((element) => element.dataset.path === "/src/fe");
    if (option === undefined) {
      throw new Error("no suggestion for /src/fe");
    }
    expect(option.dataset.kind).toBe("suggestion");
    await user.click(option);
    expect(pathInput().value).toBe("/src/fe");
    await user.click(screen.getByTestId("add-repo-submit"));
    expect(client.callsTo("workspaces.add_repo").map((call) => call.params)).toEqual([
      { workspaceId: "w1", source: { kind: "link", path: "/src/fe" } },
    ]);
    await waitFor(() => expect(pathInput().value).toBe(""));
  });

  // spec: UI-025
  test("a folder typed while an add is pending survives that add's answer and is added next", async () => {
    const { client, user } = setup();
    client.answers.delete("workspaces.add_repo");
    await user.type(pathInput(), "/tmp/a");
    await user.click(screen.getByTestId("add-repo-submit"));
    await user.clear(pathInput());
    await user.type(pathInput(), "/tmp/b");
    await answer(client.callsTo("workspaces.add_repo")[0]);
    expect(pathInput().value).toBe("/tmp/b");
    await user.click(screen.getByTestId("add-repo-submit"));
    expect(client.callsTo("workspaces.add_repo").map((call) => call.params)).toEqual([
      { workspaceId: "w1", source: { kind: "link", path: "/tmp/a" } },
      { workspaceId: "w1", source: { kind: "link", path: "/tmp/b" } },
    ]);
    await answer(client.callsTo("workspaces.add_repo")[1]);
    expect(pathInput().value).toBe("");
  });

  // spec: UI-025
  test("add a clone by URL", async () => {
    const { client, user } = setup();
    await user.click(within(screen.getByTestId("add-repo-kind")).getByRole("radio", { name: "Clone" }));
    expect(screen.queryByTestId("add-repo-path")).toBeNull();
    await user.type(screen.getByTestId("add-repo-url"), "https://example.invalid/x.git{Enter}");
    expect(client.callsTo("workspaces.add_repo").map((call) => call.params)).toEqual([
      { workspaceId: "w1", source: { kind: "clone", url: "https://example.invalid/x.git" } },
    ]);
    await waitFor(() => expect((screen.getByTestId("add-repo-url") as HTMLInputElement).value).toBe(""));
  });

  // spec: UI-025
  test("a failed action lands in the window's errors", async () => {
    const { client, host, user } = setup();
    client.answers.delete("workspaces.add_repo");
    await user.click(within(screen.getByTestId("add-repo-kind")).getByRole("radio", { name: "Clone" }));
    await user.type(screen.getByTestId("add-repo-url"), "https://example.invalid/x.git{Enter}");
    act(() => client.callsTo("workspaces.add_repo")[0]?.reject(new Error("git_error: boom")));
    await waitFor(() => expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["git_error: boom"]));
  });
});
