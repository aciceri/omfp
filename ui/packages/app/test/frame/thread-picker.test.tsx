import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, screen } from "@testing-library/react";
import { ThreadPicker } from "../../src/frame/thread-picker.tsx";
import { SELECTED_THREAD } from "../../src/sdk/index.ts";
import { thread, workspace } from "../support/fixtures.ts";
import { renderWithHost } from "../support/host.tsx";

afterEach(cleanup);

// spec: UI-022
test("fuzzy filtering and arrows preview live; Escape restores the original, Enter commits", () => {
  const test = renderWithHost(<ThreadPicker />);
  test.client.index({
    workspaces: [workspace("w1")],
    threads: [thread("t1", "Original", "w1"), thread("t2", "Alpha beta", "w1"), thread("t3", "Alpine boat", "w1")],
  });
  test.flush();
  act(() => test.host.window.set(SELECTED_THREAD, "t1"));
  const open = () =>
    act(() => {
      test.host.commands.run("threads.pick");
    });
  open();
  fireEvent.change(screen.getByTestId("thread-picker-input"), { target: { value: "apbt" } });
  expect(test.host.window.get(SELECTED_THREAD)).toBe("t2");
  fireEvent.keyDown(screen.getByTestId("thread-picker-input"), { key: "Escape" });
  expect(test.host.window.get(SELECTED_THREAD)).toBe("t1");
  open();
  fireEvent.change(screen.getByTestId("thread-picker-input"), { target: { value: "alb" } });
  fireEvent.keyDown(screen.getByTestId("thread-picker-input"), { key: "ArrowDown" });
  expect(test.host.window.get(SELECTED_THREAD)).toBe("t3");
  fireEvent.keyDown(screen.getByTestId("thread-picker-input"), { key: "Enter" });
  expect(test.host.window.get(SELECTED_THREAD)).toBe("t3");
  expect(screen.queryByTestId("thread-picker")).toBeNull();
});

// spec: UI-022
test("an empty result never loses the original selection, including a picker opened without a thread", () => {
  const test = renderWithHost(<ThreadPicker />);
  test.client.index({ workspaces: [workspace("w1")], threads: [thread("t1", "Alpha", "w1")] });
  test.flush();
  act(() => {
    test.host.commands.run("threads.pick");
  });
  fireEvent.change(screen.getByTestId("thread-picker-input"), { target: { value: "zzzz" } });
  fireEvent.keyDown(screen.getByTestId("thread-picker-input"), { key: "Enter" });
  expect(screen.getByTestId("thread-picker").textContent).toContain("No thread matches");
  fireEvent.keyDown(screen.getByTestId("thread-picker-input"), { key: "Escape" });
  expect(test.host.window.get(SELECTED_THREAD)).toBeUndefined();
});
