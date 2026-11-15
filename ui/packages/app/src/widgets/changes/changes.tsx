/**
 * `omfp:changes` (UI-080): for each repository of the current workspace, its branch, ahead/behind and changed files
 * with +/− counts, from the `workspace/<id>` topic (DMN-026); a file expands to its diff (`workspaces.diff`). The clean
 * repositories other than the workspace folder's are one collapsed group at the bottom (UI-342); a repository's head is
 * one line (UI-343). The status refreshes after each `edit`, `write` or `bash` run of the selected thread ends, and
 * from the refresh button: `workspaces.status` rescans the workspace and publishes the fresh status of every
 * repository on the topic.
 */

import type { FileContent, FileDiff } from "@omfp/protocol";
import { Badge, CodeDiff, Diff, DiffStat, Disclosure, EmptyState, Icon, IconButton, languageOfPath } from "omfp/kit";
import {
  type ChangedFile,
  type ChangesRepo,
  changeRunsKey,
  changesSections,
  changesView,
  errorCode,
  originalFromPatch,
  SELECTED_THREAD,
  strings,
  useCall,
  useCurrentWorkspaceId,
  useOpenFile,
  useReportError,
  useStored,
  useThread,
  useWindowState,
  useWorkspace,
  type WidgetProps,
  workspaceFilePath,
} from "omfp/sdk";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";

const text = strings.shell.changes;

/** `{ repo? }`: only that repository of the workspace (by name); every repository without it. */
export function ChangesWidget({ repo }: WidgetProps): ReactNode {
  const workspaceId = useCurrentWorkspaceId();
  if (workspaceId === undefined) {
    return (
      <EmptyState icon="folder-open" title={text.none} className="omfp-changes__none" data-testid="changes-none" />
    );
  }
  return <Changes workspaceId={workspaceId} only={typeof repo === "string" ? repo : undefined} />;
}

function Changes({ workspaceId, only }: { workspaceId: string; only: string | undefined }): ReactNode {
  const { workspace, status } = useWorkspace(workspaceId);
  const [selected] = useWindowState(SELECTED_THREAD);
  const runs = changeRunsKey(useThread(selected)?.state.tools);
  const call = useCall();
  const reportError = useReportError();
  const firstRepo = workspace?.repos[0]?.name;
  const refresh = useCallback(() => {
    const request =
      firstRepo === undefined
        ? call("workspaces.rescan", { workspaceId })
        : call("workspaces.status", { workspaceId, repo: firstRepo });
    request.catch(reportError);
  }, [call, reportError, workspaceId, firstRepo]);

  // Subscribing already computes a fresh status: refresh only when a run ends after that.
  const seenRuns = useRef(runs);
  useEffect(() => {
    if (runs !== seenRuns.current) {
      seenRuns.current = runs;
      refresh();
    }
  }, [runs, refresh]);

  const sections = useMemo(
    () => (workspace === undefined ? { listed: [], clean: [] } : changesSections(changesView(workspace, status), only)),
    [workspace, status, only],
  );
  return (
    <section className="omfp-changes" data-testid="changes" aria-label={text.label}>
      <div className="omfp-changes__bar">
        <IconButton
          label={text.refresh}
          icon={<Icon name="refresh-cw" />}
          onPress={refresh}
          data-testid="changes-refresh"
        />
      </div>
      {sections.listed.length === 0 && sections.clean.length === 0 ? (
        <EmptyState icon="folder-git-2" title={text.noRepos} data-testid="changes-no-repo" />
      ) : (
        <>
          {sections.listed.map((repo) => (
            <RepoChanges key={repo.name} workspaceId={workspaceId} repo={repo} runs={runs} />
          ))}
          {sections.clean.length > 0 && <CleanRepos repos={sections.clean} />}
        </>
      )}
    </section>
  );
}

/**
 * The clean repositories (UI-342): one disclosure, collapsed by default, whose state the widget instance keeps
 * (`useStored`); expanded, one line per repository.
 */
