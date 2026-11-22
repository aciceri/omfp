/** L2: the pickers' and command menu's key rows are touch-hidden `KeyHint`s (UI-418); the picker's notice stays. */

import { afterEach, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import { MenuKeys } from "../../src/frame/command-line.tsx";
import { Picker } from "../../src/frame/picker.tsx";

afterEach(cleanup);

// spec: UI-418
test("a numbered picker's footer keys and option numbers are key hints; its notice is not", () => {
  render(
    <Picker
      id="pick"
      title="Pick"
      empty="none"
      options={[
        { id: "a", title: "Alpha", description: "first" },
        { id: "b", title: "Beta", description: "second" },
      ]}
      numbered
      notice="2 found"
      choose={() => {}}
      cancel={() => {}}
    />,
  );
  const picker = screen.getByTestId("pick");
  const footer = picker.querySelector(".omfp-cmdmenu__foot");
  expect(footer?.querySelectorAll("kbd").length).toBe(5);
  expect(footer?.querySelectorAll(".omfp-keyhint kbd").length).toBe(5);
  expect(footer?.querySelectorAll(".omfp-keyhint").length).toBe(4);
  expect(footer?.querySelector('[role="status"]')?.closest(".omfp-keyhint")).toBeNull();
  const options = screen.getAllByTestId("pick-option");
  expect(options.map((option) => option.querySelector(".omfp-keyhint kbd")?.textContent)).toEqual(["1", "2"]);
  expect(options.map((option) => option.textContent?.includes(option.dataset.id === "a" ? "first" : "second"))).toEqual(
    [true, true],
  );
});

// spec: UI-418
test("the command menu's key row is three key hints, each chip with its label", () => {
  render(
    <div data-testid="keys">
      <MenuKeys />
    </div>,
  );
  const hints = [...screen.getByTestId("keys").querySelectorAll(".omfp-keyhint")];
  expect(hints.length).toBe(3);
  expect(hints.map((hint) => hint.querySelectorAll("kbd").length)).toEqual([1, 1, 1]);
  expect(screen.getByTestId("keys").children.length).toBe(3);
});
