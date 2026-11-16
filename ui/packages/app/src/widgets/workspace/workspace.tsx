/**
 * `omfp:workspace` (UI-025): the current workspace — the selected thread's, else the draft thread's (UI-023). Header:
 * name (renamed in place), folder path (copyable). Repositories: kind, name, branch, git status live from
 * `workspace/<id>`, origin (link target, worktree's main repository), a badge when the checkout is in another workspace
 * with live threads (DMN-027), remove for every kind (confirmation for anything but a link, DMN-025). One "add
 * repository" control with a Link / Clone switch; paths come with suggestions and completion (DMN-021). The daemon
 * rescans on its own (DMN-023); `:rescan` forces it.
 */

import type { Workspace } from "@omfp/protocol";
import {
  Badge,
  BorderBox,
  Button,
  ConfirmDialog,
  EmptyState,
  Icon,
  IconButton,
  KindMark,
  PathField,
  Radio,
  RadioGroup,
  TextField,
} from "omfp/kit";
import {
  ADD_KINDS,
  type AddKind,
  addRepoSource,
  type RepoRow,
  removalNeedsConfirmation,
  reportError,
  strings,
  useCall,
  useCurrentWorkspaceId,
  useHost,
  usePathSources,
  useReportSuccess,
  useThreads,
  useWorkspace,
  useWorkspaces,
  type WidgetProps,
  type WorkspaceRow,
  workspaceRow,
} from "omfp/sdk";
import { type FormEvent, type KeyboardEvent, type ReactNode, useMemo, useState } from "react";

const text = strings.shell.workspace;

/** The icon of each add kind on the Link/Clone switch. */
const ADD_ICON: Record<AddKind, "link-2" | "copy"> = { link: "link-2", clone: "copy" };

/** Runs a call, reporting its failure to the frame; resolves to whether it succeeded. */
function useAttempt(): (call: () => Promise<unknown>) => Promise<boolean> {
  const host = useHost();
  return async (call) => {
    try {
      await call();
      return true;
    } catch (error) {
      reportError(host, error);
      return false;
    }
  };
}

export function WorkspaceWidget(_props: WidgetProps): ReactNode {
  const shown = useCurrentWorkspaceId();
  return (
    <section
      className="omfp-workspace"
      data-testid="workspaces-panel"
      data-omfp-scope="workspace"
      aria-label={text.label}
    >
      {shown === undefined ? (
        <EmptyState icon="folder-open" title={text.none} data-testid="no-workspace" />
      ) : (
        <CurrentWorkspace key={shown} workspaceId={shown} />
      )}
    </section>
  );
}

function CurrentWorkspace({ workspaceId }: { workspaceId: string }): ReactNode {
  const { workspace, status } = useWorkspace(workspaceId);
  const threads = useList(useThreads());
  const workspaces = useList(useWorkspaces());
  if (workspace === undefined) {
    return null;
  }
  const row = workspaceRow(workspace, threads, workspaces, status);
  return (
    <article className="omfp-workspace__current" data-testid="workspace-current" data-workspace-id={workspaceId}>
      <Header workspace={workspace} row={row} />
      <Repos row={row} />
      <AddRepository workspaceId={workspaceId} />
    </article>
  );
}

function useList<T>(map: ReadonlyMap<string, T>): readonly T[] {
  return useMemo(() => [...map.values()], [map]);
}

function Header({ workspace, row }: { workspace: Workspace; row: WorkspaceRow }): ReactNode {
  const attempt = useAttempt();
  const reportSuccess = useReportSuccess();
  const copy = () =>
    void attempt(() => navigator.clipboard.writeText(row.path)).then((ok) => ok && reportSuccess(text.copied));
  return (
    <header className="omfp-workspace__header">
      <WorkspaceName workspace={workspace} />
      <div className="omfp-workspace__folder">
        <Icon name="folder-open" className="omfp-workspace__folder-icon" />
        <code className="omfp-workspace__path" data-testid="workspace-path" title={text.path}>
          {row.path}
        </code>
        <IconButton
          label={text.copyPath}
          icon={<Icon name="copy" />}
          data-testid="copy-workspace-path"
          onPress={copy}
        />
      </div>
      {row.shared && (
        <div className="omfp-workspace__meta">
          <Badge tone="warning" data-testid="shared-warning" title={text.sharedHint}>
            {text.shared(row.threadCount)}
          </Badge>
        </div>
      )}
    </header>
  );
}

