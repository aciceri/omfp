/**
 * The conversation search's two document-wide custom highlights (UI-736), shared by every transcript: built-in ones and
 * workspace copies of the widget, which load their own copy of the widget's modules but this one SDK module, so the
 * owners of the highlights are known in one place and one transcript's repaint or close never drops another's marks.
 */

/** Every occurrence in the rendered matching messages, and hints at matches in collapsed thinking blocks. */
export const SEARCH_MATCH_HIGHLIGHT = "omfp-search";
/** The current occurrence. */
export const SEARCH_CURRENT_HIGHLIGHT = "omfp-search-current";

/** What one transcript marks. */
export interface SearchMarks {
  readonly matches: readonly Range[];
  readonly current: Range | undefined;
}

/** The CSS highlight registry, or `undefined` where the browser has no custom highlights. */
export function highlightRegistry(): HighlightRegistry | undefined {
  return typeof CSS !== "undefined" && "highlights" in CSS && typeof Highlight === "function"
    ? CSS.highlights
    : undefined;
}

/** The marks of each transcript painting into the shared highlights. */
const OWNERS = new Map<object, SearchMarks>();

/** Sets `owner`'s marks (`undefined`: none) and paints every owner's into the registry's two highlights. */
export function paintSearchHighlights(
  registry: HighlightRegistry,
  owner: object,
  marks: SearchMarks | undefined,
): void {
  if (marks === undefined) {
    OWNERS.delete(owner);
  } else {
    OWNERS.set(owner, marks);
  }
  const all = [...OWNERS.values()];
  const set = (name: string, ranges: readonly Range[], priority: number) => {
    if (ranges.length === 0) {
      registry.delete(name);
      return;
    }
    const highlight = new Highlight(...ranges);
    highlight.priority = priority;
    registry.set(name, highlight);
  };
  set(
    SEARCH_MATCH_HIGHLIGHT,
    all.flatMap((each) => each.matches),
    0,
  );
  set(
    SEARCH_CURRENT_HIGHLIGHT,
    all.flatMap((each) => (each.current === undefined ? [] : [each.current])),
    1,
  );
}
