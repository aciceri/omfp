/**
 * What a widget reference of a view renders (AGT-010, AGT-022): a built-in component, a workspace widget module, or
 * nothing (an unknown reference shows an error in its place).
 */

import type { ViewsState, WidgetInfo } from "@omfp/protocol";
import type { WidgetComponent } from "../sdk/index.ts";

/** The built-in `omfp:*` widgets by reference. */
export type WidgetRegistry = ReadonlyMap<string, WidgetComponent>;

export type ResolvedWidget =
  | { readonly kind: "builtin"; readonly component: WidgetComponent }
  /** A widget folder the daemon serves (its `entry` is `null` when the folder has none). */
  | { readonly kind: "module"; readonly widget: WidgetInfo }
  /** A workspace widget while the workspace's views state has not arrived yet. */
  | { readonly kind: "pending" }
  | { readonly kind: "missing" };

const BUILTIN = "omfp:";
const WORKSPACE = "./widgets/";

/** The workspace's widget folder `name`, from the views state's list. */
function folder(views: ViewsState, name: string): WidgetInfo | undefined {
  return views.widgets.find((widget) => widget.name === name);
}

/**
 * Resolves `reference` in the workspace `workspaceId` with its views state (`undefined` until it arrives):
 * `omfp:<name>` is the workspace widget `<name>` when that folder exists (AGT-022), else the built-in;
 * `./widgets/<name>` the workspace's folder.
 */
export function resolveWidget(
  reference: string,
  workspaceId: string | undefined,
  views: ViewsState | undefined,
  registry: WidgetRegistry,
): ResolvedWidget {
  if (reference.startsWith(BUILTIN)) {
    const override = views === undefined ? undefined : folder(views, reference.slice(BUILTIN.length));
    if (override !== undefined) {
      return { kind: "module", widget: override };
    }
    const component = registry.get(reference);
    return component === undefined ? { kind: "missing" } : { kind: "builtin", component };
  }
  const name = reference.startsWith(WORKSPACE) ? reference.slice(WORKSPACE.length) : "";
  if (name === "" || workspaceId === undefined) {
    return { kind: "missing" };
  }
  if (views === undefined) {
    return { kind: "pending" };
  }
  const widget = folder(views, name);
  return widget === undefined ? { kind: "missing" } : { kind: "module", widget };
}

/** The URL of a widget's entry (DMN-084): `<widgetBase>/<workspace id>/<name>/<version>/<entry>`. */
export function widgetUrl(widgetBase: string, workspaceId: string, widget: WidgetInfo, entry: string): string {
  return `${widgetModulePath(widgetBase, workspaceId, widget)}${encodeURIComponent(widget.version)}/${encodeURIComponent(entry)}`;
}

/** What every URL of a widget's modules starts with, whatever the version: `<widgetBase>/<workspace id>/<name>/`
 * (stacks naming it are the widget's, AGT-026). */
export function widgetModulePath(widgetBase: string, workspaceId: string, widget: WidgetInfo): string {
  return `${widgetBase}/${encodeURIComponent(workspaceId)}/${encodeURIComponent(widget.name)}/`;
}
