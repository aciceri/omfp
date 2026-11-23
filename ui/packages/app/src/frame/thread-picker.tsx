/** Space / previews threads without discarding a draft; only confirmation commits the selection. */
import { useCallback } from "react";
import { SELECTED_THREAD, useHost, useThreadActions, useThreads, useWorkspaces } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { displayTitle, isArchived } from "../vm/threads.ts";
import { Picker, usePickerSession } from "./picker.tsx";

export function ThreadPicker() {
  const host = useHost();
  const actions = useThreadActions();
  const threads = useThreads();
  const workspaces = useWorkspaces();
  const [session, close] = usePickerSession(
    { id: "threads.pick", title: strings.shell.commands.pickThread, aliases: ["threads"] },
    () => ({ original: host.window.get(SELECTED_THREAD) }),
  );
  const preview = useCallback((id: string) => host.window.set(SELECTED_THREAD, id), [host]);
  if (session === undefined) return null;
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
