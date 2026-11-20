/**
 * The widget of an agent's card in the transcript (UI-920): an `omfp_card` call (AGT-096) draws a widget reference in
 * the slot every view widget renders in (resolution, loading, errors in place), or the module the daemon compiles
 * from its one-file source; under the instance `card:<toolCallId>` of its thread's workspace.
 */

import { createContext, type ReactNode, useContext, useMemo } from "react";
import { useAnswer } from "../sdk/answer.ts";
import type { CardWidgetProps } from "../sdk/card.tsx";
import { useThreadSummary, useViews } from "../sdk/hooks.ts";
import type { WidgetRegistry } from "./resolve.ts";
import { SourceSlot, WidgetSlot } from "./slot.tsx";

/** The window's built-in widgets, which a card's `omfp:<name>` reference resolves to; the frame provides its own. */
export const WidgetRegistryScope = createContext<WidgetRegistry>(new Map());

/** A card's widget (UI-920), its answer's state (UI-921) as `data-state`; drawn again from the call's arguments
 * whenever it mounts. The frame provides it to the SDK's `CardWidget` (`CardWidgetScope`). */
export function CardWidgetView({ threadId, toolCallId, reference, code, props }: CardWidgetProps): ReactNode {
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
