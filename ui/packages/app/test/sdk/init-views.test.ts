/** L1: the views init files build (UI-267): `InitViews`, `Layout`, `use`, `transform`, re-runs. */

import { describe, expect, test } from "bun:test";
import type { InitApi, InitContext, InitSnapshot } from "../../src/sdk/index.ts";
import { InitViews, Layout } from "../../src/sdk/init-views.ts";
import type { LayoutNode } from "../../src/view/layout.ts";
import { testHost } from "../support/host.tsx";

type Main = (omfp: InitApi) => unknown;

const MANIFEST = {
  default: "work",
  views: {
    work: {
      title: "Work",
      layout: ["row", {}, ["tabs", { id: "main" }, "omfp:chat"], ["tabs", { id: "side" }, "omfp:changes"]],
    },
    logs: { layout: "./widgets/logs" },
  },
  panels: { clock: "./widgets/clock" },
};

function context(patch: Partial<InitContext> = {}): InitContext {
  return { workspaceId: "w1", manifest: MANIFEST, manifestError: null, repos: [], ...patch };
}

function setup(modules: Record<string, Main>) {
  const runs: string[] = [];
  const test = testHost(undefined, {
    importer: (url) => {
      runs.push(url);
      const main = modules[url];
      return main === undefined ? Promise.reject(new Error(`404 ${url}`)) : Promise.resolve({ default: main });
    },
  });
  return { ...test, runs };
}

/** The window's views as `[id, title, origin]`, the default id last. */
function shown(snapshot: InitSnapshot) {
  const views = snapshot.views?.views;
  return views === undefined
    ? undefined
    : [...views.views.map((view) => [view.id, view.title, view.init ?? "views.json"]), views.defaultId];
}

describe("InitViews", () => {
  // spec: UI-267
  test("load gives views.json's views; every change mutates them and returns them, remembering who made each view", () => {
    const views = InitViews.load(MANIFEST, null, "workspace");
    expect([views.ids, views.default, views.panels.map((panel) => panel.id)]).toEqual([
      ["logs", "work"],
      "work",
      ["clock"],
    ]);
    expect(views.get("work")?.origin).toBe("views.json");

    // The style of the docs: one call after another on the same views.
    views.add("review", { title: "Review", layout: ["row", {}, "omfp:chat", ["omfp:changes", { repo: "api" }]] });
    views.add("ops", { title: "Ops", layout: "omfp:mcp" });
    expect(
      views.update("work", (layout) => {
        layout.addTab("side", "omfp:mcp");
      }),
    ).toBe(views);
    // Chaining works too.
    expect(views.remove("logs").withDefault("review")).toBe(views);
    expect([views.ids, views.default]).toEqual([["work", "review", "ops"], "review"]);
    // A views.json view the file changed is still views.json's.
    expect(views.get("work")).toMatchObject({ title: "Work", origin: "views.json" });
    expect(views.get("work")?.layout?.node).toEqual([
      "row",
      {},
      ["tabs", { id: "main" }, "omfp:chat"],
      ["tabs", { id: "side" }, "omfp:changes", "omfp:mcp"],
    ]);
    expect(views.listed.map((view) => [view.id, view.init])).toEqual([
      ["work", undefined],
      ["review", "workspace"],
      ["ops", "workspace"],
    ]);
    // The layout `get` gives changes its view.
    views.get("review")?.layout?.removeWidget("omfp:chat");
    expect(views.get("review")?.layout?.node).toEqual(["row", {}, ["omfp:changes", { repo: "api" }]]);
    // A copy changes alone.
    InitViews.copy(views).remove("ops");
    expect(views.ids).toEqual(["work", "review", "ops"]);
    // Adding an existing id replaces it in place: the file made it now.
    expect(views.add("work", { layout: "omfp:chat" }).ids).toEqual(["work", "review", "ops"]);
    expect(views.get("work")?.origin).toBe("workspace");
  });

  // spec: UI-267
  test("an invalid or missing views.json loads the default view; invalid changes throw", () => {
    expect(InitViews.load(null, null, "user").ids).toEqual(["default"]);
    expect(InitViews.load(MANIFEST, "views: no view", "user").ids).toEqual(["default"]);
    const views = InitViews.load(MANIFEST, null, "user");
    expect(() => views.add("bad", { layout: ["row", {}] })).toThrow(
      "invalid view: views.bad.layout: row has no children",
    );
    expect(() => views.add("bad", { view: "./views/ops.tsx" } as never)).toThrow("invalid view: views.bad.layout:");
    expect(() => views.update("nope", (layout) => layout)).toThrow('no view "nope"');
    expect(() => views.update("work", (layout) => layout.addTab("nope", "omfp:mcp"))).toThrow(
      'no tabs with "id": "nope" in the layout',
    );
    expect(() => views.update("work", () => ["row", {}] as unknown as LayoutNode)).toThrow("row has no children");
    expect(views.ids).toEqual(["logs", "work"]);
    expect(() => views.remove("logs").remove("work")).toThrow("cannot remove the last view");
    expect(() => views.withDefault("nope")).toThrow('no view "nope"');
    // A layout of a view removed since changes nothing.
    const fresh = InitViews.load(MANIFEST, null, "user");
    const stale = fresh.get("logs")?.layout;
    fresh.remove("logs");
    expect(() => stale?.removeWidget("nope")).toThrow('no view "logs"');
  });

  // spec: UI-267
  test("Layout edits change the layout and return it: addTab drops stale sizes, removeWidget emptied containers", () => {
    const node: LayoutNode = [
      "row",
      { sizes: [3, 1] },
      "omfp:chat",
      ["tabs", { id: "side", sizes: [1] }, "omfp:changes"],
    ];
    const layout = new Layout(node);
    expect(layout.addTab("side", new Layout("omfp:mcp"))).toBe(layout);
    expect(layout.node).toEqual([
      "row",
      { sizes: [3, 1] },
      "omfp:chat",
      ["tabs", { id: "side" }, "omfp:changes", "omfp:mcp"],
    ]);
    expect(new Layout(node).removeWidget("omfp:changes").node).toEqual(["row", { sizes: [3] }, "omfp:chat"]);
    expect(() => new Layout(node).removeWidget("omfp:chat").removeWidget("omfp:changes")).toThrow(
      "would leave the layout empty",
    );
    expect(() => new Layout(["row", {}] as unknown as LayoutNode)).toThrow("row has no children");
  });
});

