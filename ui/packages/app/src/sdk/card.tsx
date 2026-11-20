/**
 * The widget of an agent's card in the transcript (UI-920), as the chat (and a copy of it) draws it from `omfp/sdk`.
 * The window's frame provides the renderer, which needs the view slot: the SDK names it without importing it, so the
 * SDK never depends on the views that depend on it.
 */

import { type ComponentType, createContext, type ReactNode, useContext } from "react";
import type { WidgetProps } from "./widget.ts";

export interface CardWidgetProps {
  readonly threadId: string;
  readonly toolCallId: string;
  /** The widget reference of the call (`./widgets/<name>`, `user:<name>`, `omfp:<name>`). */
  readonly reference: string | undefined;
  /** The one-file source of the call, used instead of `reference`. */
  readonly code: string | undefined;
  readonly props: WidgetProps;
}

/** The renderer of a card's widget; the frame provides it. */
export const CardWidgetScope = createContext<ComponentType<CardWidgetProps> | undefined>(undefined);

/** A card's widget, drawn by the window's renderer; nothing outside a window. */
export function CardWidget(props: CardWidgetProps): ReactNode {
  const Render = useContext(CardWidgetScope);
  return Render === undefined ? null : <Render {...props} />;
}
