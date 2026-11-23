/**
 * The views init files build (UI-267): `omfp.views.load("views.json")` gives the
 * workspace's manifest as {@link InitViews} (`add`, `update`, `remove`, `get`, `withDefault`), whose layouts
 * {@link Layout} edits (`addTab`, `removeWidget`); every change mutates the views or the layout and returns it, so
 * calls follow one another or chain. `omfp.views.use(views)` shows them in this window, and
 * `omfp.views.transform(fn)` changes whatever views the window ends up with. Each view remembers whether it is
 * `views.json`'s (even once init files changed its layout: `:widget add` still writes it, UI-300) or which init file
 * added it.
 */

import { strings } from "../strings/index.ts";
import { type LayoutNode, type LayoutTree, parseLayout } from "../view/layout.ts";
import { DEFAULT_MANIFEST, parseView, type ResolvedPanel, type ResolvedView, resolveViews } from "../view/views.ts";
import { isObject } from "./json.ts";

const text = strings.init.views;

/** Which init file: the user's (`~/.config/omfp/init.ts`) or the current workspace's (`.omfp/init.ts`). */
export type InitFile = "user" | "workspace";

/** Where a view comes from: `views.json` (even once init files changed its layout), or the init file that added it. */
export type ViewOrigin = "views.json" | InitFile;

/** A view as an init file writes it: a layout (a node or a {@link Layout}). */
export interface ViewSpec {
  /** Default: the id. */
  readonly title?: string;
  readonly layout: LayoutNode | Layout;
}

/** A view as {@link InitViews.get} gives it. */
export interface ViewEntry {
  readonly id: string;
  readonly title: string;
  /** The view's layout: its edits change the view. */
  readonly layout: Layout;
  readonly origin: ViewOrigin;
}

/** A view: `views.json`'s (or the one the file that added it gave), as the files changed it. */
interface Entry {
  readonly id: string;
  readonly title: string;
  readonly origin: ViewOrigin;
  layout: LayoutNode;
}

/** Where the edits of a layout {@link InitViews.get} gave go: its view. */
const owners = new WeakMap<Layout, (node: LayoutNode) => void>();

function invalid(error: string): Error {
  return new Error(text.invalid(error));
}

/** The tree of `node`; throws when it is no layout. */
function treeOf(node: unknown): LayoutTree {
  const parsed = parseLayout(node);
  if (!parsed.ok) {
    throw invalid(parsed.error);
  }
  return parsed.value;
}

/** A layout node whose edits change it and return it (UI-267); one from {@link InitViews.get} changes its view too. */
export class Layout {
  #node: LayoutNode;

  constructor(node: LayoutNode) {
    treeOf(node);
    this.#node = node;
  }

  get node(): LayoutNode {
    return this.#node;
  }

  /** Adds `widget` as the last tab of the `tabs` container with `"id": tabsId`; throws when there is none. */
  addTab(tabsId: string, widget: LayoutNode | Layout): Layout {
    const added = widget instanceof Layout ? widget.node : widget;
    let found = false;
    const visit = (node: LayoutNode): LayoutNode => {
      if (typeof node === "string" || node.length === 2) {
        return node;
      }
      const [kind, props, ...children] = node as readonly [string, Record<string, unknown>, ...LayoutNode[]];
      const visited = children.map(visit);
      if (kind === "tabs" && props.id === tabsId && !found) {
        found = true;
        // A `sizes` list no longer has one weight per child.
        const { sizes: _sizes, ...rest } = props;
        return [kind, rest, ...visited, added] as LayoutNode;
      }
      return [kind, props, ...visited] as LayoutNode;
    };
    const node = visit(this.#node);
    if (!found) {
      throw new Error(text.noTabs(tabsId));
    }
    return this.#set(node);
  }

  /** Removes every widget `reference`, and the containers left empty; throws when nothing would remain. */
  removeWidget(reference: string): Layout {
    const visit = (node: LayoutNode): LayoutNode | undefined => {
      if (typeof node === "string") {
        return node === reference ? undefined : node;
      }
      if (node.length === 2) {
        return node[0] === reference ? undefined : node;
      }
      const [kind, props, ...children] = node as readonly [string, Record<string, unknown>, ...LayoutNode[]];
      const kept = children.map(visit);
      const remaining = kept.filter((child): child is LayoutNode => child !== undefined);
      if (remaining.length === 0) {
        return undefined;
      }
      const sizes = Array.isArray(props.sizes)
        ? (props.sizes as unknown[]).filter((_, index) => kept[index] !== undefined)
        : undefined;
      return [kind, sizes === undefined ? props : { ...props, sizes }, ...remaining] as LayoutNode;
    };
    const node = visit(this.#node);
    if (node === undefined) {
      throw new Error(text.emptyLayout(reference));
    }
    return this.#set(node);
  }

  #set(node: LayoutNode): Layout {
    treeOf(node);
    this.#node = node;
    owners.get(this)?.(node);
    return this;
  }
}

/** Views as an init file builds them: each change mutates them and returns them (UI-267). */
export class InitViews {
  readonly #entries: Entry[];
  #defaultId: string | undefined;
  readonly #panels: readonly ResolvedPanel[];
  /** The file whose changes these are: the one that loaded them, then each transform's. */
  readonly #file: InitFile;

