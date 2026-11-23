/** L4: the frame (banner, views, keymap, `:` line, theme, live region, notifications, dialogs) over a fake client. */

import { afterEach, beforeEach, describe, expect, jest, spyOn, test } from "bun:test";
import type { CallMethod } from "@omfp/client";
import { ERROR_TOAST_MS } from "@omfp/kit";
import type { BuiltinTheme, Settings, ThreadSummary, ViewsState } from "@omfp/protocol";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Frame, type FrameProps } from "../../src/frame/frame.tsx";
import { LOGIN_RUN } from "../../src/frame/login.tsx";
import type { NotificationPort } from "../../src/frame/notifications.ts";
import { COARSE_POINTER, SHORT_TOUCH_SCREEN } from "../../src/frame/responsive.ts";
import { Toasts } from "../../src/frame/status.tsx";
import {
  chooseView,
  currentWorkspaceId,
  DRAFT_THREAD,
  ERRORS,
  FOCUS_COMPOSER,
  FRAME_LAYOUT,
  type KeyValueStorage,
  MESSAGES,
  NOTICES,
  type Notice,
  type OmfpHost,
  REVEAL_WIDGET,
  reportError,
  SELECTED_THREAD,
  SUCCESSES,
  useCommand,
  useNotify,
  type WidgetComponent,
} from "../../src/sdk/index.ts";
import { VIEW_CHOICES } from "../../src/sdk/keys.ts";
import { markTabEdges } from "../../src/view/render.tsx";
import type { WidgetRegistry } from "../../src/view/resolve.ts";
import { DEFAULT_MANIFEST } from "../../src/view/views.ts";
import { COLLAPSED_WORKSPACES } from "../../src/widgets/threads/index.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { type FakeClient, ManualTimers, released, renderWithHost, testHost } from "../support/host.tsx";
import { installResizeObserver, resize } from "../support/resize.ts";

afterEach(() => {
  cleanup();
  document.head.innerHTML = "";
  delete document.documentElement.dataset.colorScheme;
  delete document.documentElement.dataset.density;
  delete document.documentElement.dataset.motion;
  delete document.documentElement.dataset.skin;
  delete document.documentElement.dataset.theme;
  document.documentElement.removeAttribute("style");
});

const stub =
  (name: string): WidgetComponent =>
  () => <p data-testid={`stub-${name}`}>{name}</p>;

const registry: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", stub("header")],
  ["omfp:chat", stub("chat")],
  ["omfp:workspace", stub("workspace")],
  ["omfp:transcript", stub("transcript")],
]);

/** A text field `i` (`composer.focus`) focuses. */
const StubComposer: WidgetComponent = () => {
  useCommand({ id: "composer.focus", title: "Focus", run: () => document.getElementById("stub-input")?.focus() });
  return <textarea id="stub-input" data-testid="stub-input" />;
};

/** A widget that notifies through the SDK (UI-194). */
const Notifying: WidgetComponent = () => {
  const notify = useNotify();
  return (
    <button type="button" onClick={() => notify({ title: "Build", body: "is green", threadId: "t2" })}>
      notify
    </button>
  );
};

const threads: ThreadSummary[] = [thread("t1", "checkout", "w1"), thread("t2", "flaky", "w1")];

function setup(props: FrameProps = {}, connect = true, options: { storage?: KeyValueStorage } = {}) {
  // Module widgets (workspace overrides) stay loading: these tests are about the frame.
  const timers = new ManualTimers();
  const noticeTimers = new ManualTimers();
  const base = testHost(undefined, {
    importer: () => new Promise<unknown>(() => {}),
    reportTimers: timers,
    noticeTimers,
    ...options,
  });
  for (const method of [
    "threads.archive",
    "threads.delete",
    "settings.set",
    "widgets.report",
  ] as const satisfies readonly CallMethod[]) {
    base.client.answers.set(method, () => ({}));
  }
  let clock = 0;
  const test = renderWithHost(<Frame registry={registry} now={() => clock} {...props} />, base);
  if (connect) {
    test.client.connected();
    test.client.index({ workspaces: [workspace("w1"), workspace("w2")], threads });
  }
  test.flush();
  return {
    ...test,
    timers,
    noticeTimers,
    /** Runs the report timers and gives the `widgets.report` params sent so far, without the window state (UI-350,
     * tested in window-requests.test.tsx). */
    reports: () => {
      timers.run();
      return test.client
        .callsTo("widgets.report")
        .map((call) => call.params as Record<string, unknown>)
        .map(({ window: _window, ...report }) => report);
    },
    user: userEvent.setup(),
    tick: (ms: number) => {
      clock += ms;
    },
  };
}

function press(key: string, init: KeyboardEventInit = {}): void {
  act(() => {
    fireEvent.keyDown(document.activeElement ?? document.body, { key, ...init });
  });
}

/** `matchMedia` answering `(prefers-reduced-motion: reduce)` from a switch the test flips; other queries never match. */
class SystemMotion {
  reduce = false;
  readonly listeners = new Set<() => void>();

  readonly matchMedia = (query: string): MediaQueryList => {
    const system = this;
    const list: Pick<MediaQueryList, "media" | "matches" | "addEventListener" | "removeEventListener"> = {
      media: query,
      get matches() {
        return query === "(prefers-reduced-motion: reduce)" && system.reduce;
      },
      addEventListener: (_type: string, listener: unknown) => system.listeners.add(listener as () => void),
      removeEventListener: (_type: string, listener: unknown) => system.listeners.delete(listener as () => void),
    };
    return list as MediaQueryList;
  };

  set(reduce: boolean): void {
    this.reduce = reduce;
    for (const listener of this.listeners) {
      listener();
    }
  }
}

/** The widgets the window shows: a `tabs` container's other tabs stay mounted, hidden (UI-400). */
function shownWidgets(): (string | undefined)[] {
  return screen
    .getAllByTestId("widget")
    .filter((widget) => widget.closest("[hidden]") === null)
    .map((widget) => widget.dataset.widget);
}

/** `matchMedia` answering the queries listed in `matching` (a screen's pointer and size); others never match. */
class SystemScreen {
  matching = new Set<string>();
  readonly listeners = new Set<() => void>();

  readonly matchMedia = (query: string): MediaQueryList => {
    const system = this;
    const list: Pick<MediaQueryList, "media" | "matches" | "addEventListener" | "removeEventListener"> = {
      media: query,
      get matches() {
        return system.matching.has(query);
      },
      addEventListener: (_type: string, listener: unknown) => system.listeners.add(listener as () => void),
      removeEventListener: (_type: string, listener: unknown) => system.listeners.delete(listener as () => void),
    };
    return list as MediaQueryList;
  };

  set(...matching: string[]): void {
    this.matching = new Set(matching);
    for (const listener of this.listeners) {
      listener();
    }
  }
}

/** Selects `t1` (workspace `w1`) and answers its views topic with `state`. */
function inWorkspace(
  state: Partial<ViewsState> = {},
  props: FrameProps = {},
  options: { storage?: KeyValueStorage } = {},
) {
  const test = setup(props, true, options);
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  test.client.views("w1", state);
  test.flush();
  return test;
}

/** Selects `threadId`. */
function select(test: { host: OmfpHost; flush: () => void }, threadId: string | undefined): void {
  act(() => test.host.window.set(SELECTED_THREAD, threadId));
  test.flush();
}

function shownView(): string | undefined {
  return screen.getByTestId("view").dataset.view;
}

const MANIFEST = {
  default: "work",
  views: {
    work: { title: "Work", layout: ["row", {}, "omfp:chat", "omfp:workspace"] },
    review: { title: "Review", layout: ["col", {}, "omfp:transcript", "omfp:chat"] },
    lean: { title: "Lean", layout: "omfp:header" },
  },
};

