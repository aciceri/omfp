/**
 * The widget of an agent's card in the transcript (UI-920): an `omfp_card` call (AGT-096) draws a widget reference in
 * the slot every view widget renders in (resolution, loading, errors in place), or the module the daemon compiles
 * from its one-file source; under the instance `card:<toolCallId>` of its thread's workspace.
 */

import { createContext, type ReactNode, useContext, useMemo } from "react";
import { useAnswer } from "../sdk/answer.ts";
import { useThreadSummary, useViews } from "../sdk/hooks.ts";
import type { WidgetProps } from "../sdk/widget.ts";
import type { WidgetRegistry } from "./resolve.ts";
import { SourceSlot, WidgetSlot } from "./slot.tsx";

/** The window's built-in widgets, which a card's `omfp:<name>` reference resolves to; the frame provides its own. */
export const WidgetRegistryScope = createContext<WidgetRegistry>(new Map());

export interface CardWidgetProps {
  readonly threadId: string;
  readonly toolCallId: string;
  /** The widget reference of the call (`./widgets/<name>`, `user:<name>`, `omfp:<name>`). */
  readonly reference: string | undefined;
  /** The one-file source of the call, used instead of `reference`. */
  readonly code: string | undefined;
  readonly props: WidgetProps;
}

/** A card's widget (UI-920), its answer's state (UI-921) as `data-state`; drawn again from the call's arguments
 * whenever it mounts. */
export function CardWidget({ threadId, toolCallId, reference, code, props }: CardWidgetProps): ReactNode {
  const { state } = useAnswer();
  const workspaceId = useThreadSummary(threadId)?.workspaceId;
  const views = useViews(workspaceId);
  const registry = useContext(WidgetRegistryScope);
  const instance = `card:${toolCallId}`;
  const slotProps = useMemo(() => ({ ...props, instance }), [props, instance]);
  const path = `card/${toolCallId}`;
  return (
    <div className="omfp-card-widget" data-testid="card-widget" data-state={state}>
      {code !== undefined ? (
        <SourceSlot code={code} props={slotProps} path={path} workspaceId={workspaceId} />
      ) : (
        reference !== undefined && (
          <WidgetSlot
            reference={reference}
            props={slotProps}
            path={path}
            workspaceId={workspaceId}
            views={views}
            registry={registry}
          />
        )
      )}
    </div>
  );
}
