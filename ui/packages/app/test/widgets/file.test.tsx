/** L4: `omfp:file` and `useFile` over a fake client (UI-110..114, AGT-027). */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import type { FileCandidate, FileContent, GitStatus } from "@omfp/protocol";
import { workspaceTopic } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Activity, type ReactNode, useState } from "react";
import { fakePdfjs } from "../../../kit/test/support/pdfjs.ts";
import {
  FILE_PICKER,
  type FilePoll,
  fileTabId,
  NO_FILE_TABS,
  OPEN_FILES,
  SELECTED_THREAD,
  SUCCESSES,
} from "../../src/sdk/index.ts";
import { TAB_CHOICES } from "../../src/sdk/keys.ts";
import { openTab } from "../../src/vm/files.ts";
import { chooseTab, chosenTab, NO_TAB_CHOICES } from "../../src/vm/tab-choices.ts";
import { FileWidget } from "../../src/widgets/file/index.ts";
import { mediaKind } from "../../src/widgets/file/media.tsx";
import { repo, thread, workspace } from "../support/fixtures.ts";
import { FakeClient, renderWithHost, testHost } from "../support/host.tsx";

afterEach(cleanup);

/** The host's file poll, run by the test. */
class ManualPoll {
  readonly checks = new Set<() => void>();
  readonly poll: FilePoll = (check) => {
    this.checks.add(check);
    return () => {
      this.checks.delete(check);
    };
  };

  run(): void {
    act(() => {
      for (const check of [...this.checks]) {
        check();
      }
    });
  }
}

const TEXT = "fn one() {}\nfn two() {}\n";

function text(content: string, extra: Partial<FileContent> = {}): FileContent {
  return {
    path: "fe/src/lib.rs",
    kind: "text",
    size: content.length,
    hash: `h-${content.length}`,
    unchanged: false,
    content,
    truncated: false,
    repo: "fe",
    head: "fn one() {}\n",
    ...extra,
  };
}

/** Hides its children (`<Activity>`, as a kept view or tab does) and shows them again on each click of `shown`. */
function Hideable({ children }: { children: ReactNode }): ReactNode {
  const [shown, setShown] = useState(true);
  return (
    <>
      <button type="button" data-testid="shown" onClick={() => setShown((now) => !now)}>
        shown
      </button>
      <Activity mode={shown ? "visible" : "hidden"}>{children}</Activity>
    </>
  );
}

function setup(props: Record<string, unknown> = {}, first: FileContent = text(TEXT)) {
  const poll = new ManualPoll();
  const base = testHost(new FakeClient(), { filePoll: poll.poll });
  base.host.window.set(SELECTED_THREAD, "t1");
  let disk = first;
  base.client.answers.set("workspaces.read_file", (params) => {
    const { knownHash } = params as { knownHash?: string };
    return knownHash === disk.hash ? { ...disk, content: null, head: null, unchanged: true } : disk;
  });
  const test = renderWithHost(
    <Hideable>
      <FileWidget path="src/lib.rs" {...props} />
    </Hideable>,
    base,
  );
  test.client.connected();
  test.client.index({
    workspaces: [workspace("w1", [repo("w1", "fe", "feat")])],
    threads: [thread("t1", "one", "w1")],
  });
  test.flush();
  return {
    ...test,
    poll,
    user: userEvent.setup(),
    /** What the file holds on disk from now on. */
    setDisk: (file: FileContent) => {
      disk = file;
    },
  };
}

function lines(element: HTMLElement): string {
  return [...element.querySelectorAll(".cm-line")].map((line) => line.textContent).join("\n");
}

async function editorText(): Promise<string> {
  const editor = await screen.findByTestId("file-editor");
  let shown = "";
  await waitFor(() => {
    shown = lines(editor);
    expect(editor.querySelector(".cm-content")).not.toBeNull();
  });
  return shown;
}

