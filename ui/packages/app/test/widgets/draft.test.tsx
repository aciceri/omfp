/** L4: the draft thread (UI-023) in `omfp:chat` and `omfp:header` over a fake client. */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createKeyHandler } from "../../src/keymap/handler.ts";
import { DEFAULT_BINDINGS } from "../../src/keymap/keymap.ts";
import { DRAFT_MESSAGE, DRAFT_THREAD, draftKey, ERRORS, SELECTED_THREAD, threadActions } from "../../src/sdk/index.ts";
import { ChatWidget } from "../../src/widgets/chat/index.ts";
import { HeaderWidget } from "../../src/widgets/header/index.ts";
import { repo, thread, workspace } from "../support/fixtures.ts";
import { renderWithHost } from "../support/host.tsx";
import { PointerMedia } from "../support/pointer.ts";

afterEach(cleanup);

/** A tiny PNG file, as pasted or dropped, and the attachment it becomes (UI-060). */
function pngFile(): File {
  return new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], "a.png", { type: "image/png" });
}
const PNG = { type: "image" as const, data: "iVBORw==", mimeType: "image/png" };

/** A PNG file whose bytes are read only once `release` is called; `read` is that read. */
function slowPngFile(): { file: File; release: () => void; read: Promise<ArrayBuffer> } {
  const file = pngFile();
  const bytes = file.arrayBuffer();
  const gate = Promise.withResolvers<void>();
  const read = gate.promise.then(() => bytes);
  Object.defineProperty(file, "arrayBuffer", { value: () => read });
  return { file, release: () => gate.resolve(), read };
}

function setup() {
  const test = renderWithHost(
    <>
      <HeaderWidget />
      <ChatWidget />
    </>,
  );
  test.client.connected();
  test.client.index({
    workspaces: [{ ...workspace("w1", [repo("w1", "fe", "main")]), name: "checkout" }],
    threads: [thread("t1", "older", "w1")],
  });
  test.flush();
  act(() => threadActions(test.host).openDraft("w1"));
  return { ...test, user: userEvent.setup() };
}

