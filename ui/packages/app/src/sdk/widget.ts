/** What a widget is (docs/specs/08-agent-control.md): a React component taking its view node's props, rendered in a
 * {@link WidgetScope} that tells it which instance it is. */

import { type ComponentType, createContext, useContext } from "react";

/** The props of a widget reference in a view (`["omfp:changes", { "repo": "x" }]` → `{ repo: "x" }`). */
export type WidgetProps = Readonly<Record<string, unknown>>;

export type WidgetComponent = ComponentType<WidgetProps>;

/** Which widget instance renders (AGT-010, AGT-016). */
export interface WidgetInfoScope {
  /** The node's `instance` prop when it is a string, else its reference; keys {@link useStored} state. */
  readonly instance: string;
  /** The reference of the view node (`omfp:chat`, `./widgets/tests`). */
  readonly reference: string;
  /** The workspace whose view holds the widget; `undefined` without a current workspace. */
  readonly workspaceId: string | undefined;
}

/** Provided by the widget slot around every widget of a view. */
export const WidgetScope = createContext<WidgetInfoScope | undefined>(undefined);

/** The widget instance being rendered; throws outside a widget slot. */
export function useWidget(): WidgetInfoScope {
  const scope = useContext(WidgetScope);
  if (scope === undefined) {
    throw new Error("useWidget outside a widget slot");
  }
  return scope;
}
