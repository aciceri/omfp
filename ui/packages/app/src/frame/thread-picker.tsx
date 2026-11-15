/** Space / previews threads without discarding a draft; only confirmation commits the selection. */
import { useCallback, useState } from "react";
import { SELECTED_THREAD, useCommand, useHost, useThreadActions, useThreads, useWorkspaces } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { displayTitle, isArchived } from "../vm/threads.ts";
import { Picker } from "./picker.tsx";

export function ThreadPicker() {
  const host = useHost();
  const actions = useThreadActions();
  const threads = useThreads();
  const workspaces = useWorkspaces();
  const [session, setSession] = useState<{ original: string | undefined; focus: Element | null }>();
  const preview = useCallback((id: string) => host.window.set(SELECTED_THREAD, id), [host]);
  useCommand({
    id: "threads.pick",
    title: strings.shell.commands.pickThread,
    aliases: ["threads"],
    run: () => setSession({ original: host.window.get(SELECTED_THREAD), focus: document.activeElement }),
  });
  if (session === undefined) return null;
  const close = () => {
    setSession(undefined);
    queueMicrotask(() => {
      if (session.focus instanceof HTMLElement && session.focus.isConnected) session.focus.focus();
    });
  };
  return (
    <Picker
      id="thread-picker"
      title={strings.shell.commands.pickThread}
      empty={strings.shell.threads.noMatch}
      options={[...threads.values()]
        .filter((thread) => !isArchived(thread))
        .map((thread) => ({
          id: thread.id,
          title: displayTitle(thread),
          description: workspaces.get(thread.workspaceId)?.name ?? "",
        }))}
      initial={session.original}
      preview={preview}
      choose={(id) => {
        actions.select(id);
        close();
      }}
      cancel={() => {
        host.window.set(SELECTED_THREAD, session.original);
        close();
      }}
    />
  );
}
