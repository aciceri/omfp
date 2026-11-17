/** L0: the todo list and the goal (UI-082), from the thread state. */

import { describe, expect, test } from "bun:test";
import { replay } from "@omfp/core";
import { todosView } from "../../src/vm/todos.ts";
import { fixtureEvents } from "../widgets/chat/fixtures.ts";

describe("todosView", () => {
  // spec: UI-082, PRT-057
  test("the phases of the recorded todo calls, with their progress", () => {
    const view = todosView(replay(fixtureEvents("todo_tool_calls")));
    expect(view.goal).toBeUndefined();
    expect(view.phases.map((phase) => [phase.name, phase.progress])).toEqual([["Tasks", "2/2"]]);
    expect(view.phases[0]?.tasks.map((task) => [task.content, task.status, task.statusLabel])).toEqual([
      ["Read the code", "completed", "done"],
      ["Fix the bug", "completed", "done"],
    ]);
    expect(view.progress).toBe("2/2");
  });

  // spec: UI-082, PRT-056
  test("the goal with its token use; abandoned tasks do not count", () => {
    const view = todosView({
      goal: {
        id: "g",
        objective: "Ship M8",
        status: "budget-limited",
        tokensUsed: 12_345,
        timeUsedSeconds: 3,
        createdAt: 0,
        updatedAt: 0,
        tokenBudget: 50_000,
      },
      todo_phases: [
        {
          name: "Build",
          tasks: [
            { content: "Widgets", status: "in_progress" },
            { content: "Old idea", status: "abandoned" },
            { content: "Reducer", status: "completed" },
          ],
        },
        { name: "Empty", tasks: [] },
      ],
    });
    expect(view.goal).toEqual({
      objective: "Ship M8",
      status: "budget-limited",
      statusLabel: "budget reached",
      usage: "12.3k / 50k tokens",
    });
    expect(view.phases.map((phase) => phase.name)).toEqual(["Build"]);
    expect(view.phases[0]?.tasks[0]).toEqual({ content: "Widgets", status: "in_progress", statusLabel: "in progress" });
    expect(view.progress).toBe("1/2");
  });
});
