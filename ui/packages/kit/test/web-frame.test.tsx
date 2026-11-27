/**
 * L1: the kit's web view and web frame (UI-670..UI-675, UI-682, UI-685): the URLs they show, the sandbox, the bare
 * view and its handle, the frame's toolbar and its refusal hint.
 */

import { afterAll, afterEach, beforeAll, describe, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createRef } from "react";
import {
  WEB_FRAME_SANDBOX,
  WebFrame,
  WebView,
  type WebViewHandle,
  type WebViewLocation,
  webFrameUrl,
} from "../src/index.ts";

afterEach(cleanup);

// happy-dom loads no frame: it reports each one on the console, which says nothing here.
const original = console.error.bind(console);
const quiet = spyOn(console, "error");
beforeAll(() => {
  quiet.mockImplementation((...args: unknown[]) => {
    if (!String(args[0]).includes("Iframe page loading is disabled")) {
      original(...args);
    }
  });
});
afterAll(() => quiet.mockRestore());

/** The window's origin in the component tests (`ui/test/dom.ts`): the daemon's for the frame. */
const OWN = "http://127.0.0.1:7421";

function frame(): HTMLIFrameElement {
  return screen.getByTestId("web-frame-frame") as HTMLIFrameElement;
}

describe("webFrameUrl", () => {
  // spec: UI-671
  test("any http or https page shows, local dev servers included", () => {
    for (const [text, href] of [
      ["https://grafana.example.com/d/abc?orgId=1", "https://grafana.example.com/d/abc?orgId=1"],
      ["http://localhost:5173", "http://localhost:5173/"],
      ["  http://127.0.0.1:3000/storybook  ", "http://127.0.0.1:3000/storybook"],
      ["http://[::1]:8080/", "http://[::1]:8080/"],
      ["http://127.0.0.1/", "http://127.0.0.1/"],
    ] as const) {
      expect([text, webFrameUrl(text, OWN)]).toEqual([text, { ok: true, href }]);
    }
  });

  // spec: UI-671
  test("other schemes, relative URLs and the daemon's own origin are refused", () => {
    for (const [text, reason] of [
      [undefined, "missing"],
      [42, "missing"],
      ["   ", "missing"],
      ["not a url", "invalid"],
      ["/widgets/x", "invalid"],
      ["javascript:alert(1)", "scheme"],
      ["JavaScript:alert(1)", "scheme"],
      ["data:text/html,<script>parent.postMessage(1)</script>", "scheme"],
      ["blob:http://127.0.0.1:7421/1234", "scheme"],
      ["file:///etc/passwd", "scheme"],
      ["about:blank", "scheme"],
      ["http://127.0.0.1:7421/", "own-origin"],
      ["http://127.0.0.1:7421/#token=t", "own-origin"],
      // Loopback aliases of the daemon's address are the daemon too.
      ["http://localhost:7421/", "own-origin"],
      ["http://[::1]:7421/", "own-origin"],
      ["http://127.0.0.2:7421/x", "own-origin"],
    ] as const) {
      expect([text, webFrameUrl(text, OWN)]).toEqual([text, { ok: false, reason }]);
    }
  });

  // spec: UI-671
  test("a remote daemon's origin is refused with its default port; loopback pages of the user's machine are not", () => {
    const remote = "https://omfp.example.com";
    expect(webFrameUrl("https://omfp.example.com:443/", remote)).toEqual({ ok: false, reason: "own-origin" });
    expect(webFrameUrl("http://omfp.example.com/", remote)).toEqual({ ok: true, href: "http://omfp.example.com/" });
    expect(webFrameUrl("http://localhost:443/", remote)).toEqual({ ok: true, href: "http://localhost:443/" });
  });
});

