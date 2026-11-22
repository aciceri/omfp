/**
 * What the window writes into `views.json` itself: a widget added with `:widget add` (UI-300) and a layout tab closed
 * (UI-375), written with `views.set` before the window renders what the daemon pushes back. Splitter sizes stay this
 * window's own (UI-206): {@link resizeSizes} computes them. Agents change views through the daemon (`views.mount`,
 * `views.set`, AGT-061).
 */

import { isObject, sameJson } from "../sdk/json.ts";
import { strings } from "../strings/index.ts";
import { CONTAINER_KINDS, childKeys, type LayoutNode, type LayoutTree, type Parsed, parseLayout } from "./layout.ts";
import { DEFAULT_MANIFEST } from "./views.ts";

/** One keyboard step of a splitter: 5 % of the container. */
export const RESIZE_STEP = 0.05;

/** The smallest share a resized cell keeps. */
const MIN_SHARE = 0.05;

/** The `id` of the `tabs` container widgets are added to (UI-300). */
export const SIDE_TABS = "side";

const text = strings.shell.layout;

function round(value: number): number {
  return Math.round(value * 10_000) / 10_000;
}

/**
 * `sizes` after moving `delta` (a share of their total) from cell `index` to cell `index - 1`; both cells keep at
 * least 5 % of the total. A splitter's drag, keys and preview use it.
 */
export function resizeSizes(sizes: readonly number[], index: number, delta: number): number[] {
  const next = [...sizes];
  const before = next[index - 1] ?? 1;
  const after = next[index] ?? 1;
  const total = next.reduce((sum, value) => sum + value, 0);
  const pair = before + after;
  const least = Math.min(total * MIN_SHARE, pair / 2);
  const grown = round(Math.min(Math.max(before + delta * total, least), pair - least));
  next[index - 1] = grown;
  next[index] = round(pair - grown);
  return next;
}

/** A container node's kind, props and children; `undefined` for a widget node. */
function container(
  node: LayoutNode,
): { kind: string; props: Record<string, unknown>; children: LayoutNode[] } | undefined {
  if (!Array.isArray(node)) {
    return undefined;
  }
  const [kind, props, ...children] = node as readonly unknown[];
  return typeof kind === "string" && CONTAINER_KINDS.some((candidate) => candidate === kind) && isObject(props)
    ? { kind, props, children: children as LayoutNode[] }
    : undefined;
}

/** `props` with one more weight in `sizes` (the mean of the others) when it has sizes. */
function grownSizes(props: Record<string, unknown>): Record<string, unknown> {
  const { sizes } = props;
  if (!Array.isArray(sizes)) {
    return props;
  }
  const weights = sizes.filter((size): size is number => typeof size === "number");
  const mean = weights.length === 0 ? 1 : round(weights.reduce((sum, value) => sum + value, 0) / weights.length);
  return { ...props, sizes: [...weights, mean] };
}

/** `node` with `reference` as the last tab of its first `tabs` container with `"id": "side"`; `undefined`: none. */
function addToSide(node: LayoutNode, reference: string): LayoutNode | undefined {
  const found = container(node);
  if (found === undefined) {
    return undefined;
  }
  if (found.kind === "tabs" && found.props.id === SIDE_TABS) {
    return [found.kind, grownSizes(found.props), ...found.children, reference] as LayoutNode;
  }
  for (const [index, child] of found.children.entries()) {
    const added = addToSide(child, reference);
    if (added !== undefined) {
      const children = found.children.with(index, added);
      return [found.kind, found.props, ...children] as LayoutNode;
    }
  }
  return undefined;
}

/**
 * Adds `reference` as the last tab of the `tabs` with `"id": "side"`, else at the end of the root container (a root
 * widget becomes a `row` of both), as the daemon's `views.mount` places it (UI-300, PRT-080).
 */
export function addWidget(layout: LayoutNode, reference: string): LayoutNode {
  const side = addToSide(layout, reference);
  if (side !== undefined) {
    return side;
  }
  const root = container(layout);
  if (root === undefined) {
    return ["row", {}, layout, reference];
  }
  return [root.kind, grownSizes(root.props), ...root.children, reference] as LayoutNode;
}

/** What the views topic says about the workspace's manifest (a `ViewsState`). */
export interface ManifestSource {
  /** The valid manifest; `null` or absent without a file (or when it is invalid). */
  readonly manifest?: unknown;
  /** Why the file is refused; `null` or absent when valid or missing. */
  readonly error?: string | null;
}

type JsonObject = Readonly<Record<string, unknown>>;

