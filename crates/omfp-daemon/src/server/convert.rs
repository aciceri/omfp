//! Conversions between the daemon's records and the protocol's DTOs and errors (PRT-011).

use std::collections::HashMap;
use std::path::PathBuf;

use omfp_protocol::{
    CreatedBy, DiffPage, ErrorBody, ErrorCode, FileChange, FileDiff, FileStatus, ForkOrigin,
    GitStatus, JobCounts, Provenance, RepoKind, ThreadSummary, TitleSource, Workspace,
    WorkspaceRepo,
};
use serde_json::json;

use crate::git::{self, GitError};
use crate::store::{self, StoreError};
use crate::supervisor::{SupervisorError, ThreadStatus};
use crate::threads::ThreadError;
use crate::workspaces::{self, Discovered, WorkspaceError};

pub(crate) fn bad_request(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::BadRequest, message)
}

pub(crate) fn internal(error: impl std::fmt::Display) -> ErrorBody {
    ErrorBody::new(ErrorCode::Internal, error.to_string())
}

impl From<StoreError> for ErrorBody {
    fn from(error: StoreError) -> Self {
        let message = error.to_string();
        let code = match error {
            StoreError::NotFound { .. } => ErrorCode::NotFound,
            StoreError::Conflict(_) => ErrorCode::Conflict,
            StoreError::InvalidPath(_) => ErrorCode::BadRequest,
            StoreError::UnknownSchemaVersion { .. }
            | StoreError::Json(_)
            | StoreError::Migration(_)
            | StoreError::Sqlite(_) => ErrorCode::Internal,
        };
        ErrorBody::new(code, message)
    }
}

/// The protocol error of a failed `mcp.tools` or `mcp.call` (DMN-354): `not_found` for an unknown server,
/// `bad_request` for one the daemon cannot call or a JSON-RPC error of the server (its code in `details.mcpCode`),
/// `unauthorized` when the server refused omp's login (`details.reauth` the command that renews it), `internal` for
/// transport failures, timeouts and a refresh that failed.
impl From<crate::mcp_client::McpError> for ErrorBody {
    fn from(error: crate::mcp_client::McpError) -> Self {
        use crate::mcp_client::McpError;
        let message = error.to_string();
        match error {
            McpError::NotFound(_) => ErrorBody::new(ErrorCode::NotFound, message),
            McpError::Disabled(_) | McpError::Stdio(_) | McpError::NoUrl(_) => {
                ErrorBody::new(ErrorCode::BadRequest, message)
            }
            McpError::Rpc { code, .. } => ErrorBody::new(ErrorCode::BadRequest, message)
                .with_details(json!({ "mcpCode": code })),
            McpError::Unauthorized { server, status } => {
                ErrorBody::new(ErrorCode::Unauthorized, message).with_details(
                    json!({ "status": status, "reauth": format!("/mcp reauth {server}") }),
                )
            }
            McpError::Transport { .. } | McpError::Timeout { .. } | McpError::Login { .. } => {
                ErrorBody::new(ErrorCode::Internal, message)
            }
        }
    }
}

pub(crate) fn thread_error(error: ThreadError) -> ErrorBody {
    match error {
        ThreadError::Store(error) => ErrorBody::from(error),
        ThreadError::Io(_) => internal(error),
    }
}

pub(crate) fn workspace_error(error: WorkspaceError) -> ErrorBody {
    let error = match error {
        WorkspaceError::Store(error) => return ErrorBody::from(error),
        other => other,
    };
    let code = match &error {
        WorkspaceError::NotADirectory(_)
        | WorkspaceError::NotARepository(_)
        | WorkspaceError::NotARepositoryRoot(_)
        | WorkspaceError::DetachedHead(_)
        | WorkspaceError::InvalidName(_)
        | WorkspaceError::IsRoot(_)
        | WorkspaceError::OutsideFolder(_)
        | WorkspaceError::ReadOnly(_)
        | WorkspaceError::Unreadable(_)
        | WorkspaceError::IsADirectory(_)
        | WorkspaceError::Git(GitError::NotARepository(_)) => ErrorCode::BadRequest,
        WorkspaceError::NoSuchRepo { .. }
        | WorkspaceError::NoSuchFile(_)
        | WorkspaceError::NoSuchFolder(_) => ErrorCode::NotFound,
        WorkspaceError::Dirty(_)
        | WorkspaceError::EntryExists(_)
        | WorkspaceError::HasThreads(_)
        | WorkspaceError::FileChanged(_) => ErrorCode::Conflict,
        WorkspaceError::Git(_) => ErrorCode::GitError,
        WorkspaceError::Store(_) | WorkspaceError::Io(_) => ErrorCode::Internal,
    };
    ErrorBody::new(code, error.to_string())
}