  private constructor(
    entries: Entry[],
    defaultId: string | undefined,
    panels: readonly ResolvedPanel[],
    file: InitFile,
  ) {
    this.#entries = entries;
    this.#defaultId = defaultId;
    this.#panels = panels;
    this.#file = file;
  }

  /**
   * The views of `views.json` (`manifest` and its `error` as on the `views/<id>` topic), changed by `file`: the
   * default view's manifest while the file is missing or invalid.
   */
  static load(manifest: unknown, error: string | null | undefined, file: InitFile): InitViews {
    const shown =
      manifest === undefined || manifest === null || (error !== undefined && error !== null)
        ? DEFAULT_MANIFEST
        : manifest;
    const views = resolveViews(shown);
    const wanted = isObject(shown) && typeof shown.default === "string" ? shown.default : undefined;
    return new InitViews(
      views.views.map(({ id, title, layout }): Entry => ({ id, title, origin: "views.json", layout })),
      wanted,
      views.panels,
      file,
    );
  }

  /** A copy of `views` that `file` changes from now on: what `use` keeps, what each transform gets. */
  static copy(views: InitViews, file: InitFile = views.#file): InitViews {
    return new InitViews(
      views.#entries.map((entry) => ({ ...entry })),
      views.#defaultId,
      views.#panels,
      file,
    );
  }

  /** View ids, in switcher order. */
  get ids(): string[] {
    return this.#entries.map((entry) => entry.id);
  }

  /** The default view's id. */
  get default(): string | undefined {
    return this.#defaultId ?? this.#entries[0]?.id;
  }

  /** The floating panels of `views.json` (AGT-060). */
  get panels(): readonly ResolvedPanel[] {
    return this.#panels;
  }

  /** The views as the window lists them: a view a file added carries the file in `init`. */
  get listed(): readonly ResolvedView[] {
    return this.#entries.map((entry): ResolvedView => {
      const { id, title, origin } = entry;
      const init = origin === "views.json" ? {} : { init: origin };
      return { id, title, layout: entry.layout, tree: treeOf(entry.layout), ...init };
    });
  }

  get(id: string): ViewEntry | undefined {
    const entry = this.#entries.find((candidate) => candidate.id === id);
    if (entry === undefined) {
      return undefined;
    }
    return { id, title: entry.title, layout: this.#bound(entry), origin: entry.origin };
  }

  /** Adds the view `id`, or replaces the view with that id in place; throws when `view` is invalid. */
  add(id: string, view: ViewSpec): InitViews {
    const written: Record<string, unknown> = {};
    if (view.title !== undefined) {
      written.title = view.title;
    }
    written.layout = view.layout instanceof Layout ? view.layout.node : view.layout;
    const parsed = parseView(id, written);
    if (!parsed.ok) {
      throw invalid(parsed.error);
    }
    const { title, layout } = parsed.value;
    const entry: Entry = { id, title, origin: this.#file, layout };
    const index = this.#entries.findIndex((candidate) => candidate.id === id);
    if (index < 0) {
      this.#entries.push(entry);
    } else {
      this.#entries[index] = entry;
    }
    return this;
  }

  /**
   * Changes the layout of the view `id`: `change` edits the {@link Layout} it gets, or returns the new layout; throws
   * for a missing view.
   */
  update(id: string, change: (layout: Layout) => Layout | LayoutNode | undefined): InitViews {
    const entry = this.#entries.find((candidate) => candidate.id === id);
    if (entry === undefined) {
      throw new Error(text.noView(id));
    }
    const layout = this.#bound(entry);
    const result = change(layout);
    if (result !== undefined && result !== layout) {
      this.#setLayout(entry, result instanceof Layout ? result.node : result);
    }
    return this;
  }

  /** Removes the view `id`; throws when it is missing or the last one. */
  remove(id: string): InitViews {
    const index = this.#entries.findIndex((entry) => entry.id === id);
    if (index < 0) {
      throw new Error(text.noView(id));
    }
    if (this.#entries.length === 1) {
      throw new Error(text.lastView);
    }
    this.#entries.splice(index, 1);
    if (this.#defaultId === id) {
      this.#defaultId = undefined;
    }
    return this;
  }

  /** Makes `id` the default view; throws when there is no such view. */
  withDefault(id: string): InitViews {
    if (!this.#entries.some((entry) => entry.id === id)) {
      throw new Error(text.noView(id));
    }
    this.#defaultId = id;
    return this;
  }

  /** The layout of `entry`, whose edits change it. */
  #bound(entry: Entry): Layout {
    const layout = new Layout(entry.layout);
    owners.set(layout, (node) => this.#setLayout(entry, node));
    return layout;
  }

  /** `node` as the layout of `entry`; throws when it is no layout or the view is gone. */
  #setLayout(entry: Entry, node: LayoutNode): void {
    if (!this.#entries.includes(entry)) {
      throw new Error(text.noView(entry.id));
    }
    treeOf(node);
    entry.layout = node;
  }
}
