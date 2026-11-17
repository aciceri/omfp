/**
 * A thread's conversation as Markdown (UI-792): what `:copy` puts on the clipboard (UI-790) and `:export` saves
 * (UI-791). The window renders it from the thread's reducer state, the rows the transcript shows: no omp process is
 * needed, so a stopped thread's conversation copies too.
 *
 * Each message goes under its author (`## You`, `## Assistant`, the record kind), a turn's later assistant messages
 * without a heading of their own (UI-470); text as written, thinking left out, images as placeholders, each tool call
 * as one line with its result in a code block; omp's bookkeeping rows and the notices are left out.
 */

import type { ThreadState } from "@omfp/core";
import { strings } from "../strings/index.ts";
import { formatDuration, type ToolCard } from "./tool.ts";
import { type Block, items, type MessageView, rowContent } from "./transcript.ts";

const text = strings.chat.markdown;

/** The conversation of `state` as Markdown under `# <title>`, ending with a newline. */
export function conversationMarkdown(state: ThreadState, title: string): string {
  const sections = [`# ${title.replace(/\s+/g, " ").trim()}`];
  for (const item of items(state)) {
    const content = rowContent(state, item);
    if (content?.type !== "message") {
      continue;
    }
    const { view } = content;
    const heading = view.steering ? `${view.label} (${strings.chat.steeringMark})` : view.label;
    const body = messageMarkdown(view);
    const parts = content.continues ? [body] : [`## ${heading}`, body];
    const section = parts.filter((part) => part !== "").join("\n\n");
    if (section !== "") {
      sections.push(section);
    }
  }
  return `${sections.join("\n\n")}\n`;
}

/** One message's Markdown without its heading (what `:copy` copies of a focused message); `""` when it shows nothing
 * but thinking. */
export function messageMarkdown(view: MessageView): string {
  const parts = view.blocks.map(blockMarkdown);
  if (view.error !== undefined && view.error !== "") {
    parts.push(`> **${text.error}:** ${view.error.replace(/\n/g, "\n> ")}`);
  }
  return parts.filter((part) => part !== "").join("\n\n");
}

/** Longest name of an exported file before its extension. */
const FILE_NAME_CHARS = 80;

/** The file `:export` saves a conversation titled `title` as: its letters and digits in lower case, joined by `-`. */
export function exportFileName(title: string): string {
  const slug = title
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .slice(0, FILE_NAME_CHARS)
    .replace(/^-+|-+$/g, "");
  return `${slug === "" ? text.fileName : slug}.md`;
}

function blockMarkdown(block: Block): string {
  switch (block.type) {
    case "text":
      // Only blank lines go at the start: the first line's indentation may make an indented code block.
      return block.text.replace(/^(?:[ \t]*\r?\n)+/, "").trimEnd();
    case "thinking":
      return "";
    case "image":
      return `*${text.image(block.mime)}*`;
    case "tool":
      return toolMarkdown(block.card);
    case "readGroup":
      // A read group (UI-765) is its reads, each one line with its result like any call.
      return block.cards.map(toolMarkdown).join("\n\n");
    case "record":
      return block.summary;
  }
}

/** `**Tool `name`**: summary (status, duration)`, then the result as a code block. */
function toolMarkdown(card: ToolCard): string {
  const facts = [
    strings.chat.tool.status[card.status],
    ...(card.durationMs === undefined ? [] : [formatDuration(card.durationMs)]),
  ];
  const summary = card.summary === "" ? "" : ` ${card.summary}`;
  const line = `**${text.tool} ${codeSpan(card.name)}**${summary} (${facts.join(", ")})`;
  const result = card.result?.replace(/\s+$/, "") ?? "";
  return result === "" ? line : `${line}\n\n${fenced(result)}`;
}

/** `value` as inline code, with a backtick run longer than any inside it. */
function codeSpan(value: string): string {
  const ticks = "`".repeat(longestRun(value) + 1);
  const pad = value.startsWith("`") || value.endsWith("`") ? " " : "";
  return `${ticks}${pad}${value}${pad}${ticks}`;
}

/** `body` as a fenced code block whose fence is longer than any backtick run inside it. */
function fenced(body: string): string {
  const fence = "`".repeat(Math.max(3, longestRun(body) + 1));
  return `${fence}\n${body}\n${fence}`;
}

function longestRun(value: string): number {
  return Math.max(0, ...(value.match(/`+/g) ?? []).map((run) => run.length));
}