describe("omfp:file", () => {
  // spec: UI-110, AGT-027
  test("reads the file of the current workspace and shows it read-only with its repository", async () => {
    const test = setup();
    expect(test.client.callsTo("workspaces.read_file").map((call) => call.params)).toEqual([
      { workspaceId: "w1", path: "src/lib.rs" },
    ]);
    await waitFor(() => expect(screen.getByTestId("file").dataset.status).toBe("ready"));
    expect(screen.getByTestId("file-path").textContent).toBe("fe/src/lib.rs");
    expect(within(screen.getByTestId("file-bar")).getByText("fe")).toBeDefined();
    expect(await editorText()).toBe(TEXT);
    expect(screen.getByTestId("file-editor").dataset.readOnly).toBe("true");
    expect(screen.getByTestId("file-edit")).toBeDefined();
  });

  // spec: UI-110, AGT-027
  test("the poll asks with the known hash and reloads the view when the file changed; a git status change reads it again", async () => {
    const test = setup();
    await editorText();
    test.poll.run();
    const calls = test.client.callsTo("workspaces.read_file");
    expect(calls.at(-1)?.params).toEqual({ workspaceId: "w1", path: "src/lib.rs", knownHash: `h-${TEXT.length}` });
    expect(await editorText()).toBe(TEXT);
    test.setDisk(text("fn changed() {}\n"));
    test.poll.run();
    await waitFor(() => expect(lines(screen.getByTestId("file-editor"))).toBe("fn changed() {}\n"));
    const status: GitStatus = { branch: "feat", ahead: 0, behind: 0, dirty: true, files: [] };
    for (const [seq, dirty] of [[0, true] as const, [1, false] as const]) {
      test.client.model.applySubscribed(workspaceTopic("w1"), {
        mode: "snapshot",
        epoch: "e1",
        seq,
        state: [{ name: "fe", status: { ...status, dirty } }],
      });
      test.client.notify();
      test.flush();
    }
    // The first snapshot is no change; the second one reads the file again, `HEAD` included (no known hash).
    expect(test.client.callsTo("workspaces.read_file").at(-1)?.params).toEqual({
      workspaceId: "w1",
      path: "src/lib.rs",
    });
  });

  // spec: UI-111, AGT-027, UI-520
  test("Edit switches to editing; Save writes with the hash the text was based on, with a success toast", async () => {
    const test = setup();
    await editorText();
    await test.user.click(screen.getByTestId("file-edit"));
    expect(screen.getByTestId("file-editor").dataset.readOnly).toBe("false");
    await test.user.click(screen.getByTestId("file-save"));
    const write = test.client.callsTo("workspaces.write_file");
    expect(write.map((call) => call.params)).toEqual([
      { workspaceId: "w1", path: "fe/src/lib.rs", content: TEXT, expectedHash: `h-${TEXT.length}` },
    ]);
    act(() => write[0]?.resolve({ path: "fe/src/lib.rs", hash: "h-saved", size: TEXT.length }));
    await waitFor(() =>
      expect(test.host.window.get(SUCCESSES).map((success) => success.text)).toEqual(["Saved fe/src/lib.rs"]),
    );
    // Done goes back to read-only.
    await test.user.click(await screen.findByTestId("file-done"));
    await waitFor(() => expect(screen.getByTestId("file-editor").dataset.readOnly).toBe("true"));
  });

  // spec: UI-113, AGT-027
  test("a refused save shows the file on disk against the user's text; Discard mine reloads, nothing is written", async () => {
    const test = setup();
    await editorText();
    await test.user.click(screen.getByTestId("file-edit"));
    await test.user.click(screen.getByTestId("file-save"));
    test.setDisk(text("fn theirs() {}\n"));
    act(() =>
      test.client
        .callsTo("workspaces.write_file")[0]
        ?.reject(new CallError({ kind: "daemon", error: { code: "conflict", message: "changed on disk" } })),
    );
    const conflict = await screen.findByTestId("file-conflict");
    expect(screen.getByTestId("file-editor").hidden).toBe(true);
    const diff = within(conflict).getByTestId("file-conflict-diff");
    expect(diff.dataset.layout).toBe("split");
    await waitFor(() => expect(diff.querySelectorAll(".cm-mergeViewEditor").length).toBe(2));
    await waitFor(() => expect(lines(diff.querySelector(".cm-merge-a") as HTMLElement)).toBe("fn theirs() {}\n"));
    expect(lines(diff.querySelector(".cm-merge-b") as HTMLElement)).toBe(TEXT);
    await test.user.click(screen.getByTestId("file-discard"));
    expect(screen.queryByTestId("file-conflict")).toBeNull();
    await waitFor(() => expect(lines(screen.getByTestId("file-editor"))).toBe("fn theirs() {}\n"));
    expect(test.client.callsTo("workspaces.write_file")).toHaveLength(1);
  });

  // spec: UI-113
  test("Overwrite saves the user's text with the hash of the file now on disk", async () => {
    const test = setup();
    await editorText();
    await test.user.click(screen.getByTestId("file-edit"));
    await test.user.click(screen.getByTestId("file-save"));
    test.setDisk(text("theirs\n"));
    act(() =>
      test.client
        .callsTo("workspaces.write_file")[0]
        ?.reject(new CallError({ kind: "daemon", error: { code: "conflict", message: "changed" } })),
    );
    await screen.findByTestId("file-conflict");
    await waitFor(() =>
      expect(test.client.callsTo("workspaces.read_file").at(-1)?.params).toEqual({
        workspaceId: "w1",
        path: "src/lib.rs",
      }),
    );
    await test.user.click(screen.getByTestId("file-overwrite"));
    expect(test.client.callsTo("workspaces.write_file").at(-1)?.params).toEqual({
      workspaceId: "w1",
      path: "fe/src/lib.rs",
      content: TEXT,
      expectedHash: "h-7",
    });
  });

  // spec: UI-114
  test("diff shows the whole file against HEAD, unified first, then side by side", async () => {
    const test = setup({ diff: true });
    const diff = await screen.findByTestId("file-head-diff");
    expect(diff.dataset.layout).toBe("unified");
    await waitFor(() => expect(diff.querySelector(".cm-changedLine")?.textContent).toBe("fn two() {}"));
    await test.user.click(screen.getByTestId("file-layout"));
    expect(screen.getByTestId("file-head-diff").dataset.layout).toBe("split");
    expect(screen.queryByTestId("file-editor")).toBeNull();
  });

  // spec: UI-403
  test("hidden and shown again, the view keeps its diff toggle and layout, the edit mode its text", async () => {
    const test = setup({ diff: true });
    await screen.findByTestId("file-head-diff");
    await test.user.click(screen.getByTestId("file-layout"));
    await test.user.click(screen.getByTestId("shown"));
    await test.user.click(screen.getByTestId("shown"));
    expect(screen.getByTestId("file-head-diff").dataset.layout).toBe("split");
    await test.user.click(screen.getByTestId("file-diff"));
    expect(screen.queryAllByTestId("file-head-diff").length).toBe(0);
    await test.user.click(screen.getByTestId("shown"));
    await test.user.click(screen.getByTestId("shown"));
    expect(screen.queryAllByTestId("file-head-diff").length).toBe(0);
    expect(await editorText()).toBe(TEXT);
    await test.user.click(screen.getByTestId("file-edit"));
    await test.user.keyboard("XY");
    await test.user.click(screen.getByTestId("shown"));
    await test.user.click(screen.getByTestId("shown"));
    expect(await editorText()).toBe(`XY${TEXT}`);
    expect(screen.getByTestId("file-editor").dataset.dirty).toBe("true");
  });

  // spec: UI-114
  test("a file HEAD lacks is all new; outside a repository there is nothing to compare with", async () => {
    setup({ diff: true }, text("new\n", { head: null }));
    expect((await screen.findByTestId("file-head-note")).textContent).toContain("Not in HEAD");
    cleanup();
    setup({ diff: true }, text("x\n", { head: null, repo: null }));
    expect((await screen.findByTestId("file-head-note")).textContent).toContain("Not in a repository");
  });

  // spec: UI-110
  test("images render, binaries are named, cut files say so and cannot be edited, missing files say so", async () => {
    setup({}, { ...text(""), kind: "image", content: "aGk=", mime: "image/png", head: null });
    expect((await screen.findByTestId("file-image")).querySelector("img")?.getAttribute("src")).toBe(
      "data:image/png;base64,aGk=",
    );
    cleanup();
    setup({}, { ...text(""), kind: "binary", content: null, size: 2048, head: null });
    expect((await screen.findByTestId("file-binary")).textContent).toBe("Binary file (2.0 KiB), not shown.");
    cleanup();
    setup({}, text("start\n", { truncated: true, size: 3 * 1024 * 1024, head: null }));
    expect(await screen.findByText(/Only the beginning of this 3\.0 MiB file/)).toBeDefined();
    expect(screen.queryByTestId("file-edit")).toBeNull();
    cleanup();
    const base = testHost(new FakeClient(), { filePoll: new ManualPoll().poll });
    base.host.window.set(SELECTED_THREAD, "t1");
    base.client.answers.set("workspaces.find_file", () => ({ candidates: [], truncated: false }));
    const test = renderWithHost(<FileWidget path="gone.rs" />, base);
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "one", "w1")] });
    test.flush();
    act(() =>
      test.client
        .callsTo("workspaces.read_file")[0]
        ?.reject(new CallError({ kind: "daemon", error: { code: "not_found", message: "no such file" } })),
    );
    expect((await screen.findByTestId("file-missing")).textContent).toBe("gone.rs does not exist.");
  });

  // spec: UI-700
  test("a file outside the workspace folder shows read-only with a note, without Edit", async () => {
    const path = "/tmp/elsewhere/notes.txt";
    const test = setup({ path }, text("# Elsewhere\n", { path, repo: null, head: null, outside: true }));
    expect(test.client.callsTo("workspaces.read_file").map((call) => call.params)).toEqual([
      { workspaceId: "w1", path },
    ]);
    expect(await editorText()).toBe("# Elsewhere\n");
    expect(screen.getByTestId("file").dataset.outside).toBe("true");
    expect(screen.getByTestId("file-path").textContent).toBe(path);
    expect(screen.getByTestId("file-outside").textContent).toBe(
      "Outside the workspace: this file can be read here, not edited.",
    );
    expect(screen.queryAllByTestId("file-edit").length).toBe(0);
    expect(screen.getByTestId("file-editor").dataset.readOnly).toBe("true");
    cleanup();
    // A file of the workspace has neither.
    setup();
    await editorText();
    expect(screen.queryAllByTestId("file-outside").length).toBe(0);
    expect(screen.getByTestId("file").dataset.outside).toBeUndefined();
  });

  // spec: UI-704
  test("an outside file in a git working tree shows its repository and compares with its HEAD", async () => {
    const path = "/tmp/elsewhere/notes.txt";
    setup(
      { path, diff: true },
      text("# Elsewhere\nmore\n", { path, repo: "elsewhere", head: "# Elsewhere\n", outside: true }),
    );
    expect(within(await screen.findByTestId("file-bar")).getByText("elsewhere")).toBeDefined();
    const diff = await screen.findByTestId("file-head-diff");
    await waitFor(() => expect(diff.querySelector(".cm-changedLine")?.textContent).toBe("more"));
    expect(screen.queryAllByTestId("file-head-note").length).toBe(0);
    cleanup();
    setup({ path, diff: true }, text("x\n", { path, repo: null, head: null, outside: true }));
    expect((await screen.findByTestId("file-head-note")).textContent).toContain("Not in a repository");
  });

  // spec: UI-705
  test("a missing absolute path outside the workspace says so: no file of the workspace stands for it", async () => {
    const path = "/tmp/elsewhere/gone.md";
    const base = testHost(new FakeClient(), { filePoll: new ManualPoll().poll });
    base.host.window.set(SELECTED_THREAD, "t1");
    // The daemon finds nothing for an absolute path outside the folder (DMN-312).
    base.client.answers.set("workspaces.find_file", () => ({ candidates: [], truncated: false }));
    const test = renderWithHost(<FileWidget path={path} />, base);
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "one", "w1")] });
    test.flush();
    act(() =>
      test.client
        .callsTo("workspaces.read_file")[0]
        ?.reject(new CallError({ kind: "daemon", error: { code: "not_found", message: "no such file" } })),
    );
    expect((await screen.findByTestId("file-missing")).textContent).toBe(`${path} does not exist.`);
    expect(test.client.callsTo("workspaces.find_file").map((call) => call.params)).toEqual([
      { workspaceId: "w1", query: path, threadId: "t1" },
    ]);
    expect(test.host.window.get(FILE_PICKER)).toBeUndefined();
  });

  // spec: UI-541, UI-542
  test("video and audio files play from their raw bytes, again when they change, and say when they cannot", async () => {
    const clip: FileContent = {
      ...text(""),
      path: "media/clip one.WebM",
      kind: "binary",
      content: null,
      size: 4096,
      hash: "v1",
      head: null,
    };
    const { setDisk, poll } = setup({ path: "media/clip one.WebM" }, clip);
    const player = await screen.findByTestId("file-player");
    expect(player.tagName).toBe("VIDEO");
    expect(player.getAttribute("src")).toBe("/files/test/w1/media/clip%20one.WebM?v=v1");
    expect(player.hasAttribute("controls")).toBe(true);
    expect(player.getAttribute("aria-label")).toBe("Video media/clip one.WebM");
    expect(screen.getByTestId("file-media").getAttribute("data-kind")).toBe("video");

    setDisk({ ...clip, hash: "v2" });
    poll.run();
    await waitFor(() =>
      expect(screen.getByTestId("file-player").getAttribute("src")).toBe("/files/test/w1/media/clip%20one.WebM?v=v2"),
    );
    fireEvent.error(screen.getByTestId("file-player"));
    expect((await screen.findByTestId("file-media-error")).textContent).toBe(
      "This window cannot play this video file (4.0 KiB).",
    );
    cleanup();

    setup({ path: "song.flac" }, { ...clip, path: "song.flac" });
    expect((await screen.findByTestId("file-player")).tagName).toBe("AUDIO");
    cleanup();
    // Only binaries with a media extension get a player.
    setup({ path: "track.bin" }, { ...clip, path: "track.bin" });
    expect((await screen.findByTestId("file-binary")).textContent).toBe("Binary file (4.0 KiB), not shown.");
    expect(mediaKind("notes/.wav")).toBeUndefined();
    expect(mediaKind("a.b/clip")).toBeUndefined();
    expect(mediaKind("Movie.MKV")).toBe("video");
  });

  // spec: UI-547
  test("a .pdf file opens in the PDF viewer from its raw bytes, even when the daemon reads it as text, and again when it changes", async () => {
    fakePdfjs.reset();
    fakePdfjs.documents.set("/files/test/w1/docs/Guide%20v2.PDF?v=p1", { pages: 2, width: 612, height: 792 });
    fakePdfjs.documents.set("/files/test/w1/docs/Guide%20v2.PDF?v=p2", { pages: 3, width: 612, height: 792 });
    // An all-ASCII PDF is text to the daemon.
    const guide = text("%PDF-1.4\n%%EOF\n", { path: "docs/Guide v2.PDF", hash: "p1", head: null });
    const { setDisk, poll } = setup({ path: "docs/Guide v2.PDF" }, guide);
    const view = await screen.findByTestId("pdf-view");
    expect(view.getAttribute("data-url")).toBe("/files/test/w1/docs/Guide%20v2.PDF?v=p1");
    expect(view.getAttribute("aria-label")).toBe("PDF docs/Guide v2.PDF");
    expect(screen.queryAllByTestId("file-editor").length).toBe(0);
    expect((await screen.findByTestId("pdf-page-number")).textContent).toBe("1 / 2");

    setDisk({ ...guide, kind: "binary", content: null, hash: "p2" });
    poll.run();
    await waitFor(() => expect(screen.getByTestId("pdf-page-number").textContent).toBe("1 / 3"));
    expect(fakePdfjs.destroyed).toEqual(["/files/test/w1/docs/Guide%20v2.PDF?v=p1"]);
    cleanup();
    // The extension decides: a PDF named otherwise is what the daemon says it is.
    setup({ path: "notes.pdf.txt" }, text("%PDF-1.4\n", { path: "notes.pdf.txt" }));
    await screen.findByTestId("file-editor");
    expect(screen.queryAllByTestId("pdf-view").length).toBe(0);
  });

  // spec: UI-548
  test("an image over the daemon's cap loads from its raw bytes into the viewer, again when it changes", async () => {
    const big: FileContent = {
      ...text(""),
      path: "art/big photo.png",
      kind: "image",
      mime: "image/png",
      content: null,
      size: 9 * 1024 * 1024,
      hash: "b1",
      head: null,
    };
    const { setDisk, poll } = setup({ path: "art/big photo.png" }, big);
    const image = await screen.findByTestId("image-after");
    expect(image.getAttribute("src")).toBe("/files/test/w1/art/big%20photo.png?v=b1");
    expect(screen.getByTestId("file-image").getAttribute("aria-label")).toBe("Image art/big photo.png");
    expect(screen.getByTestId("file-path").textContent).toBe("art/big photo.png");
    expect(screen.queryAllByTestId("file-binary").length).toBe(0);
    setDisk({ ...big, hash: "b2" });
    poll.run();
    await waitFor(() =>
      expect(screen.getByTestId("image-after").getAttribute("src")).toBe("/files/test/w1/art/big%20photo.png?v=b2"),
    );
  });

  // spec: UI-553
  test("an image that differs from HEAD compares with it side by side or under a slider", async () => {
    const logo: FileContent = {
      ...text(""),
      path: "fe/logo.png",
      kind: "image",
      mime: "image/png",
      content: "aGk=",
      hash: "now",
      head: null,
      headHash: "then",
    };
    const { user } = setup({ path: "fe/logo.png" }, logo);
    const diff = await screen.findByTestId("file-diff");
    expect([diff.textContent, diff.getAttribute("aria-pressed")]).toEqual(["Diff", "false"]);
    expect(screen.getAllByTestId("image-viewport").length).toBe(1);
    await user.click(diff);
    expect(screen.getByTestId("file-image").dataset.compare).toBe("split");
    expect(screen.getByTestId("image-before").getAttribute("src")).toBe("/files/test/w1/fe/logo.png?rev=HEAD&v=then");
    expect(screen.getByTestId("image-before").getAttribute("alt")).toBe("fe/logo.png at HEAD");
    expect(screen.getByTestId("image-after").getAttribute("src")).toBe("data:image/png;base64,aGk=");
    const layout = screen.getByTestId("file-layout");
    expect(layout.textContent).toBe("Slider");
    await user.click(layout);
    expect([screen.getByTestId("file-image").dataset.compare, layout.textContent]).toEqual(["slider", "Side by side"]);
    await user.click(diff);
    expect(screen.getByTestId("file-image").dataset.compare).toBe("none");
    cleanup();

    // `diff` opens it compared; an image as at HEAD, or not at HEAD, has nothing to compare.
    setup({ path: "fe/logo.png", diff: true }, logo);
    expect((await screen.findByTestId("file-image")).dataset.compare).toBe("split");
    cleanup();
    for (const headHash of ["now", null]) {
      setup({ path: "fe/logo.png" }, { ...logo, headHash });
      await screen.findByTestId("file-image");
      expect(screen.queryAllByTestId("file-diff").length).toBe(0);
      cleanup();
    }
  });
});

