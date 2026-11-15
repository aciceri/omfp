/** Space f searches all workspace files on the daemon before its result limit, not a client-side prefix. */
import type { FileCandidates } from "@omfp/protocol";
import { useEffect, useState } from "react";
import { errorMessage, openFile, SELECTED_THREAD, useCommand, useCurrentWorkspaceId, useHost } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { Picker } from "./picker.tsx";

const text = strings.shell.files;

export function WorkspaceFilePicker() {
  const host = useHost();
  const workspaceId = useCurrentWorkspaceId();
  const [session, setSession] = useState<{ focus: Element | null }>();
  useCommand({
    id: "file.pick",
    title: strings.shell.commands.pickFile,
    enabled: () => workspaceId !== undefined,
    run: () => setSession({ focus: document.activeElement }),
  });
  if (session === undefined || workspaceId === undefined) return null;
  const close = () => {
    setSession(undefined);
    queueMicrotask(() => {
      if (session.focus instanceof HTMLElement && session.focus.isConnected) session.focus.focus();
    });
  };
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
  const [found, setFound] = useState<{
    query: string;
    result: FileCandidates | undefined;
    error: string | undefined;
  }>();
  useEffect(() => {
    let live = true;
    host.client
      .call("workspaces.find_file", {
        workspaceId,
        query,
        fuzzy: true,
        ...(threadId === undefined ? {} : { threadId }),
      })
      .then(
        (result) => {
          if (live) setFound({ query, result, error: undefined });
        },
        (error: unknown) => {
          if (live) setFound({ query, result: undefined, error: errorMessage(error) });
        },
      );
    return () => {
      live = false;
    };
  }, [host, workspaceId, threadId, query]);
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
