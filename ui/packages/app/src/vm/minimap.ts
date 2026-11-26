/**
 * The transcript's minimap (UI-860..UI-863): a schematic overview of the whole conversation, drawn on one canvas beside
 * the transcript's scrollbar while the user scrolls. This module decides what the canvas shows — one block per
 * transcript row colored by its kind, faded lines for its text and bars for its tool cards, marks on the search's
 * matching rows and the focused row — where the viewport box sits, and where a press on the map scrolls to; the widget
 * (`widgets/chat/minimap.tsx`) draws it. Positions come from the virtualizer's measurements (estimates for rows never
 * rendered), scaled so the whole scroll content fits the map's height.
 */

import type { Block, RowContent } from "./transcript.ts";
import type { ItemExtent, ScrollPosition } from "./virtual.ts";

/** What a row is, as its block's color shows: a tool row is an assistant message of tool calls only. */
export type MinimapKind = "user" | "assistant" | "tool" | "error" | "system";

/** One displayed part of a row, top to bottom: text (faded lines), a tool card (a bar) or a failure (a red bar). */
export type MinimapPart = "text" | "tool" | "error";

/** How a row is drawn. */
export interface MinimapRow {
  readonly kind: MinimapKind;
  readonly parts: readonly MinimapPart[];
}

/** A fill of the drawing: a row kind's block, a part, or a mark. */
export type MinimapPaint = MinimapKind | "line" | "toolBar" | "errorBar" | "match" | "focus";

/** Everything a drawing depends on, in CSS px. */
export interface MinimapScene {
  /** How each transcript row is drawn; `undefined` for a row with nothing to show. */
  readonly rows: readonly (MinimapRow | undefined)[];
  /** Each row's extent in the list, from the virtualizer. */
  readonly items: readonly ItemExtent[];
  /** Where the list starts in the scroll content. */
  readonly listTop: number;
  readonly scrollHeight: number;
  /** The map's size. */
  readonly width: number;
  readonly height: number;
  /** Rows matching the conversation search (UI-730). */
  readonly matches: readonly number[];
  /** The focused row (UI-095). */
  readonly current: number | undefined;
}

/** The viewport's box on the map. */
export interface MinimapBox {
  readonly top: number;
  readonly height: number;
}

/** Rows at least this tall on the map keep a 1 px gap below them, so neighbours stay apart. */
const GAP_FROM_PX = 3;
/** Rows at least this tall on the map show their parts; smaller ones are a plain block. */
const PARTS_FROM_PX = 6;
/** Text parts draw a 1 px line every this many px; a part needs at least this much room. */
const LINE_PITCH_PX = 2;
/** Parts start this far from the map's left edge, where the focused row's mark goes. */
const INSET_PX = 2;
/** Width of the focus and search marks, and their least height. */
const MARK_PX = 2;
/** Lengths of successive text lines, as fractions of the room: ragged like prose. */
const LINE_SPANS = [0.92, 0.74, 0.86, 0.63, 0.97, 0.8, 0.55];
/** The viewport's box is at least this tall, so it still shows on a long thread. */
const MIN_BOX_PX = 6;

const SYSTEM_ROW: MinimapRow = { kind: "system", parts: [] };
const ROWS = new WeakMap<RowContent, MinimapRow>();

function partOf(block: Block): MinimapPart {
  switch (block.type) {
    case "tool":
      return block.card.status === "failed" ? "error" : "tool";
    case "readGroup":
      return block.status === "failed" ? "error" : "tool";
    default:
      return "text";
  }
}

/** How a row showing `content` is drawn; the same object while the content stays the same. */
export function minimapRow(content: RowContent): MinimapRow {
  if (content.type !== "message") {
    return SYSTEM_ROW;
  }
  const known = ROWS.get(content);
  if (known !== undefined) {
    return known;
  }
  const { view } = content;
  const parts = view.blocks.map(partOf);
  if (view.error !== undefined) {
    parts.push("error");
  }
  const kind: MinimapKind = parts.includes("error")
    ? "error"
    : view.role === "user"
      ? "user"
      : view.role === "record"
        ? "system"
        : parts.length > 0 && parts.every((part) => part === "tool")
          ? "tool"
          : "assistant";
  const row = { kind, parts };
  ROWS.set(content, row);
  return row;
}