describe("a path that does not exist", () => {
  function missing(path: string, candidates: readonly FileCandidate[]) {
    const base = testHost(new FakeClient(), { filePoll: new ManualPoll().poll });
    base.host.window.set(SELECTED_THREAD, "t1");
    base.host.window.set(OPEN_FILES, { w1: openTab(NO_FILE_TABS, path, { line: 4 }) });
    // Opened by `t1`, which selected its tab (UI-372).
    base.host.window.set(
      TAB_CHOICES,
      chooseTab(NO_TAB_CHOICES, { kind: "thread", threadId: "t1" }, "default", "main", fileTabId(path)),
    );
    base.client.answers.set("workspaces.find_file", () => ({ candidates, truncated: false }));
    const test = renderWithHost(<FileWidget path={path} line={4} />, base);
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "one", "w1")] });
    test.flush();
    act(() =>
      test.client
        .callsTo("workspaces.read_file")[0]
        ?.reject(new CallError({ kind: "daemon", error: { code: "not_found", message: "no such file" } })),
    );
    return test;
  }

  const candidate = (path: string, match: FileCandidate["match"] = "basename"): FileCandidate => ({
    path,
    match,
    mentioned: false,
  });

  // spec: UI-117, DMN-092
  test("with one candidate, the candidate takes the tab's place with its line", async () => {
    const test = missing("theme.ts", [candidate("ui/src/theme.ts")]);
    await waitFor(() =>
      expect(test.host.window.get(OPEN_FILES).w1?.tabs.map((tab) => tab.path)).toEqual(["ui/src/theme.ts"]),
    );
    expect(test.client.callsTo("workspaces.find_file").map((call) => call.params)).toEqual([
      { workspaceId: "w1", query: "theme.ts", threadId: "t1" },
    ]);
    expect(test.host.window.get(OPEN_FILES).w1?.tabs[0]?.line).toBe(4);
    expect(chosenTab(test.host.window.get(TAB_CHOICES), { kind: "thread", threadId: "t1" }, "default", "main")).toBe(
      fileTabId("ui/src/theme.ts"),
    );
    expect(test.host.window.get(FILE_PICKER)).toBeUndefined();
  });

  // spec: UI-117
  test("with several, the frame's picker gets them and the tab says so", async () => {
    const candidates = [candidate("app/theme.ts"), candidate("lib/theme.ts")];
    const test = missing("theme.ts", candidates);
    expect((await screen.findByTestId("file-choosing")).textContent).toContain("Several files match theme.ts");
    expect(test.host.window.get(FILE_PICKER)).toEqual({
      workspaceId: "w1",
      path: "theme.ts",
      options: { line: 4, diff: false },
      candidates,
    });
    expect(test.host.window.get(OPEN_FILES).w1?.tabs.map((tab) => tab.path)).toEqual(["theme.ts"]);
  });
});
