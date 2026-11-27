/**
 * The transcript's minimap (UI-860..UI-863): the whole conversation in miniature, drawn on one canvas beside the
 * transcript's scrollbar while the user scrolls. This module decides what the canvas shows — each transcript row at its
 * measured place, {@link MINIMAP_SCALE} of its size, tinted by its kind, with its real content: the words of its text
 * wrapped at the transcript's columns, code blocks on their own background, headings stronger, tool cards and images
 * as boxes sharing what the row's measured height leaves them, failures in `error`, and marks on the search's matching
 * rows and the focused row — which band of the map the canvas holds, how far the map scrolls with the view when the
 * miniature is taller than the map, where the viewport box sits, where a press or a drag on the map scrolls to, and
 * which scrolls are the user's ({@link ScrollIntent}); the widget (`widgets/chat/minimap.tsx`) draws it. Positions come
 * from the virtualizer's measurements (estimates for rows never rendered). The transcript is virtualized, so a row's
 * miniature comes from its model, never the DOM: it is laid out the first time the canvas draws the row and kept with
 * the row's content (`minimapRow`), so only the rows near the map's window are ever laid out and only a streaming row
 * lays out again.
 */

import { expandedByDefault, type ToolCard } from "./tool.ts";
import type { MessageView, RowContent } from "./transcript.ts";
import type { ItemExtent, ScrollPosition } from "./virtual.ts";

/** Map px per transcript px: the miniature is the transcript at an eighth of its size. */
export const MINIMAP_SCALE = 0.125;

/** What a row is, as its tint shows: a tool row is an assistant message of tool calls only. */
export type MinimapKind = "user" | "assistant" | "tool" | "error" | "system";

/**
 * How a piece of a row's miniature is drawn: a line of words (a name, text, a heading, code, a failure's text) or a box
 * (a code block's background, an image, a tool card, a failed one).
 */
export type MinimapInk =
  | "label"
  | "text"
  | "heading"
  | "code"
  | "errorText"
  | "codeBlock"
  | "image"
  | "toolBar"
  | "errorBar";

/** One piece of a row's miniature, top to bottom; heights in the transcript's text lines. */
export interface MinimapPiece {
  readonly ink: MinimapInk;
  /** From the row's top. */
  readonly top: number;
  readonly lines: number;
  /** A line's words, `[column, length]` pairs; `undefined` for a box. */
  readonly words: readonly number[] | undefined;
  /** A box whose real height only the DOM knows (a tool card, an image): it takes a share of the row's spare height. */
  readonly flex: boolean;
  /** Lines of the flexible boxes above this piece: how far their share of the spare height pushes it down. */
  readonly pushed: number;
}

/** A row's miniature at some number of columns. */
export interface MinimapLayout {
  readonly pieces: readonly MinimapPiece[];
  /** Its height, in text lines. */
  readonly lines: number;
  /** Lines of its flexible boxes. */
  readonly flex: number;
}

/** How a row is drawn: its tint, and its miniature laid out at `columns` columns (kept while they stay). */
export interface MinimapRow {
  readonly kind: MinimapKind;
  layout(columns: number): MinimapLayout;
}

/** A fill of the drawing: a row kind's tint, a piece's ink, or a mark. */
export type MinimapPaint = MinimapKind | MinimapInk | "match" | "focus";

/** Everything a drawing depends on, in CSS px. */
export interface MinimapScene {
  /** How each transcript row is drawn; `undefined` for a row with nothing to show. */
  readonly rows: readonly (MinimapRow | undefined)[];
  /** Each row's extent in the list, from the virtualizer. */
  readonly items: readonly ItemExtent[];
  /** Where the list starts in the scroll content. */
  readonly listTop: number;
  /** The map's width. */
  readonly width: number;
  /** The band of the map's content the canvas holds, from the content's top ({@link minimapBand}). */
  readonly top: number;
  readonly height: number;
  /** The columns the transcript's text wraps at, and the height of one of its lines on the map ({@link minimapMetrics}). */
  readonly columns: number;
  readonly pitch: number;
  /** Rows matching the conversation search (UI-730). */
  readonly matches: readonly number[];
  /** The focused row (UI-095). */
  readonly current: number | undefined;
}