describe("draft thread", () => {
  // spec: UI-023
  test("it shows an empty conversation, a placeholder title with its workspace's chip, and a focused composer", async () => {
    setup();
    expect(screen.getByTestId("draft-thread-view").dataset.workspaceId).toBe("w1");
    expect(screen.getByTestId("draft-empty")).toBeDefined();
    const header = screen.getByTestId("thread-header");
    expect(header.dataset.draft).toBe("true");
    expect(within(header).getByTestId("thread-title").textContent).toBe("New thread");
    expect(
      within(header)
        .getAllByTestId("workspace-chip")
        .map((chip) => chip.textContent),
    ).toEqual(["checkout · 1 repo"]);
    await waitFor(() => expect(document.activeElement === screen.getByTestId("composer-input")).toBe(true));
  });

  // spec: UI-023
  test("the first message creates the daemon thread with it, then the real thread is selected", async () => {
    const { client, host, user } = setup();
    expect(client.callsTo("threads.create")).toHaveLength(0);
    await user.type(screen.getByTestId("composer-input"), "  fix the checkout {Enter}");
    expect(client.callsTo("threads.create").map((call) => call.params)).toEqual([
      { workspace: { id: "w1" }, prompt: "fix the checkout" },
    ]);
    // Pressing again while it is created sends nothing more.
    await user.click(screen.getByTestId("send"));
    expect(client.callsTo("threads.create")).toHaveLength(1);
    act(() => client.callsTo("threads.create")[0]?.resolve(thread("t9", "", "w1")));
    await waitFor(() => expect(host.window.get(SELECTED_THREAD)).toBe("t9"));
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
    expect(host.window.get(DRAFT_MESSAGE).text).toBe("");
  });

  // spec: UI-023, UI-030
  test("a model selected before creation stays through a failed send and is used on retry", async () => {
    const { client, host, user } = setup();
    act(() =>
      client.callsTo("auth.models")[0]?.resolve({
        models: [{ id: "chosen", name: "Chosen model", provider: "fake" }],
      }),
    );
    await user.click(within(screen.getByTestId("model-select")).getByRole("button"));
    await user.click(await screen.findByRole("option", { name: "Chosen model (fake)" }));
    expect(client.callsTo("threads.create")).toHaveLength(0);
    await user.type(screen.getByTestId("composer-input"), "first prompt{Enter}");
    act(() => client.callsTo("threads.create")[0]?.reject(new Error("try again")));
    await waitFor(() => expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(false));
    expect(host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "w1", model: "fake/chosen" });
    await user.click(screen.getByTestId("send"));
    expect(client.callsTo("threads.create").map((call) => call.params)).toEqual([
      { workspace: { id: "w1" }, prompt: "first prompt", model: "fake/chosen" },
      { workspace: { id: "w1" }, prompt: "first prompt", model: "fake/chosen" },
    ]);
  });

  // spec: UI-023
  test("when the creation fails, the draft and its text stay and the error is reported", async () => {
    const { client, host, user } = setup();
    await user.type(screen.getByTestId("composer-input"), "fix it{Enter}");
    act(() => client.callsTo("threads.create")[0]?.reject(new Error("daemon down")));
    await waitFor(() => expect(host.window.get(ERRORS)).toEqual(["daemon down"]));
    expect(host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "w1" });
    expect(screen.getByTestId("composer-input")).toHaveProperty("value", "fix it");
    await waitFor(() => expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(false));
  });

  // spec: UI-732, UI-023
  test("`@` completes the draft workspace's files; Enter on the highlighted file inserts it instead of sending", async () => {
    const { client, user } = setup();
    await user.type(screen.getByTestId("composer-input"), "explain @rea");
    await waitFor(() => expect(client.callsTo("workspaces.find_file")).toHaveLength(1));
    expect(client.callsTo("workspaces.find_file")[0]?.params).toEqual({ workspaceId: "w1", query: "rea", fuzzy: true });
    await act(async () =>
      client.callsTo("workspaces.find_file")[0]?.resolve({
        candidates: [
          { path: "README.md", match: "fuzzy" },
          { path: "fe/README.md", repo: "fe", match: "fuzzy" },
        ],
      }),
    );
    const menu = await screen.findByTestId("file-suggestions");
    expect(
      within(menu)
        .getAllByTestId("file-suggestion")
        .map((item) => item.textContent),
    ).toEqual(["@README.md", "@fe/README.md"]);
    await user.keyboard("{ArrowDown}{Enter}");
    expect(screen.getByTestId("composer-input")).toHaveProperty("value", "explain @fe/README.md ");
    expect(client.callsTo("threads.create")).toHaveLength(0);
    await user.keyboard("{Enter}");
    expect(client.callsTo("threads.create").map((call) => call.params)).toEqual([
      { workspace: { id: "w1" }, prompt: "explain @fe/README.md" },
    ]);
  });

  // spec: UI-023, UI-060
  test("pasted and dropped images become removable attachments sent with the first message", async () => {
    const { client, user } = setup();
    const input = screen.getByTestId("composer-input");
    fireEvent.paste(input, { clipboardData: { files: [pngFile()], getData: () => "" } });
    await waitFor(() => expect(screen.getAllByTestId("attachment").length).toBe(1));
    expect(input).toHaveProperty("value", "");
    const text = new File(["hi"], "a.txt", { type: "text/plain" });
    fireEvent.drop(screen.getByTestId("composer"), { dataTransfer: { files: [pngFile(), text] } });
    await waitFor(() => expect(screen.getAllByTestId("attachment").length).toBe(2));
    await user.click(screen.getAllByTestId("attachment-remove")[1] as HTMLElement);
    expect(screen.getAllByTestId("attachment").length).toBe(1);
    await user.type(input, "what is this?{Enter}");
    expect(client.callsTo("threads.create").map((call) => call.params)).toEqual([
      { workspace: { id: "w1" }, prompt: "what is this?", images: [PNG] },
    ]);
  });

  // spec: UI-023, UI-060
  test("a first message of images only creates the thread", async () => {
    const { client, host, user } = setup();
    fireEvent.paste(screen.getByTestId("composer-input"), { clipboardData: { files: [pngFile()], getData: () => "" } });
    await waitFor(() => expect(screen.getAllByTestId("attachment").length).toBe(1));
    await user.click(screen.getByTestId("send"));
    expect(client.callsTo("threads.create").map((call) => call.params)).toEqual([
      { workspace: { id: "w1" }, images: [PNG] },
    ]);
    act(() => client.callsTo("threads.create")[0]?.resolve(thread("t9", "", "w1")));
    await waitFor(() => expect(host.window.get(SELECTED_THREAD)).toBe("t9"));
    expect(host.window.get(DRAFT_MESSAGE).images.length).toBe(0);
  });

  // spec: UI-023, UI-060
  test("when the creation fails, the draft keeps its text and its images", async () => {
    const { client, host, user } = setup();
    const input = screen.getByTestId("composer-input");
    fireEvent.paste(input, { clipboardData: { files: [pngFile()], getData: () => "" } });
    await waitFor(() => expect(screen.getAllByTestId("attachment").length).toBe(1));
    await user.type(input, "fix it{Enter}");
    act(() => client.callsTo("threads.create")[0]?.reject(new Error("daemon down")));
    await waitFor(() => expect(host.window.get(ERRORS)).toEqual(["daemon down"]));
    expect(host.window.get(DRAFT_MESSAGE).images).toEqual([PNG]);
    expect(screen.getAllByTestId("attachment").length).toBe(1);
    expect(input).toHaveProperty("value", "fix it");
    await waitFor(() => expect(screen.getByTestId("send").hasAttribute("disabled")).toBe(false));
    await user.click(screen.getByTestId("send"));
    expect(client.callsTo("threads.create")[1]?.params).toEqual({
      workspace: { id: "w1" },
      prompt: "fix it",
      images: [PNG],
    });
  });

  // spec: UI-023, UI-060
  test("a send right after a paste waits for the image being read and sends it", async () => {
    const { client, user } = setup();
    const input = screen.getByTestId("composer-input");
    const slow = slowPngFile();
    await user.type(input, "what is this?");
    fireEvent.paste(input, { clipboardData: { files: [slow.file], getData: () => "" } });
    await user.keyboard("{Enter}");
    expect(client.callsTo("threads.create")).toHaveLength(0);
    await act(async () => {
      slow.release();
      await slow.read;
    });
    await waitFor(() => expect(client.callsTo("threads.create")).toHaveLength(1));
    expect(client.callsTo("threads.create")[0]?.params).toEqual({
      workspace: { id: "w1" },
      prompt: "what is this?",
      images: [PNG],
    });
  });

  // spec: UI-023, UI-060
  test("an image still being read when its draft is discarded joins no other draft, even one opened meanwhile", async () => {
    const { host, user } = setup();
    const slow = slowPngFile();
    fireEvent.paste(screen.getByTestId("composer-input"), { clipboardData: { files: [slow.file], getData: () => "" } });
    await user.keyboard("{Escape}");
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
    // Another draft opens before the discarded draft's read ends.
    act(() => threadActions(host).openDraft("w1"));
    await act(async () => {
      slow.release();
      await slow.read;
    });
    fireEvent.paste(screen.getByTestId("composer-input"), { clipboardData: { files: [pngFile()], getData: () => "" } });
    await waitFor(() => expect(screen.getAllByTestId("attachment").length).toBe(1));
    expect(host.window.get(DRAFT_MESSAGE).images.length).toBe(1);
  });

  // spec: UI-023, UI-060
  test("a send waiting for an image is dropped with its draft and never sends the draft opened after it", async () => {
    const { client, host, user } = setup();
    const slow = slowPngFile();
    fireEvent.paste(screen.getByTestId("composer-input"), { clipboardData: { files: [slow.file], getData: () => "" } });
    await user.keyboard("{Enter}");
    await user.keyboard("{Escape}");
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
    act(() => threadActions(host).openDraft("w1"));
    await user.type(screen.getByTestId("composer-input"), "not submitted");
    await act(async () => {
      slow.release();
      await slow.read;
    });
    fireEvent.paste(screen.getByTestId("composer-input"), { clipboardData: { files: [pngFile()], getData: () => "" } });
    await waitFor(() => expect(screen.getAllByTestId("attachment").length).toBe(1));
    expect(client.callsTo("threads.create")).toHaveLength(0);
    expect(host.window.get(DRAFT_MESSAGE).text).toBe("not submitted");
  });

  // spec: UI-023, UI-060, UI-064
  test("a draft reopened after a thread switch still waits for the image it was reading before sending", async () => {
    const { client, host, user } = setup();
    const slow = slowPngFile();
    await user.type(screen.getByTestId("composer-input"), "what is this?");
    fireEvent.paste(screen.getByTestId("composer-input"), { clipboardData: { files: [slow.file], getData: () => "" } });
    act(() => threadActions(host).select("t1"));
    expect(screen.queryByTestId("draft-thread-view")).toBeNull();
    act(() => threadActions(host).openDraft("w1"));
    await user.click(screen.getByTestId("send"));
    expect(client.callsTo("threads.create")).toHaveLength(0);
    await act(async () => {
      slow.release();
      await slow.read;
    });
    await waitFor(() => expect(client.callsTo("threads.create")).toHaveLength(1));
    expect(client.callsTo("threads.create")[0]?.params).toEqual({
      workspace: { id: "w1" },
      prompt: "what is this?",
      images: [PNG],
    });
  });
  // spec: UI-023, UI-030, UI-064
  test("a failed first prompt selects the surviving thread and retries its full draft without creating another", async () => {
    const { client, host, user, flush } = setup();
    const message = {
      text: "review [Paste #1, 12 chars]",
      pastes: [{ id: 1, text: "first\nsecond" }],
      images: [{ type: "image" as const, mimeType: "image/png", data: "aW1hZ2U=" }],
    };
    act(() => {
      host.window.set(DRAFT_THREAD, { workspaceId: "w1", model: "fake/chosen" });
      host.window.set(DRAFT_MESSAGE, message);
    });
    await user.click(screen.getByTestId("send"));
    client.index({ workspaces: [workspace("w1")], threads: [thread("survivor", "", "w1")] });
    client.thread("survivor", []);
    flush();
    await act(async () =>
      client.callsTo("threads.create")[0]?.reject(
        new CallError({
          kind: "daemon",
          error: { code: "omp_error", message: "prompt rejected", details: { threadId: "survivor" } },
        }),
      ),
    );
    expect(host.window.get(SELECTED_THREAD)).toBe("survivor");
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
    expect(host.window.get(DRAFT_MESSAGE).text).toBe("");
    expect(host.window.get(draftKey("survivor"))).toEqual(message);
    expect(host.window.get(ERRORS).length).toBe(1);
    expect(client.callsTo("threads.create")[0]?.params).toEqual({
      workspace: { id: "w1" },
      prompt: "review first\nsecond",
      images: message.images,
      model: "fake/chosen",
    });
    await user.click(screen.getByTestId("send"));
    expect(client.callsTo("thread.prompt").map((call) => call.params)).toEqual([
      { threadId: "survivor", text: "review first\nsecond", images: message.images },
    ]);
    await act(async () => client.callsTo("thread.prompt")[0]?.resolve({ promptId: "retry", agentInvoked: true }));
    expect(client.callsTo("threads.create").length).toBe(1);
    expect(client.callsTo("threads.delete").length).toBe(0);
    expect(host.window.get(draftKey("survivor")).text).toBe("");
  });

  // spec: UI-023
  test("Esc leaves the composer, discarding the draft only when it is empty", async () => {
    const { host, user } = setup();
    await user.type(screen.getByTestId("composer-input"), "keep{Escape}");
    expect(host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "w1" });
    await user.clear(screen.getByTestId("composer-input"));
    await user.keyboard("{Escape}");
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
    expect(screen.queryByTestId("draft-thread-view")).toBeNull();
    expect(screen.getByTestId("no-thread")).toBeDefined();
  });

  // spec: UI-360, UI-023
  test("a large paste in the draft thread's composer is a chip, expanded in the first message", async () => {
    const { client, host, user } = setup();
    const input = screen.getByTestId("composer-input");
    await user.type(input, "look: ");
    const long = Array.from({ length: 30 }, (_, index) => `row ${index}`).join("\n");
    act(() => {
      fireEvent.paste(input, { clipboardData: { files: [], getData: () => long } });
    });
    expect(screen.getAllByTestId("paste-toggle").map((chip) => chip.textContent)).toEqual([
      "Pasted text #1 · 30 lines",
    ]);
    expect(host.window.get(DRAFT_MESSAGE).text).toBe("look: [Paste #1, +30 lines]");
    await user.keyboard("{Enter}");
    expect(client.callsTo("threads.create").map((call) => call.params)).toEqual([
      { workspace: { id: "w1" }, prompt: `look: ${long}` },
    ]);
  });

  // spec: UI-421, UI-422, UI-423, UI-023
  test("on a touch screen the draft's Enter makes a new line and Send creates the thread", async () => {
    const pointer = new PointerMedia(true);
    try {
      const { client, user } = setup();
      const input = screen.getByTestId("composer-input");
      expect(
        ["autocorrect", "autocapitalize", "spellcheck", "enterkeyhint"].map((name) => input.getAttribute(name)),
      ).toEqual(["on", "sentences", "true", "enter"]);
      expect(screen.getByTestId("composer").querySelectorAll(".omfp-compose__hint, .omfp-kbd").length).toBe(0);
      await user.type(input, "fix{Enter}the checkout");
      expect(client.callsTo("threads.create")).toHaveLength(0);
      await user.click(screen.getByTestId("send"));
      expect(client.callsTo("threads.create").map((call) => call.params)).toEqual([
        { workspace: { id: "w1" }, prompt: "fix\nthe checkout" },
      ]);
    } finally {
      pointer.restore();
    }
  });
});