pub(crate) fn supervisor_error(error: SupervisorError) -> ErrorBody {
    let error = match error {
        SupervisorError::Thread(error) => return thread_error(error),
        SupervisorError::Workspace(error) => return workspace_error(error),
        other => other,
    };
    let code = match &error {
        SupervisorError::Thread(_)
        | SupervisorError::Workspace(_)
        | SupervisorError::ShuttingDown
        | SupervisorError::Internal(_) => ErrorCode::Internal,
        SupervisorError::LimitReached(_) => ErrorCode::LimitReached,
        SupervisorError::SessionLeased { .. } | SupervisorError::ProcessGone(_) => {
            ErrorCode::ThreadFailed
        }
        // omp's own words on why it would not start (DMN-212).
        SupervisorError::Start { stderr_tail, .. } => {
            let details = serde_json::json!({ "stderrTail": stderr_tail });
            return ErrorBody::new(ErrorCode::ThreadFailed, error.to_string())
                .with_details(details);
        }
        SupervisorError::Omp { code, .. } => {
            let mut body = ErrorBody::new(ErrorCode::OmpError, error.to_string());
            if let Some(code) = code {
                body = body.with_omp_code(code.clone());
            }
            return body;
        }
        SupervisorError::AlreadyResolved(_) => ErrorCode::AlreadyResolved,
        SupervisorError::InvalidResponse(_)
        | SupervisorError::EmptyOffer
        | SupervisorError::UserMcpConfig => ErrorCode::BadRequest,
        SupervisorError::LiveActive { .. }
        | SupervisorError::NoLiveSession
        | SupervisorError::Busy
        | SupervisorError::BusyMove => ErrorCode::Conflict,
        SupervisorError::Live { omp, .. } => {
            // omp's own refusal of a second session (DMN-104): the same conflict the daemon reports itself.
            let code = if omp.contains("A live session is already active") {
                ErrorCode::Conflict
            } else {
                ErrorCode::OmpError
            };
            let mut body = ErrorBody::new(code, error.to_string());
            body.details = Some(serde_json::json!({ "ompError": omp }));
            return body;
        }
    };
    ErrorBody::new(code, error.to_string())
}

fn provenance(created_by: &store::Provenance, client: Option<&String>) -> Provenance {
    Provenance {
        created_by: match created_by {
            store::Provenance::User => CreatedBy::User,
            store::Provenance::Mcp => CreatedBy::Mcp,
            store::Provenance::Thread(thread_id) => CreatedBy::Thread {
                thread_id: thread_id.clone(),
            },
            store::Provenance::Job(job) => CreatedBy::Job { job: job.clone() },
        },
        client: client.cloned(),
    }
}

/// The protocol's workspace: the record, its discovered repositories (with the other workspaces holding the same
/// checkout, from `holders`), its threads (DMN-027), its place in the column (DMN-284) and the counts of its declared
/// jobs (PRT-210).
pub(crate) fn workspace(
    ws: &store::Workspace,
    repos: &[Discovered],
    thread_ids: Vec<String>,
    holders: &HashMap<PathBuf, Vec<String>>,
    clients: &HashMap<String, String>,
    position: u32,
    jobs: Option<JobCounts>,
) -> Workspace {
    let text = |path: &std::path::Path| path.to_string_lossy().into_owned();
    Workspace {
        id: ws.id.clone(),
        name: ws.name.clone(),
        path: text(&ws.path),
        repos: repos
            .iter()
            .map(|repo| WorkspaceRepo {
                name: repo.name.clone(),
                path: text(&repo.path),
                kind: match &repo.kind {
                    workspaces::RepoKind::Link { target } => RepoKind::Link {
                        target: text(target),
                    },
                    workspaces::RepoKind::Worktree { main } => {
                        RepoKind::Worktree { main: text(main) }
                    }
                    workspaces::RepoKind::Clone => RepoKind::Clone,
                },
                branch: repo.branch.clone(),
                head: repo.head.clone(),
                is_root: repo.is_root,
                canonical_path: text(&repo.canonical),
                shared_with: holders
                    .get(&repo.canonical)
                    .into_iter()
                    .flatten()
                    .filter(|id| **id != ws.id)
                    .cloned()
                    .collect(),
            })
            .collect(),
        thread_ids,
        provenance: provenance(&ws.created_by, clients.get(&ws.id)),
        created_at: ws.created_at,
        position,
        jobs,
    }
}