describe("frame", () => {
  // spec: UI-274
  test("a reveal request selects the tab holding the widget, else floats it in a panel of this window", async () => {
    const tabs = {
      views: { work: { layout: ["row", {}, "omfp:chat", ["tabs", {}, "omfp:workspace", "omfp:background"]] } },
    };
    const withBackground = { registry: new Map([...registry, ["omfp:background", stub("background")]]) };
    const test = inWorkspace({ manifest: tabs }, withBackground);
    // The tab not selected keeps its widget mounted, hidden (UI-400).
    const hidden = () => screen.getByTestId("stub-background").closest("[hidden]") !== null;
    expect(hidden()).toBe(true);
    act(() => test.host.window.set(REVEAL_WIDGET, { reference: "omfp:background", nonce: 1 }));
    test.flush();
    await waitFor(() => expect(hidden()).toBe(false));
    expect(screen.queryByTestId("floating-widget")).toBeNull();
    // The view showed it: the request is done.
    expect(test.host.window.get(REVEAL_WIDGET)).toBeUndefined();
    cleanup();

    const without = inWorkspace(
      { manifest: { views: { work: { layout: ["row", {}, "omfp:chat", "omfp:workspace"] } } } },
      withBackground,
    );
    act(() => without.host.window.set(REVEAL_WIDGET, { reference: "omfp:background", nonce: 2 }));
    without.flush();
    const panel = screen.getByTestId("floating-widget");
    expect(within(panel).getByTestId("floating-widget-title").textContent).toBe("Background");
    expect(within(panel).queryByTestId("stub-background")).not.toBeNull();
    await without.user.click(within(panel).getByTestId("floating-widget-close"));
    expect(screen.queryByTestId("floating-widget")).toBeNull();
    expect(without.client.callsTo("views.unmount")).toEqual([]);
  });

  // spec: UI-452
  test("a side tab bar is the main slot's one-row bar and scrolls its selected tab into view", async () => {
    const tabs = {
      views: { work: { layout: ["row", {}, "omfp:chat", ["tabs", {}, "omfp:workspace", "omfp:background"]] } },
    };
    const withBackground = { registry: new Map([...registry, ["omfp:background", stub("background")]]) };
    const scrolled: Array<string | null> = [];
    const scroll = spyOn(Element.prototype, "scrollIntoView").mockImplementation(function (this: Element, options) {
      expect(options).toEqual({ block: "nearest", inline: "nearest" });
      scrolled.push(this.textContent);
    });
    try {
      const test = inWorkspace({ manifest: tabs }, withBackground);
      const bar = document.querySelector('.omfp-view__tabs:not([data-slot]) > .omfp-view__tabbar > [role="tablist"]');
      expect(bar?.querySelectorAll('[role="tab"]').length).toBe(2);
      act(() => test.host.window.set(REVEAL_WIDGET, { reference: "omfp:background", nonce: 1 }));
      test.flush();
      await waitFor(() => expect(scrolled.at(-1)).toBe("Background"));
    } finally {
      scroll.mockRestore();
    }
  });

  // spec: UI-452
  test("a tab list too wide for its bar scrolls sideways under the wheel and marks the edges with more tabs", () => {
    const tabs = {
      views: { work: { layout: ["row", {}, "omfp:chat", ["tabs", {}, "omfp:workspace", "omfp:background"]] } },
    };
    inWorkspace({ manifest: tabs }, { registry: new Map([...registry, ["omfp:background", stub("background")]]) });
    const list = document.querySelector<HTMLElement>('.omfp-view__tabs:not([data-slot]) [role="tablist"]');
    if (list === null) {
      throw new Error("no tab list");
    }
    // happy-dom lays nothing out: a list 100 px wide over 300 px of tabs.
    Object.defineProperty(list, "clientWidth", { value: 100 });
    Object.defineProperty(list, "scrollWidth", { value: 300 });
    const edges = () => [list.hasAttribute("data-more-start"), list.hasAttribute("data-more-end")];
    markTabEdges(list);
    expect(edges()).toEqual([false, true]);
    const wheel = new WheelEvent("wheel", { deltaY: 120, cancelable: true });
    act(() => {
      list.dispatchEvent(wheel);
    });
    expect([list.scrollLeft, wheel.defaultPrevented]).toEqual([120, true]);
    // Ctrl with the wheel is the page's zoom, not the list's (happy-dom's WheelEvent takes no modifier keys).
    const zoom = new WheelEvent("wheel", { deltaY: 120, cancelable: true });
    Object.defineProperty(zoom, "ctrlKey", { value: true });
    act(() => {
      list.dispatchEvent(zoom);
    });
    expect([list.scrollLeft, zoom.defaultPrevented]).toEqual([120, false]);
    markTabEdges(list);
    expect(edges()).toEqual([true, true]);
    list.scrollLeft = 200;
    markTabEdges(list);
    expect(edges()).toEqual([true, false]);
  });

  // spec: UI-024, UI-100, UI-303
  test("the left column is the frame's; the default view renders its widgets; one view needs no view tabs", () => {
    setup();
    const left = screen.getByTestId("left-column");
    expect(within(left).getByTestId("sidebar")).toBeDefined();
    expect(left.contains(screen.getByTestId("view"))).toBe(false);
    expect(screen.getByTestId("view").dataset.view).toBe("default");
    expect(shownWidgets()).toEqual(["omfp:header", "omfp:chat", "omfp:workspace"]);
    expect(screen.queryByTestId("mode-views")).toBeNull();
    expect(screen.queryByTestId("layout-edit")).toBeNull();
    expect(screen.queryByTestId("view-switcher")).toBeNull();
    expect(screen.queryByTestId("safety-net")).toBeNull();
  });

  // spec: UI-003
  test("the banner shows while connecting and goes away once connected, without blocking the view", () => {
    const { client, flush } = setup({}, false);
    expect(screen.getByTestId("connection-banner").dataset.tone).toBe("info");
    expect(screen.getByTestId("view")).toBeDefined();
    client.connected();
    flush();
    expect(screen.queryByTestId("connection-banner")).toBeNull();
    act(() => client.model.setConnection({ kind: "reconnecting", attempt: 2, retryInMs: 1500, error: "closed" }));
    client.notify();
    flush();
    expect(screen.getByTestId("connection-banner").textContent).toBe(
      "Connection lost. Reconnecting in 2 s (attempt 2)…",
    );
  });

  // spec: UI-023, UI-095
  test("a `:` line opened before the index arrives keeps the focus when the column then shows no workspace", () => {
    const { client, flush } = setup({}, false);
    client.connected();
    flush();
    press(":");
    expect(document.activeElement?.getAttribute("data-testid")).toBe("command-line-input");
    client.index({ workspaces: [], threads: [] });
    flush();
    expect(screen.getByTestId("no-workspaces-hint")).toBeDefined();
    expect(document.activeElement?.getAttribute("data-testid")).toBe("command-line-input");
  });

  // spec: UI-095, UI-091, UI-094, UI-097
  test("`:` opens the command line; `:theme dark` is window-local, `:density normal` goes to the daemon", async () => {
    const { client, flush, user } = setup();
    expect(document.documentElement.dataset.colorScheme).toBe("system");
    expect(document.documentElement.dataset.density).toBe("compact");
    press(":");
    const input = screen.getByTestId("command-line-input");
    expect(document.activeElement === input).toBe(true);
    await user.type(input, "the");
    expect(screen.getAllByTestId("command-option").map((option) => option.dataset.value)).toEqual(["theme"]);
    await user.keyboard("{Tab}");
    expect((input as HTMLInputElement).value).toBe("theme ");
    expect(screen.getAllByTestId("command-option").map((option) => option.dataset.value)).toEqual([
      "theme system",
      "theme light",
      "theme dark",
      "theme synth",
      "theme catppuccin",
      "theme gruvbox",
      "theme tokyo-night",
      "theme nord",
      "theme solarized",
      "theme rose-pine",
      "theme graphite",
      "theme paper",
      "theme clay",
      "theme primer",
      "theme accent",
      "theme matugen",
    ]);
    await user.type(input, "d{Enter}");
    expect(screen.queryByTestId("command-line")).toBeNull();
    expect(document.documentElement.dataset.colorScheme).toBe("dark");
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "density normal{Enter}");
    expect(client.callsTo("settings.set").map((call) => call.params)).toEqual([{ ui: { density: "normal" } }]);
    expect(document.documentElement.dataset.density).toBe("compact");
    client.indexEvents({ kind: "settings_changed", settings: { ui: { density: "normal" }, keymap: [], error: null } });
    flush();
    expect(document.documentElement.dataset.density).toBe("normal");
  });

  // spec: UI-513, UI-510, UI-511
  test("`:theme <theme> <scheme>` sets the daemon's theme and the window's scheme; `<html>` follows the setting", async () => {
    const { client, flush, host, user } = setup();
    expect(document.documentElement.dataset.theme).toBe("synth");
    press(":");
    const input = screen.getByTestId("command-line-input");
    await user.type(input, "theme gruvbox ");
    expect(screen.getAllByTestId("command-option").map((option) => option.dataset.value)).toEqual([
      "theme gruvbox system",
      "theme gruvbox light",
      "theme gruvbox dark",
    ]);
    await user.type(input, "dark{Enter}");
    expect(client.callsTo("settings.set").map((call) => call.params)).toEqual([{ ui: { theme: "gruvbox" } }]);
    expect(document.documentElement.dataset.colorScheme).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("synth");
    client.indexEvents({ kind: "settings_changed", settings: { ui: { theme: "gruvbox" }, keymap: [], error: null } });
    flush();
    expect(document.documentElement.dataset.theme).toBe("gruvbox");
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "theme dracula{Enter}");
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["Unknown colour scheme or theme: dracula"]);
    expect(client.callsTo("settings.set").length).toBe(1);
  });

  // spec: UI-513, UI-510
  test("`:theme matugen` selects the user theme, Synth without one", async () => {
    const { client, flush, user } = setup();
    client.indexEvents({ kind: "settings_changed", settings: { ui: { theme: "nord" }, keymap: [], error: null } });
    flush();
    expect(document.documentElement.dataset.theme).toBe("nord");
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "theme matugen{Enter}");
    expect(client.callsTo("settings.set").map((call) => call.params)).toEqual([{ ui: { theme: "matugen" } }]);
    client.indexEvents({ kind: "settings_changed", settings: { ui: { theme: "matugen" }, keymap: [], error: null } });
    flush();
    expect(document.documentElement.dataset.theme).toBe("synth");
    expect(document.head.querySelector("style[data-omfp-theme]")?.textContent).toBe("");
  });

  // spec: UI-999, UI-097
  test("`:voice` completes omp's voices and sets the daemon's live_voice; an unknown voice is an error", async () => {
    const { client, host, user } = setup();
    press(":");
    const input = screen.getByTestId("command-line-input");
    await user.type(input, "voice ");
    expect(screen.getAllByTestId("command-option").map((option) => option.dataset.value)).toEqual(
      ["arbor", "breeze", "cove", "ember", "juniper", "maple", "sol", "spruce", "vale"].map(
        (voice) => `voice ${voice}`,
      ),
    );
    await user.type(input, "maple{Enter}");
    expect(client.callsTo("settings.set").map((call) => call.params)).toEqual([{ ui: { liveVoice: "maple" } }]);
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "voice alloy{Enter}");
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["Invalid value for live_voice: alloy"]);
    expect(client.callsTo("settings.set").length).toBe(1);
  });

  // spec: UI-794, UI-097
  test("`:set skin=boring` goes to the daemon; `<html data-skin>` follows the setting, whatever the theme", async () => {
    const { client, flush, user } = setup();
    expect(document.documentElement.dataset.skin).toBe("crush");
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "set skin=boring{Enter}");
    expect(client.callsTo("settings.set").map((call) => call.params)).toEqual([{ ui: { skin: "boring" } }]);
    expect(document.documentElement.dataset.skin).toBe("crush");
    client.indexEvents({
      kind: "settings_changed",
      settings: { ui: { skin: "boring", theme: "nord" }, keymap: [], error: null },
    });
    flush();
    expect([document.documentElement.dataset.skin, document.documentElement.dataset.theme]).toEqual(["boring", "nord"]);
    client.indexEvents({ kind: "settings_changed", settings: { ui: {}, keymap: [], error: null } });
    flush();
    expect(document.documentElement.dataset.skin).toBe("crush");
  });

  // spec: UI-091, UI-510, UI-794
  test("`<html>` keeps the appearance the page loaded with until the first `index` snapshot gives the settings", () => {
    const root = document.documentElement;
    Object.assign(root.dataset, { theme: "graphite", skin: "boring", density: "normal", motion: "reduced" });
    root.style.fontSize = "20px";
    const { client, flush } = setup({}, false);
    client.connected();
    flush();
    const appearance = () => [
      root.dataset.colorScheme,
      root.dataset.theme,
      root.dataset.skin,
      root.dataset.density,
      root.dataset.motion,
      root.style.fontSize,
    ];
    expect(appearance()).toEqual(["system", "graphite", "boring", "normal", "reduced", "20px"]);
    client.index({ settings: { ui: { theme: "nord" }, keymap: [], error: null } });
    flush();
    expect(appearance()).toEqual(["system", "nord", "crush", "compact", "full", "17px"]);
  });

  // spec: UI-526
  test("the `:` line runs `window.theme` by its id and its alias", async () => {
    const { user } = setup();
    const schemes: Array<string | undefined> = [];
    for (const line of ["window.theme dark", "theme light"]) {
      press(":");
      await user.type(screen.getByTestId("command-line-input"), `${line}{Enter}`);
      schemes.push(document.documentElement.dataset.colorScheme);
    }
    expect(schemes).toEqual(["dark", "light"]);
  });

  // spec: UI-336
  test("the arrows scroll the highlighted option of the `:` menu into view", async () => {
    const { user } = setup();
    const scrolled: Array<string | null> = [];
    const scroll = spyOn(Element.prototype, "scrollIntoView").mockImplementation(function (this: Element, options) {
      expect(options).toEqual({ block: "nearest" });
      scrolled.push(this.getAttribute("data-value"));
    });
    try {
      press(":");
      await user.type(screen.getByTestId("command-line-input"), "theme ");
      expect(scrolled).toEqual([]);
      await user.keyboard("{ArrowDown}{ArrowDown}{ArrowUp}{ArrowUp}");
      expect(scrolled).toEqual(["theme system", "theme light", "theme system", "theme matugen"]);
    } finally {
      scroll.mockRestore();
    }
  });

  // spec: UI-095, UI-026
  test("`:archive` runs on the selected thread and Escape closes the line, giving the focus back", async () => {
    const { client, host, user } = setup();
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    const row = screen.getAllByTestId("thread-row")[0];
    row?.focus();
    press(":");
    await user.keyboard("{Escape}");
    expect(screen.queryByTestId("command-line")).toBeNull();
    expect(document.activeElement === row).toBe(true);
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "arch");
    await user.click(screen.getAllByTestId("command-option")[0] ?? document.body);
    expect(client.callsTo("threads.archive").map((call) => call.params)).toEqual([{ threadId: "t1" }]);
    expect(host.window.get(SELECTED_THREAD)).toBeUndefined();
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "nope{Enter}");
    expect(screen.getByTestId("command-line-error").textContent).toBe("Unknown command: nope");
  });

  // spec: UI-095, UI-407
  test("the `:` line neither lists nor runs a command of a scope it is not in, and says which scope runs it", async () => {
    const { host, user } = setup();
    const ran: string[] = [];
    act(() => {
      host.commands.register({
        id: "draft.only",
        title: "Only in the composer",
        aliases: ["draftonly"],
        scope: "composer",
        run: () => {
          ran.push("draft.only");
        },
      });
    });
    document.body.focus();
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "draftonl");
    expect(screen.queryAllByTestId("command-option").length).toBe(0);
    await user.type(screen.getByTestId("command-line-input"), "y{Enter}");
    expect(screen.getByTestId("command-line-error").textContent).toBe(
      "draftonly runs only with the focus in the composer scope",
    );
    expect(ran.length).toBe(0);
  });

  // spec: UI-095, UI-023, UI-021, UI-321
  test("vim keys move through threads, `Space a` reaches the waiting thread, `Space n` opens a draft, `Space w` the + workspace field", () => {
    const { client, flush, host, tick } = setup();
    document.body.focus();
    press("j");
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
    press("j");
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    press("g");
    press("g");
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
    press("G", { shiftKey: true });
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    client.indexEvents({ kind: "thread_upserted", thread: thread("t1", "checkout", "w1", { needsAttention: true }) });
    flush();
    // A sequence whose next key comes too late is dropped (the leader menu, shown at once, waits instead: UI-321).
    press("g");
    tick(5000);
    press("g");
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    press("Escape");
    press(" ");
    press("a");
    expect(host.window.get(SELECTED_THREAD)).toBe("t1");
    press(" ");
    press("n");
    expect(host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "w1" });
    expect(host.window.get(SELECTED_THREAD)).toBeUndefined();
    expect(screen.getByTestId("draft-thread")).toBeDefined();
    press(" ");
    press("w");
    expect(document.activeElement === screen.getByTestId("new-workspace-input")).toBe(true);
  });

  // spec: UI-023
  test("`:new` without a current workspace focuses the + workspace field", async () => {
    const { host, user } = setup();
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "new{Enter}");
    expect(host.window.get(DRAFT_THREAD)).toBeUndefined();
    expect(document.activeElement === screen.getByTestId("new-workspace-input")).toBe(true);
  });

  // spec: UI-095, UI-025
  test("`:rescan` completes like other commands and rescans the current workspace", async () => {
    const { client, host, user } = setup();
    client.answers.set("workspaces.rescan", () => workspace("w1"));
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "resc");
    // Without a current workspace there is nothing to rescan.
    expect(screen.queryAllByTestId("command-option").map((option) => option.dataset.value)).toEqual([]);
    await user.keyboard("{Escape}");
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "resc");
    expect(screen.getAllByTestId("command-option").map((option) => option.dataset.value)).toEqual(["rescan"]);
    await user.click(screen.getAllByTestId("command-option")[0] ?? document.body);
    expect(client.callsTo("workspaces.rescan").map((call) => call.params)).toEqual([{ workspaceId: "w1" }]);
  });

  // spec: UI-026
  test("`:delete` immediately deletes the thread and clears the selection", async () => {
    const { client, host, user } = setup();
    act(() => host.window.set(SELECTED_THREAD, "t2"));
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "delete{Enter}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(client.callsTo("threads.delete").map((call) => call.params)).toEqual([{ threadId: "t2" }]);
    expect(host.window.get(SELECTED_THREAD)).toBeUndefined();
  });

  // spec: UI-091, UI-510, DMN-240
  test("the daemon's theme replaces the bundle's link once the index is live, and follows theme_changed", () => {
    const link = document.createElement("link");
    link.setAttribute("data-omfp-theme", "");
    document.head.append(link);
    const { client, flush } = setup({}, false);
    expect(document.head.querySelector("link[data-omfp-theme]") === link).toBe(true);
    client.connected();
    const settings = (theme: BuiltinTheme): Settings => ({ ui: { theme }, keymap: [], error: null });
    client.index({ theme: { css: ":root { --omfp-primary: red; }" }, settings: settings("matugen") });
    flush();
    expect(document.head.querySelector("link[data-omfp-theme]")).toBeNull();
    expect(document.head.querySelector("style[data-omfp-theme]")?.textContent).toBe(":root { --omfp-primary: red; }");
    client.indexEvents({ kind: "theme_changed", theme: { css: null } });
    flush();
    expect(document.head.querySelector("style[data-omfp-theme]")?.textContent).toBe("");
  });

  // spec: UI-091, UI-510, DMN-240
  test("the user theme shows only while the theme is `matugen`, and follows theme_changed under any theme", () => {
    const link = document.createElement("link");
    link.setAttribute("data-omfp-theme", "");
    document.head.append(link);
    const { client, flush } = setup({}, false);
    client.connected();
    const settings = (theme: BuiltinTheme): Settings => ({ ui: { theme }, keymap: [], error: null });
    const shown = () => document.head.querySelector("style[data-omfp-theme]")?.textContent;
    const red = ":root { --omfp-primary: red; }";
    const blue = ":root { --omfp-primary: blue; }";
    client.index({ theme: { css: red }, settings: settings("nord") });
    flush();
    expect([document.head.querySelector("link[data-omfp-theme]"), shown()]).toEqual([null, ""]);
    client.indexEvents({ kind: "settings_changed", settings: settings("matugen") });
    flush();
    expect([document.documentElement.dataset.theme, shown()]).toEqual(["synth", red]);
    client.indexEvents({ kind: "settings_changed", settings: settings("gruvbox") });
    flush();
    expect([document.documentElement.dataset.theme, shown()]).toEqual(["gruvbox", ""]);
    client.indexEvents({ kind: "theme_changed", theme: { css: blue } });
    flush();
    expect(shown()).toBe("");
    client.indexEvents({ kind: "settings_changed", settings: settings("matugen") });
    flush();
    expect(shown()).toBe(blue);
    client.indexEvents({ kind: "theme_changed", theme: { css: red } });
    flush();
    expect(shown()).toBe(red);
  });

  // spec: UI-092
  test("the live region announces the selected thread's state and the threads awaiting input", () => {
    const { client, flush, host } = setup();
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    expect(screen.getByTestId("live-region").textContent).toBe("checkout: idle");
    expect(screen.getByTestId("live-region").getAttribute("aria-live")).toBe("polite");
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { needsAttention: true }) });
    flush();
    expect(screen.getByTestId("live-region").textContent).toBe("checkout: idle. 1 thread awaits input");
  });

  // spec: UI-098, UI-167
  test("the mode line follows the focus; only key-caused mode changes are announced", async () => {
    const { user } = setup({ registry: new Map([...registry, ["omfp:chat", StubComposer]]) });
    // The mode pill is rendered again on each mode change (its entrance motion).
    const line = () => screen.getByTestId("mode-line");
    const live = screen.getByTestId("live-region");
    expect([line().textContent, screen.getByTestId("app").dataset.mode]).toEqual(["NORMAL", "normal"]);
    expect(line().dataset.mode).toBe("normal");
    press("i");
    expect([line().textContent, screen.getByTestId("app").dataset.mode]).toEqual(["INSERT", "insert"]);
    expect(line().dataset.mode).toBe("insert");
    expect(live.textContent).toBe("INSERT mode");
    press("Escape");
    expect(line().textContent).toBe("NORMAL");
    expect(live.textContent).toBe("NORMAL mode");
    press(":");
    expect([line().textContent, line().dataset.mode]).toEqual(["COMMAND", "command"]);
    // The `:` line takes the place of the mode line's segments (UI-168).
    expect(screen.getByTestId("mode-bar").contains(screen.getByTestId("command-line"))).toBe(true);
    press("Escape");
    expect(line().textContent).toBe("NORMAL");
    await user.click(screen.getByTestId("stub-input"));
    expect(line().textContent).toBe("INSERT");
    expect(live.textContent).toBe("NORMAL mode");
  });

  // spec: UI-096
  test("notifications only without focus; permission on a gesture; a click focuses the window on the thread", () => {
    const shown: Array<{ notification: Notice; click: () => void }> = [];
    let permission: NotificationPermission = "default";
    let requested = 0;
    let focused = true;
    let windowFocused = 0;
    const port: NotificationPort = {
      permission: () => permission,
      requestPermission: async () => {
        requested += 1;
        permission = "granted";
        return permission;
      },
      show: (notification, click) => shown.push({ notification, click }),
    };
    const { client, flush, host } = setup({
      notifications: port,
      hasFocus: () => focused,
      focusWindow: () => {
        windowFocused += 1;
      },
    });
    fireEvent.pointerDown(document.body);
    fireEvent.keyDown(document.body, { key: "x" });
    expect(requested).toBe(1);
    client.indexEvents({ kind: "thread_upserted", thread: thread("t1", "checkout", "w1", { needsAttention: true }) });
    flush();
    expect(shown).toEqual([]);
    focused = false;
    client.indexEvents(
      { kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { process: { state: "failed", message: "x" } }) },
      { kind: "thread_upserted", thread: thread("t1", "checkout", "w1", { settled: false }) },
    );
    flush();
    client.indexEvents({ kind: "thread_upserted", thread: thread("t1", "checkout", "w1") });
    flush();
    expect(shown.map(({ notification }) => [notification.threadId, notification.tone, notification.title])).toEqual([
      ["t2", "failed", "flaky"],
      ["t1", "finished", "checkout"],
    ]);
    // Shown as system notifications, so not in the mode line too (UI-192).
    expect(host.window.get(NOTICES).map((notice) => notice.threadId)).toEqual(["t1"]);
    act(() => shown[0]?.click());
    expect(windowFocused).toBe(1);
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
  });

  // spec: UI-192, UI-193
  test("with focus, another thread's changes queue notices; :notice opens the newest thread", () => {
    const { client, flush, host } = setup({ hasFocus: () => true });
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { needsAttention: true }) });
    flush();
    const notice = () => screen.getByTestId("mode-notice");
    expect([notice().dataset.tone, notice().dataset.thread]).toEqual(["attention", "t2"]);
    expect(notice().textContent).toContain("flaky");
    // It sits in the mode line, not over the view.
    expect(screen.getByTestId("mode-bar").contains(notice())).toBe(true);
    client.indexEvents({
      kind: "thread_upserted",
      thread: thread("t2", "flaky", "w1", { needsAttention: true, process: { state: "failed", message: "x" } }),
    });
    flush();
    expect(notice().dataset.tone).toBe("error");
    expect(notice().textContent).toContain("+1");
    expect(screen.getByTestId("live-region").textContent).toBe("flaky failed");
    act(() => {
      host.commands.run("notice.open");
    });
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    // Visiting the thread clears all its notices.
    expect(screen.queryByTestId("mode-notice")).toBeNull();
  });

  // spec: UI-192
  test("a click on the notice opens its thread; visiting the thread clears it; notices expire", async () => {
    const { client, flush, host, user, noticeTimers } = setup({ hasFocus: () => true });
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { settled: false }) });
    flush();
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1") });
    flush();
    expect(screen.getByTestId("mode-notice").dataset.tone).toBe("ok");
    await user.click(screen.getByTestId("mode-notice"));
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    expect(screen.queryByTestId("mode-notice")).toBeNull();
    // Visiting by any other way clears it too.
    client.indexEvents({ kind: "thread_upserted", thread: thread("t1", "checkout", "w1", { needsAttention: true }) });
    flush();
    expect(screen.getByTestId("mode-notice").dataset.thread).toBe("t1");
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    expect(screen.queryByTestId("mode-notice")).toBeNull();
    // After a short time it goes away by itself.
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { needsAttention: true }) });
    flush();
    expect(screen.getByTestId("mode-notice").dataset.thread).toBe("t2");
    act(() => noticeTimers.run());
    expect(screen.queryByTestId("mode-notice")).toBeNull();
  });

  // spec: UI-192
  test("no notice about the thread the user is on while the window has focus", () => {
    const shown: string[] = [];
    const port: NotificationPort = {
      permission: () => "granted",
      requestPermission: async () => "granted",
      show: (notice) => shown.push(notice.title),
    };
    const { client, flush, host } = setup({ notifications: port, hasFocus: () => true });
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    client.indexEvents({ kind: "thread_upserted", thread: thread("t1", "checkout", "w1", { needsAttention: true }) });
    flush();
    expect([screen.queryByTestId("mode-notice"), host.window.get(NOTICES), shown]).toEqual([null, [], []]);
  });

  // spec: UI-194
  test("a widget's notify: a mode-line notice with focus, a system notification without", async () => {
    let focused = true;
    const shown: Array<{ notice: Notice; click: () => void }> = [];
    const port: NotificationPort = {
      permission: () => "granted",
      requestPermission: async () => "granted",
      show: (notice, click) => shown.push({ notice, click }),
    };
    const { user, host } = inWorkspace(
      {},
      {
        registry: new Map([...registry, ["omfp:header", Notifying]]),
        notifications: port,
        hasFocus: () => focused,
      },
    );
    await user.click(screen.getByRole("button", { name: "notify" }));
    expect(screen.getByTestId("mode-notice").dataset.tone).toBe("info");
    expect(screen.getByTestId("mode-notice").textContent).toContain("Build");
    expect(shown).toEqual([]);
    expect(screen.getByTestId("live-region").textContent).toBe("Build is green");
    focused = false;
    await user.click(screen.getByRole("button", { name: "notify" }));
    expect(shown.map(({ notice }) => [notice.title, notice.body, notice.threadId, notice.tone])).toEqual([
      ["Build", "is green", "t2", "info"],
    ]);
    expect(host.window.get(NOTICES)).toHaveLength(1);
    act(() => shown[0]?.click());
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
  });

  // spec: UI-092, UI-160
  test("errors of failed actions show as error toasts, closed by their ×", async () => {
    const { host, user } = setup();
    act(() => host.window.set(ERRORS, [{ key: 1, text: "boom" }]));
    const shown = screen.getAllByTestId("error");
    expect(shown.map((error) => [error.textContent, error.dataset.tone, error.getAttribute("role")])).toEqual([
      ["boom", "error", "alert"],
    ]);
    await user.click(screen.getByTestId("toast-dismiss"));
    expect(screen.queryByTestId("toasts")).toBeNull();
  });

  // spec: UI-536
  test("error toasts go by themselves each on its own clock, held while focused, and stay in the messages", () => {
    jest.useFakeTimers();
    try {
      const { host } = renderWithHost(<Toasts />);
      const shown = () => screen.queryAllByTestId("error").length;
      const elapse = (ms: number) =>
        act(() => {
          jest.advanceTimersByTime(ms);
        });
      act(() => reportError(host, new Error("disk full")));
      elapse(ERROR_TOAST_MS / 2);
      act(() => reportError(host, new Error("No API key found for anthropic")));
      const details = screen.getByTestId("error-details");
      act(() => details.focus());
      // The first one goes; the second keeps its toast, its focus and its time.
      elapse(ERROR_TOAST_MS / 2);
      expect(shown()).toBe(1);
      expect(document.activeElement === screen.getByTestId("error-details")).toBe(true);
      elapse(10 * ERROR_TOAST_MS);
      expect(shown()).toBe(1);
      act(() => screen.getByTestId("error-details").blur());
      elapse(ERROR_TOAST_MS - 1);
      expect(shown()).toBe(1);
      elapse(1);
      expect(screen.queryByTestId("toasts")).toBeNull();
      expect(host.window.get(MESSAGES).map(({ tone, text }) => [tone, text])).toEqual([
        ["error", "disk full"],
        ["error", "No API key found for anthropic"],
      ]);
      // The running login's toast has no timer: it stays until the login ends.
      act(() =>
        host.window.set(LOGIN_RUN, {
          id: 1,
          providerId: "anthropic",
          name: "Anthropic",
          progress: undefined,
          cancelled: false,
        }),
      );
      elapse(60 * 60_000);
      expect(screen.getAllByTestId("login-progress").length).toBe(1);
    } finally {
      jest.useRealTimers();
    }
  });

  // spec: UI-536
  test("an error reported twice is two toasts, each going on its own clock", () => {
    jest.useFakeTimers();
    try {
      const { host } = renderWithHost(<Toasts />);
      const shown = () => screen.queryAllByTestId("error").map((error) => error.textContent);
      const elapse = (ms: number) =>
        act(() => {
          jest.advanceTimersByTime(ms);
        });
      act(() => reportError(host, new Error("disk full")));
      elapse(ERROR_TOAST_MS / 2);
      act(() => reportError(host, new Error("disk full")));
      expect(shown()).toEqual(["disk full", "disk full"]);
      elapse(ERROR_TOAST_MS / 2);
      expect(shown()).toEqual(["disk full"]);
      // The second keeps its own time: it goes 12 s after it showed, not 12 s after the first went.
      elapse(ERROR_TOAST_MS / 2 - 1);
      expect(shown()).toEqual(["disk full"]);
      elapse(1);
      expect(screen.queryByTestId("toasts")).toBeNull();
    } finally {
      jest.useRealTimers();
    }
  });
});