describe("WebFrame", () => {
  // spec: UI-670, UI-672
  test("the page shows in a sandboxed frame that never navigates the window nor shares its origin", () => {
    render(<WebFrame url="http://localhost:5173/app" title="Dev server" />);
    const element = frame();
    expect(element.getAttribute("src")).toBe("http://localhost:5173/app");
    expect(element.getAttribute("title")).toBe("Dev server");
    expect(element.getAttribute("referrerpolicy")).toBe("no-referrer");
    const sandbox = (element.getAttribute("sandbox") ?? "").split(" ").sort();
    expect(sandbox).toEqual([
      "allow-forms",
      "allow-popups",
      "allow-popups-to-escape-sandbox",
      "allow-same-origin",
      "allow-scripts",
    ]);
    expect(element.getAttribute("sandbox")).toBe(WEB_FRAME_SANDBOX);
    expect(sandbox.filter((flag) => flag.startsWith("allow-top-navigation"))).toEqual([]);
  });

  // spec: UI-670
  test("without a title the frame is named by the page's host", () => {
    render(<WebFrame url="https://docs.example.com/guide" />);
    expect(frame().getAttribute("title")).toBe("docs.example.com");
  });

  // spec: UI-675
  test("a refused URL shows why and how to fix it, and no frame", () => {
    render(<WebFrame url="javascript:alert(1)" />);
    const refused = screen.getByTestId("web-view-refused");
    expect(refused.dataset.reason).toBe("scheme");
    expect(refused.textContent).toContain("Only http and https pages can show here");
    expect(refused.textContent).toContain("http://localhost:5173/");
    expect(screen.queryAllByTestId("web-frame-frame").length).toBe(0);
    cleanup();
    render(<WebFrame url="http://127.0.0.1:7421/" />);
    expect(screen.getByTestId("web-view-refused").dataset.reason).toBe("own-origin");
    expect(screen.getByTestId("web-view-refused").textContent).toContain("omfp's own pages cannot show in a frame");
  });

  // spec: UI-673
  test("the toolbar shows the URL, reloads the frame and opens the page in the browser", async () => {
    const user = userEvent.setup();
    const open = spyOn(globalThis, "open").mockImplementation(() => null);
    try {
      render(<WebFrame url="https://grafana.example.com/d/abc" />);
      expect(screen.getByRole("toolbar").getAttribute("aria-label")).toBe("Web page");
      expect(screen.getByTestId("web-frame-url").textContent).toBe("https://grafana.example.com/d/abc");
      frame().dataset.marked = "before";
      await user.click(screen.getByRole("button", { name: "Reload" }));
      // A new frame: the page loads again (a cross-origin frame cannot be told to reload).
      expect(frame().dataset.marked).toBeUndefined();
      expect(frame().getAttribute("src")).toBe("https://grafana.example.com/d/abc");
      await user.click(screen.getByRole("button", { name: "Open in browser" }));
      expect(open.mock.calls).toEqual([["https://grafana.example.com/d/abc", "_blank", "noopener,noreferrer"]]);
    } finally {
      open.mockRestore();
    }
  });

  // spec: UI-674
  test("a page that has not loaded in time gets the hint that the site may refuse framing", async () => {
    render(<WebFrame url="https://linear.app/" loadTimeoutMs={20} />);
    expect(screen.queryAllByTestId("web-frame-hint").length).toBe(0);
    const hint = await screen.findByTestId("web-frame-hint");
    expect(hint.textContent).toContain("X-Frame-Options");
    expect(hint.textContent).toContain("Open it in the browser");
    // A late load takes it back.
    fireEvent.load(frame());
    expect(screen.queryAllByTestId("web-frame-hint").length).toBe(0);
    expect(screen.getByTestId("web-frame").dataset.loaded).toBe("true");
  });

  // spec: UI-674
  test("a loaded page gets no hint by itself, but Blank? shows it and Dismiss hides it", async () => {
    const user = userEvent.setup();
    render(
      <>
        <WebFrame url="https://github.com/" loadTimeoutMs={20} data-testid="loaded" />
        <WebFrame url="https://example.com/" loadTimeoutMs={20} data-testid="waiting" />
      </>,
    );
    const [loaded] = screen.getAllByTestId("web-frame-frame");
    fireEvent.load(loaded as HTMLIFrameElement);
    // Once the frame that never loaded says it, the loaded one's wait (started first, as long) is over too.
    await screen.findByTestId("web-frame-hint");
    expect(screen.getByTestId("loaded").querySelectorAll("[data-testid=web-frame-hint]").length).toBe(0);
    expect(screen.getByTestId("waiting").querySelectorAll("[data-testid=web-frame-hint]").length).toBe(1);
    cleanup();
    render(<WebFrame url="https://github.com/" />);
    fireEvent.load(frame());
    expect(screen.queryAllByTestId("web-frame-hint").length).toBe(0);
    await user.click(screen.getByRole("button", { name: "Blank?" }));
    expect(screen.getByTestId("web-frame-hint").textContent).toContain("frame-ancestors");
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryAllByTestId("web-frame-hint").length).toBe(0);
  });
});

