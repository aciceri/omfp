/**
 * The `omfp:todos` widget (UI-082): the thread's todo list from its state (`todo_phases`, PRT-057) and the session's
 * goal (`goal`, PRT-056), as data.
 */

import type { Goal, ThreadState } from "@omfp/core";
import { compactNumber } from "@omfp/kit";
import { strings } from "../strings/index.ts";

const text = strings.shell.todos;

/** One task with omp's status (`pending`, `in_progress`, `completed`, `abandoned`). */
export interface TodoRow {
  readonly content: string;
  readonly status: string;
  /** The status in words, for assistive technology. */
  readonly statusLabel: string;
}

export interface TodoPhaseView {
  readonly name: string;
  readonly tasks: readonly TodoRow[];
  /** `completed / all` of the phase, abandoned tasks left out. */
  readonly progress: string;
}

export interface GoalView {
  readonly objective: string;
  /** omp's status (`active`, `paused`, `budget-limited`, `complete`, `dropped`). */
  readonly status: string;
  readonly statusLabel: string;
  /** `12.3k tokens`, or `12.3k / 50k tokens` with a budget. */
  readonly usage: string;
}

export interface TodosView {
  readonly goal: GoalView | undefined;
  readonly phases: readonly TodoPhaseView[];
  /** `completed / all` over every phase; `undefined` without tasks. */
  readonly progress: string | undefined;
}

function goalView(goal: Goal): GoalView {
  const used = compactNumber(goal.tokensUsed);
  return {
    objective: goal.objective,
    status: goal.status,
    statusLabel: text.goalStatus[goal.status] ?? goal.status,
    usage: goal.tokenBudget === undefined ? text.tokens(used) : text.tokensOf(used, compactNumber(goal.tokenBudget)),
  };
}

/** `completed / counted` of `tasks`, abandoned ones not counted. */
function progress(tasks: readonly { status: string }[]): string {
  const counted = tasks.filter((task) => task.status !== "abandoned");
  return `${counted.filter((task) => task.status === "completed").length}/${counted.length}`;
}

export function todosView(state: Pick<ThreadState, "goal" | "todo_phases">): TodosView {
  const phases = state.todo_phases
    .filter((phase) => phase.tasks.length > 0)
    .map((phase) => ({
      name: phase.name,
      tasks: phase.tasks.map((task) => ({
        content: task.content,
        status: task.status,
        statusLabel: text.taskStatus[task.status] ?? task.status,
      })),
      progress: progress(phase.tasks),
    }));
  const tasks = phases.flatMap((phase) => phase.tasks);
  return {
    goal: state.goal === null ? undefined : goalView(state.goal),
    phases,
    progress: tasks.length === 0 ? undefined : progress(tasks),
  };
}
