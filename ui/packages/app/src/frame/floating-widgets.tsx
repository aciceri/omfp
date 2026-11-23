/**
 * Floating panels (UI-230, AGT-060): each panel of the current workspace's valid manifest floats over whatever view the
 * window shows, with a bar (grip, title, minimise, close) over its node and a resize corner. Panels open below the
 * view's top bars, side by side, then cascaded (UI-454); each keeps its box and minimised state per workspace and id
 * in this window. Closing one removes it from
 * `views.json` with `views.unmount` (UI-231); closing one an init file added hides it in this window (UI-261).
 */

import { IconButton } from "@omfp/kit";
import type { ViewsState } from "@omfp/protocol";
import type { ReactNode } from "react";
import { useEffect } from "react";
import { selectOpenedTab } from "../sdk/files.ts";
import {
  floatingWidgetKey,
  NO_FILE_TABS,
  OPEN_FILES,
  openWidgetTab,
  REVEAL_WIDGET,
  reportError,
  useHost,
  useWindowState,
  useWorkspaces,
  widgetTabId,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { containsWidget, hasMainSlot } from "../view/layout.ts";
import { PanelLayout, widgetTitle } from "../view/render.tsx";
import type { WidgetRegistry } from "../view/resolve.ts";
import type { ResolvedPanel, ResolvedView } from "../view/views.ts";
import { floatingControls, useFloatingBox } from "./floating-box.tsx";

const text = strings.shell.panels;

export interface FloatingWidgetsProps {
  readonly panels: readonly ResolvedPanel[];
  readonly workspaceId: string | undefined;
  readonly views: ViewsState | undefined;
  readonly registry: WidgetRegistry;
}

export function FloatingWidgets({ panels, workspaceId, views, registry }: FloatingWidgetsProps): ReactNode {
  if (workspaceId === undefined) {
    return null;
  }
  return panels.map((panel, index) => (
    <FloatingWidget
      key={panel.id}
      panel={panel}
      index={index}
      workspaceId={workspaceId}
      views={views}
      registry={registry}
    />
  ));
}

export interface RevealedWidgetProps extends FloatingWidgetsProps {
  readonly view: ResolvedView | undefined;
}

/**
 * The widget of the window's latest reveal request (UI-274, UI-373) when neither the view nor a panel of the manifest
 * holds it: a closable tab of the view's `main` slot (with the open files, window-local per workspace, UI-373), else,
 * in a view without a `main` slot, a floating panel of this window only (`views.json` is not written); closing it
 * drops the request. A request for another workspace than the current one always floats, its widget given that
 * workspace as its `workspaceId` prop (UI-741).
 */
export function RevealedWidget({ view, panels, workspaceId, views, registry }: RevealedWidgetProps): ReactNode {
  const app = useHost();
  const [request, setRequest] = useWindowState(REVEAL_WIDGET);
  const [, setOpened] = useWindowState(OPEN_FILES);
  const workspaces = useWorkspaces();
  const other =
    request?.workspaceId !== undefined && request.workspaceId !== workspaceId ? request.workspaceId : undefined;
  const names = request === undefined || other !== undefined ? [] : [request.reference];
  const held =
    names.length > 0 &&
    ((view !== undefined && containsWidget(view.tree, names)) ||
      panels.some((panel) => containsWidget(panel.tree, names)));
  const tab = !held && other === undefined && workspaceId !== undefined && view !== undefined && hasMainSlot(view.tree);
  // Once the view's tabs showed it (their effects run first) or a tab of `main` opened it, the request is done: a later
  // view without the widget must not float it. The tab opens for every thread of the workspace and is selected for the
  // shown one only (UI-372).
  useEffect(() => {
    if (request === undefined) {
      return;
    }
    if (tab && workspaceId !== undefined) {
      setOpened((all) => ({
        ...all,
        [workspaceId]: openWidgetTab(all[workspaceId] ?? NO_FILE_TABS, request.reference),
      }));
      selectOpenedTab(app, workspaceId, widgetTabId(request.reference));
      setRequest(undefined);
    } else if (held) {
      setRequest(undefined);
    }
  }, [app, request, held, tab, workspaceId, setOpened, setRequest]);
  // Without a current workspace, the other workspace's panel floats in its own.
  const host = workspaceId ?? other;
  if (request === undefined || host === undefined || held || tab) {
    return null;
  }
  const title = widgetTitle(request.reference);
  const panel: ResolvedPanel =
    other === undefined
      ? {
          id: `reveal/${request.reference}`,
          title,
          node: request.reference,
          tree: { kind: "widget", name: request.reference, props: {} },
        }
      : {
          id: `reveal/${request.reference}/${other}`,
          title: text.ofWorkspace(title, workspaces.get(other)?.name ?? other),
          node: [request.reference, { workspaceId: other }],
          tree: { kind: "widget", name: request.reference, props: { workspaceId: other } },
        };
  return (
    <FloatingWidget
      panel={panel}
      index={panels.length}
      workspaceId={host}
      views={host === workspaceId ? views : undefined}
      registry={registry}
      onClose={() => setRequest(undefined)}
    />
  );
}

interface FloatingWidgetProps {
  readonly panel: ResolvedPanel;
  /** Closes the panel; by default `views.unmount` removes it from `views.json` (UI-231). */
  readonly onClose?: () => void;
  /** Its place in id order: panels without a box of their own open side by side, then cascaded, by it (UI-454). */
  readonly index: number;
  readonly workspaceId: string;
  readonly views: ViewsState | undefined;
  readonly registry: WidgetRegistry;
}

function FloatingWidget({ panel, index, workspaceId, views, registry, onClose }: FloatingWidgetProps): ReactNode {
  const host = useHost();
  const box = useFloatingBox(floatingWidgetKey(workspaceId, panel.id), index);
  const { title } = panel;
  const controls = floatingControls(box, "floating-widget", {
    move: text.move(title),
    resize: text.resize(title),
    minimise: text.minimise(title),
    restore: text.restore(title),
  });
  const close = () => {
    if (onClose !== undefined) {
      onClose();
      return;
    }
    if (panel.init !== undefined) {
      host.init.closePanel(panel.id);
      return;
    }
    host.client
      .call("views.unmount", { workspaceId, id: panel.id })
      .catch((error: unknown) => reportError(host, error));
  };
  return (
    <section
      ref={box.root}
      className="omfp-floating-widget"
      aria-label={title}
      data-testid="floating-widget"
      data-panel={panel.id}
      data-minimised={box.panel.minimised ? "" : undefined}
      style={box.style}
    >
      <div className="omfp-floating-file__bar">
        {controls.grip}
        <span className="omfp-floating-file__title" data-testid="floating-widget-title">
          {title}
        </span>
        {controls.minimise}
        <IconButton label={text.close(title)} icon="×" onPress={close} data-testid="floating-widget-close" />
      </div>
      {/* Minimised, the widget stays mounted (its state too), hidden. */}
      <div className="omfp-floating-file__body" hidden={box.panel.minimised}>
        <PanelLayout id={panel.id} tree={panel.tree} registry={registry} workspaceId={workspaceId} views={views} />
      </div>
      {controls.resize}
    </section>
  );
}