describe("daemon settings", () => {
  function settings(ui: Settings["ui"], keymap: Settings["keymap"] = [], error: string | null = null): Settings {
    return { ui, keymap, error };
  }

  // spec: UI-097, UI-094, UI-143
  test("fonts become inline font tokens on <html> as given, the chrome following code; removed again when unset", () => {
    const { client, flush } = setup();
    client.indexEvents({
      kind: "settings_changed",
      settings: settings({ fonts: { prose: "Iowan Old Style, serif", code: "Iosevka" } }),
    });
    flush();
    const style = document.documentElement.style;
    expect(style.getPropertyValue("--omfp-font-prose")).toBe("Iowan Old Style, serif");
    expect(style.getPropertyValue("--omfp-font-code")).toBe("Iosevka");
    expect(style.getPropertyValue("--omfp-font-ui")).toBe("");
    client.indexEvents({ kind: "settings_changed", settings: settings({}) });
    flush();
    expect(style.getPropertyValue("--omfp-font-prose")).toBe("");
  });

  // spec: UI-097, UI-094, UI-144
  test("the font size becomes the root font-size on <html>, 17px without one", () => {
    const { client, flush } = setup();
    const style = document.documentElement.style;
    expect(style.fontSize).toBe("17px");
    client.indexEvents({ kind: "settings_changed", settings: settings({ fontSize: 20 }) });
    flush();
    expect(style.fontSize).toBe("20px");
    client.indexEvents({ kind: "settings_changed", settings: settings({}) });
    flush();
    expect(style.fontSize).toBe("17px");
  });

  // spec: UI-097, UI-145
  test("motion becomes data-motion on <html>, full without one", () => {
    const { client, flush } = setup();
    const root = document.documentElement;
    expect(root.dataset.motion).toBe("full");
    client.indexEvents({ kind: "settings_changed", settings: settings({ motion: "reduced" }) });
    flush();
    expect(root.dataset.motion).toBe("reduced");
    client.indexEvents({ kind: "settings_changed", settings: settings({ motion: "none" }) });
    flush();
    expect(root.dataset.motion).toBe("none");
  });

  // spec: UI-145
  test("prefers-reduced-motion caps the effective level at none, live", () => {
    const original = window.matchMedia;
    const system = new SystemMotion();
    window.matchMedia = system.matchMedia;
    try {
      const { client, flush } = setup();
      const root = document.documentElement;
      client.indexEvents({ kind: "settings_changed", settings: settings({ motion: "reduced" }) });
      flush();
      expect(root.dataset.motion).toBe("reduced");
      act(() => system.set(true));
      expect(root.dataset.motion).toBe("none");
      act(() => system.set(false));
      expect(root.dataset.motion).toBe("reduced");
    } finally {
      window.matchMedia = original;
    }
  });

  // spec: UI-145, UI-091
  test("prefers-reduced-motion caps the motion the page loaded with before the first `index` snapshot", () => {
    const original = window.matchMedia;
    const system = new SystemMotion();
    system.set(true);
    window.matchMedia = system.matchMedia;
    try {
      const root = document.documentElement;
      root.dataset.motion = "full";
      const { client, flush } = setup({}, false);
      client.connected();
      flush();
      expect(root.dataset.motion).toBe("none");
    } finally {
      window.matchMedia = original;
    }
  });

  // spec: UI-097, UI-095
  test("keymap entries apply live over the defaults", () => {
    const { client, flush, host } = setup();
    document.body.focus();
    client.indexEvents({
      kind: "settings_changed",
      settings: settings({}, [
        { mode: "normal", keys: "j", command: "" },
        { mode: "normal", keys: "n", command: "thread.new" },
      ]),
    });
    flush();
    press("j");
    expect(host.window.get(SELECTED_THREAD)).toBeUndefined();
    press("k");
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    press("n");
    expect(host.window.get(DRAFT_THREAD)).toEqual({ workspaceId: "w1" });
  });

  // spec: UI-097
  test("`:set` changes a setting through settings.set; a bad line is reported", async () => {
    const { client, host, user } = setup();
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "set fonts.ui=Inter, sans-serif{Enter}");
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "set font_size=18{Enter}");
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "set keymap.insert.Ctrl-s=composer.send{Enter}");
    expect(client.callsTo("settings.set").map((call) => call.params)).toEqual([
      { ui: { fonts: { ui: "Inter, sans-serif" } } },
      { ui: { fontSize: 18 } },
      { keymap: [{ mode: "insert", keys: "Ctrl-s", command: "composer.send" }] },
    ]);
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "set density=huge{Enter}");
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["Invalid value for density: huge"]);
  });

  // spec: UI-097
  test("a config file error shows a banner until the file is valid again", () => {
    const { client, flush } = setup();
    client.indexEvents({ kind: "settings_changed", settings: settings({}, [], "line 3: unknown key `colour`") });
    flush();
    expect(screen.getByTestId("settings-error").textContent).toContain("line 3: unknown key `colour`");
    client.indexEvents({ kind: "settings_changed", settings: settings({}) });
    flush();
    expect(screen.queryByTestId("settings-error")).toBeNull();
  });

  // spec: UI-097, UI-096
  test("notifications `always` show even with focus, `never` shows none and asks no permission", () => {
    const shown: (string | undefined)[] = [];
    let requested = 0;
    const port: NotificationPort = {
      permission: () => "granted",
      requestPermission: async () => {
        requested += 1;
        return "granted";
      },
      show: (notification) => shown.push(notification.threadId),
    };
    const { client, flush } = setup({ notifications: port, hasFocus: () => true });
    client.indexEvents({ kind: "settings_changed", settings: settings({ notifications: "always" }) });
    flush();
    client.indexEvents({ kind: "thread_upserted", thread: thread("t1", "checkout", "w1", { needsAttention: true }) });
    flush();
    expect(shown).toEqual(["t1"]);
    client.indexEvents({ kind: "settings_changed", settings: settings({ notifications: "never" }) });
    flush();
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { needsAttention: true }) });
    flush();
    expect([shown, requested]).toEqual([["t1"], 0]);
  });
});