/**
 * The rectangles of each paint, `[x, y, width, height]` one after the other: a block per row across the map, its parts
 * inset from the left edge when the row is tall enough (a few faded lines per text part, a bar per tool card), a mark
 * at the right edge of each matching row and one at the left edge of the focused row.
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
  const scale = scene.scrollHeight > 0 ? scene.height / scene.scrollHeight : 0;
  const extent = (index: number): MinimapBox | undefined => {
    const item = scene.items[index];
    return item === undefined
      ? undefined
      : { top: (scene.listTop + item.start) * scale, height: (item.end - item.start) * scale };
  };
  const room = scene.width - 2 * INSET_PX;
  const count = Math.min(scene.rows.length, scene.items.length);
  for (let index = 0; index < count; index += 1) {
    const row = scene.rows[index];
    const box = extent(index);
    if (row === undefined || box === undefined) {
      continue;
    }
    const height = box.height >= GAP_FROM_PX ? box.height - 1 : box.height;
    add(row.kind, 0, box.top, scene.width, height);
    const part = height / Math.max(1, row.parts.length);
    if (row.parts.length === 0 || height < PARTS_FROM_PX || part < LINE_PITCH_PX) {
      continue;
    }
    for (const [at, kind] of row.parts.entries()) {
      const top = box.top + at * part;
      if (kind === "text") {
        const lines = Math.floor(part / LINE_PITCH_PX);
        for (let line = 0; line < lines; line += 1) {
          const span = LINE_SPANS[(index + at + line) % LINE_SPANS.length] ?? 1;
          add("line", INSET_PX, top + line * LINE_PITCH_PX, room * span, 1);
        }
      } else {
        add(kind === "tool" ? "toolBar" : "errorBar", INSET_PX, top, room, part - 1);
      }
    }
  }
  for (const index of scene.matches) {
    const box = extent(index);
    if (box !== undefined) {
      add("match", scene.width - MARK_PX, box.top, MARK_PX, Math.max(MARK_PX, box.height));
    }
  }
  const focused = scene.current === undefined ? undefined : extent(scene.current);
  if (focused !== undefined) {
    add("focus", 0, focused.top, MARK_PX, Math.max(MARK_PX, focused.height));
  }
  return shapes;
}

/** Whether two scenes draw the same picture (rows compared one by one: {@link minimapRow} keeps their identity). */
export function sameMinimapScene(a: MinimapScene, b: MinimapScene): boolean {
  return (
    a.items === b.items &&
    a.listTop === b.listTop &&
    a.scrollHeight === b.scrollHeight &&
    a.width === b.width &&
    a.height === b.height &&
    a.current === b.current &&
    sameValues(a.matches, b.matches) &&
    sameValues(a.rows, b.rows)
  );
}

function sameValues<T>(a: readonly T[], b: readonly T[]): boolean {
  return a === b || (a.length === b.length && a.every((value, index) => value === b[index]));
}

/**
 * The viewport's box on a map `height` tall showing the whole scroll content: where the view is, at least
 * {@link MIN_BOX_PX} tall (centred on the view then), within the map.
 */
export function minimapBox({ scrollTop, scrollHeight, clientHeight }: ScrollPosition, height: number): MinimapBox {
  if (scrollHeight <= 0 || height <= 0) {
    return { top: 0, height: 0 };
  }
  const scale = height / scrollHeight;
  const exact = Math.min(height, clientHeight * scale);
  const size = Math.min(height, Math.max(MIN_BOX_PX, exact));
  const top = scrollTop * scale + (exact - size) / 2;
  return { top: Math.min(height - size, Math.max(0, top)), height: size };
}

/** The scroll position that centres the view on what a press `y` px down a map `height` tall points at. */
export function minimapScrollTop(
  y: number,
  height: number,
  { scrollHeight, clientHeight }: Pick<ScrollPosition, "scrollHeight" | "clientHeight">,
): number {
  if (height <= 0) {
    return 0;
  }
  const top = (y / height) * scrollHeight - clientHeight / 2;
  return Math.round(Math.min(Math.max(0, scrollHeight - clientHeight), Math.max(0, top)));
}