/**
 * The manifest whose view `viewId` the window changes, with its views and that view: the current manifest, or, for a
 * workspace without `views.json`, the default one. An invalid file is never overwritten: the change is refused until
 * it is fixed.
 */
function writtenView(
  source: ManifestSource,
  viewId: string,
): Parsed<{ manifest: JsonObject; views: JsonObject; view: JsonObject }> {
  if (source.error !== undefined && source.error !== null) {
    return { ok: false, error: text.invalidManifest };
  }
  const manifest = source.manifest ?? DEFAULT_MANIFEST;
  const views = isObject(manifest) ? manifest.views : undefined;
  const view = isObject(views) ? views[viewId] : undefined;
  if (!isObject(manifest) || !isObject(views) || !isObject(view) || !Object.hasOwn(view, "layout")) {
    return { ok: false, error: text.noView(viewId) };
  }
  return { ok: true, value: { manifest, views, view } };
}

/**
 * The tree of the view `viewId` as `views.json` holds it (the default view's while there is no file): a tab closes
 * only where the view shown has the same node (UI-375), not where an init file changed it. `undefined` while the file
 * is invalid or lacks that view as a layout.
 */
export function writtenTree(source: ManifestSource, viewId: string): LayoutTree | undefined {
  const written = writtenView(source, viewId);
  const parsed = written.ok ? parseLayout(written.value.view.layout) : undefined;
  return parsed?.ok === true ? parsed.value : undefined;
}

/** A manifest with the layout of the view `viewId` changed by `edit`, as {@link writtenView} finds it. */
function editView(
  source: ManifestSource,
  viewId: string,
  edit: (layout: LayoutNode) => Parsed<LayoutNode>,
): Parsed<unknown> {
  const written = writtenView(source, viewId);
  if (!written.ok) {
    return written;
  }
  const { manifest, views, view } = written.value;
  // The manifest is valid (the daemon pushes only valid ones), so the layout is a node.
  const layout = edit(view.layout as LayoutNode);
  return layout.ok
    ? { ok: true, value: { ...manifest, views: { ...views, [viewId]: { ...view, layout: layout.value } } } }
    : layout;
}

/** The manifest `views.set` writes to add `reference` to the view `viewId` (UI-300), as {@link editView} does. */
export function manifestWithWidget(source: ManifestSource, viewId: string, reference: string): Parsed<unknown> {
  return editView(source, viewId, (layout) => ({ ok: true, value: addWidget(layout, reference) }));
}

/**
 * `node` (shown as `tree`) without the tab its children's identity `keys` lead to ({@link childKeys}, from `node`'s
 * children down); `undefined` when there is no such tab, it is the last of its `tabs` container, or it is no longer the
 * node `expected` the window showed. A container with `sizes` loses the tab's weight.
 */
function withoutTab(
  node: LayoutNode,
  tree: LayoutTree,
  keys: readonly string[],
  expected: LayoutTree,
): LayoutNode | undefined {
  const found = container(node);
  const [key, ...rest] = keys;
  if (found === undefined || tree.kind === "widget" || key === undefined) {
    return undefined;
  }
  const index = childKeys(tree.children).indexOf(key);
  const child = found.children[index];
  const shown = tree.children[index];
  if (child === undefined || shown === undefined) {
    return undefined;
  }
  if (rest.length > 0) {
    const inner = withoutTab(child, shown, rest, expected);
    return inner === undefined
      ? undefined
      : ([found.kind, found.props, ...found.children.with(index, inner)] as LayoutNode);
  }
  if (found.kind !== "tabs" || found.children.length < 2 || !sameJson(shown, expected)) {
    return undefined;
  }
  const { sizes } = found.props;
  const props = Array.isArray(sizes) ? { ...found.props, sizes: sizes.toSpliced(index, 1) } : found.props;
  return [found.kind, props, ...found.children.toSpliced(index, 1)] as LayoutNode;
}

/**
 * The manifest `views.set` writes to close a tab of the view `viewId` (UI-375), as {@link editView} does: the tab is
 * found by the identity `keys` of its node and its ancestors from the view's root ({@link keysAt}), so tabs closed
 * before it never move it, and it must still be the node `expected` the window showed. Only a tab of a `tabs`
 * container that keeps another one closes.
 */
export function manifestWithoutTab(
  source: ManifestSource,
  viewId: string,
  keys: readonly string[],
  expected: LayoutTree,
): Parsed<unknown> {
  return editView(source, viewId, (layout) => {
    const tree = parseLayout(layout);
    const changed = tree.ok ? withoutTab(layout, tree.value, keys, expected) : undefined;
    return changed === undefined ? { ok: false, error: text.noTab(viewId) } : { ok: true, value: changed };
  });
}
