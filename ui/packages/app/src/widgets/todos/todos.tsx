/**
 * `omfp:todos` (UI-082): the selected thread's todo list, phase by phase, from its state (`todo_phases`), under the
 * session's goal when it has one (`goal`). Each phase is a box titled on its border with its progress at the end
 * (UI-187) holding the kit's todo list, the todo tool card's (UI-163, UI-447): blocked tasks say on what.
 */

import { BorderBox, EmptyState, TodoList } from "omfp/kit";
import { SELECTED_THREAD, strings, todosView, useThread, useWindowState, type WidgetProps } from "omfp/sdk";
import { type ReactNode, useMemo } from "react";

const text = strings.shell.todos;

export function TodosWidget(_props: WidgetProps): ReactNode {
  const [threadId] = useWindowState(SELECTED_THREAD);
  const state = useThread(threadId)?.state;
  const view = useMemo(() => (state === undefined ? undefined : todosView(state)), [state]);
  if (threadId === undefined) {
    return <EmptyState icon="inbox" title={text.noThread} data-testid="todos-no-thread" />;
  }
  return (
    <section className="omfp-todos" data-testid="todos" aria-label={text.label}>
      {view?.goal !== undefined && (
        <BorderBox
          title={text.goal}
          level={2}
          end={view.goal.statusLabel}
          className="omfp-todos__goal"
          data-testid="todos-goal"
          data-status={view.goal.status}
        >
          <p className="omfp-todos__objective">{view.goal.objective}</p>
          <p className="omfp-todos__meta">{view.goal.usage}</p>
        </BorderBox>
      )}
      {view === undefined || view.phases.length === 0 ? (
        <EmptyState icon="check" title={text.none} data-testid="todos-none">
          {text.noneHint}
        </EmptyState>
      ) : (
        view.phases.map((phase) => (
          <BorderBox key={phase.name} title={phase.name} level={2} end={phase.progress} data-testid="todos-phase">
            <TodoList
              itemTestId="todos-task"
              items={phase.tasks.map((task) => ({
                text: task.content,
                status: task.status,
                statusLabel: task.statusLabel,
              }))}
            />
          </BorderBox>
        ))
      )}
    </section>
  );
}
