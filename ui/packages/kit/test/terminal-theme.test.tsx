/** L4: the kit's `TerminalEmulator` in the theme's colors (TRM-110, TRM-111), over xterm.js's DOM renderer. */

import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import type { ITerminalOptions } from "@xterm/xterm";
import { TerminalEmulator } from "../src/index.ts";

/** Two themes as tokens: the default one, and `paper` on `<html data-theme="paper">`. */
const THEMES = `
:root {
  --omfp-terminal-bg: #101418;
  --omfp-terminal-fg: #e0e4e8;
  --omfp-terminal-cursor: #40c0e0;
  --omfp-terminal-cursor-accent: #003040;
  --omfp-terminal-selection: #203848;
  --omfp-terminal-red: #ff8080;
  --omfp-terminal-bright-cyan: #80f0f0;
}
:root[data-theme="paper"] {
  --omfp-terminal-bg: #f8f6f0;
  --omfp-terminal-fg: #202428;
  --omfp-terminal-cursor: #006070;
  --omfp-terminal-cursor-accent: #ffffff;
  --omfp-terminal-selection: #c0dce0;
  --omfp-terminal-red: #a01818;
  --omfp-terminal-bright-cyan: #005860;
}`;

/** A widget's own colors: a stable object, as a new one re-creates the emulator. */
const OVERRIDE: ITerminalOptions = { theme: { background: "#010203", red: "#ff0000" } };

afterEach(() => {
  cleanup();
  for (const style of document.head.querySelectorAll("style[data-test-theme]")) {
    style.remove();
  }
  document.documentElement.removeAttribute("data-theme");
});

function installThemes(): void {
  const style = document.createElement("style");
  style.dataset.testTheme = "";
  style.textContent = THEMES;
  document.head.append(style);
}

/** The colors xterm.js paints with, from its DOM renderer's stylesheet and its viewport. */
function painted(container: HTMLElement): Record<string, string | undefined> {
  const css = [...container.querySelectorAll("style")].map((style) => style.textContent).join("\n");
  const pick = (pattern: RegExp) => pattern.exec(css)?.[1];
  return {
    background: /background-color: (#[0-9a-f]{6})/.exec(
      container.querySelector(".xterm-viewport")?.getAttribute("style") ?? "",
    )?.[1],
    foreground: pick(/\.xterm-rows \{ color: (#[0-9a-f]{6});/),
    cursor: pick(/\.xterm-cursor-block \{ background-color: (#[0-9a-f]{6});/),
    cursorAccent: pick(/\.xterm-cursor-block \{ background-color: #[0-9a-f]{6}; color: (#[0-9a-f]{6});/),
    selection: pick(/\.xterm-selection div \{ position: absolute; background-color: (#[0-9a-f]{6});/),
    red: pick(/\.xterm-fg-1 \{ color: (#[0-9a-f]{6});/),
    brightCyan: pick(/\.xterm-fg-14 \{ color: (#[0-9a-f]{6});/),
  };
}

describe("TerminalEmulator colors", () => {
  // spec: TRM-110
  test("the canvas, text, cursor, selection and ANSI colors are the theme's terminal tokens", () => {
    installThemes();
    const { container } = render(<TerminalEmulator sessionKey="one" chunks={[]} onData={() => undefined} />);

    expect(painted(container)).toEqual({
      background: "#101418",
      foreground: "#e0e4e8",
      cursor: "#40c0e0",
      cursorAccent: "#003040",
      selection: "#203848",
      red: "#ff8080",
      brightCyan: "#80f0f0",
    });
  });

  // spec: TRM-111
  test("a theme change repaints the open emulator in place, without re-creating it", async () => {
    installThemes();
    let resizes = 0;
    const { container } = render(
      <TerminalEmulator
        sessionKey="one"
        chunks={[]}
        onData={() => undefined}
        onTerminalResize={() => {
          resizes += 1;
        }}
      />,
    );
    const emulator = container.querySelector(".xterm");
    const resizesBefore = resizes;

    document.documentElement.setAttribute("data-theme", "paper");

    await waitFor(() =>
      expect(painted(container)).toEqual({
        background: "#f8f6f0",
        foreground: "#202428",
        cursor: "#006070",
        cursorAccent: "#ffffff",
        selection: "#c0dce0",
        red: "#a01818",
        brightCyan: "#005860",
      }),
    );
    expect([container.querySelectorAll(".xterm").length, container.querySelector(".xterm") === emulator]).toEqual([
      1,
      true,
    ]);
    expect(resizes).toBe(resizesBefore);
  });

  // spec: TRM-111
  test("the colors of a widget's options.theme win over the tokens, also after a theme change", async () => {
    installThemes();
    const { container } = render(
      <TerminalEmulator sessionKey="one" chunks={[]} onData={() => undefined} options={OVERRIDE} />,
    );
    expect([painted(container).background, painted(container).red, painted(container).foreground]).toEqual([
      "#010203",
      "#ff0000",
      "#e0e4e8",
    ]);

    document.documentElement.setAttribute("data-theme", "paper");

    await waitFor(() => expect(painted(container).foreground).toBe("#202428"));
    expect([painted(container).background, painted(container).red]).toEqual(["#010203", "#ff0000"]);
  });
});