describe("views of the current workspace", () => {
  // spec: UI-100
  test("the frame follows the current workspace's views topic; the default view until it answers", async () => {
    const { client, flush, host } = setup();
    expect(client.subscribed.filter((topic) => topic.startsWith("views/"))).toEqual([]);
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    expect(client.subscribed).toContain("views/w1");
    expect(screen.getByTestId("view").dataset.view).toBe("default");
    client.views("w1", { manifest: MANIFEST });
    flush();
    expect(screen.getByTestId("view").dataset.view).toBe("work");
    expect(shownWidgets()).toEqual(["omfp:chat", "omfp:workspace"]);
    // The draft thread's workspace is current once no thread is selected.
    act(() => {
      host.window.set(SELECTED_THREAD, undefined);
      host.window.set(DRAFT_THREAD, { workspaceId: "w2" });
    });
    await released();
    expect(client.unsubscribed).toContain("views/w1");
    expect(client.subscribed).toContain("views/w2");
    expect(screen.getByTestId("view").dataset.view).toBe("default");
    client.views("w2");
    flush();
    expect(screen.getByTestId("view").dataset.view).toBe("default");
  });

  // spec: UI-101, UI-301
  test("the mode line's view tabs list views by title in id order; a click switches the thread's view; so does `:view <id>`", async () => {
    const { client, flush, host, user } = inWorkspace({ manifest: MANIFEST });
    const tabs = () => screen.getAllByTestId("mode-view");
    expect(tabs().map((tab) => tab.textContent)).toEqual(["Lean", "Review", "Work"]);
    expect(tabs().map((tab) => tab.getAttribute("aria-current"))).toEqual([null, null, "true"]);
    expect(screen.getByTestId("mode-views").getAttribute("aria-label")).toBe("Views");
    await user.click(tabs()[1] ?? document.body);
    expect(host.window.get(VIEW_CHOICES).threads).toEqual([{ threadId: "t1", viewId: "review" }]);
    expect(screen.getByTestId("view").dataset.view).toBe("review");
    expect(tabs().map((tab) => tab.dataset.current ?? "")).toEqual(["", "true", ""]);
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "view lean{Enter}");
    expect(screen.getByTestId("view").dataset.view).toBe("lean");
    // The chosen view goes away: the manifest's default shows.
    client.viewsEvents("w1", {
      kind: "views_manifest",
      manifest: { ...MANIFEST, views: { work: MANIFEST.views.work, review: MANIFEST.views.review } },
      error: null,
    });
    flush();
    expect(screen.getByTestId("view").dataset.view).toBe("work");
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "view gone{Enter}");
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["No view gone"]);
  });

  // spec: UI-302, UI-431
  test("`] v` and `[ v` cycle views; Space v fuzzy filtering, arrows and Enter commit a view", async () => {
    const { host, user } = inWorkspace({ manifest: MANIFEST });
    const shown = () => screen.getByTestId("view").dataset.view;
    expect(shown()).toBe("work");
    press("]");
    press("v");
    expect(shown()).toBe("lean");
    press("[");
    press("v");
    expect(shown()).toBe("work");
    press("[");
    press("v");
    expect(shown()).toBe("review");
    press(" ");
    press("v");
    const input = screen.getByTestId("view-picker-input") as HTMLInputElement;
    await user.type(input, "ln");
    expect(screen.getAllByTestId("view-picker-option").map((option) => option.dataset.id)).toEqual(["lean"]);
    await user.keyboard("{Enter}");
    expect(shown()).toBe("lean");
    expect(host.window.get(VIEW_CHOICES)).toEqual({
      threads: [{ threadId: "t1", viewId: "lean" }],
      workspaces: { w1: "lean" },
    });
  });

  // spec: UI-302, UI-431
  test("the view picker cancels without changing the view, wraps arrows and never resets views.json", async () => {
    const { client, user } = inWorkspace({ manifest: MANIFEST });
    press(" ");
    press("v");
    await user.keyboard("{ArrowUp}{Escape}");
    expect(screen.getByTestId("view").dataset.view).toBe("work");
    press(" ");
    press("v");
    await user.keyboard("{ArrowDown}{Enter}");
    expect(screen.getByTestId("view").dataset.view).toBe("lean");
    expect(client.callsTo("views.set")).toEqual([]);
    expect(screen.queryAllByTestId("view-reset-dialog").length).toBe(0);
  });

  // spec: UI-302
  test("1–9 choose the matching numbered view, including after filtering; a single view remains pickable", async () => {
    const views = Object.fromEntries(
      Array.from({ length: 10 }, (_, at) => [
        `view-${String(at + 1).padStart(2, "0")}`,
        { ...MANIFEST.views.work, title: at === 9 ? "Release" : `View ${at + 1}` },
      ]),
    );
    const { user } = inWorkspace({ manifest: { default: "view-01", views } });
    press(" ");
    press("v");
    expect(
      screen.getAllByTestId("view-picker-option").map((option) => option.querySelector("kbd")?.textContent),
    ).toEqual(["1", "2", "3", "4", "5", "6", "7", "8", "9", undefined]);
    await user.keyboard("9");
    expect(screen.getByTestId("view").dataset.view).toBe("view-09");
    press(" ");
    press("v");
    await user.type(screen.getByTestId("view-picker-input"), "rls");
    await user.keyboard("9");
    expect((screen.getByTestId("view-picker-input") as HTMLInputElement).value).toBe("rls");
    await user.keyboard("1");
    expect(screen.getByTestId("view").dataset.view).toBe("view-10");
    cleanup();
    const single = inWorkspace({ manifest: { default: "work", views: { work: MANIFEST.views.work } } });
    act(() => {
      single.host.commands.run("view.pick");
    });
    await single.user.keyboard("1");
    expect(screen.getByTestId("view").dataset.view).toBe("work");
    expect(screen.queryAllByTestId("view-picker-input").length).toBe(0);
  });

  // spec: UI-302, UI-367
  test("the views chosen per thread are kept in localStorage: a reload shows each thread with its view", () => {
    const stored = new Map<string, string>();
    const storage = {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => stored.set(key, value),
    };
    const first = inWorkspace({ manifest: MANIFEST }, {}, { storage });
    act(() => first.host.commands.run("view.switch", ["review"]));
    select(first, "t2");
    act(() => first.host.commands.run("view.switch", ["lean"]));
    expect(JSON.parse(stored.get("omfp.viewChoices") ?? "null")).toEqual({
      threads: [
        { threadId: "t1", viewId: "review" },
        { threadId: "t2", viewId: "lean" },
      ],
      workspaces: { w1: "lean" },
    });
    cleanup();

    const reloaded = inWorkspace({ manifest: MANIFEST }, {}, { storage });
    expect(shownView()).toBe("review");
    select(reloaded, "t2");
    expect(shownView()).toBe("lean");
  });

  // spec: UI-367
  test("each thread keeps its view; a thread shown for the first time takes its workspace's last; workspaces keep their own", async () => {
    const test = inWorkspace({ manifest: MANIFEST });
    const { client, flush, host, user } = test;
    client.indexEvents(
      { kind: "thread_upserted", thread: thread("t3", "fresh", "w1") },
      { kind: "thread_upserted", thread: thread("t4", "other", "w2") },
    );
    flush();
    expect(shownView()).toBe("work");
    // The tab switches the selected thread's view.
    await user.click(screen.getAllByTestId("mode-view")[1] ?? document.body);
    expect(shownView()).toBe("review");
    // Another thread of the workspace, never shown: the workspace's last view; `:view` switches it alone.
    select(test, "t2");
    expect(shownView()).toBe("review");
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "view lean{Enter}");
    expect(shownView()).toBe("lean");
    select(test, "t1");
    expect(shownView()).toBe("review");
    select(test, "t2");
    expect(shownView()).toBe("lean");
    // A new thread takes the view last chosen in its workspace and keeps it when other threads switch.
    select(test, "t3");
    expect(shownView()).toBe("lean");
    select(test, "t1");
    press("]");
    press("v");
    expect(shownView()).toBe("work");
    select(test, "t3");
    expect(shownView()).toBe("lean");
    // Another workspace starts on its default view and keeps its own choice; its draft thread shows it too.
    select(test, "t4");
    client.views("w2", { manifest: MANIFEST });
    flush();
    expect(shownView()).toBe("work");
    act(() => host.commands.run("view.switch", ["review"]));
    expect(shownView()).toBe("review");
    act(() => {
      host.window.set(SELECTED_THREAD, undefined);
      host.window.set(DRAFT_THREAD, { workspaceId: "w2" });
    });
    flush();
    expect(shownView()).toBe("review");
    // The draft's switch is its workspace's last view, which the next new thread there takes.
    press("[");
    press("v");
    expect(shownView()).toBe("lean");
    select(test, "t4");
    expect(shownView()).toBe("review");
    client.indexEvents({ kind: "thread_upserted", thread: thread("t5", "next", "w2") });
    flush();
    select(test, "t5");
    expect(shownView()).toBe("lean");
    select(test, "t1");
    client.views("w1", { manifest: MANIFEST });
    flush();
    expect(shownView()).toBe("work");
    expect(host.window.get(VIEW_CHOICES).workspaces).toEqual({ w1: "work", w2: "lean" });
  });

  // spec: UI-367
  test("a thread whose view is gone shows its workspace's last view, else the default, and gets it back", () => {
    const test = inWorkspace({ manifest: MANIFEST });
    const { client, flush, host } = test;
    act(() => host.commands.run("view.switch", ["review"]));
    select(test, "t2");
    act(() => host.commands.run("view.switch", ["lean"]));
    select(test, "t1");
    const manifest = (views: Partial<typeof MANIFEST.views>) => {
      client.viewsEvents("w1", { kind: "views_manifest", manifest: { ...MANIFEST, views }, error: null });
      flush();
    };
    manifest({ work: MANIFEST.views.work, lean: MANIFEST.views.lean });
    expect(shownView()).toBe("lean");
    manifest({ work: MANIFEST.views.work });
    expect(shownView()).toBe("work");
    manifest(MANIFEST.views);
    expect(shownView()).toBe("review");
    select(test, "t2");
    expect(shownView()).toBe("lean");
  });

  // spec: UI-367
  test("the views of threads and workspaces the index no longer lists are dropped", () => {
    const test = inWorkspace({ manifest: MANIFEST });
    const { client, flush, host } = test;
    act(() => host.commands.run("view.switch", ["review"]));
    select(test, "t2");
    act(() => host.commands.run("view.switch", ["lean"]));
    select(test, undefined);
    client.indexEvents({ kind: "thread_removed", threadId: "t1" });
    flush();
    expect(host.window.get(VIEW_CHOICES)).toEqual({
      threads: [{ threadId: "t2", viewId: "lean" }],
      workspaces: { w1: "lean" },
    });
    client.indexEvents({ kind: "thread_removed", threadId: "t2" }, { kind: "workspace_removed", workspaceId: "w1" });
    flush();
    expect(host.window.get(VIEW_CHOICES)).toEqual({ threads: [], workspaces: {} });
  });

  // spec: UI-102, AGT-011
  test("an invalid manifest shows the daemon's error above the default view, until a valid one arrives", () => {
    const { client, flush } = inWorkspace({ manifest: null, error: "views.broken.layout: row has no children" });
    expect(screen.getByTestId("view-error").textContent).toBe(
      "The view is invalid, showing the default view: views.broken.layout: row has no children",
    );
    expect(screen.getByTestId("view").dataset.view).toBe("default");
    client.viewsEvents("w1", { kind: "views_manifest", manifest: MANIFEST, error: null });
    flush();
    expect(screen.queryByTestId("view-error")).toBeNull();
    expect(screen.getByTestId("view").dataset.view).toBe("work");
  });

  // spec: AGT-015, AGT-022
  test("a view without built-in chat or interactions gets the safety net; a workspace override does not count", () => {
    const { client, flush, host } = inWorkspace({ manifest: MANIFEST });
    expect(screen.queryByTestId("safety-net")).toBeNull();
    act(() => chooseView(host, "lean"));
    expect(screen.getByTestId("safety-net")).toBeDefined();
    act(() => chooseView(host, "work"));
    expect(screen.queryByTestId("safety-net")).toBeNull();
    client.viewsEvents("w1", {
      kind: "views_widgets",
      widgets: [{ name: "chat", source: "workspace", version: "v1", entry: "index.tsx" }],
    });
    flush();
    expect(screen.getByTestId("safety-net")).toBeDefined();
  });
});

