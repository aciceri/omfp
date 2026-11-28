/** L4: `omfp:canvas`, the default view's empty side panel (UI-1000). */

import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, screen } from "@testing-library/react";
import { BUILTIN_WIDGETS } from "../../src/view/registry.ts";
import { DEFAULT_VIEW } from "../../src/view/views.ts";
import { CanvasWidget } from "../../src/widgets/canvas/index.ts";
import { renderWithHost } from "../support/host.tsx";

afterEach(cleanup);

describe("omfp:canvas", () => {
  // spec: UI-1000
  test("the default view's only side tab says the panel is empty, what the agent can put there, and the views", () => {
    expect(BUILTIN_WIDGETS.get("omfp:canvas")).toBe(CanvasWidget);
    expect(DEFAULT_VIEW.layout.at(-1)).toEqual(["tabs", { id: "side" }, "omfp:canvas"]);
    renderWithHost(<CanvasWidget />);
    const canvas = screen.getByTestId("canvas");
    expect(canvas.textContent).toContain("This panel is empty");
    for (const said of [
      "Ask the agent to fill it: GitHub or Linear issues, the git diff, the repos in your workspace, anything.",
      "It can add tabs or remove the panel too.",
      "Each workspace can have several views; this is the default one.",
      "New here? Ask the agent for a crash course on what omfp can do",
    ]) {
      expect(canvas.textContent).toContain(said);
    }
  });
});
