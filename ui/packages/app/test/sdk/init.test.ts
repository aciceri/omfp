/** L1: init files' runs (UI-260..UI-263, UI-266) over a fake importer. */

import { describe, expect, test } from "bun:test";
import type { ThreadSummary } from "@omfp/protocol";
import { contextMenuEntries, contextTargetAt, type InitApi, NOTICES, runContextEntry } from "../../src/sdk/index.ts";
import { thread } from "../support/fixtures.ts";
import { ManualTimers, testHost } from "../support/host.tsx";

type Main = (omfp: InitApi) => unknown;

/** A host whose importer answers `modules` by URL (a missing URL rejects like a 404). */
function setup(modules: Record<string, unknown>) {
  const imported: string[] = [];
  const test = testHost(undefined, {
    noticeTimers: new ManualTimers(),
    importer: (url) => {
      imported.push(url);
      return url in modules ? Promise.resolve(modules[url]) : Promise.reject(new Error(`404 ${url}`));
    },
  });
  return { ...test, imported };
}

const module = (main: Main) => ({ default: main });

describe("init files", () => {
  // spec: UI-260, UI-261
  test("each file runs once per version; a new version cleans the previous run up first, then runs", async () => {
    const log: string[] = [];
    const version =
      (name: string): Main =>
      (omfp) => {
        log.push(`run ${name} ${omfp.file}`);
        omfp.command({
          id: "my.hello",
          title: `Hello ${name}`,
          run: () => {
            log.push(`hello ${name}`);
          },
        });
        omfp.onCleanup(() => log.push(`onCleanup ${name}`));
        return () => log.push(`cleanup ${name}`);
      };
    const { host, imported } = setup({ "/u/1": module(version("one")), "/u/2": module(version("two")) });
    await host.init.load("user", "/u/1");
    await host.init.load("user", "/u/1");
    expect(imported).toEqual(["/u/1"]);
    expect(host.commands.resolve("my.hello")?.title).toBe("Hello one");
    expect(host.commands.resolve("my.hello")?.init).toBe("user");
    host.commands.run("my.hello");

    await host.init.load("user", "/u/2");
    // The returned cleanup runs first (newest first), then the registrations go.
    expect(log).toEqual(["run one user", "hello one", "cleanup one", "onCleanup one", "run two user"]);
    expect(
      host.commands
        .list()
        .filter((command) => command.id === "my.hello")
        .map((c) => c.title),
    ).toEqual(["Hello two"]);

    await host.init.load("user", undefined);
    expect(log.at(-1)).toBe("onCleanup two");
    expect(host.commands.resolve("my.hello")).toBeUndefined();
  });

  // spec: UI-260, UI-262
  test("the user's file loads before the workspace's; bindings are kept per file and removed with the run", async () => {
    const order: string[] = [];
    const { host } = setup({
      "/u": module((omfp) => {
        order.push("user");
        omfp.bind({ keys: "Space  h", command: "my.hello", args: ["user"] });
      }),
      "/w1": module((omfp) => {
        order.push(`workspace ${omfp.workspaceId}`);
        omfp.bind([
          { keys: "Space h", command: "my.hello", args: ["workspace"] },
          { mode: "insert", keys: "Ctrl-h", command: "" },
        ]);
      }),
    });
    const user = host.init.load("user", "/u");
    const workspace = host.init.load("workspace", "/w1", "w1");
    await Promise.all([user, workspace]);
    expect(order).toEqual(["user", "workspace w1"]);
    expect(host.init.getSnapshot().bindings).toEqual({
      user: [{ mode: "normal", keys: "Space h", command: "my.hello", args: ["user"] }],
      workspace: [
        { mode: "normal", keys: "Space h", command: "my.hello", args: ["workspace"] },
        { mode: "insert", keys: "Ctrl-h", command: "" },
      ],
    });
    // The same file for another workspace runs again.
    await host.init.load("workspace", "/w1", "w2");
    expect(order).toEqual(["user", "workspace w1", "workspace w2"]);
    await host.init.load("workspace", undefined, "w3");
    expect(host.init.getSnapshot().bindings.workspace).toEqual([]);
  });

  // spec: UI-263
  test("a compile error, a failed import, a throw, a rejected promise or a failing command becomes the file's error", async () => {
    const { host } = setup({
      "/compile": { compileError: { file: "init.ts", line: 3, column: 7, message: "Unexpected token" } },
      "/nothing": {},
      "/throws": module(() => {
        throw new Error("boom");
      }),
      "/rejects": module(async () => {
        throw new Error("later");
      }),
      "/command": module((omfp) => {
        omfp.command({
          id: "my.broken",
          title: "Broken",
          run: () => {
            throw new Error("nope");
          },
        });
      }),
    });
    const errors = () => host.init.getSnapshot().errors;
    await host.init.load("user", "/compile");
    expect(errors()).toEqual([{ file: "user", message: "init.ts:3:7: Unexpected token" }]);
    await host.init.load("user", "/missing");
    expect(errors()).toEqual([{ file: "user", message: "cannot load it: 404 /missing" }]);
    await host.init.load("user", "/nothing");
    expect(errors()[0]?.message).toContain("no default export");
    await host.init.load("user", "/throws");
    expect(errors()).toEqual([{ file: "user", message: "boom" }]);
    await host.init.load("workspace", "/rejects", "w1");
    await Promise.resolve();
    expect(errors()).toEqual([
      { file: "user", message: "boom" },
      { file: "workspace", message: "later" },
    ]);

    await host.init.load("user", "/command");
    expect(errors()).toEqual([{ file: "workspace", message: "later" }]);
    expect(host.commands.run("my.broken")).toBe(true);
    expect(errors()).toEqual([
      { file: "user", message: "its command my.broken failed: nope" },
      { file: "workspace", message: "later" },
    ]);
  });

  // spec: UI-261, UI-266, UI-267
  test("thread listeners, notices, views and panels; invalid layouts are refused", async () => {
    const seen: string[] = [];
    const { host } = setup({
      "/u": module((omfp) => {
        omfp.on("attention", (summary: ThreadSummary) => {
          seen.push(`attention ${summary.id}`);
          omfp.notify({ title: summary.title ?? "", body: "auto", threadId: summary.id });
        });
        omfp.on("finished", () => {
          throw new Error("listener bug");
        });
        omfp.views.transform((views) =>
          views.add("logs", { title: "Logs", layout: ["row", {}, "omfp:chat", "./widgets/logs"] }),
        );
        omfp.panel("clock", ["./widgets/clock", { title: "Clock" }]);
      }),
      "/bad": module((omfp) => {
        omfp.views.use(omfp.views.load("views.json").add("broken", { layout: ["row", {}, ""] }));
      }),
    });
    await host.init.load("user", "/u");
    const summary = thread("t1", "checkout", "w1");
    host.init.emit("attention", summary);
    host.init.emit("failed", summary);
    expect(seen).toEqual(["attention t1"]);
    expect(host.window.get(NOTICES).map((notice) => [notice.title, notice.body, notice.threadId])).toEqual([
      ["checkout", "auto", "t1"],
    ]);
    host.init.emit("finished", summary);
    expect(host.init.getSnapshot().errors).toEqual([
      { file: "user", message: 'its "finished" listener failed: listener bug' },
    ]);

    const snapshot = host.init.getSnapshot();
    expect(snapshot.views?.views.views.map((view) => [view.id, view.title, view.init])).toEqual([
      ["default", "Default", undefined],
      ["logs", "Logs", "user"],
    ]);
    expect(snapshot.panels.map((panel) => [panel.id, panel.title, panel.init])).toEqual([["clock", "Clock", "user"]]);
    host.init.closePanel("clock");
    expect(host.init.getSnapshot().panels).toEqual([]);

    await host.init.load("workspace", "/bad", "w1");
    expect(host.init.getSnapshot().errors.find((error) => error.file === "workspace")?.message).toBe(
      "invalid view: views.broken.layout[2]: a widget name is empty",
    );
  });

  // spec: UI-261, UI-319
  test("contextMenu adds items after the defaults, the user's file first; a rerun drops them; a bad menu is an error", async () => {
    const ran: string[] = [];
    const item =
      (file: string): Main =>
      (omfp) => {
        omfp.command({
          id: `my.${file}`,
          title: `Mine ${file}`,
          run: (args) => void ran.push(`${file} ${args.join(" ")}`),
        });
        omfp.contextMenu({ on: "thread", items: [{ command: `my.${file}`, title: `Pin (${file})` }] });
      };
    const { host } = setup({
      "/u": module(item("user")),
      "/w": module(item("workspace")),
      "/bad": module((omfp) => {
        omfp.contextMenu({ on: "web" as "thread", items: ["my.user"] });
      }),
    });
    await host.init.load("workspace", "/w", "w1");
    await host.init.load("user", "/u");
    const element = document.createElement("div");
    element.setAttribute("data-omfp-thread", "t1");
    const target = contextTargetAt(element);
    if (target === undefined) {
      throw new Error("no target");
    }
    const entries = contextMenuEntries(host.commands, host.contextMenus, target);
    // No built-in command is registered here: only the files' items, the user's first.
    expect(entries.map((entry) => entry.title)).toEqual(["Pin (user)", "Pin (workspace)"]);
    for (const entry of entries) {
      runContextEntry(host.commands, entry, target);
    }
    expect(ran).toEqual(["user t1", "workspace t1"]);

    await host.init.load("user", "/bad");
    expect(contextMenuEntries(host.commands, host.contextMenus, target).map((entry) => entry.title)).toEqual([
      "Pin (workspace)",
    ]);
    expect(host.init.getSnapshot().errors).toEqual([
      { file: "user", message: 'A context menu\'s "on" is "message", "file" or "thread", not "web"' },
    ]);
    expect(() => host.contextMenus.register({ on: "file", items: [{ command: "" }] })).toThrow(
      'A context menu item is a command\'s id or alias, or { command, title? }, not {"command":""}',
    );
  });
});