/// The protocol's thread summary, with its place among the threads of its workspace (DMN-284).
pub(crate) fn thread_summary(
    thread: &store::Thread,
    status: ThreadStatus,
    clients: &HashMap<String, String>,
    forks: &HashMap<String, store::ThreadFork>,
    updated_at: i64,
    position: u32,
) -> ThreadSummary {
    ThreadSummary {
        id: thread.id.clone(),
        title: thread.title.clone(),
        title_source: match thread.title_source {
            store::TitleSource::Placeholder => TitleSource::Placeholder,
            store::TitleSource::Generated => TitleSource::Generated,
            store::TitleSource::Set => TitleSource::Set,
        },
        workspace_id: thread.workspace_id.clone(),
        process: status.process,
        settled: status.settled,
        needs_attention: status.needs_attention,
        restart_pending: status.restart_pending,
        provenance: provenance(&thread.created_by, clients.get(&thread.id)),
        created_at: thread.created_at,
        updated_at,
        archived_at: thread.archived_at,
        position,
        forked_from: forks.get(&thread.id).map(|fork| ForkOrigin {
            thread_id: fork.source_thread_id.clone(),
            title: fork.source_title.clone(),
            entry_id: fork.entry_id.clone(),
            message: fork.message.clone(),
        }),
    }
}

/// One file of a unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FilePatch<'a> {
    path: String,
    old_path: Option<String>,
    status: FileStatus,
    patch: &'a str,
}

/// Splits a `git diff` patch into its files.
fn split_patch(diff: &str) -> Vec<FilePatch<'_>> {
    let mut starts: Vec<usize> = diff
        .match_indices("diff --git ")
        .map(|(index, _)| index)
        .filter(|&index| index == 0 || diff.as_bytes()[index - 1] == b'\n')
        .collect();
    starts.push(diff.len());
    starts
        .windows(2)
        .map(|window| parse_file(&diff[window[0]..window[1]]))
        .collect()
}

fn parse_file(patch: &str) -> FilePatch<'_> {
    let mut status = FileStatus::Modified;
    let (mut old, mut new, mut rename_from, mut rename_to) = (None, None, None, None);
    for line in patch.lines().skip(1) {
        if line.starts_with("@@") || line.starts_with("Binary files") {
            break;
        }
        if line.starts_with("new file mode") {
            status = FileStatus::Added;
        } else if line.starts_with("deleted file mode") {
            status = FileStatus::Deleted;
        } else if let Some(from) = line.strip_prefix("rename from ") {
            rename_from = Some(from.to_owned());
        } else if let Some(to) = line.strip_prefix("rename to ") {
            rename_to = Some(to.to_owned());
        } else if let Some(path) = line.strip_prefix("--- a/") {
            old = Some(path.to_owned());
        } else if let Some(path) = line.strip_prefix("+++ b/") {
            new = Some(path.to_owned());
        }
    }
    let header_path = patch
        .lines()
        .next()
        .and_then(|header| header.rsplit_once(" b/"))
        .map(|(_, path)| path.to_owned());
    let path = rename_to
        .clone()
        .or(new)
        .or(old)
        .or(header_path)
        .unwrap_or_default();
    let old_path = rename_from.filter(|_| rename_to.is_some());
    if old_path.is_some() {
        status = FileStatus::Renamed;
    }
    FilePatch {
        path,
        old_path,
        status,
        patch,
    }
}

/// The protocol's git status: counts from git, the kind of change of each file from the diff headers (DMN-026).
pub(crate) fn git_status(status: &git::GitStatus, base_ref: Option<&str>, diff: &str) -> GitStatus {
    let kinds: HashMap<String, FilePatch<'_>> = split_patch(diff)
        .into_iter()
        .map(|file| (file.path.clone(), file))
        .collect();
    let count = |value: Option<u64>| u32::try_from(value.unwrap_or(0)).unwrap_or(u32::MAX);
    GitStatus {
        branch: status.branch.clone(),
        base_ref: base_ref.map(str::to_owned),
        ahead: count(status.ahead),
        behind: count(status.behind),
        dirty: status.dirty,
        files: status
            .files
            .iter()
            .map(|file| {
                let kind = kinds.get(&file.path);
                FileChange {
                    path: file.path.clone(),
                    old_path: kind.and_then(|kind| kind.old_path.clone()),
                    status: kind.map_or(FileStatus::Modified, |kind| kind.status),
                    additions: count(file.additions),
                    deletions: count(file.deletions),
                }
            })
            .collect(),
    }
}