describe("draft thread readline keys", () => {
  // spec: UI-760
  test("the draft's composer takes the same keys, editing its own message; no history at its ends", async () => {
    const { host, user, client } = setup();
    const handler = createKeyHandler({ bindings: DEFAULT_BINDINGS, commands: host.commands, now: () => 0 });
    document.addEventListener("keydown", handler);
    try {
      const field = screen.getByTestId("composer-input") as HTMLTextAreaElement;
      await waitFor(() => expect(document.activeElement === field).toBe(true));
      await user.keyboard("fix the bug");
      const ctrl = (key: string) => act(() => void fireEvent.keyDown(field, { key, ctrlKey: true }));
      ctrl("a");
      expect(field.selectionStart).toBe(0);
      ctrl("k");
      expect(field.value).toBe("");
      expect(host.window.get(DRAFT_MESSAGE).text).toBe("");
      ctrl("y");
      expect(field.value).toBe("fix the bug");
      expect(host.window.get(DRAFT_MESSAGE).text).toBe("fix the bug");
      ctrl("p");
      expect(field.selectionStart).toBe(0);
      ctrl("n");
      expect(field.selectionStart).toBe(11);
      expect(client.callsTo("prompts.history")).toHaveLength(0);
    } finally {
      document.removeEventListener("keydown", handler);
    }
  });
});
