/** The left column's workspace fields: "+ workspace" (UI-020, UI-170) and a section's inline rename (UI-020). */
import { ActionRow, PathField, TextField } from "@omfp/kit";
import type { Workspace } from "@omfp/protocol";
import { type FormEvent, type KeyboardEvent, type ReactNode, type RefObject, useEffect, useRef, useState } from "react";
import { errorMessage, useCall, useReportError, useThreadActions } from "../../sdk/index.ts";
import { usePathSources } from "../../sdk/paths.ts";
import { strings } from "../../strings/index.ts";
import { isWorkspacePath, newWorkspaceParams } from "../../vm/threads.ts";

const text = strings.shell.threads;

/** "+ workspace"'s field (UI-020): it takes the focus when it opens and a name (optional) or, when it starts with `/`
 * or `~`, the path of a repository to link (completed as in DMN-021); Enter creates the workspace and opens a draft
 * thread in it. Creating the workspace, Escape, or leaving the field empty closes it. */
function NewWorkspace({
  inputRef,
  onClose,
}: {
  inputRef: RefObject<HTMLInputElement | null>;
  onClose: (refocus: boolean) => void;
}): ReactNode {
  const call = useCall();
  const actions = useThreadActions();
  const sources = usePathSources();
  const labels = strings.shell.newWorkspace;
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>(undefined);
  useEffect(() => {
    inputRef.current?.focus();
  }, [inputRef]);
  const submit = (typed: string) => {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(undefined);
    call("workspaces.create", newWorkspaceParams(typed)).then(
      (workspace) => {
        setBusy(false);
        setValue("");
        onClose(false);
        actions.openDraft(workspace.id);
      },
      (failure: unknown) => {
        setBusy(false);
        setError(errorMessage(failure));
      },
    );
  };
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the field's own Escape and blur bubble here to close it
    <div
      className="omfp-threads__new-workspace"
      data-testid="new-workspace"
      data-mode={isWorkspacePath(value) ? "path" : "name"}
      onKeyDown={(event) => {
        // The path list's own Escape stops here (it closes the list first).
        if (event.key === "Escape") {
          onClose(true);
        }
      }}
      onBlur={(event) => {
        const next = event.relatedTarget;
        const inside = next instanceof Node && event.currentTarget.contains(next);
        if (!inside && value === "" && !busy) {
          onClose(false);
        }
      }}
    >
      <PathField
        label={labels.label}
        hideLabel
        placeholder={labels.placeholder}
        value={value}
        onChange={setValue}
        onSubmit={submit}
        complete={sources.complete}
        inputRef={inputRef}
        data-testid="new-workspace-input"
      />
      {error !== undefined && (
        <p className="omfp-form-error" role="alert" data-testid="new-workspace-error">
          {error}
        </p>
      )}
    </div>
  );
}

/** "+ workspace" below the last section (UI-020, UI-170): a row that turns into {@link NewWorkspace} when pressed or
 * `open` (`workspace.new`); closing the field turns it back, Escape with the focus on the row. */
export function AddWorkspace({
  open,
  onOpenChange,
  inputRef,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  inputRef: RefObject<HTMLInputElement | null>;
}): ReactNode {
  const box = useRef<HTMLDivElement>(null);
  const refocus = useRef(false);
  useEffect(() => {
    if (!open && refocus.current) {
      refocus.current = false;
      box.current?.querySelector<HTMLElement>('[data-testid="add-workspace"]')?.focus();
    }
  }, [open]);
  return (
    <div ref={box} className="omfp-threads__add-workspace">
      {open ? (
        <NewWorkspace
          inputRef={inputRef}
          onClose={(back) => {
            refocus.current ||= back;
            onOpenChange(false);
          }}
        />
      ) : (
        <ActionRow data-testid="add-workspace" onPress={() => onOpenChange(true)}>
          {strings.shell.newWorkspace.label}
        </ActionRow>
      )}
    </div>
  );
}

export function RenameWorkspace({ workspace, close }: { workspace: Workspace; close: () => void }): ReactNode {
  const call = useCall();
  const report = useReportError();
  const [name, setName] = useState(workspace.name);
  const save = (event: FormEvent) => {
    event.preventDefault();
    close();
    const trimmed = name.trim();
    if (trimmed !== "" && trimmed !== workspace.name) {
      call("workspaces.update", { workspaceId: workspace.id, name: trimmed }).catch(report);
    }
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    }
  };
  return (
    <form className="omfp-threads__rename" onSubmit={save}>
      <TextField
        label={text.renameLabel}
        hideLabel
        value={name}
        onChange={setName}
        onKeyDown={onKeyDown}
        // Opened from the section's menu to type the new name at once
        autoFocus
        inputProps={{ "data-testid": "workspace-name-input" }}
      />
    </form>
  );
}