/// Bytes of patch text per diff page, and per file before truncation (PRT-013).
const PAGE_BYTES: usize = 4 * 1024 * 1024;
const FILE_BYTES: usize = 1024 * 1024;

/// One page of a workspace diff: whole files from `cursor` (an index) until the page is full; a single file larger
/// than [`FILE_BYTES`] is cut and marked `truncated` (PRT-013). `path` keeps only that file.
pub(crate) fn diff_page(
    diff: &str,
    path: Option<&str>,
    cursor: Option<&str>,
) -> Result<DiffPage, ErrorBody> {
    let files: Vec<FilePatch<'_>> = split_patch(diff)
        .into_iter()
        .filter(|file| path.is_none_or(|path| file.path == path))
        .collect();
    let start = match cursor {
        None => 0,
        Some(cursor) => cursor
            .parse::<usize>()
            .ok()
            .filter(|&start| start <= files.len())
            .ok_or_else(|| {
                bad_request(format!("invalid cursor {cursor:?}"))
                    .with_details(json!({ "cursor": cursor }))
            })?,
    };
    let mut page = DiffPage {
        files: Vec::new(),
        next_cursor: None,
    };
    let mut used = 0;
    for (index, file) in files.iter().enumerate().skip(start) {
        let (patch, truncated) = truncate(file.patch, FILE_BYTES);
        if !page.files.is_empty() && used + patch.len() > PAGE_BYTES {
            page.next_cursor = Some(index.to_string());
            break;
        }
        used += patch.len();
        page.files.push(FileDiff {
            path: file.path.clone(),
            patch: patch.to_owned(),
            truncated,
        });
    }
    Ok(page)
}

fn truncate(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/lib.rs b/src/lib.rs
index 1..2 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-a
+b
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/old name.md b/new name.md
similarity index 100%
rename from old name.md
rename to new name.md
";

    #[test]
    fn diff_headers_give_each_file_its_kind_of_change() {
        let files = split_patch(DIFF);
        let kinds: Vec<(&str, FileStatus, Option<&str>)> = files
            .iter()
            .map(|file| (file.path.as_str(), file.status, file.old_path.as_deref()))
            .collect();

        assert_eq!(
            kinds,
            [
                ("src/lib.rs", FileStatus::Modified, None),
                ("new.txt", FileStatus::Added, None),
                ("gone.txt", FileStatus::Deleted, None),
                ("new name.md", FileStatus::Renamed, Some("old name.md")),
            ]
        );
    }

    // spec: PRT-013
    #[test]
    fn diff_pages_follow_cursors_and_filter_by_path() {
        let page = diff_page(DIFF, None, None).unwrap();
        assert_eq!(page.files.len(), 4);
        assert_eq!(page.next_cursor, None);

        let one = diff_page(DIFF, Some("new.txt"), None).unwrap();
        assert_eq!(one.files.len(), 1);
        assert!(one.files[0].patch.contains("+hello"));

        let rest = diff_page(DIFF, None, Some("3")).unwrap();
        assert_eq!(rest.files[0].path, "new name.md");
        assert_eq!(
            diff_page(DIFF, None, Some("x")).unwrap_err().code,
            ErrorCode::BadRequest
        );
    }

    // spec: PRT-013
    #[test]
    fn huge_files_are_truncated_and_pages_stay_bounded() {
        let big = format!(
            "diff --git a/big b/big\n--- a/big\n+++ b/big\n@@ -1 +1 @@\n{}\n",
            "+é".repeat(FILE_BYTES)
        );
        let diff = big.repeat(6);

        let first = diff_page(&diff, None, None).unwrap();

        assert!(
            first
                .files
                .iter()
                .all(|file| file.truncated && file.patch.len() <= FILE_BYTES)
        );
        assert_eq!(first.files.len(), 4);
        assert_eq!(first.next_cursor.as_deref(), Some("4"));
        let second = diff_page(&diff, None, first.next_cursor.as_deref()).unwrap();
        assert_eq!(second.files.len(), 2);
        assert_eq!(second.next_cursor, None);
    }

    // spec: DMN-071
    #[test]
    fn omp_errors_keep_omps_code() {
        let body = supervisor_error(SupervisorError::Omp {
            command: "navigate_tree".into(),
            message: "no such entry".into(),
            code: Some("entry_not_found".into()),
        });

        assert_eq!(body.code, ErrorCode::OmpError);
        assert_eq!(body.omp_code.as_deref(), Some("entry_not_found"));
    }
}
