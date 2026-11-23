/**
 * `:widget add <reference>` (UI-300): adds a widget to the view this window shows and writes `views.json` with
 * `views.set`; the window then renders the manifest the daemon pushes back. Views an init file added exist only in
 * code: they refuse it and name the file to edit.
 */

import type { ViewsState } from "@omfp/protocol";
import { useCallback } from "react";
import { useCall, useReportError } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { manifestWithWidget } from "../view/edit.ts";
import type { WidgetRegistry } from "../view/resolve.ts";
import type { ResolvedView } from "../view/views.ts";

const text = strings.shell.layout;

/**
 * Why `:widget add` cannot write the view `view` (UI-300): an init file added it, so writing `views.json` would change
 * nothing the window shows; `undefined` for a view of `views.json` (init transforms included).
 */
export function codeOnly(view: ResolvedView): string | undefined {
  return view.init === undefined ? undefined : text.codeInit(view.id, strings.init.files[view.init]);
}

/** The references `:widget add` completes: built-ins and the workspace's widgets. */
export function widgetReferences(registry: WidgetRegistry, views: ViewsState | undefined): string[] {
  return [...registry.keys(), ...(views?.widgets ?? []).map((widget) => `./widgets/${widget.name}`)];
}

/**
 * Adds a widget to `view` (the view the window shows) of `workspaceId` (UI-300); refusals and failures go to the
 * frame's error list. Resolves to whether `views.json` was written.
 */
export function useAddWidget(
  workspaceId: string | undefined,
  views: ViewsState | undefined,
  view: ResolvedView | undefined,
): (reference: string) => Promise<boolean> {
  const call = useCall();
  const report = useReportError();
  return useCallback(
    async (reference: string) => {
      if (workspaceId === undefined) {
        report(text.noWorkspace);
        return false;
      }
      // Before the views topic answers, an existing views.json is unknown and must not be overwritten.
      if (views === undefined || view === undefined) {
        report(text.loading);
        return false;
      }
      const refused = codeOnly(view);
      if (refused !== undefined) {
        report(refused);
        return false;
      }
      const manifest = manifestWithWidget(views, view.id, reference);
      if (!manifest.ok) {
        report(manifest.error);
        return false;
      }
      try {
        await call("views.set", { workspaceId, manifest: manifest.value });
        return true;
      } catch (error) {
        report(error);
        return false;
      }
    },
    [call, report, workspaceId, views, view],
  );
}
