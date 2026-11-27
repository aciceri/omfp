/** L4: a window opening on what its address bar named (`omfp desktop --thread/--workspace/--view/--prompt`). */

import { describe, expect, test } from "bun:test";
import { DRAFT_MESSAGE, DRAFT_THREAD, ERRORS, SELECTED_THREAD } from "../../src/sdk/index.ts";
import { VIEW_CHOICES } from "../../src/sdk/keys.ts";
import { applyLaunch } from "../../src/sdk/launch.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { testHost } from "../support/host.tsx";

describe("launch", () => {
  // spec: UI-218
  test("a thread is selected at once, with the view named", () => {
    const { host } = testHost();
    applyLaunch(host, { thread: "t1", view: "web" });
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
    expect(host.window.get(VIEW_CHOICES).threads).toEqual([{ threadId: "t1", viewId: "web" }]);
  });

  // spec: UI-214
  test("a workspace opens its new thread on the view named, the prompt unsent in its composer, once the index is live", () => {
    const test = testHost();
    const { host, client } = test;
    applyLaunch(host, { workspace: "home", view: "web", prompt: "hello" });
    test.flush();
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
    client.connected();
    client.index({
      workspaces: [{ ...workspace("w1"), name: "Home" }],
      threads: [thread("t1", "Old", "w1")],
    });
    test.flush();
    expect(host.window.get(DRAFT_THREAD)?.workspaceId).toBe("w1");
    expect(host.window.get(SELECTED_THREAD)).toBeUndefined();
    expect(host.window.get(VIEW_CHOICES).draft).toBe("web");
    expect(host.window.get(DRAFT_MESSAGE).text).toBe("hello");
    expect(client.callsTo("threads.create")).toHaveLength(0);
    // Once only: a later index change leaves the window's own choices alone.
    host.window.set(DRAFT_THREAD, undefined);
    client.index({ workspaces: [{ ...workspace("w1"), name: "Home" }] });
    test.flush();
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
  });

  // spec: UI-214
  test("a workspace the daemon lacks is reported, and nothing opens", () => {
    const test = testHost();
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1")] });
    test.flush();
    applyLaunch(test.host, { workspace: "nope", view: "web" });
    expect(test.host.window.get(DRAFT_THREAD)).toBeUndefined();
    expect(test.host.window.get(VIEW_CHOICES).draft).toBeUndefined();
    expect(test.host.window.get(ERRORS).map((error) => error.text)).toEqual([
      "No workspace named “nope”: give its id, name or folder.",
    ]);
  });
});