describe("window reports", () => {
  // spec: AGT-026
  test("the window reports the workspace and views position it shows, and reports again after a reconnect", () => {
    const test = setup();
    expect(test.reports()).toEqual([{ workspaceId: null, views: null, widgets: [], fileErrors: [] }]);

    act(() => test.host.window.set(SELECTED_THREAD, "t1"));
    test.flush();
    test.client.views("w1", {});
    test.flush();
    expect(test.reports().at(-1)).toEqual({
      workspaceId: "w1",
      views: { epoch: "e1", seq: 0 },
      widgets: [],
      fileErrors: [],
    });
    test.client.viewsEvents("w1", { kind: "widget_stored", instance: "x", key: "k", value: 1 });
    test.flush();
    expect(test.reports().at(-1)).toEqual({
      workspaceId: "w1",
      views: { epoch: "e1", seq: 1 },
      widgets: [],
      fileErrors: [],
    });
    const sent = test.reports().length;

    act(() => test.client.model.setConnection({ kind: "reconnecting", attempt: 1, retryInMs: 500, error: "closed" }));
    test.client.notify();
    test.flush();
    expect(test.reports().length).toBe(sent);
    test.client.connected();
    test.flush();
    // The daemon forgot the closed connection's report: the same report goes out again.
    expect(test.reports().slice(sent)).toEqual([
      { workspaceId: "w1", views: { epoch: "e1", seq: 1 }, widgets: [], fileErrors: [] },
    ]);
  });
});

