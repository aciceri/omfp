/** L2: key hints hidden on touch-only screens (UI-418): the shared `KeyHint` marker and its media rule. */

import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { cleanup, render, screen } from "@testing-library/react";
import { HelpRow, LivePanel, WhichKeyItem } from "../src/index.ts";
import { KIT_SRC, parseCss } from "./support/css.ts";

afterEach(cleanup);

const TOUCH = "@media (hover: none) and (pointer: coarse)";

describe("KeyHint", () => {
  // spec: UI-418
  test("touch-only screens hide the hint rows whole, next to the decorative chips", () => {
    const rules = parseCss(readFileSync(join(KIT_SRC, "kit.css"), "utf8"));
    const hidden = rules.filter(
      (rule) => rule.at.includes(TOUCH) && rule.selector.split(",").some((part) => part.trim() === "span.omfp-keyhint"),
    );
    expect(hidden.length).toBe(1);
    expect(hidden[0]?.declarations.get("display")).toBe("none");
    expect(hidden[0]?.selector).toContain('.omfp-kbd[aria-hidden="true"]');
  });

  // spec: UI-418
  test("the live panel's key hints carry the marker with their separator and label; the status stays", () => {
    render(
      <LivePanel
        title="Live voice"
        status="listening"
        statusLabel="listening"
        lines={[]}
        hints={[
          { keys: "spc", label: "mute" },
          { keys: "esc", label: "end" },
        ]}
        data-testid="live"
      />,
    );
    const panel = screen.getByTestId("live");
    const hints = [...panel.querySelectorAll("span.omfp-keyhint")];
    expect(hints.map((hint) => hint.textContent)).toEqual(["·spc mute", "·esc end"]);
    expect(panel.querySelectorAll(".omfp-keyhint .omfp-live__sep").length).toBe(2);
    expect(panel.querySelector(".omfp-live__status")?.textContent).toBe("●listening·spc mute·esc end");
    expect(panel.querySelector(".omfp-keyhint .omfp-live__dot")).toBeNull();
  });

  // spec: UI-418
  test("content chips of help and which-key rows do not carry the marker", () => {
    render(
      <dl data-testid="rows">
        <HelpRow keys={["spc n"]} description="new thread" />
        <WhichKeyItem chip="n" description="new thread" />
      </dl>,
    );
    const rows = screen.getByTestId("rows");
    expect(rows.querySelectorAll("kbd").length).toBe(2);
    expect(rows.querySelectorAll(".omfp-keyhint").length).toBe(0);
    expect(rows.querySelectorAll('kbd[aria-hidden="true"]').length).toBe(0);
  });
});