/** A vertical extent on the map: the viewport's box, or the band the canvas holds. */
export interface MinimapBox {
  readonly top: number;
  readonly height: number;
}

/** The map's width, the columns its rows' text wraps at, and the height of a text line on it. */
export interface MinimapMetrics {
  readonly width: number;
  readonly columns: number;
  readonly pitch: number;
}

/** Rows at least this tall on the map keep a 1 px gap below them, so neighbours stay apart. */
const GAP_FROM_PX = 3;
/** Pieces start this far from the map's left edge, where the focused row's mark goes, and end as far from its right. */
const INSET_PX = 2;
/** Width of the focus and search marks, and their least height. */
const MARK_PX = 2;
/** The viewport's box is at least this tall. */
const MIN_BOX_PX = 6;
/** The map's least and greatest width. */
const MIN_WIDTH_PX = 48;
const MAX_WIDTH_PX = 160;
/** The average width of a character of the transcript's prose, in ems: how many columns its text wraps at. */
const CHAR_EM = 0.5;
/** Text wraps at no fewer columns. */
const MIN_COLUMNS = 20;
/** Without a font size, or a line height, from the page. */
const FONT_PX = 16;
const LINE_EM = 1.5;
/** The canvas holds this many map heights around the map's window, so the map scrolls a while before a redraw. */
const BAND_HEIGHTS = 3;

/** Heights, in text lines: between blocks and paragraphs, of a heading, around a code block's lines, of a card. */
const GAP_LINES = 0.5;
const HEADING_LINES = 1.25;
const CODE_PAD_LINES = 0.25;
const CARD_LINES = 1.5;
/** An open card's body shows at most this many lines (UI-935). */
const CARD_BODY_LINES = 12;
const IMAGE_LINES = 6;
/** Columns of a quote's indent and a code block's. */
const QUOTE_INDENT = 2;
const CODE_INDENT = 1;
/** Words sit this far down their line, as a fraction of its height, and fill this much of it. */
const WORD_TOP = 0.2;
const WORD_FILL = 0.6;

