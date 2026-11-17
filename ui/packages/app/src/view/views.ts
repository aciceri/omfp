/**
 * The views a window can show (AGT-010): the built-in default view, or those of a workspace's `.omfp/views.json`
 * manifest, checked in id order with the same rules and messages as the daemon (`fixtures/views/cases.json`). A view
 * is a layout, or a TSX file of `.omfp/views/` whose layout comes from rendering it (AGT-074, UI-268).
 */

import { isObject } from "../sdk/json.ts";
import { strings } from "../strings/index.ts";
import { type LayoutNode, type LayoutTree, type Parsed, parseLayout, type View } from "./layout.ts";

/** Header over the `main` tabs with the conversation (the slot reserved for opened files), the workspace and changes
 * widgets as the `side` tabs on the right; the frame renders the threads column itself (UI-100). */
export const DEFAULT_VIEW: View = {
  id: "default",
  title: strings.shell.frame.defaultView,
  layout: [
    "row",
    { sizes: [3, 1] },
    ["col", {}, "omfp:header", ["tabs", { id: "main" }, "omfp:chat"]],
    ["tabs", { id: "side" }, "omfp:workspace", "omfp:changes", "omfp:background", "omfp:mcp"],
  ],
};

/**
 * The default manifest (AGT-010): the daemon writes the same one into every workspace (DMN-088,
 * `crates/omfp-daemon/src/widgets/default-views.json`, checked equal by a test); the window falls back to it while
 * `views.json` is missing or invalid, and `:view-reset` writes it (UI-107).
 */
export const DEFAULT_MANIFEST = {
  $schema: "omfp://schemas/views.json",
  default: DEFAULT_VIEW.id,
  views: { [DEFAULT_VIEW.id]: { layout: DEFAULT_VIEW.layout, title: DEFAULT_VIEW.title } },
} as const;

/** A validated view: its layout as written (what layout edits change) and as a tree (what the renderer walks). */
export interface ResolvedView {
  readonly id: string;
  readonly title: string;
  readonly layout: LayoutNode;
  readonly tree: LayoutTree;
  /** The init file whose views or transforms made it (UI-261, UI-267); `undefined` for `views.json`'s. */
  readonly init?: "user" | "workspace";
  /** The TSX file (`./views/<name>.tsx`) the layout was rendered from (UI-268); `undefined` for a written layout. */
  readonly file?: string;
}

/** A view whose layout is a TSX file of `.omfp/views/` (AGT-074, UI-268): the window renders it to get its tree. */
export interface FileView {
  readonly id: string;
  readonly title: string;
  /** `./views/<name>.tsx` as written. */
  readonly file: string;
  readonly init?: "user" | "workspace";
}

/** A view of the switcher: a layout, or a TSX file still to render. */
export type ListedView = ResolvedView | FileView;

/** Whether `view` is a TSX file whose layout is not rendered yet. */
export function isFileView(view: ListedView): view is FileView {
  return !("tree" in view);
}

/** `./views/<name>.<tsx | ts | jsx | js>`, `<name>` a widget folder name (AGT-012, AGT-074). */
const VIEW_FILE = /^\.\/views\/[A-Za-z0-9_-][A-Za-z0-9_.-]*\.(?:tsx|ts|jsx|js)$/;

/** A floating panel (AGT-060): its node as written and as a tree, with its title (the node's `title` prop, else the
 * id). */
export interface ResolvedPanel {
  readonly id: string;
  readonly title: string;
  readonly node: LayoutNode;
  readonly tree: LayoutTree;
  /** The init file that added it to this window (UI-261): closing it only hides it here. */
  readonly init?: "user" | "workspace";
}

export interface Views {
  readonly views: readonly ListedView[];
  readonly defaultId: string;
  /** The manifest's floating panels in id order (AGT-060); none for the default view. */
  readonly panels: readonly ResolvedPanel[];
  /** Why the manifest was refused; the default view is shown instead. */
  readonly error: string | undefined;
}

function resolveDefault(): ResolvedView {
  const parsed = parseLayout(DEFAULT_VIEW.layout);
  if (!parsed.ok) {
    throw new Error(`the default view is invalid: ${parsed.error}`);
  }
  return { id: DEFAULT_VIEW.id, title: DEFAULT_VIEW.title, layout: DEFAULT_VIEW.layout, tree: parsed.value };
}

/** The built-in default view: shown while the manifest is missing or invalid, and instead of a failing TSX view. */
export const BUILTIN_VIEW: ResolvedView = resolveDefault();