describe("adding a widget", () => {
  /** The manifests written so far. */
  function written({ client }: { client: FakeClient }): unknown[] {
    return client.callsTo("views.set").map((call) => call.params);
  }

  /** In `w1` with `state`; `views.set` succeeds unless `answer` is false. */
  function adding(state: Partial<ViewsState> = {}, answer = true) {
    const test = inWorkspace(state);
    if (answer) {
      test.client.answers.set("views.set", () => ({}));
    }
    return test;
  }

  // spec: UI-300, UI-520
  test("`:widget add ` completes built-ins and workspace widgets; the widget goes to the current view, written", async () => {
    const test = adding({
      manifest: MANIFEST,
      widgets: [
        { name: "tests", source: "workspace", version: "a", entry: "index.tsx" },
        { name: "clock", source: "workspace", version: "b", entry: "index.ts" },
      ],
    });
    const { client, flush, user } = test;
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "widget ");
    expect(screen.getAllByTestId("command-option").map((option) => option.getAttribute("data-value"))).toEqual([
      "widget add",
      "widget copy",
    ]);
    await user.type(screen.getByTestId("command-line-input"), "add ");
    expect(screen.getAllByTestId("command-option").map((option) => option.getAttribute("data-value"))).toEqual([
      "widget add omfp:header",
      "widget add omfp:chat",
      "widget add omfp:workspace",
      "widget add omfp:transcript",
      "widget add ./widgets/tests",
      "widget add ./widgets/clock",
    ]);
    await user.type(screen.getByTestId("command-line-input"), "./widgets/clock{Enter}");
    const manifest = {
      ...MANIFEST,
      views: {
        ...MANIFEST.views,
        work: { title: "Work", layout: ["row", {}, "omfp:chat", "omfp:workspace", "./widgets/clock"] },
      },
    };
    expect(written(test)).toEqual([{ workspaceId: "w1", manifest }]);
    await waitFor(() =>
      expect(screen.getByTestId("live-region").textContent).toBe("Added ./widgets/clock to view work"),
    );
    expect(screen.getAllByTestId("success").map((success) => success.textContent)).toEqual([
      "Added ./widgets/clock to view work",
    ]);
    // Rendering follows the daemon's push, not the local change.
    expect(shownWidgets()).toEqual(["omfp:chat", "omfp:workspace"]);
    client.viewsEvents("w1", { kind: "views_manifest", manifest, error: null });
    flush();
    expect(shownWidgets()).toEqual(["omfp:chat", "omfp:workspace", "./widgets/clock"]);
  });

  // spec: UI-300
  test("without views.json the default view is written with the widget as its last side tab", async () => {
    const test = adding();
    press(":");
    await test.user.type(screen.getByTestId("command-line-input"), "widget add omfp:transcript{Enter}");
    expect(written(test)).toEqual([
      {
        workspaceId: "w1",
        manifest: {
          ...DEFAULT_MANIFEST,
          views: {
            default: {
              title: "Default",
              layout: [
                "row",
                { sizes: [3, 1] },
                ["col", {}, "omfp:header", ["tabs", { id: "main" }, "omfp:chat"]],
                [
                  "tabs",
                  { id: "side" },
                  "omfp:workspace",
                  "omfp:changes",
                  "omfp:background",
                  "omfp:mcp",
                  "omfp:transcript",
                ],
              ],
            },
          },
        },
      },
    ]);
  });

  // spec: UI-300
  test("failed or refused writes land in the error list; an invalid views.json is never overwritten", async () => {
    const failing = adding({ manifest: MANIFEST }, false);
    press(":");
    await failing.user.type(screen.getByTestId("command-line-input"), "widget add omfp:header{Enter}");
    const [pending] = failing.client.callsTo("views.set");
    await act(async () => pending?.reject(new Error("views.json: permission denied")));
    expect(failing.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["views.json: permission denied"]);
    cleanup();
    const invalid = adding({ manifest: null, error: "views: no view" });
    press(":");
    await invalid.user.type(screen.getByTestId("command-line-input"), "widget add omfp:header{Enter}");
    expect(written(invalid)).toEqual([]);
    expect(invalid.host.window.get(ERRORS).map(({ text }) => text)).toEqual([
      "The layout cannot change while .omfp/views.json is invalid: fix the file first",
    ]);
    cleanup();
    const none = setup();
    press(":");
    await none.user.type(screen.getByTestId("command-line-input"), "widget add omfp:header{Enter}");
    expect(none.host.window.get(ERRORS).map(({ text }) => text)).toEqual(["No current workspace"]);
  });
});