const WORD = /\S+/g;
const FENCE = /^\s*(?:```|~~~)/;
const HEADING = /^\s{0,3}#{1,6}\s+(.*)$/;
const RULE = /^\s*([-*_])(?:\s*\1){2,}\s*$/;
const TABLE = /^\s*\|/;
const ITEM = /^(\s*)(?:[-*+]|\d+[.)])\s/;
const QUOTE = /^\s*>\s?(.*)$/;

/**
 * The words of `text` wrapped greedily at `columns`, every line starting at column `indent`: one list of `[column,
 * length]` pairs per line; a word longer than a line is cut across lines.
 */
export function wrapWords(text: string, columns: number, indent = 0): number[][] {
  const width = Math.max(1, columns - indent);
  const lines: number[][] = [];
  let line: number[] = [];
  let column = indent;
  for (const [word] of text.matchAll(WORD)) {
    let length = word.length;
    if (line.length > 0 && column + length > columns) {
      lines.push(line);
      line = [];
      column = indent;
    }
    while (length > width) {
      lines.push([indent, width]);
      length -= width;
    }
    line.push(column, length);
    column += length + 1;
  }
  if (line.length > 0) {
    lines.push(line);
  }
  return lines;
}

/** The words of a line that does not wrap (code, a table), at their own columns from `indent`, cut at `columns`. */
function lineWords(text: string, columns: number, indent: number): number[] {
  const words: number[] = [];
  for (const match of text.matchAll(WORD)) {
    const column = indent + match.index;
    if (column >= columns) {
      break;
    }
    words.push(column, Math.min(match[0].length, columns - column));
  }
  return words;
}

/** How many lines `text` has, counting up to `most`. */
function lineCount(text: string, most: number): number {
  let count = 1;
  for (let at = text.indexOf("\n"); at >= 0 && count < most; at = text.indexOf("\n", at + 1)) {
    count += 1;
  }
  return count;
}

/** Lays out a row's miniature top to bottom. */
class Miniature {
  readonly #columns: number;
  readonly #pieces: MinimapPiece[] = [];
  #top = 0;
  #flex = 0;

  constructor(columns: number) {
    this.#columns = columns;
  }

  /** A line of `words`, `lines` tall. */
  line(ink: MinimapInk, words: readonly number[], lines = 1): void {
    if (words.length > 0) {
      this.#pieces.push({ ink, top: this.#top, lines, words, flex: false, pushed: this.#flex });
    }
    this.#top += lines;
  }

  /** `text` wrapped from column `indent`, each line `lines` tall. */
  wrap(ink: MinimapInk, text: string, indent = 0, lines = 1): void {
    for (const words of wrapWords(text, this.#columns, indent)) {
      this.line(ink, words, lines);
    }
  }

  /** The first line of `text` only, as a collapsed block or a muted row shows it. */
  summary(ink: MinimapInk, text: string): void {
    const [words] = wrapWords(text.slice(0, 4 * this.#columns), this.#columns);
    this.line(ink, words ?? []);
  }

  /** A line of code, a table's row: not wrapped. */
  code(text: string, indent = CODE_INDENT): void {
    this.line("code", lineWords(text, this.#columns, indent));
  }

  /** A box `lines` tall with `title` on its first line; a flexible one takes a share of the row's spare height. */
  box(ink: MinimapInk, lines: number, flex: boolean, title = ""): void {
    const pushed = this.#flex;
    this.#pieces.push({ ink, top: this.#top, lines, words: undefined, flex, pushed });
    const [words] = wrapWords(title.slice(0, 4 * this.#columns), this.#columns, CODE_INDENT);
    if (words !== undefined) {
      this.#pieces.push({ ink: "label", top: this.#top, lines: 1, words, flex: false, pushed });
    }
    if (flex) {
      this.#flex += lines;
    }
    this.#top += lines;
  }

  gap(lines = GAP_LINES): void {
    this.#top += lines;
  }

  /** Opens a code block; returns what {@link closeCode} needs to put its background behind its lines. */
  openCode(): { readonly at: number; readonly top: number } {
    const opened = { at: this.#pieces.length, top: this.#top };
    this.#top += CODE_PAD_LINES;
    return opened;
  }

  closeCode({ at, top }: { readonly at: number; readonly top: number }): void {
    this.#top += CODE_PAD_LINES;
    const lines = this.#top - top;
    this.#pieces.splice(at, 0, { ink: "codeBlock", top, lines, words: undefined, flex: false, pushed: this.#flex });
  }

  /** Markdown, as the transcript renders the assistant's text (UI-040): paragraphs, headings, lists, quotes, code. */
  markdown(text: string): void {
    let paragraph = "";
    let indent = 0;
    let code: { readonly at: number; readonly top: number } | undefined;
    const flush = (gap: boolean) => {
      if (paragraph !== "") {
        this.wrap("text", paragraph, indent);
        if (gap) {
          this.gap();
        }
      }
      paragraph = "";
      indent = 0;
    };
    for (const line of text.split("\n")) {
      if (FENCE.test(line)) {
        if (code === undefined) {
          flush(true);
          code = this.openCode();
        } else {
          this.closeCode(code);
          this.gap();
          code = undefined;
        }
        continue;
      }
      if (code !== undefined) {
        this.code(line);
        continue;
      }
      const trimmed = line.trim();
      if (trimmed === "") {
        flush(true);
        continue;
      }
      const heading = HEADING.exec(line);
      if (heading !== null) {
        flush(true);
        this.wrap("heading", heading[1] ?? "", 0, HEADING_LINES);
        this.gap();
        continue;
      }
      if (RULE.test(line)) {
        flush(true);
        this.gap(1);
        continue;
      }
      if (TABLE.test(line)) {
        flush(false);
        this.code(line, 0);
        continue;
      }
      const item = ITEM.exec(line);
      if (item !== null) {
        flush(false);
        indent = Math.min(item[1]?.length ?? 0, Math.floor(this.#columns / 2));
        paragraph = trimmed;
        continue;
      }
      const quote = QUOTE.exec(line);
      if (quote !== null) {
        if (indent !== QUOTE_INDENT) {
          flush(true);
        }
        indent = QUOTE_INDENT;
        paragraph = paragraph === "" ? (quote[1] ?? "") : `${paragraph} ${quote[1] ?? ""}`;
        continue;
      }
      paragraph = paragraph === "" ? trimmed : `${paragraph} ${trimmed}`;
    }
    if (code !== undefined) {
      // A block still streaming.
      this.closeCode(code);
    }
    flush(false);
  }

  /** Plain text, as the transcript shows the user's (its line breaks kept). */
  plain(text: string): void {
    for (const line of text.split("\n")) {
      if (line.trim() === "") {
        this.gap(1);
      } else {
        this.wrap("text", line);
      }
    }
  }

  done(): MinimapLayout {
    return { pieces: this.#pieces, lines: this.#top, flex: this.#flex };
  }
}

/** A card's height before it is measured: its header, and its body's preview when it starts open (UI-052, UI-935). */
function cardLines(card: ToolCard): number {
  return expandedByDefault(card) ? CARD_LINES + lineCount(card.result ?? card.args, CARD_BODY_LINES) : CARD_LINES;
}

function messageLayout(view: MessageView, continues: boolean, columns: number): MinimapLayout {
  const miniature = new Miniature(columns);
  if (!continues) {
    miniature.summary("label", view.label);
  }
  for (const block of view.blocks) {
    switch (block.type) {
      case "text":
        if (view.role === "assistant") {
          miniature.markdown(block.text);
        } else {
          miniature.plain(block.text);
        }
        break;
      case "thinking":
        miniature.summary("label", block.text);
        break;
      case "image":
        miniature.box("image", IMAGE_LINES, true);
        break;
      case "tool":
        miniature.box(
          block.card.status === "failed" ? "errorBar" : "toolBar",
          cardLines(block.card),
          true,
          `${block.card.name} ${block.card.summary}`,
        );
        break;
      case "readGroup":
        miniature.box(
          block.status === "failed" ? "errorBar" : "toolBar",
          CARD_LINES,
          true,
          block.cards.map((card) => `${card.name} ${card.summary}`).join(" "),
        );
        break;
      case "record":
        miniature.summary("label", block.summary);
        break;
    }
    miniature.gap();
  }
  if (view.error !== undefined) {
    miniature.wrap("errorText", view.error);
    miniature.gap();
  }
  return miniature.done();
}

function layoutOf(content: RowContent, columns: number): MinimapLayout {
  if (content.type === "message") {
    return messageLayout(content.view, content.continues, columns);
  }
  const miniature = new Miniature(columns);
  if (content.type === "system") {
    miniature.summary("label", content.row.summary);
  } else {
    miniature.summary(content.row.level === "error" ? "errorText" : "label", content.row.text);
  }
  return miniature.done();
}

function kindOf(content: RowContent): MinimapKind {
  if (content.type !== "message") {
    return "system";
  }
  const { view } = content;
  const failed =
    view.error !== undefined ||
    view.blocks.some(
      (block) =>
        (block.type === "tool" && block.card.status === "failed") ||
        (block.type === "readGroup" && block.status === "failed"),
    );
  if (failed) {
    return "error";
  }
  if (view.role === "user") {
    return "user";
  }
  if (view.role === "record") {
    return "system";
  }
  return view.blocks.length > 0 && view.blocks.every((block) => block.type === "tool" || block.type === "readGroup")
    ? "tool"
    : "assistant";
}

/** A row's drawing, laid out on first use and again only when the columns change. */
class Row implements MinimapRow {
  readonly kind: MinimapKind;
  readonly #content: RowContent;
  #layout: MinimapLayout | undefined;
  #columns = 0;

  constructor(content: RowContent) {
    this.kind = kindOf(content);
    this.#content = content;
  }

  layout(columns: number): MinimapLayout {
    if (this.#layout === undefined || this.#columns !== columns) {
      this.#layout = layoutOf(this.#content, columns);
      this.#columns = columns;
    }
    return this.#layout;
  }
}

const ROWS = new WeakMap<RowContent, MinimapRow>();

/** How a row showing `content` is drawn; the same object while the content stays the same. */
export function minimapRow(content: RowContent): MinimapRow {
  const known = ROWS.get(content);
  if (known !== undefined) {
    return known;
  }
  const row = new Row(content);
  ROWS.set(content, row);
  return row;
}

/**
 * The map's metrics for a transcript whose text is `textWidth` px wide in a font `fontSize` px tall, its lines
 * `lineHeight` apart: the map as wide as that text at {@link MINIMAP_SCALE} (within bounds), the text wrapping at the
 * columns an average character leaves it, a line {@link MINIMAP_SCALE} of its height.
 */
export function minimapMetrics(textWidth: number, fontSize: number, lineHeight: number): MinimapMetrics {
  const font = fontSize > 0 ? fontSize : FONT_PX;
  const line = lineHeight > 0 ? lineHeight : font * LINE_EM;
  const width = Math.round(Math.min(MAX_WIDTH_PX, Math.max(MIN_WIDTH_PX, textWidth * MINIMAP_SCALE + 2 * INSET_PX)));
  const columns = Math.max(MIN_COLUMNS, Math.round(Math.max(0, textWidth) / (font * CHAR_EM)));
  return { width, columns, pitch: line * MINIMAP_SCALE };
}

/** The first of the first `count` items ending below `y` (px from the list's top); the items are in order. */
function firstBelow(items: readonly ItemExtent[], count: number, y: number): number {
  let low = 0;
  let high = count;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if ((items[middle]?.end ?? 0) <= y) {
      low = middle + 1;
    } else {
      high = middle;
    }
  }
  return low;
}

/**
 * The rectangles of each paint, `[x, y, width, height]` one after the other, from the band's top: the rows in the band,
 * each a tint across the map with its miniature inset from the edges — its pieces from the row's top, a line of text
 * {@link MinimapScene.pitch} tall, its flexible boxes sharing what the row's height leaves (shrinking to a quarter at
 * most), all cut at the row's end — a mark at the right edge of each matching row and one at the left edge of the
 * focused row.
 */
export function minimapShapes(scene: MinimapScene): ReadonlyMap<MinimapPaint, readonly number[]> {
  const shapes = new Map<MinimapPaint, number[]>();
  const add = (paint: MinimapPaint, x: number, y: number, width: number, height: number) => {
    const rects = shapes.get(paint);
    if (rects === undefined) {
      shapes.set(paint, [x, y, width, height]);
    } else {
      rects.push(x, y, width, height);
    }
  };
  const extent = (index: number): MinimapBox | undefined => {
    const item = scene.items[index];
    return item === undefined
      ? undefined
      : {
          top: (scene.listTop + item.start) * MINIMAP_SCALE - scene.top,
          height: (item.end - item.start) * MINIMAP_SCALE,
        };
  };
  const { pitch } = scene;
  const room = scene.width - 2 * INSET_PX;
  const column = room / Math.max(1, scene.columns);
  const count = Math.min(scene.rows.length, scene.items.length);
  for (
    let index = firstBelow(scene.items, count, scene.top / MINIMAP_SCALE - scene.listTop);
    index < count;
    index += 1
  ) {
    const box = extent(index);
    if (box === undefined || box.top >= scene.height) {
      break;
    }
    const row = scene.rows[index];
    if (row === undefined) {
      continue;
    }
    const height = box.height >= GAP_FROM_PX ? box.height - 1 : box.height;
    add(row.kind, 0, box.top, scene.width, height);
    if (height < pitch) {
      continue;
    }
    const layout = row.layout(scene.columns);
    const end = box.top + height;
    // What each flexible line of the row gets of its spare height (less when the row is shorter than its miniature).
    const share = layout.flex > 0 ? Math.max(-0.75 * pitch, (height - layout.lines * pitch) / layout.flex) : 0;
    for (const piece of layout.pieces) {
      const top = box.top + piece.top * pitch + piece.pushed * share;
      if (top >= end) {
        break;
      }
      const size = piece.lines * (piece.flex ? pitch + share : pitch);
      const { words } = piece;
      if (words === undefined) {
        add(piece.ink, INSET_PX, top, room, Math.min(size, end - top));
        continue;
      }
      const wordTop = top + size * WORD_TOP;
      const wordHeight = size * WORD_FILL;
      if (wordTop + wordHeight > end) {
        break;
      }
      for (let at = 0; at + 1 < words.length; at += 2) {
        add(piece.ink, INSET_PX + (words[at] ?? 0) * column, wordTop, (words[at + 1] ?? 0) * column, wordHeight);
      }
    }
  }
  const inBand = (box: MinimapBox | undefined): box is MinimapBox =>
    box !== undefined && box.top < scene.height && box.top + Math.max(MARK_PX, box.height) > 0;
  for (const index of scene.matches) {
    const box = extent(index);
    if (inBand(box)) {
      add("match", scene.width - MARK_PX, box.top, MARK_PX, Math.max(MARK_PX, box.height));
    }
  }
  const focused = scene.current === undefined ? undefined : extent(scene.current);
  if (inBand(focused)) {
    add("focus", 0, focused.top, MARK_PX, Math.max(MARK_PX, focused.height));
  }
  return shapes;
}

/** Whether two scenes draw the same picture (rows compared one by one: {@link minimapRow} keeps their identity). */
export function sameMinimapScene(a: MinimapScene, b: MinimapScene): boolean {
  return (
    a.items === b.items &&
    a.listTop === b.listTop &&
    a.width === b.width &&
    a.top === b.top &&
    a.height === b.height &&
    a.columns === b.columns &&
    a.pitch === b.pitch &&
    a.current === b.current &&
    sameValues(a.matches, b.matches) &&
    sameValues(a.rows, b.rows)
  );
}

function sameValues<T>(a: readonly T[], b: readonly T[]): boolean {
  return a === b || (a.length === b.length && a.every((value, index) => value === b[index]));
}

/**
 * How far the map's content is scrolled on a map `height` tall: not at all while the miniature fits the map, else in
 * step with the view, its top at the view's top and its end at the view's end.
 */
export function minimapOffset({ scrollTop, scrollHeight, clientHeight }: ScrollPosition, height: number): number {
  const overflow = scrollHeight * MINIMAP_SCALE - height;
  const range = scrollHeight - clientHeight;
  if (overflow <= 0 || range <= 0) {
    return 0;
  }
  return overflow * Math.min(1, Math.max(0, scrollTop / range));
}

/**
 * The band of the map's content the canvas holds for a map `height` tall: {@link BAND_HEIGHTS} map heights centred on
 * the map's window, within the content, or the map's window alone while the miniature fits the map.
 */
export function minimapBand(position: ScrollPosition, height: number): MinimapBox {
  const content = position.scrollHeight * MINIMAP_SCALE;
  const size = Math.max(height, Math.min(content, BAND_HEIGHTS * height));
  const top = minimapOffset(position, height) - (size - height) / 2;
  return { top: Math.max(0, Math.min(content - size, top)), height: size };
}

/** Whether `band` holds all the map shows at `position`: else the canvas is drawn again. */
export function minimapCovers(band: MinimapBox, position: ScrollPosition, height: number): boolean {
  const offset = minimapOffset(position, height);
  const end = Math.min(offset + height, position.scrollHeight * MINIMAP_SCALE);
  return band.top <= offset + 0.5 && end <= band.top + band.height + 0.5;
}

/**
 * The viewport's box on a map `height` tall: where the view is on the scrolled map, {@link MINIMAP_SCALE} of its
 * height (at least {@link MIN_BOX_PX}), within the map.
 */
export function minimapBox(position: ScrollPosition, height: number): MinimapBox {
  if (position.scrollHeight <= 0 || height <= 0) {
    return { top: 0, height: 0 };
  }
  const size = Math.min(height, Math.max(MIN_BOX_PX, position.clientHeight * MINIMAP_SCALE));
  const top = position.scrollTop * MINIMAP_SCALE - minimapOffset(position, height);
  return { top: Math.min(height - size, Math.max(0, top)), height: size };
}

/**
 * The scroll position that puts the viewport's box's top `top` px down a map `height` tall, so a drag moves the box
 * with the pointer: the box moves {@link MINIMAP_SCALE} px per px of scroll, less what the map scrolls meanwhile.
 */
export function minimapDragScrollTop(top: number, height: number, position: ScrollPosition): number {
  const { scrollHeight, clientHeight } = position;
  const range = scrollHeight - clientHeight;
  const overflow = scrollHeight * MINIMAP_SCALE - height;
  const rate = overflow > 0 && range > 0 ? MINIMAP_SCALE - overflow / range : MINIMAP_SCALE;
  const scroll = rate > 0 ? top / rate : 0;
  return Math.round(Math.min(Math.max(0, range), Math.max(0, scroll)));
}

/** The scroll position that centres the view on what a press `y` px down a map `height` tall points at. */
export function minimapScrollTop(y: number, height: number, position: ScrollPosition): number {
  if (height <= 0) {
    return 0;
  }
  const { scrollHeight, clientHeight } = position;
  const top = (y + minimapOffset(position, height)) / MINIMAP_SCALE - clientHeight / 2;
  return Math.round(Math.min(Math.max(0, scrollHeight - clientHeight), Math.max(0, top)));
}

/** The user's scroll goes on this long after their last input: a smooth scroll's frames, a released scrollbar's last
 * page, a touch's momentum. */
const USER_SCROLL_MS = 500;

/**
 * Which scroll events of the transcript the user caused, the only ones that show the minimap (UI-860). A scroll event
 * is not the user's when it finds the view where the app last put it itself ({@link ScrollIntent.wrote}: following the
 * bottom, a restore, the jump to the bottom, the virtualizer's corrections) since the user's last input, even while
 * they are scrolling (a wheel down at the bottom just before a streamed delta). Otherwise it is the user's while a
 * gesture lasts ({@link ScrollIntent.hold}: a press on the scrollbar, a touch) or up to {@link USER_SCROLL_MS} after the
 * user's last input ({@link ScrollIntent.input}: a wheel notch, a scroll key, a `nav.*` command, a search visit, a
 * minimap jump, the end of a gesture that scrolled), and with no input, when the view moved while the content and the
 * view kept their sizes: nothing but the user moves it so (dragging a scrollbar that sends the page no pointer events,
 * as WebKitGTK's overlay ones, or a smooth scroll lasting longer), while layout moving it (the browser clamping a
 * content that shrank or a view that grew after a message is sent, a row measured) changes a size.
 */
export class ScrollIntent {
  #until = Number.NEGATIVE_INFINITY;
  #held = 0;
  /** Where the app last put the view itself, since the user's last input. */
  #written: number | undefined;
  /** Where the last scroll event found the view. */
  #last: ScrollPosition | undefined;

  /** The user scrolled at `now` (ms). */
  input(now: number): void {
    this.#until = now + USER_SCROLL_MS;
    this.#written = undefined;
  }

  /** A gesture started: every scroll is the user's until it ends. */
  hold(): void {
    this.#held += 1;
    this.#written = undefined;
  }

  /** A gesture ended at `now`; one that scrolled goes on a while (`goesOn`). */
  release(now: number, goesOn: boolean): void {
    this.#held = Math.max(0, this.#held - 1);
    if (goesOn) {
      this.input(now);
    }
  }

  /** Whether the user is scrolling at `now`: a gesture lasts, or their last input is recent. */
  active(now: number): boolean {
    return this.#held > 0 || now < this.#until;
  }

  /** The app put the view at `scrollTop` itself. */
  wrote(scrollTop: number): void {
    this.#written = scrollTop;
  }

  /** A scroll event found the view at `position` at `now`: whether the user moved it there. */
  scrolled(position: ScrollPosition, now: number): boolean {
    const last = this.#last;
    this.#last = position;
    if (this.#written !== undefined && Math.abs(position.scrollTop - this.#written) < 1) {
      return false;
    }
    return (
      this.active(now) ||
      (last !== undefined &&
        Math.abs(position.scrollTop - last.scrollTop) >= 1 &&
        position.scrollHeight === last.scrollHeight &&
        position.clientHeight === last.clientHeight)
    );
  }
}