const DEFAULT_VIEWS: Views = { views: [BUILTIN_VIEW], defaultId: DEFAULT_VIEW.id, panels: [], error: undefined };

/**
 * The views of `manifest` (`{ default?, views: { <id>: { title, layout } }, panels? }`) in id order, or the default
 * view when there is none or it is invalid (with the error, which the frame shows in a banner).
 */
export function resolveViews(manifest: unknown): Views {
  if (manifest === undefined || manifest === null) {
    return DEFAULT_VIEWS;
  }
  const parsed = parseManifest(manifest);
  return parsed.ok ? parsed.value : { ...DEFAULT_VIEWS, error: parsed.error };
}

/**
 * `views` with the floating panels the init files add (UI-261): one with the id of a manifest panel replaces it in
 * place, the others follow in the order given.
 */
export function withInitPanels(views: Views, panels: readonly ResolvedPanel[]): Views {
  if (panels.length === 0) {
    return views;
  }
  const merged = [...views.panels];
  for (const panel of panels) {
    const index = merged.findIndex((existing) => existing.id === panel.id);
    if (index < 0) {
      merged.push(panel);
    } else {
      merged[index] = panel;
    }
  }
  return { ...views, panels: merged };
}

/**
 * Checks the view `id` of a manifest (AGT-010, AGT-074): `{ title?, layout }`, or `{ title?, view }` naming a TSX file
 * of `.omfp/views/`, never both.
 */
export function parseView(id: string, view: unknown): Parsed<ListedView> {
  if (!isObject(view)) {
    return { ok: false, error: `views.${id}: expected { title, layout }` };
  }
  const title = typeof view.title === "string" ? view.title : id;
  if (Object.hasOwn(view, "view")) {
    if (Object.hasOwn(view, "layout")) {
      return { ok: false, error: `views.${id}: a layout or a view file, not both` };
    }
    const file = view.view;
    return typeof file === "string" && VIEW_FILE.test(file)
      ? { ok: true, value: { id, title, file } }
      : { ok: false, error: `views.${id}.view: expected "./views/<name>.tsx"` };
  }
  const tree = parseLayout(view.layout, `views.${id}.layout`);
  if (!tree.ok) {
    return tree;
  }
  // Checked by `parseLayout` just above.
  return { ok: true, value: { id, title, layout: view.layout as LayoutNode, tree: tree.value } };
}

/** Checks a manifest (AGT-010, AGT-060): views, then panels, then the default; the error names the JSON path and the
 * reason, as the daemon's does. */
function parseManifest(manifest: unknown): Parsed<Views> {
  if (!isObject(manifest) || !isObject(manifest.views)) {
    return { ok: false, error: "views: expected an object of views" };
  }
  const views: ListedView[] = [];
  for (const [id, view] of Object.entries(manifest.views).sort(([a], [b]) => compareIds(a, b))) {
    const parsed = parseView(id, view);
    if (!parsed.ok) {
      return parsed;
    }
    views.push(parsed.value);
  }
  const [first] = views;
  if (first === undefined) {
    return { ok: false, error: "views: no view" };
  }
  const panels = parsePanels(manifest.panels);
  if (!panels.ok) {
    return panels;
  }
  const wanted = manifest.default;
  if (wanted !== undefined && !views.some((view) => view.id === wanted)) {
    return { ok: false, error: `default: no view ${String(wanted)}` };
  }
  return {
    ok: true,
    value: {
      views,
      defaultId: typeof wanted === "string" ? wanted : first.id,
      panels: panels.value,
      error: undefined,
    },
  };
}

/** The `panels` of a manifest (AGT-060), each node checked like a layout at `panels.<id>`, in id order. */
function parsePanels(panels: unknown): Parsed<ResolvedPanel[]> {
  if (panels === undefined) {
    return { ok: true, value: [] };
  }
  if (!isObject(panels)) {
    return { ok: false, error: "panels: expected an object of panels" };
  }
  const parsed: ResolvedPanel[] = [];
  for (const [id, node] of Object.entries(panels).sort(([a], [b]) => compareIds(a, b))) {
    const tree = parseLayout(node, `panels.${id}`);
    if (!tree.ok) {
      return tree;
    }
    const { title } = tree.value.props;
    // Checked by `parseLayout` just above.
    parsed.push({ id, title: typeof title === "string" ? title : id, node: node as LayoutNode, tree: tree.value });
  }
  return { ok: true, value: parsed };
}

/** View ids in code unit order (the order the daemon lists them in). */
function compareIds(a: string, b: string): number {
  if (a === b) {
    return 0;
  }
  return a < b ? -1 : 1;
}