describe("views of init files", () => {
  // spec: UI-267
  test("the workspace's use wins, then transforms apply, the user's first; failures skip the transform", async () => {
    const { host } = setup({
      "/u": (omfp) => {
        // A transform changes the views it gets, or returns them.
        omfp.views.transform((views) => {
          views.update("work", (layout) => layout.addTab("side", "omfp:mcp"));
        });
        omfp.views.transform((views) => views.add("mine", { layout: "omfp:todos" }));
      },
      "/w": (omfp) => {
        const views = omfp.views.load("views.json");
        views.add("review", { title: "Review", layout: "omfp:chat" });
        views.withDefault("review");
        omfp.views.use(views);
        // `use` keeps the views as they were.
        views.remove("logs");
        omfp.views.transform((next) => next.remove("work"));
      },
    });
    host.init.setContext(context());
    expect(host.init.getSnapshot().views).toBeUndefined();
    await host.init.load("user", "/u");
    expect(shown(host.init.getSnapshot())).toEqual([
      ["logs", "logs", "views.json"],
      ["work", "Work", "views.json"],
      ["mine", "mine", "user"],
      "work",
    ]);
    await host.init.load("workspace", "/w", "w1");
    expect(shown(host.init.getSnapshot())).toEqual([
      ["logs", "logs", "views.json"],
      ["review", "Review", "workspace"],
      ["mine", "mine", "user"],
      "review",
    ]);
    // Computing the views again applies the transforms to the used views again, not to their last result.
    host.init.setContext(context());
    expect(shown(host.init.getSnapshot())?.length).toBe(4);
    expect(host.init.getSnapshot().errors).toEqual([]);
    expect(host.init.getSnapshot().views?.views.panels.map((panel) => panel.id)).toEqual(["clock"]);
    expect(host.init.getSnapshot().views?.manifest).toBe(MANIFEST);

    // A transform that throws or returns something else is skipped and is its file's error.
    const failing = setup({
      "/u": (omfp) => {
        omfp.views.transform((views) => {
          views.add("half", { layout: "omfp:chat" });
          throw new Error("oops");
        });
        omfp.views.transform(() => ({}) as InitViews);
        omfp.views.transform((views) => views.add("kept", { layout: "omfp:chat" }));
      },
    });
    failing.host.init.setContext(context());
    await failing.host.init.load("user", "/u");
    expect(failing.host.init.getSnapshot().errors).toEqual([
      { file: "user", message: "its views transform failed: oops" },
    ]);
    expect(shown(failing.host.init.getSnapshot())?.map((row) => (Array.isArray(row) ? row[0] : row))).toEqual([
      "logs",
      "work",
      "kept",
      "work",
    ]);
    // Another run without transforms leaves views.json's views.
    await failing.host.init.load("user", undefined);
    expect(failing.host.init.getSnapshot().views).toBeUndefined();
    expect(failing.host.init.getSnapshot().errors).toEqual([]);
  });

  // spec: UI-267
  test("a file runs again when the views.json or the repositories it read change, not otherwise", async () => {
    const seen: string[] = [];
    const { host, runs } = setup({
      "/w": (omfp) => {
        const views = omfp.views.load("views.json");
        const repos = omfp.workspace.repos;
        seen.push(
          `${omfp.workspace.id}: ${views.ids.join(",")} | ${repos.map((repo) => `${repo.name}:${repo.kind}`).join(",")}`,
        );
        omfp.views.use(
          repos.reduce(
            (next, repo) => next.add(`changes-${repo.name}`, { layout: ["omfp:changes", { repo: repo.name }] }),
            views,
          ),
        );
      },
      "/u": () => {
        seen.push("user");
      },
    });
    host.init.setContext(context({ repos: [{ name: "api", path: "api", kind: "link" }] }));
    await host.init.load("user", "/u");
    await host.init.load("workspace", "/w", "w1");
    expect(seen).toEqual(["user", "w1: logs,work | api:link"]);

    host.init.setContext(context({ repos: [{ name: "api", path: "api", kind: "link" }] }));
    await host.init.settled();
    expect(seen.length).toBe(2);

    host.init.setContext(
      context({
        repos: [
          { name: "api", path: "api", kind: "link" },
          { name: "web", path: "web", kind: "worktree" },
        ],
      }),
    );
    await host.init.settled();
    expect(seen.at(-1)).toBe("w1: logs,work | api:link,web:worktree");
    expect(shown(host.init.getSnapshot())?.slice(-3)).toEqual([
      ["changes-api", "changes-api", "workspace"],
      ["changes-web", "changes-web", "workspace"],
      "work",
    ]);

    const edited = { ...MANIFEST, views: { ...MANIFEST.views, more: { layout: "omfp:chat" } } };
    host.init.setContext(
      context({
        manifest: edited,
        repos: [
          { name: "api", path: "api", kind: "link" },
          { name: "web", path: "web", kind: "worktree" },
        ],
      }),
    );
    await host.init.settled();
    expect(seen.at(-1)).toBe("w1: logs,more,work | api:link,web:worktree");
    // The user's file read neither: it ran once.
    expect(seen.filter((line) => line === "user").length).toBe(1);
    expect(runs.filter((url) => url === "/w").length).toBe(3);
  });

  // spec: UI-267
  test("only views.json loads, and use takes loaded views", async () => {
    const { host } = setup({
      "/a": (omfp) => {
        omfp.views.load("other.json" as "views.json");
      },
      "/b": (omfp) => {
        omfp.views.use({} as InitViews);
      },
    });
    await host.init.load("user", "/a");
    expect(host.init.getSnapshot().errors).toEqual([
      { file: "user", message: 'only "views.json" can be loaded, not "other.json"' },
    ]);
    await host.init.load("user", "/b");
    expect(host.init.getSnapshot().errors).toEqual([
      { file: "user", message: "expected views from omfp.views.load(…)" },
    ]);
  });
});