describe("widget copy", () => {
  // spec: UI-106, UI-520
  test("`:widget copy <reference> [<name>]` copies into the current workspace; a success toast says ./widgets/<name>", async () => {
    const { client, host, user } = setup();
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "widget copy omfp:workspace{Enter}");
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["No current workspace"]);
    act(() => host.window.set(ERRORS, []));
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    client.answers.set("widgets.copy", () => ({ name: "board" }));
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "widget copy omfp:workspace board{Enter}");
    expect(client.callsTo("widgets.copy").map((call) => call.params)).toEqual([
      { workspaceId: "w1", widget: "omfp:workspace", name: "board" },
    ]);
    await waitFor(() => expect(screen.getByTestId("live-region").textContent).toBe("Copied to ./widgets/board"));
    expect(screen.getAllByTestId("success").map((success) => success.textContent)).toEqual([
      "Copied to ./widgets/board",
    ]);
    client.answers.delete("widgets.copy");
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "widget copy omfp:clock{Enter}");
    const [, failing] = client.callsTo("widgets.copy");
    expect(failing?.params).toEqual({ workspaceId: "w1", widget: "omfp:clock" });
    await act(async () => failing?.reject(new Error("./widgets/clock already exists")));
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["./widgets/clock already exists"]);
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "widget paste{Enter}");
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual([
      "./widgets/clock already exists",
      "Usage: :widget add <reference> | :widget copy <reference> [<name>]",
    ]);
  });

  // spec: UI-107, UI-430, DMN-088, UI-520
  test("`:view-reset` asks first; only Reset writes the default manifest over views.json, with a success toast", async () => {
    const { client, host, user } = setup();
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "view-reset{Enter}");
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["No current workspace"]);
    act(() => host.window.set(ERRORS, []));
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    client.views("w1", { manifest: null, error: "views: expected an object of views" });
    client.answers.set("views.set", () => ({}));
    press(":");
    await user.type(screen.getByTestId("command-line-input"), "view-reset{Enter}");
    expect(screen.getByTestId("view-reset-dialog").textContent).toContain("Reset .omfp/views.json?");
    await user.click(screen.getByTestId("cancel-view-reset"));
    expect(screen.queryAllByTestId("view-reset-dialog")).toEqual([]);
    expect(client.callsTo("views.set")).toEqual([]);
    act(() => {
      host.commands.run("view.reset");
    });
    await user.click(screen.getByTestId("confirm-view-reset"));
    expect(client.callsTo("views.set").map((call) => call.params)).toEqual([
      { workspaceId: "w1", manifest: DEFAULT_MANIFEST },
    ]);
    await waitFor(() =>
      expect(screen.getByTestId("live-region").textContent).toBe(".omfp/views.json reset to the default view"),
    );
    expect(host.window.get(SUCCESSES).map((success) => success.text)).toEqual([
      ".omfp/views.json reset to the default view",
    ]);
    client.answers.delete("views.set");
    act(() => {
      host.commands.run("view.reset");
    });
    await user.click(screen.getByTestId("confirm-view-reset"));
    const [, failing] = client.callsTo("views.set");
    await act(async () => failing?.reject(new Error("disk full")));
    expect(host.window.get(ERRORS).map(({ text }) => text)).toEqual(["disk full"]);
  });
});

