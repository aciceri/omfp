/**
 * `omfp:canvas` (UI-1000): the default view's side panel, empty on purpose. It says what the panel is for: the agent
 * fills it (issues, the git diff, the workspace's repositories, anything), adds tabs or removes it, and a workspace can
 * have several views besides this default one.
 */

import { EmptyState } from "omfp/kit";
import { strings, type WidgetProps } from "omfp/sdk";
import type { ReactNode } from "react";

const text = strings.shell.canvas;

export function CanvasWidget(_props: WidgetProps): ReactNode {
  return (
    <EmptyState icon="panel-right" title={text.title} bare className="omfp-canvas" data-testid="canvas">
      <span className="omfp-canvas__line">{text.body}</span>
      <span className="omfp-canvas__line">{text.views}</span>
      <span className="omfp-canvas__line">{text.tour}</span>
    </EmptyState>
  );
}
