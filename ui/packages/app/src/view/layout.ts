/**
 * Layout trees (AGT-010): a view's layout is a widget reference (`"omfp:chat"` or `["./widgets/tests", { … }]`) or a
 * container (`["row" | "col" | "tabs" | "stack", { sizes?, … }, ...children]`). Layouts come from data written by
 * agents, so they are validated: an invalid one never breaks the window, the frame shows the default view and the
 * error instead (AGT-011).
 */

import type { FileTab, WidgetProps } from "../sdk/index.ts";
import { isObject } from "../sdk/json.ts";
import { builtinReference } from "./resolve.ts";

export type ContainerKind = "row" | "col" | "tabs" | "stack";

export const CONTAINER_KINDS: readonly ContainerKind[] = ["row", "col", "tabs", "stack"];

/** Container props: `sizes` are flex weights, one per child (equal when absent). */
export interface ContainerProps {
  readonly sizes?: readonly number[];
  readonly [key: string]: unknown;
}

/** A layout node as written in `views.json`. */
export type LayoutNode =
  | string
  | readonly [string, WidgetProps]
  | readonly [ContainerKind, ContainerProps, ...LayoutNode[]];

/** A layout node after validation. */
export type LayoutTree =
  | { readonly kind: "widget"; readonly name: string; readonly props: WidgetProps }
  | {
      readonly kind: ContainerKind;
      readonly props: ContainerProps;
      readonly children: readonly LayoutTree[];
    };

export interface View {
  readonly id: string;
  readonly title: string;
  readonly layout: LayoutNode;
}

export type Parsed<T> = { readonly ok: true; readonly value: T } | { readonly ok: false; readonly error: string };

/** Checks `node` (at `path`, for error messages) and turns it into a {@link LayoutTree}. */
export function parseLayout(node: unknown, path = "layout"): Parsed<LayoutTree> {
  if (typeof node === "string") {
    return node.trim() === ""
      ? { ok: false, error: `${path}: a widget name is empty` }
      : { ok: true, value: { kind: "widget", name: node, props: {} } };
  }
  if (!Array.isArray(node) || node.length === 0 || typeof node[0] !== "string") {
    return { ok: false, error: `${path}: expected a widget name, [widget, props] or [container, props, ...children]` };
  }
  const [head, props, ...children] = node as [string, unknown, ...unknown[]];
  if (head.trim() === "") {
    return { ok: false, error: `${path}: a widget name is empty` };
  }
  if (!isObject(props)) {
    return { ok: false, error: `${path}: ${head} needs a props object` };
  }
  const kind = CONTAINER_KINDS.find((candidate) => candidate === head);
  if (kind === undefined) {
    return children.length > 0
      ? { ok: false, error: `${path}: a widget reference is [name, props]` }
      : { ok: true, value: { kind: "widget", name: head, props } };
  }
  return parseContainer(kind, props, children, path);
}

function parseContainer(
  kind: ContainerKind,
  props: Readonly<Record<string, unknown>>,
  children: readonly unknown[],
  path: string,
): Parsed<LayoutTree> {
  if (children.length === 0) {
    return { ok: false, error: `${path}: ${kind} has no children` };
  }
  const { sizes } = props;
  if (
    sizes !== undefined &&
    !(
      Array.isArray(sizes) &&
      sizes.length === children.length &&
      sizes.every((size) => typeof size === "number" && Number.isFinite(size) && size > 0)
    )
  ) {
    return { ok: false, error: `${path}: sizes must be ${children.length} positive numbers` };
  }
  const trees: LayoutTree[] = [];
  for (const [index, child] of children.entries()) {
    const parsed = parseLayout(child, `${path}[${index + 2}]`);
    if (!parsed.ok) {
      return parsed;
    }
    trees.push(parsed.value);
  }
  return { ok: true, value: { kind, props: props as ContainerProps, children: trees } };
}

/** Whether any widget of `tree` is one of `names`; a retired built-in counts as its successor (UI-493). */
export function containsWidget(tree: LayoutTree, names: readonly string[]): boolean {
  return tree.kind === "widget"
    ? names.some((name) => builtinReference(name) === builtinReference(tree.name))
    : tree.children.some((child) => containsWidget(child, names));
}

/**
 * Each child's identity among its siblings: its widget instance (the `instance` prop, else the reference) or its
 * container kind, numbered among equal siblings (`omfp:changes#0`). A path is a position, so a tab closed before a
 * sibling would move it (UI-375); this key stays: the renderer keys and names tabs by it, and a close finds its node
 * by the keys from the view's root.
 */
export function childKeys(children: readonly LayoutTree[]): string[] {
  const seen = new Map<string, number>();
  return children.map((child) => {
    const instance = child.kind === "widget" ? child.props.instance : undefined;
    const base = child.kind !== "widget" ? child.kind : typeof instance === "string" ? instance : child.name;
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    return `${base}#${count}`;
  });
}

/** The {@link childKeys} from `tree`, a view's root (the renderer's path `0`), down to its node at the renderer's
 * `path` (`0.1.2`); `undefined` when it has none there. */
export function keysAt(tree: LayoutTree, path: string): string[] | undefined {
  const [root, ...indices] = path.split(".").map(Number);
  if (root !== 0) {
    return undefined;
  }
  const keys: string[] = [];
  let node: LayoutTree | undefined = tree;
  for (const index of indices) {
    if (node === undefined || node.kind === "widget") {
      return undefined;
    }
    const key = childKeys(node.children)[index];
    node = node.children[index];
    if (key === undefined) {
      return undefined;
    }
    keys.push(key);
  }
  return keys;
}

/** The `id` of the `tabs` container where files open (UI-115). */
export const MAIN_SLOT = "main";

/** The widget a file tab shows. */
export const FILE_WIDGET = "omfp:file";

/** Whether the view has the `tabs` container files open in; without one they open in a floating panel (UI-115). */
export function hasMainSlot(tree: LayoutTree): boolean {
  return (
    tree.kind !== "widget" && ((tree.kind === "tabs" && tree.props.id === MAIN_SLOT) || tree.children.some(hasMainSlot))
  );
}

/** The area a phone shows while the threads are closed (UI-204). */
export type Area = "chat" | "side";

/**
 * On a phone the view shows one area at a time (UI-204): when its root is a `row` of several cells, the cell with the
 * conversation (the `main` slot, else `omfp:chat`, else the first) is the chat area and the others the side area;
 * `undefined` when the view has no side area.
 */
export function chatCell(tree: LayoutTree): number | undefined {
  if (tree.kind !== "row" || tree.children.length < 2) {
    return undefined;
  }
  const main = tree.children.findIndex(hasMainSlot);
  return main >= 0
    ? main
    : Math.max(
        0,
        tree.children.findIndex((child) => containsWidget(child, ["omfp:chat"])),
      );
}

/** The props of the `omfp:file` widget of an open file tab (`jump` repeats the jump to `line` when reopened). */
export function fileProps(tab: FileTab): WidgetProps {
  return {
    path: tab.path,
    ...(tab.line === undefined ? {} : { line: tab.line }),
    ...(tab.diff === true ? { diff: true } : {}),
    jump: tab.seq,
  };
}