describe("responsive frame", () => {
  let restore = () => {};
  beforeEach(() => {
    restore = installResizeObserver();
  });
  afterEach(() => restore());

  // spec: UI-200, UI-201, UI-207
  test("a narrow frame folds the left column into a strip that opens over the view; Escape, a press outside or a pick closes it", async () => {
    const { client, flush, host, user } = setup();
    const app = screen.getByTestId("app");
    const column = screen.getByTestId("left-column");
    const open = () => column.dataset.open === "true";
    expect(app.dataset.layout).toBe("wide");
    expect([screen.queryAllByTestId("left-splitter").length, screen.queryAllByTestId("left-strip").length]).toEqual([
      1, 0,
    ]);
    resize(app, 900);
    expect(app.dataset.layout).toBe("narrow");
    expect([screen.queryAllByTestId("left-splitter").length, screen.queryAllByTestId("left-strip").length]).toEqual([
      0, 1,
    ]);
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { needsAttention: true }) });
    flush();
    expect(screen.getAllByTestId("strip-thread").map((button) => button.dataset.threadId)).toEqual(["t2"]);
    expect(open()).toBe(false);
    await user.click(screen.getByTestId("left-toggle"));
    expect(open()).toBe(true);
    expect(screen.getByTestId("left-toggle").getAttribute("aria-expanded")).toBe("true");
    press("Escape");
    expect(open()).toBe(false);
    // Closing gave the focus back to the strip's toggle (compared by test id: a failing diff of an element prints its
    // whole React fiber graph); the keys work from anywhere.
    expect(document.activeElement?.getAttribute("data-testid")).toBe("left-toggle");
    act(() => (document.activeElement as HTMLElement | null)?.blur());
    press(" ");
    press("t");
    expect(open()).toBe(true);
    act(() => {
      fireEvent.pointerDown(screen.getByTestId("view"));
    });
    expect(open()).toBe(false);
    await user.click(screen.getByTestId("left-toggle"));
    expect(open()).toBe(true);
    await user.click(screen.getByTestId("strip-thread"));
    expect(host.window.get(SELECTED_THREAD)).toBe("t2");
    expect(open()).toBe(false);
    resize(app, 1400);
    expect(app.dataset.layout).toBe("wide");
    expect(screen.queryAllByTestId("left-strip").length).toBe(0);
  });

  // spec: UI-200, UI-204
  test("a phone shows one area at a time from the bottom bar, without splitters; picking a thread goes back to the chat", async () => {
    const { host, user } = setup();
    const app = screen.getByTestId("app");
    resize(app, 390);
    expect(app.dataset.layout).toBe("mobile");
    const areas = () =>
      screen.getAllByTestId("area-button").map((button) => [button.dataset.area, button.getAttribute("aria-pressed")]);
    expect(areas()).toEqual([
      ["threads", "false"],
      ["chat", "true"],
      ["side", "false"],
    ]);
    expect([screen.queryAllByTestId("splitter").length, screen.queryAllByTestId("left-splitter").length]).toEqual([
      0, 0,
    ]);
    expect(screen.getByTestId("view").dataset.area).toBe("chat");
    await user.click(screen.getAllByTestId("area-button")[2] ?? document.body);
    expect(screen.getByTestId("view").dataset.area).toBe("side");
    expect(areas()[2]).toEqual(["side", "true"]);
    await user.click(screen.getAllByTestId("area-button")[0] ?? document.body);
    expect(screen.getByTestId("left-column").dataset.open).toBe("true");
    expect(areas()[0]).toEqual(["threads", "true"]);
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    expect(screen.getByTestId("left-column").dataset.open).toBeUndefined();
    expect(screen.getByTestId("view").dataset.area).toBe("chat");
    expect(areas()[1]).toEqual(["chat", "true"]);
  });

  // spec: UI-419
  test("on a phone the mode line takes the header's place: the state as its mark, then the thread's ⋯", () => {
    const { host } = setup();
    const app = screen.getByTestId("app");
    act(() => host.window.set(SELECTED_THREAD, "t1"));
    const state = () => within(screen.getByTestId("mode-bar")).getByTestId("mode-thread-state");
    const more = () => within(screen.getByTestId("mode-bar")).queryAllByTestId("header-more").length;
    expect([host.window.get(FRAME_LAYOUT), state().textContent, more()]).toEqual(["wide", "idle", 0]);
    resize(app, 390);
    expect([host.window.get(FRAME_LAYOUT), state().textContent, more()]).toEqual(["mobile", "", 1]);
    expect(within(state()).getByRole("img").getAttribute("aria-label")).toBe("idle");
  });

  // spec: UI-205
  test("on a phone the app is as tall as the visual viewport, following the on-screen keyboard", () => {
    const viewport = Object.assign(new EventTarget(), { height: 800 });
    const original = Object.getOwnPropertyDescriptor(window, "visualViewport");
    Object.defineProperty(window, "visualViewport", { configurable: true, value: viewport });
    try {
      setup();
      const app = screen.getByTestId("app");
      const height = () => document.documentElement.style.getPropertyValue("--vvh");
      expect(height()).toBe("");
      resize(app, 390);
      expect(height()).toBe("800px");
      viewport.height = 450;
      act(() => {
        viewport.dispatchEvent(new Event("resize"));
      });
      expect(height()).toBe("450px");
      resize(app, 900);
      expect(height()).toBe("");
    } finally {
      if (original === undefined) {
        Reflect.deleteProperty(window, "visualViewport");
      } else {
        Object.defineProperty(window, "visualViewport", original);
      }
    }
  });

  // spec: UI-414
  test("a short touch screen (a phone held sideways) is a phone at any width; any touch screen follows the visual viewport", () => {
    const viewport = Object.assign(new EventTarget(), { height: 412 });
    const originalViewport = Object.getOwnPropertyDescriptor(window, "visualViewport");
    const originalMedia = window.matchMedia;
    const system = new SystemScreen();
    Object.defineProperty(window, "visualViewport", { configurable: true, value: viewport });
    window.matchMedia = system.matchMedia;
    try {
      setup();
      const app = screen.getByTestId("app");
      const height = () => document.documentElement.style.getPropertyValue("--vvh");
      resize(app, 915);
      expect([app.dataset.layout, height()]).toEqual(["narrow", ""]);
      act(() => system.set(COARSE_POINTER, SHORT_TOUCH_SCREEN));
      expect([app.dataset.layout, height()]).toEqual(["mobile", "412px"]);
      expect(screen.getAllByTestId("area-button").length).toBe(3);
      // A tablet: tall enough for the strip, still followed above its keyboard.
      act(() => system.set(COARSE_POINTER));
      expect([app.dataset.layout, height()]).toEqual(["narrow", "412px"]);
      act(() => system.set());
      expect([app.dataset.layout, height()]).toEqual(["narrow", ""]);
    } finally {
      window.matchMedia = originalMedia;
      if (originalViewport === undefined) {
        Reflect.deleteProperty(window, "visualViewport");
      } else {
        Object.defineProperty(window, "visualViewport", originalViewport);
      }
    }
  });
});

describe("focus after a jump", () => {
  // spec: UI-092, UI-286
  test("the jump to the next thread needing attention (`Space a`) hands the focus to its composer", () => {
    const { client, flush, host } = setup();
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { needsAttention: true }) });
    flush();
    act(() => {
      host.commands.run("thread.nextAttention");
    });
    expect([host.window.get(SELECTED_THREAD), host.window.get(FOCUS_COMPOSER)]).toEqual(["t2", "t2"]);
  });

  // spec: UI-092, UI-192, UI-286
  test(":notice opens the newest notice's thread with the focus in its composer", () => {
    const { client, flush, host } = setup({ hasFocus: () => true });
    client.indexEvents({ kind: "thread_upserted", thread: thread("t2", "flaky", "w1", { needsAttention: true }) });
    flush();
    act(() => {
      host.commands.run("notice.open");
    });
    expect([host.window.get(SELECTED_THREAD), host.window.get(FOCUS_COMPOSER)]).toEqual(["t2", "t2"]);
  });
});

describe("next and previous thread", () => {
  let restore = () => {};
  beforeEach(() => {
    restore = installResizeObserver();
  });
  afterEach(() => restore());

  // spec: UI-989, UI-990, UI-991
  test("Alt-j / Alt-k walk the column across workspaces from the composer, the column and a folded column", () => {
    const { client, flush, host } = setup({ registry: new Map([...registry, ["omfp:chat", StubComposer]]) });
    client.indexEvents({ kind: "thread_upserted", thread: thread("t3", "deploy", "w2") });
    client.indexEvents({ kind: "thread_upserted", thread: thread("t4", "old", "w2", { archivedAt: 5 }) });
    flush();
    const state = () => [host.window.get(SELECTED_THREAD), host.window.get(FOCUS_COMPOSER)];
    const step = (key: "j" | "k") => {
      act(() => host.window.set(FOCUS_COMPOSER, undefined));
      press(key, { altKey: true, code: key === "j" ? "KeyJ" : "KeyK" });
    };
    select({ host, flush }, "t1");
    // Typing in the composer (insert mode): the new thread's composer takes the focus.
    press("i");
    expect(document.activeElement?.getAttribute("data-testid")).toBe("stub-input");
    step("j");
    expect(state()).toEqual(["t2", "t2"]);
    // Past w1's last thread, w2's first: w2 becomes the current workspace; the archived t4 is skipped.
    step("j");
    expect(state()).toEqual(["t3", "t3"]);
    expect(currentWorkspaceId(host)).toBe("w2");
    step("j");
    expect(state()).toEqual(["t1", "t1"]);
    step("k");
    expect(state()).toEqual(["t3", "t3"]);
    // Normal mode outside the column: the selection moves, the focus stays.
    act(() => (document.activeElement as HTMLElement | null)?.blur());
    step("k");
    expect(state()).toEqual(["t2", undefined]);
    // A collapsed workspace is skipped.
    act(() => host.window.set(COLLAPSED_WORKSPACES, ["w1"]));
    step("j");
    expect(state()).toEqual(["t3", undefined]);
    step("j");
    expect(state()).toEqual(["t3", undefined]);
    act(() => host.window.set(COLLAPSED_WORKSPACES, []));
    // On a row of the column, the focus follows to the new row.
    act(() =>
      screen
        .getAllByTestId("thread-row")
        .find((row) => row.dataset.threadId === "t3")
        ?.focus(),
    );
    step("k");
    expect(state()).toEqual(["t2", undefined]);
    expect(document.activeElement?.closest<HTMLElement>("[data-thread-id]")?.dataset.threadId).toBe("t2");
    // A narrow window with the column folded away: the keys still work.
    resize(screen.getByTestId("app"), 900);
    expect(screen.getByTestId("left-column").dataset.open).toBeUndefined();
    act(() => (document.activeElement as HTMLElement | null)?.blur());
    step("k");
    expect(state()).toEqual(["t1", undefined]);
  });
});
