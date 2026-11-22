/**
 * Compacting a thread's context on demand (UI-910, UI-913): `:compact`'s and `/compact`'s arguments read as omp's
 * `/compact` reads them.
 */

import { isObject } from "../sdk/json.ts";

/**
 * omp 19.1.0's one-off `/compact` modes (`session/compact-modes.ts`), each a compaction method; its fourth mode,
 * `handoff`, runs in the background while the turn goes on and is not offered here. omp's RPC `compact`
 * (`thread.compact`) takes none: a mode reaches omp only through its `/compact` builtin sent as a prompt.
 */
export const COMPACT_MODES = ["soft", "remote", "snapcompact"] as const;

export type CompactMode = (typeof COMPACT_MODES)[number];

/** The mode that archives the history as images without a summary, so it takes no focus (omp's `rejectsFocus`). */
export const UNFOCUSED_MODE: CompactMode = "snapcompact";

/** What to compact with: an optional mode, then the summary's focus (`""`: none). */
export interface CompactRequest {
  readonly mode: CompactMode | undefined;
  readonly focus: string;
}

/**
 * Reads `args` as omp's `parseCompactArgs`: a first word naming a mode (in any case) is the mode, the rest the focus;
 * otherwise the whole text is the focus.
 */
export function parseCompact(args: string): CompactRequest {
  const text = args.trim();
  const first = text.split(/\s/, 1)[0] ?? "";
  const mode = COMPACT_MODES.find((name) => name === first.toLowerCase());
  return mode === undefined ? { mode: undefined, focus: text } : { mode, focus: text.slice(first.length).trim() };
}

/** omp's `/compact` with `mode` and `focus`, the prompt that runs a mode over RPC. */
export function compactPrompt(mode: CompactMode, focus: string): string {
  return focus === "" ? `/compact ${mode}` : `/compact ${mode} ${focus}`;
}

/** The size `thread.compact` reports (omp's `CompactionResult.tokensBefore`), when it is a positive number. */
export function tokensBefore(result: unknown): number | undefined {
  const before = isObject(result) ? result.tokensBefore : undefined;
  return typeof before === "number" && before > 0 ? before : undefined;
}
