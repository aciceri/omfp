import { describe, expect, test } from "bun:test";
import { type AgentMessage, emptyThreadState, type NoticeFrame, type ThreadState } from "@omfp/core";
import { conversationMarkdown, exportFileName, messageMarkdown } from "../../src/vm/conversation.ts";
import { messageView } from "../../src/vm/transcript.ts";

function conversation(messages: readonly AgentMessage[]): ThreadState {
  return { ...emptyThreadState(), transcript: messages.map((message) => ({ message_id: null, message })) };
}

const png = { type: "image", data: "iVBORw0KGgo=", mimeType: "image/png" } as const;

/** A question with an image, a turn of a thought, a tool call and its result, and a reply; a steer and a failure. */
const MESSAGES: AgentMessage[] = [
  { role: "user", content: [{ type: "text", text: "What is in this plot?" }, png], timestamp: 1_000 },
  {
    role: "assistant",
    content: [
      { type: "thinking", thinking: "Let me look at the data first." },
      { type: "text", text: "Reading the data." },
      { type: "toolCall", id: "c1", name: "lookup", arguments: { query: "plot data" } },
    ],
    timestamp: 2_000,
  },
  {
    role: "toolResult",
    toolCallId: "c1",
    toolName: "lookup",
    content: [{ type: "text", text: "x = 1\n```inner fence```" }, png],
    timestamp: 3_200,
  },
  { role: "assistant", content: [{ type: "text", text: "A **line** going up.\n" }], timestamp: 4_000 },
  { role: "user", steering: true, content: "Shorter, please", timestamp: 5_000 },
  { role: "assistant", content: [], errorMessage: "Overloaded\nretry later", timestamp: 6_000 },
];

describe("the conversation as Markdown", () => {
  // spec: UI-792
  test("messages go under their author; thinking is left out, images are placeholders, tool calls one line", () => {
    expect(conversationMarkdown(conversation(MESSAGES), "Plot\nquestions")).toBe(
      `${[
        "# Plot questions",
        "## You",
        "What is in this plot?",
        "*[image: image/png]*",
        "## Assistant",
        "Reading the data.",
        "**Tool `lookup`** query: plot data (done, 1.2 s)",
        "````\nx = 1\n```inner fence```\n[image]\n````",
        "A **line** going up.",
        "## You (steering)",
        "Shorter, please",
        "## Assistant",
        "> **Error:** Overloaded\n> retry later",
      ].join("\n\n")}\n`,
    );
  });

  // spec: UI-792
  test("records keep their summary; omp's bookkeeping and the notices are left out", () => {
    const frame: NoticeFrame = { type: "unknown", data: { type: "mystery" } };
    const state: ThreadState = {
      ...conversation([
        { role: "user", content: "run it" },
        { role: "bashExecution", command: "ls -a", output: ".\n..", exitCode: 0 } as AgentMessage,
      ]),
      notices: [{ position: 1, frame }],
    };
    expect(conversationMarkdown(state, "")).toBe("# \n\n## You\n\nrun it\n\n## bash\n\nls -a\n");
  });

  // spec: UI-792
  test("the reply being streamed is there as it stands", () => {
    const state: ThreadState = {
      ...conversation([{ role: "user", content: "hi" }]),
      streaming: { message_id: null, message: { role: "assistant", content: [{ type: "text", text: "Hel" }] } },
    };
    expect(conversationMarkdown(state, "t")).toBe("# t\n\n## You\n\nhi\n\n## Assistant\n\nHel\n");
  });

  // spec: UI-790
  test("one message is its blocks without the heading; a thought alone is nothing", () => {
    const state = conversation(MESSAGES);
    expect(messageMarkdown(messageView(state, 0))).toBe("What is in this plot?\n\n*[image: image/png]*");
    expect(messageMarkdown(messageView(state, 3))).toBe("A **line** going up.");
    const thought = conversation([{ role: "assistant", content: [{ type: "thinking", thinking: "hmm" }] }]);
    expect(messageMarkdown(messageView(thought, 0))).toBe("");
  });

  // spec: UI-792
  test("a text keeps the indentation of its first line (an indented code block); blank edges go", () => {
    const state = conversation([
      { role: "assistant", content: [{ type: "text", text: "\n  \n    let x = 1;\n    x  \n\n" }] },
    ]);
    expect(messageMarkdown(messageView(state, 0))).toBe("    let x = 1;\n    x");
  });

  // spec: UI-792
  test("a tool name with backticks and a call without a result stay readable", () => {
    const state = conversation([
      { role: "assistant", content: [{ type: "toolCall", id: "c1", name: "a`b", arguments: {} }] },
    ]);
    expect(messageMarkdown(messageView(state, 0))).toBe("**Tool ``a`b``** (pending)");
  });

  // spec: UI-791
  test("the exported file is named after the title", () => {
    expect(exportFileName("Fix the parser: ü & “quotes”!")).toBe("fix-the-parser-ü-quotes.md");
    expect(exportFileName("  ")).toBe("conversation.md");
    expect(exportFileName(`${"a".repeat(79)} b`)).toBe(`${"a".repeat(79)}.md`);
  });
});
