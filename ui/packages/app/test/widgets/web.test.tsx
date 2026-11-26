/** L4: `omfp:web` (UI-670): the layout node's `url` and `title` props reach the kit's web frame. */

import { afterAll, afterEach, beforeAll, describe, expect, spyOn, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import { strings } from "../../src/strings/index.ts";
import { BUILTIN_WIDGETS } from "../../src/view/registry.ts";
import { WebWidget } from "../../src/widgets/web/index.ts";

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

describe("omfp:web", () => {
  // spec: UI-670
  test("is the built-in omfp:web, titled Web, showing its url prop in a frame named by its title prop", () => {
    expect(BUILTIN_WIDGETS.get("omfp:web")).toBe(WebWidget);
    expect(strings.shell.frame.widgetTitles["omfp:web"]).toBe("Web");
    render(<WebWidget url="http://localhost:5173/" title="Preview" />);
    const frame = screen.getByTestId("web-frame-frame");
    expect([frame.getAttribute("src"), frame.getAttribute("title")]).toEqual(["http://localhost:5173/", "Preview"]);
    expect((screen.getByTestId("web-frame-url") as HTMLInputElement).value).toBe("http://localhost:5173/");
  });

  // spec: UI-670, UI-675
  test("a node without a usable url says so instead of a frame", () => {
    render(<WebWidget title={3} />);
    expect(screen.getByTestId("web-frame-refused").dataset.reason).toBe("missing");
    expect(screen.queryAllByTestId("web-frame-frame").length).toBe(0);
  });
});
