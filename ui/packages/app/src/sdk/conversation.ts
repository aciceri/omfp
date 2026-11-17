/**
 * Copying and exporting a thread's conversation (UI-790, UI-791): the Markdown of `vm/conversation.ts`, rendered from
 * the thread's state in the window, goes to the clipboard or to a file the browser downloads (the desktop window saves
 * it in the Downloads folder). A thread no widget follows is followed until its snapshot arrives. Each shows a success
 * toast or an error (UI-520).
 */

import type { ThreadState } from "@omfp/core";
import { threadTopic } from "@omfp/protocol";
import { useMemo } from "react";
import { strings } from "../strings/index.ts";
import { conversationMarkdown, exportFileName, messageMarkdown } from "../vm/conversation.ts";
import { displayTitle } from "../vm/threads.ts";
import type { MessageView } from "../vm/transcript.ts";
import { reportError, reportSuccess } from "./actions.ts";
import { type OmfpHost, useHost } from "./host.tsx";

const text = strings.shell.conversation;

/** How long a downloaded file's object URL stays valid: the download reads it after the click returns. */
const OBJECT_URL_MS = 60_000;

export interface ConversationActions {
  /** Copies the conversation of `threadId` as Markdown. */
  copy(threadId: string): Promise<void>;
  /** Copies one message (`messageView`) as Markdown, without its heading. */
  copyMessage(view: MessageView): Promise<void>;
  /** Saves the conversation of `threadId` as a Markdown file named after its title. */
  export(threadId: string): Promise<void>;
}

/** `threadId`'s state; when no widget follows the thread, its topic is followed until the snapshot arrives. */
async function threadState(host: OmfpHost, threadId: string): Promise<ThreadState> {
  const known = host.frames.getSnapshot().threads.get(threadId)?.state;
  if (known !== undefined) {
    return known;
  }
  const topic = threadTopic(threadId);
  const release = host.topics.retain(topic);
  const { promise, resolve, reject } = Promise.withResolvers<ThreadState>();
  const unsubscribe = host.frames.subscribe(() => {
    const snapshot = host.frames.getSnapshot();
    const state = snapshot.threads.get(threadId)?.state;
    const sync = snapshot.topics.find((followed) => followed.topic === topic)?.sync;
    if (state !== undefined) {
      resolve(state);
    } else if (sync?.kind === "failed") {
      reject(new Error(`${text.notLoaded}: ${sync.error.message}`));
    }
  });
  try {
    return await promise;
  } finally {
    unsubscribe();
    release();
  }
}

/** The title and Markdown of `threadId`'s conversation. */
async function conversation(host: OmfpHost, threadId: string): Promise<{ title: string; markdown: string }> {
  const state = await threadState(host, threadId);
  const summary = host.frames.getSnapshot().index.threads.get(threadId);
  const title = summary === undefined ? strings.shell.untitled : displayTitle(summary);
  return { title, markdown: conversationMarkdown(state, title) };
}

/** Lets the browser download `markdown` as `file` (the desktop window saves downloads in the Downloads folder). */
function download(file: string, markdown: string): void {
  const url = URL.createObjectURL(new Blob([markdown], { type: "text/markdown;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = file;
  link.hidden = true;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), OBJECT_URL_MS);
}

export function conversationActions(host: OmfpHost): ConversationActions {
  // `navigator.clipboard` is missing outside secure contexts: its throw is reported like a refusal.
  const attempt = (act: () => Promise<string>) =>
    act().then(
      (done) => reportSuccess(host, done),
      (error: unknown) => reportError(host, error),
    );
  return {
    copy: (threadId) =>
      attempt(async () => {
        // Before any await: Safari allows the write only during the user's gesture, while the text may wait for the
        // thread's snapshot. A `ClipboardItem` takes the text as a promise; without one, it is written once ready.
        const markdown = conversation(host, threadId).then((found) => found.markdown);
        await (typeof ClipboardItem === "function" && typeof navigator.clipboard.write === "function"
          ? Promise.all([
              navigator.clipboard.write([
                new ClipboardItem({
                  "text/plain": markdown.then((value) => new Blob([value], { type: "text/plain" })),
                }),
              ]),
              markdown,
            ])
          : markdown.then((value) => navigator.clipboard.writeText(value)));
        return text.copied;
      }),
    copyMessage: (view) =>
      attempt(async () => {
        await navigator.clipboard.writeText(`${messageMarkdown(view)}\n`);
        return text.copiedMessage;
      }),
    export: (threadId) =>
      attempt(async () => {
        const { title, markdown } = await conversation(host, threadId);
        const file = exportFileName(title);
        download(file, markdown);
        return text.exported(file);
      }),
  };
}

/** {@link ConversationActions} of this window. */
export function useConversationActions(): ConversationActions {
  const host = useHost();
  return useMemo(() => conversationActions(host), [host]);
}