describe("WebView", () => {
  // spec: UI-670, UI-685
  test("is the page alone; in a frame its handle reloads it and onLoad says when it loaded", () => {
    let loads = 0;
    const handle = createRef<WebViewHandle>();
    render(
      <WebView
        ref={handle}
        url="https://example.com/"
        onLoad={() => {
          loads += 1;
        }}
      />,
    );
    expect(screen.queryAllByRole("toolbar").length).toBe(0);
    expect(handle.current?.native).toBe(false);
    frame().dataset.marked = "before";
    act(() => handle.current?.reload());
    // A new frame: a cross-origin frame cannot be told to reload; back and forward do nothing there.
    expect(frame().dataset.marked).toBeUndefined();
    act(() => {
      handle.current?.back();
      handle.current?.forward();
    });
    fireEvent.load(frame());
    expect(loads).toBe(1);
  });

  // spec: UI-687
  test("in a frame, onMessage gets what its own page posts to the window, not what another window does", () => {
    const messages: unknown[] = [];
    render(<WebView url="https://example.com/" onMessage={(data) => messages.push(data)} />);
    // happy-dom loads no frame, so the frame has no window: give it one to post from.
    const own: Window = Object.create(null);
    Object.defineProperty(frame(), "contentWindow", { value: own });
    globalThis.dispatchEvent(new MessageEvent("message", { data: { from: "page" }, source: own }));
    globalThis.dispatchEvent(new MessageEvent("message", { data: { from: "elsewhere" } }));
    expect(messages).toEqual([{ from: "page" }]);
  });
});

describe("WebView in a window that draws web pages (UI-682)", () => {
  // The desktop window's page script defines it before the app runs.
  const commands: unknown[][] = [];
  beforeAll(() =>
    Object.assign(globalThis, { __omfpWebViews: { command: (...args: unknown[]) => commands.push(args) } }),
  );
  afterAll(() => Reflect.deleteProperty(globalThis, "__omfpWebViews"));

  // spec: UI-682
  test("is a placeholder naming its page, and a web frame there has no Blank?", () => {
    render(<WebView url="https://accounts.google.com/" />);
    expect(screen.queryAllByTestId("web-frame-frame").length).toBe(0);
    const placeholder = screen.getByTestId("web-view-native");
    expect(placeholder.getAttribute("data-url")).toBe("https://accounts.google.com/");
    expect(placeholder.getAttribute("aria-label")).toBe("accounts.google.com");
    expect(placeholder.getAttribute("data-omfp-web-view")).toBeTruthy();
    cleanup();
    render(<WebFrame url="https://accounts.google.com/" />);
    expect(screen.queryAllByRole("button", { name: "Blank?" }).length).toBe(0);
  });

  // spec: UI-685
  test("hands the widget where its page went and when it loaded, and its handle's commands to the window", () => {
    const locations: WebViewLocation[] = [];
    let loads = 0;
    const handle = createRef<WebViewHandle>();
    render(
      <WebView
        ref={handle}
        url="https://example.com/"
        onNavigate={(location) => locations.push(location)}
        onLoad={() => {
          loads += 1;
        }}
      />,
    );
    const placeholder = screen.getByTestId("web-view-native");
    const id = placeholder.getAttribute("data-omfp-web-view");
    const detail = { url: "https://example.com/next", canGoBack: true, canGoForward: null };
    placeholder.dispatchEvent(new CustomEvent("omfp-web-view-navigate", { detail }));
    placeholder.dispatchEvent(new CustomEvent("omfp-web-view-navigate", { detail: { nothing: 1 } }));
    placeholder.dispatchEvent(new CustomEvent("omfp-web-view-load"));
    expect(locations).toEqual([detail]);
    expect(loads).toBe(1);
    expect(handle.current?.native).toBe(true);
    act(() => {
      handle.current?.back();
      handle.current?.forward();
      handle.current?.reload();
    });
    expect(commands).toEqual([
      [id, "back"],
      [id, "forward"],
      [id, "reload"],
    ]);
  });

  // spec: UI-687
  test("tells the window its scripts and whether its page gets a channel, and hands the widget what it posts", () => {
    const messages: unknown[] = [];
    const { rerender } = render(<WebView url="https://example.com/" />);
    const setup = () => JSON.parse(screen.getByTestId("web-view-native").getAttribute("data-setup") ?? "null");
    expect(setup()).toEqual({ scripts: [], channel: false });
    rerender(<WebView url="https://example.com/" scripts={["hide()"]} onMessage={(data) => messages.push(data)} />);
    expect(setup()).toEqual({ scripts: ["hide()"], channel: true });
    const placeholder = screen.getByTestId("web-view-native");
    placeholder.dispatchEvent(new CustomEvent("omfp-web-view-message", { detail: { data: { n: 1 } } }));
    expect(messages).toEqual([{ n: 1 }]);
  });
});