function WorkspaceName({ workspace }: { workspace: Workspace }): ReactNode {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(workspace.name);
  const call = useCall();
  const attempt = useAttempt();
  if (!editing) {
    return (
      <div className="omfp-workspace__title">
        <h2 data-testid="workspace-name">{workspace.name}</h2>
        <Button
          size="sm"
          variant="quiet"
          data-testid="rename-workspace"
          onPress={() => {
            setName(workspace.name);
            setEditing(true);
          }}
        >
          {text.rename}
        </Button>
      </div>
    );
  }
  const save = (event: FormEvent) => {
    event.preventDefault();
    setEditing(false);
    const trimmed = name.trim();
    if (trimmed !== "" && trimmed !== workspace.name) {
      void attempt(() => call("workspaces.update", { workspaceId: workspace.id, name: trimmed }));
    }
  };
  return (
    <form className="omfp-workspace__title" data-testid="workspace-name-form" onSubmit={save}>
      <TextField
        label={text.nameLabel}
        hideLabel
        value={name}
        onChange={setName}
        // The editor opens on request and takes the focus (UI-092)
        autoFocus
        inputProps={{
          "data-testid": "workspace-name-input",
          onKeyDown: (event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              setEditing(false);
            }
          },
        }}
      />
      <Button type="submit" size="sm" variant="primary" data-testid="workspace-name-save">
        {text.save}
      </Button>
    </form>
  );
}

function Repos({ row }: { row: WorkspaceRow }): ReactNode {
  const call = useCall();
  const attempt = useAttempt();
  const [confirming, setConfirming] = useState<RepoRow | undefined>(undefined);
  const remove = (name: string) => void attempt(() => call("workspaces.remove_repo", { workspaceId: row.id, name }));
  return (
    <BorderBox aria-labelledby="omfp-workspace-repos" title={text.repos} level={3} titleId="omfp-workspace-repos">
      {row.repos.length === 0 ? (
        <EmptyState icon="folder-git-2" title={text.noRepo} />
      ) : (
        <ul>
          {row.repos.map((repo) => (
            <li
              key={repo.name}
              className="omfp-workspace__repo"
              data-testid="workspace-repo"
              data-kind={repo.kind}
              data-name={repo.name}
              data-dirty={repo.dirty === undefined ? undefined : String(repo.dirty)}
            >
              <strong className="omfp-workspace__repo-name">{repo.name}</strong>
              <KindMark
                kind={repo.kind}
                className="omfp-workspace__kind"
                role="img"
                aria-label={text.kinds[repo.kind]}
                title={text.kinds[repo.kind]}
                data-testid="repo-kind"
              >
                {text.kinds[repo.kind].toLowerCase()}
              </KindMark>
              {repo.git !== undefined && (
                <span className="omfp-workspace__git" data-testid="git-status">
                  {repo.git}
                </span>
              )}
              <IconButton
                className="omfp-workspace__remove"
                label={text.removeRepo(repo.name)}
                icon={<Icon name="x" />}
                data-testid="remove-repo"
                onPress={() => (removalNeedsConfirmation(repo.kind) ? setConfirming(repo) : remove(repo.name))}
              />
              <span className="omfp-workspace__where">
                <span className="omfp-workspace__branch" data-testid="repo-branch">
                  <Icon name="git-branch" />
                  {repo.branch}
                </span>
                {repo.origin !== undefined && (
                  <span className="omfp-workspace__origin" data-testid="repo-origin">
                    {repo.origin}
                  </span>
                )}
              </span>
              {repo.alsoIn.length > 0 && (
                <Badge tone="warning" data-testid="shared-repo" title={text.alsoInHint}>
                  {text.alsoIn(repo.alsoIn.join(", "))}
                </Badge>
              )}
            </li>
          ))}
        </ul>
      )}
      <ConfirmDialog
        isOpen={confirming !== undefined}
        onOpenChange={(open) => {
          if (!open) {
            setConfirming(undefined);
          }
        }}
        title={text.removeRepoTitle(confirming?.name ?? "")}
        message={text.removeRepoMessage}
        confirmLabel={text.removeNow}
        cancelLabel={text.cancel}
        tone="danger"
        onConfirm={() => {
          if (confirming !== undefined) {
            remove(confirming.name);
          }
        }}
        testId="remove-repo-dialog"
        confirmTestId="confirm-remove-repo"
        cancelTestId="cancel-remove-repo"
      />
    </BorderBox>
  );
}

