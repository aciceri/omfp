/** Space f searches all workspace files on the daemon before its result limit, not a client-side prefix. */
import { useState } from "react";
import { openFile, SELECTED_THREAD, useCurrentWorkspaceId, useFindFile, useHost } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { Picker, usePickerSession } from "./picker.tsx";

const text = strings.shell.files;

export function WorkspaceFilePicker() {
  const host = useHost();
  const workspaceId = useCurrentWorkspaceId();
  const [session, close] = usePickerSession(
    { id: "file.pick", title: strings.shell.commands.pickFile, enabled: () => workspaceId !== undefined },
    () => ({}),
  );
  if (session === undefined || workspaceId === undefined) return null;
  return (
    <SearchInput
      key={workspaceId}
      workspaceId={workspaceId}
      threadId={host.window.get(SELECTED_THREAD)}
      close={close}
    />
  );
}

function SearchInput({
  workspaceId,
  threadId,
  close,
}: {
  workspaceId: string;
  threadId: string | undefined;
  close: () => void;
}) {
  const host = useHost();
  const [query, setQuery] = useState("");
  const found = useFindFile(workspaceId, threadId, query);
  const answer = found?.query === query ? found : undefined;
  return (
    <Picker
      id="workspace-file-picker"
      title={strings.shell.commands.pickFile}
      empty={answer?.error ?? (answer === undefined ? text.searching : text.noMatch)}
      busy={answer === undefined}
      notice={answer?.result?.truncated ? text.moreMatches : undefined}
      options={(answer?.result?.candidates ?? []).map((file) => {
        const slash = file.path.lastIndexOf("/");
        return {
          id: file.path,
          title: file.path.slice(slash + 1),
          description: `${file.repo ?? text.workspaceFolder} · ${slash < 0 ? "." : file.path.slice(0, slash)}`,
        };
      })}
      search={setQuery}
      choose={(path) => {
        openFile(host, path);
        close();
      }}
      cancel={close}
    />
  );
}
