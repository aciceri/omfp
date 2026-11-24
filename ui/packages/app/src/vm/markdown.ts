/**
 * The Markdown of a streaming reply in chunks (UI-191): parsing the whole reply again for every delta costs in
 * proportion to its length, so the transcript renders a streaming text block as top-level chunks, each parsed on its
 * own and parsed again only while it changes — in practice the last one. A finished message is rendered whole.
 *
 * A chunk starts at a line that cannot belong to the block before it: an unindented line after a blank line, outside
 * a fenced code block or a `$$` math block (UI-850), that does not continue a list or a block quote (`-`, `*`, `+`,
 * `1.`, `1)`, `>`). Indented lines (list item paragraphs, indented code) stay with what precedes them. A text with a
 * link reference definition is never split, since a definition applies to the whole document.
 */

/** A fence: backticks or tildes before an info string, or a line of two or more `$` alone. */
const FENCE = /^ {0,3}(`{3,}|~{3,}|\${2,}(?=\s*$))/;
const CONTINUES = /^(?:[-*+>]|\d{1,9}[.)])(?:\s|$)/;
const DEFINITION = /^ {0,3}\[[^\]]+\]:/m;

/** `text` split into top-level Markdown chunks; joined, they give `text` back. */
export function markdownChunks(text: string): string[] {
  if (DEFINITION.test(text)) {
    return [text];
  }
  const chunks: string[] = [];
  let start = 0;
  let fence: string | undefined;
  let blank = false;
  let at = 0;
  while (at < text.length) {
    const end = text.indexOf("\n", at);
    const next = end === -1 ? text.length : end + 1;
    const line = text.slice(at, end === -1 ? text.length : end);
    const marker = FENCE.exec(line)?.[1];
    if (fence !== undefined) {
      // A closing fence: the same character, at least as long, nothing after it.
      if (marker !== undefined && marker[0] === fence[0] && marker.length >= fence.length && line.trim() === marker) {
        fence = undefined;
      }
    } else {
      if (blank && at > start && /^\S/.test(line) && !CONTINUES.test(line)) {
        chunks.push(text.slice(start, at));
        start = at;
      }
      fence = marker;
    }
    blank = fence === undefined && line.trim() === "";
    at = next;
  }
  chunks.push(text.slice(start));
  return chunks;
}
