/**
 * The frame's dialogs: the workspace delete confirmation (UI-020), the `views.json` reset confirmation (UI-430), the
 * question about a closed thread's running commands (UI-744) and the token screen (UI-002).
 */

import { Button, Checkbox, ConfirmDialog, Dialog, TextField } from "@omfp/kit";
import type { CloseJobs, RunInfo, ThreadSummary, Workspace } from "@omfp/protocol";
import { type FormEvent, type ReactNode, useMemo, useState } from "react";
import { keyLabel } from "../keymap/keymap.ts";
import {
  CLOSE_THREAD,
  closeThread,
  DELETE_WORKSPACE,
  formatDuration,
  RESET_VIEWS,
  removeWorkspace,
  reportError,
  reportSuccess,
  useHost,
  useThreadSummary,
  useThreads,
  useWindowState,
  useWorkspaces,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { DEFAULT_MANIFEST } from "../view/views.ts";
import { displayTitle } from "../vm/threads.ts";

/**
 * Closing a thread whose agent's commands still run (UI-744, PRT-208): asked once, the dialog lists them (command, how
 * long they ran) and repeats the archive or delete with *Stop them* (`jobs: "kill"`) or *Keep them running*
 * (`jobs: "keep"`); Cancel, which has the focus, leaves the thread as it was.
 */
export function CloseThreadDialog(): ReactNode {
  const host = useHost();
  const [request, setRequest] = useWindowState(CLOSE_THREAD);
  const summary = useThreadSummary(request?.threadId);
  const text = strings.shell.closeRuns;
  const choose = (jobs: CloseJobs) => {
    if (request !== undefined) {
      setRequest(undefined);
      void closeThread(host, request.method, request.threadId, jobs);
    }
  };
  return (
    <Dialog
      isOpen={request !== undefined}
      onOpenChange={(open) => {
        if (!open) {
          setRequest(undefined);
        }
      }}
      title={text.title(summary === undefined ? (request?.threadId ?? "") : displayTitle(summary))}
      role="alertdialog"
      data-testid="close-runs-dialog"
    >
      {({ close }) =>
        request === undefined ? null : (
          <>
            <div className="omfp-dialog__message">
              <p>{text.message(request.runs.length)}</p>
              <ul data-testid="close-runs-list">
                {request.runs.map((run) => (
                  <ClosedRun key={run.id} run={run} />
                ))}
              </ul>
            </div>
            <div className="omfp-dialog__actions">
              <Button
                variant="quiet"
                onPress={close}
                autoFocus
                kbd={keyLabel("Escape")}
                data-testid="close-runs-cancel"
              >
                {text.cancel}
              </Button>
              <Button onPress={() => choose("keep")} data-testid="close-runs-keep">
                {text.keep}
              </Button>
              <Button variant="danger" onPress={() => choose("kill")} data-testid="close-runs-stop">
                {text.stop}
              </Button>
            </div>
          </>
        )
      }
    </Dialog>
  );
}

/** One run of {@link CloseThreadDialog}: its command (PRT-201) and how long it ran. */
function ClosedRun({ run }: { run: RunInfo }): ReactNode {
  const text = strings.shell.closeRuns;
  const command = run.command ?? undefined;
  return (
    <li data-testid="close-run" data-run={run.id}>
      <code>{command ?? text.command}</code> {text.ran(formatDuration(Date.now() - run.startedAt))}
    </li>
  );
}
/**
 * `:view-reset` (UI-107, UI-430): the current default manifest replaces the workspace's views.json, valid or not, only
 * once confirmed; the result is announced and shown as a success toast, failures go to the error list.
 */
export function ResetViewsDialog(): ReactNode {
  const host = useHost();
  const [workspaceId, setWorkspaceId] = useWindowState(RESET_VIEWS);
  const text = strings.shell.resetViews;
  const reset = async (id: string) => {
    try {
      await host.client.call("views.set", { workspaceId: id, manifest: DEFAULT_MANIFEST });
      reportSuccess(host, strings.shell.layout.reset);
    } catch (error) {
      reportError(host, error);
    }
  };
  return (
    <ConfirmDialog
      isOpen={workspaceId !== undefined}
      onOpenChange={(open) => {
        if (!open) {
          setWorkspaceId(undefined);
        }
      }}
      title={text.title}
      message={text.message}
      confirmLabel={text.confirm}
      cancelLabel={text.cancel}
      tone="danger"
      onConfirm={() => {
        if (workspaceId !== undefined) {
          void reset(workspaceId);
        }
      }}
      testId="view-reset-dialog"
      confirmTestId="confirm-view-reset"
      cancelTestId="cancel-view-reset"
      cancelKbd={keyLabel("Escape")}
    />
  );
}

/**
 * Deleting a workspace (UI-020, DMN-028): the daemon refuses while it has threads, so the dialog lists them and
 * deletes them first once "delete the threads too" is ticked; the folder is deleted, linked projects are kept. Cancel
 * has the focus when it opens.
 */
export function DeleteWorkspaceDialog(): ReactNode {
  const [workspaceId, setWorkspaceId] = useWindowState(DELETE_WORKSPACE);
  const workspace = useWorkspaces().get(workspaceId ?? "");
  const threadMap = useThreads();
  const threads = useMemo(
    () => [...threadMap.values()].filter((thread) => thread.workspaceId === workspaceId),
    [threadMap, workspaceId],
  );
  return (
    <Dialog
      isOpen={workspace !== undefined}
      onOpenChange={(open) => {
        if (!open) {
          setWorkspaceId(undefined);
        }
      }}
      title={strings.shell.deleteWorkspace.title(workspace?.name ?? "")}
      role="alertdialog"
      data-testid="delete-workspace-dialog"
    >
      {({ close }) =>
        workspace === undefined ? null : (
          <DeleteWorkspaceForm key={workspace.id} workspace={workspace} threads={threads} close={close} />
        )
      }
    </Dialog>
  );
}

function DeleteWorkspaceForm({
  workspace,
  threads,
  close,
}: {
  workspace: Workspace;
  threads: readonly ThreadSummary[];
  close: () => void;
}): ReactNode {
  const host = useHost();
  const [deleteThreads, setDeleteThreads] = useState(false);
  const text = strings.shell.deleteWorkspace;
  const blocked = threads.length > 0 && !deleteThreads;
  return (
    <>
      <div className="omfp-dialog__message" data-testid="delete-workspace-message">
        <p>{text.message(workspace.path)}</p>
        {threads.length > 0 && (
          <>
            <p>{text.threads(threads.length)}</p>
            <ul data-testid="delete-workspace-thread-list">
              {threads.map((thread) => (
                <li key={thread.id} data-thread-id={thread.id}>
                  {displayTitle(thread)}
                </li>
              ))}
            </ul>
            <Checkbox isSelected={deleteThreads} onChange={setDeleteThreads} data-testid="delete-workspace-threads">
              {text.deleteThreads}
            </Checkbox>
          </>
        )}
      </div>
      <div className="omfp-dialog__actions">
        <Button
          variant="quiet"
          onPress={close}
          autoFocus
          kbd={keyLabel("Escape")}
          data-testid="cancel-delete-workspace"
        >
          {text.cancel}
        </Button>
        <Button
          variant="danger"
          isDisabled={blocked}
          data-testid="confirm-delete-workspace"
          onPress={() => {
            close();
            void removeWorkspace(host, workspace.id, deleteThreads ? threads.map((thread) => thread.id) : []);
          }}
        >
          {text.confirm}
        </Button>
      </div>
    </>
  );
}

/** Asks for the token printed by `omfp daemon` when none is known or the daemon refused it (UI-002). */
export function TokenScreen({ rejected, onSubmit }: { rejected: boolean; onSubmit: (token: string) => void }) {
  const [token, setToken] = useState("");
  const text = strings.shell.token;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const value = token.trim();
    if (value !== "") {
      onSubmit(value);
    }
  };
  return (
    <main className="omfp-token" data-testid="token-screen">
      <form className="omfp-token__form" aria-labelledby="omfp-token-title" onSubmit={submit}>
        <h1 id="omfp-token-title">{text.title}</h1>
        <p>{text.help}</p>
        <TextField
          label={text.label}
          type="password"
          autoComplete="off"
          value={token}
          onChange={setToken}
          // The token is the only thing the page asks for
          autoFocus
          inputProps={{ "data-testid": "token-input" }}
        />
        {rejected && (
          <p className="omfp-form-error" role="alert" data-testid="token-rejected">
            {text.rejected}
          </p>
        )}
        <Button type="submit" variant="primary" data-testid="token-submit">
          {text.submit}
        </Button>
      </form>
    </main>
  );
}