function CleanRepos({ repos }: { repos: readonly ChangesRepo[] }): ReactNode {
  const [expanded, setExpanded] = useStored("cleanExpanded", false);
  return (
    <Disclosure
      className="omfp-changes__clean"
      data-testid="changes-clean"
      level={2}
      toggleTestId="changes-clean-toggle"
      expanded={expanded}
      onExpandedChange={setExpanded}
      title={text.cleanRepos(repos.length)}
    >
      {expanded && (
        <ul className="omfp-changes__clean-list">
          {repos.map((repo) => (
            <li key={repo.name}>
              <section
                className="omfp-changes__repo"
                data-testid="changes-repo"
                data-repo={repo.name}
                data-clean="true"
                aria-label={repo.name}
              >
                <RepoHead repo={repo} level={3} summary={false} />
              </section>
            </li>
          ))}
        </ul>
      )}
    </Disclosure>
  );
}

interface RepoChangesProps {
  readonly workspaceId: string;
  readonly repo: ChangesRepo;
  readonly runs: string;
}

/**
 * A repository's changes; the workspace folder's own one (UI-341) has the workspace icon and mark, and a rule below. A
 * clean one is its head alone (UI-342).
 */
function RepoChanges({ workspaceId, repo, runs }: RepoChangesProps): ReactNode {
  return (
    <section
      className="omfp-changes__repo"
      data-testid="changes-repo"
      data-repo={repo.name}
      data-workspace={repo.isWorkspace ? "true" : undefined}
      data-clean={repo.clean ? "true" : undefined}
      aria-label={repo.isWorkspace ? text.workspaceLabel(repo.name) : repo.name}
    >
      <RepoHead repo={repo} level={2} summary={true} />
      {repo.files === undefined ? (
        <p className="omfp-changes__empty">{text.loading}</p>
      ) : repo.files.length === 0 ? null : (
        <ul className="omfp-changes__files">
          {repo.files.map((file) => (
            <li key={file.path}>
              <FileChanges
                workspaceId={workspaceId}
                repo={repo.name}
                filePath={workspaceFilePath({ name: repo.name, isRoot: repo.isWorkspace }, file.path)}
                file={file}
                runs={runs}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/**
 * A repository's head on one line (UI-343): the name, then the branch, each cut with an ellipsis when the line is
 * short (the whole text as their tooltip), then ahead/behind and the summary, never cut nor wrapped.
 */
function RepoHead({ repo, level, summary }: { repo: ChangesRepo; level: 2 | 3; summary: boolean }): ReactNode {
  const Heading = level === 2 ? "h2" : "h3";
  return (
    <Heading className="omfp-changes__repo-head">
      {repo.isWorkspace && <Icon name="folder" className="omfp-changes__workspace-icon" />}
      <span className="omfp-changes__repo-name" title={repo.name}>
        {repo.name}
      </span>
      {repo.isWorkspace && <Badge data-testid="changes-workspace-mark">{text.workspace}</Badge>}
      <span className="omfp-changes__branch" title={repo.branch} data-testid="changes-branch">
        <Icon name="git-branch" />
        <span className="omfp-changes__branch-name">{repo.branch}</span>
      </span>
      {repo.aheadBehind !== undefined && (
        <span className="omfp-changes__meta" data-testid="changes-ahead-behind">
          {repo.aheadBehind}
        </span>
      )}
      {summary && repo.summary !== undefined && (
        <span className="omfp-changes__meta omfp-changes__summary" data-testid="changes-summary">
          {repo.summary}
        </span>
      )}
    </Heading>
  );
}

interface FileChangesProps {
  readonly workspaceId: string;
  readonly repo: string;
  /** The file's path in the workspace folder (what `workspaces.read_file` and `openFile` take). */
  readonly filePath: string;
  readonly file: ChangedFile;
  readonly runs: string;
}

function FileChanges({ workspaceId, repo, filePath, file, runs }: FileChangesProps): ReactNode {
  const [expanded, setExpanded] = useState(false);
  const openFile = useOpenFile();
  return (
    <Disclosure
      className="omfp-changes__file"
      data-testid="changes-file"
      data-path={file.path}
      data-status={file.status}
      // `:open` on the focused file opens it against `HEAD` (UI-116).
      data-omfp-path={filePath}
      data-omfp-diff=""
      level={3}
      toggleTestId="changes-file-toggle"
      expanded={expanded}
      onExpandedChange={setExpanded}
      title={
        <>
          <span className="omfp-changes__status" data-status={file.status} title={text.statusLabel[file.status]}>
            {text.status[file.status]}
          </span>
          {/* Inside the toggle: a pointer click opens without toggling; the keyboard uses `:open` (UI-116). */}
          {/* biome-ignore lint/a11y/useKeyWithClickEvents: see above */}
          {/* biome-ignore lint/a11y/noStaticElementInteractions: see above */}
          <span
            className="omfp-changes__path omfp-path-link"
            title={text.open(filePath)}
            data-testid="changes-open"
            onPointerDown={(event) => event.stopPropagation()}
            onPointerUp={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation();
              openFile(filePath, { diff: true });
            }}
          >
            {file.path}
          </span>
          {file.from !== undefined && <span className="omfp-changes__meta">{text.renamedFrom(file.from)}</span>}
          <DiffStat
            added={file.additions}
            removed={file.deletions}
            className="omfp-changes__counts"
            data-testid="changes-counts"
          />
        </>
      }
    >
      {expanded && <FileDiffView workspaceId={workspaceId} repo={repo} filePath={filePath} file={file} runs={runs} />}
    </Disclosure>
  );
}

/** A file's diff and its text now (`undefined`: the file is gone). */
interface Loaded {
  readonly diff: FileDiff;
  readonly current: FileContent | undefined;
}

/** The text the merge view shows: the whole file now, when it is text the daemon did not cut. */
function currentText(current: FileContent | undefined): string | undefined {
  if (current === undefined) {
    return "";
  }
  return current.kind === "text" && current.truncated !== true && typeof current.content === "string"
    ? current.content
    : undefined;
}

/**
 * The diff of one file, read again whenever its status or the selected thread's ended runs change: the whole file
 * against its text before the change (the patch applied in reverse) in CodeMirror's unified merge view, else the
 * patch itself (UI-080).
 */
function FileDiffView({
  workspaceId,
  repo,
  filePath,
  file,
  runs,
}: {
  workspaceId: string;
  repo: string;
  filePath: string;
  file: ChangedFile;
  runs: string;
}): ReactNode {
  const call = useCall();
  const reportError = useReportError();
  const [loaded, setLoaded] = useState<Loaded | undefined>(undefined);
  const { path, status, additions, deletions } = file;
  // biome-ignore lint/correctness/useExhaustiveDependencies: the status, counts and runs say when to read it again
  useEffect(() => {
    let live = true;
    const current = call("workspaces.read_file", { workspaceId, path: filePath }).catch((error: unknown) => {
      if (errorCode(error) === "not_found") {
        return undefined;
      }
      throw error;
    });
    Promise.all([call("workspaces.diff", { workspaceId, repo, path }), current])
      .then(([page, now]) => {
        if (live) {
          setLoaded({
            diff: page.files.find((entry) => entry.path === path) ?? { path, patch: "", truncated: false },
            current: now,
          });
        }
      })
      .catch(reportError);
    return () => {
      live = false;
    };
  }, [call, reportError, workspaceId, repo, path, filePath, status, additions, deletions, runs]);
  const merge = useMemo(() => {
    const value = loaded === undefined ? undefined : currentText(loaded.current);
    if (loaded === undefined || value === undefined || loaded.diff.truncated) {
      return undefined;
    }
    const original = originalFromPatch(value, loaded.diff.patch);
    return original === undefined ? undefined : { original, value };
  }, [loaded]);
  if (loaded === undefined) {
    return <p className="omfp-changes__empty">{text.loadingDiff}</p>;
  }
  if (merge !== undefined) {
    return (
      <CodeDiff
        className="omfp-changes__merge"
        original={merge.original}
        value={merge.value}
        path={path}
        layout="unified"
        aria-label={text.diffLabel(path)}
        data-testid="changes-diff"
        data-view="merge"
        loading={<p className="omfp-changes__empty">{text.loadingDiff}</p>}
      />
    );
  }
  return (
    <>
      <Diff
        patch={loaded.diff.patch}
        language={languageOfPath(path)}
        aria-label={text.diffLabel(path)}
        data-testid="changes-diff"
        data-view="patch"
      />
      {loaded.diff.truncated && <p className="omfp-changes__empty">{text.truncated}</p>}
    </>
  );
}
