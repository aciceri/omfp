/**
 * `omfp:web` (UI-670): a web page — a dashboard, docs, a local dev server's preview, any site — filling its slot, and
 * nothing else: the kit's `WebView`, a native view of the desktop window (UI-682) and a sandboxed frame elsewhere.
 * Props: `url`, an absolute `http(s)` URL other than the daemon's own origin; `title`, the tab's label and the page's
 * accessible name. Agents place it with `omfp_ui_mount { id, widget: "omfp:web", props: { url } }`; a browser with
 * an address bar, back and forward is a workspace widget of its own around `WebView` (UI-685).
 */

import { WebView } from "omfp/kit";
import type { WidgetProps } from "omfp/sdk";
import type { ReactNode } from "react";

export function WebWidget({ url, title }: WidgetProps): ReactNode {
  return <WebView url={url} title={typeof title === "string" ? title : undefined} />;
}