/** One control adds a link or a clone (UI-025): a kind switch, the kind's field, one submit. Enter in a field submits
 * too; once the repository is added its field clears, unless something else was typed there meanwhile: the daemon
 * publishes the new repository before it answers (DMN-023), so the next one may already be typed. */
function AddRepository({ workspaceId }: { workspaceId: string }): ReactNode {
  const call = useCall();
  const attempt = useAttempt();
  const { suggest, complete } = usePathSources();
  const [kind, setKind] = useState<AddKind>("link");
  const [path, setPath] = useState("");
  const [url, setUrl] = useState("");
  const source = addRepoSource(kind, { path, url });
  const submit = (typedPath = path) => {
    const chosen = addRepoSource(kind, { path: typedPath, url });
    if (chosen === undefined) {
      return;
    }
    void attempt(() => call("workspaces.add_repo", { workspaceId, source: chosen })).then((ok) => {
      if (ok) {
        setPath((current) => (current === typedPath ? "" : current));
        setUrl((current) => (current === url ? "" : current));
      }
    });
  };
  const submitOnEnter = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      submit();
    }
  };
  return (
    <BorderBox
      aria-labelledby="omfp-workspace-add"
      data-testid="add-repo"
      title={text.addRepository}
      level={3}
      titleId="omfp-workspace-add"
    >
      <RadioGroup
        label={text.addKind}
        hideLabel
        orientation="horizontal"
        value={kind}
        onChange={(value) => setKind(ADD_KINDS.find((candidate) => candidate === value) ?? "link")}
        data-testid="add-repo-kind"
      >
        <div className="omfp-workspace__kind-options">
          {ADD_KINDS.map((candidate) => (
            <Radio key={candidate} value={candidate} data-value={candidate} className="omfp-focus-inset">
              <Icon name={ADD_ICON[candidate]} />
              {text.kinds[candidate]}
            </Radio>
          ))}
        </div>
      </RadioGroup>
      <div className="omfp-workspace__add-fields">
        {kind === "clone" ? (
          <TextField
            label={text.cloneUrl}
            hideLabel
            placeholder={text.clonePlaceholder}
            value={url}
            onChange={setUrl}
            inputProps={{ "data-testid": "add-repo-url", onKeyDown: submitOnEnter }}
          />
        ) : (
          <PathField
            label={text.linkPath}
            hideLabel
            placeholder={text.linkPlaceholder}
            value={path}
            onChange={setPath}
            onSubmit={submit}
            suggest={suggest}
            complete={complete}
            data-testid="add-repo-path"
          />
        )}
        <Button
          size="sm"
          variant="primary"
          isDisabled={source === undefined}
          data-testid="add-repo-submit"
          onPress={() => submit()}
        >
          {text.add}
        </Button>
      </div>
    </BorderBox>
  );
}
