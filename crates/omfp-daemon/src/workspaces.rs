//! Workspace folders (02-daemon, "Workspaces and their repositories"): creating and removing workspaces, discovering
//! their repositories (DMN-022) and suggesting the ones seen before (DMN-021), adding and removing repositories
//! (DMN-024, DMN-025), their git status (DMN-026), the cheap check that polls watched workspaces (DMN-023), and
//! reading and writing their files (`files`, DMN-089, DMN-090) and finding them by a partial name (`find`, DMN-092).
//!
//! Calls are blocking; the server layer runs them on blocking tasks.

mod files;
mod find;
mod fingerprint;
mod scan;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

pub use files::{IMAGE_CAP, TEXT_CAP, image_mime};
pub use find::{MAX_CANDIDATES, named_files};
pub use fingerprint::{Fingerprint, fingerprint};
pub use scan::{Discovered, RepoKind, scan};

use omfp_protocol::CreatedBy;

use crate::git::{Git, GitError, GitStatus, WorktreeBranch};
use crate::names::{is_entry_name, slugify, unique};
use crate::store::{SeenRepository, Store, StoreError, Workspace, new_id, now_ms};

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),
    #[error("{} is not in a git repository", .0.display())]
    NotARepository(PathBuf),
    #[error("{} is not the root of a git repository", .0.display())]
    NotARepositoryRoot(PathBuf),
    #[error("{} has a detached HEAD: give a base ref", .0.display())]
    DetachedHead(PathBuf),
    #[error("{0:?} is not a valid entry name (one visible path component)")]
    InvalidName(String),
    #[error("the workspace already has an entry named {0:?}")]
    EntryExists(String),
    #[error("workspace {workspace} has no repository named {name:?}")]
    NoSuchRepo { workspace: String, name: String },
    #[error("{0:?} is the workspace folder itself; remove the workspace instead")]
    IsRoot(String),
    #[error("{0} has uncommitted changes")]
    Dirty(String),
    #[error("workspace {0} still has threads; delete them first")]
    HasThreads(String),
    #[error("{0:?} is outside the workspace folder")]
    OutsideFolder(String),
    /// An absolute path outside the workspace folder given to a write: such files are read-only (DMN-313).
    #[error("{0:?} is outside the workspace folder: it can be read, not written")]
    ReadOnly(String),
    /// A file the daemon's user may not read (DMN-315).
    #[error("{0:?} cannot be read: permission denied")]
    Unreadable(String),
    #[error("{0:?} is a directory")]
    IsADirectory(String),
    #[error("no file {0:?} in the workspace")]
    NoSuchFile(String),
    #[error("the folder of {0:?} does not exist (folders are not created)")]
    NoSuchFolder(String),
    /// The file is not the one the change was based on (DMN-090).
    #[error("{0}")]
    FileChanged(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = WorkspaceError> = std::result::Result<T, E>;

/// A repository the daemon has seen in a workspace, offered when adding one (DMN-021).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub path: PathBuf,
    /// The path's last component.
    pub name: String,
    /// Unix milliseconds.
    pub last_seen_at: i64,
}

/// Parameters of [`Workspaces::create`] (DMN-020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkspace {
    /// Exact name; taken names are refused.
    pub name: Option<String>,
    /// Name used when `name` is absent, made unique (default: the folder name of `link`, else `workspace`).
    pub fallback_name: Option<String>,
    /// A git repository to link into the new folder, as [`Workspaces::add_repo`] does with [`RepoSource::Link`].
    pub link: Option<PathBuf>,
    pub created_by: CreatedBy,
}

impl NewWorkspace {
    /// A new empty folder with every default.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: None,
            fallback_name: None,
            link: None,
            created_by: CreatedBy::User,
        }
    }
}

impl Default for NewWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a repository added to a workspace comes from (DMN-024).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoSource {
    /// A symlink to the repository at this path.
    Link(PathBuf),
    /// A new worktree of the repository containing `repo`: on `branch` (existing, or created from `base_ref`),
    /// default a new `omfp/<slug>-<n>` branch.
    Worktree {
        repo: PathBuf,
        branch: Option<String>,
        base_ref: Option<String>,
    },
    /// `git clone <url>`.
    Clone(String),
}

/// A workspace with its discovered repositories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceView {
    pub workspace: Workspace,
    pub repos: Vec<Discovered>,
}

/// Workspace folders and the repositories in them.
#[derive(Debug)]
pub struct Workspaces {
    store: Arc<Store>,
    git: Arc<dyn Git>,
    /// Where workspace folders are made (`--workspaces-root`).
    root: PathBuf,
    /// One lock per repository (keyed by its main working tree), held during git operations that mutate the shared
    /// `.git` (DMN-025).
    repo_locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    /// The last scan of each workspace (DMN-023).
    scans: Mutex<HashMap<String, Vec<Discovered>>>,
    /// Workspaces with threads being deleted, by how many (DMN-039): those threads have no record any more while
    /// their omp may still be starting in the folder, so the workspace cannot be removed yet (DMN-028).
    deleting: Mutex<HashMap<String, usize>>,
}

/// Holds off the removal of a workspace while one of its threads is deleted ([`Workspaces::hold_removal`]).
#[derive(Debug)]
pub struct RemovalHold<'a> {
    workspaces: &'a Workspaces,
    id: String,
}

impl Drop for RemovalHold<'_> {
    fn drop(&mut self) {
        let mut deleting = self
            .workspaces
            .deleting
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = deleting.get_mut(&self.id) {
            *count -= 1;
            if *count == 0 {
                deleting.remove(&self.id);
            }
        }
    }
}

impl Workspaces {
    #[must_use]
    pub fn new(store: Arc<Store>, git: Arc<dyn Git>, root: PathBuf) -> Self {
        Self {
            store,
            git,
            root,
            repo_locks: Mutex::new(HashMap::new()),
            scans: Mutex::new(HashMap::new()),
            deleting: Mutex::new(HashMap::new()),
        }
    }

    fn repo_lock(&self, main_root: &Path) -> Arc<Mutex<()>> {
        self.repo_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(main_root.to_owned())
            .or_default()
            .clone()
    }

    /// The git port, for status and diff of discovered repositories.
    #[must_use]
    pub fn git(&self) -> &dyn Git {
        &*self.git
    }

    // Suggestions

    /// The repositories seen in any workspace whose path or name contains `query` (case-insensitive; empty: all),
    /// most recently seen first (DMN-021).
    pub fn suggest(&self, query: &str) -> Result<Vec<Suggestion>> {
        let query = query.trim().to_lowercase();
        Ok(self
            .store
            .seen_repositories()?
            .into_iter()
            .map(|SeenRepository { path, last_seen_at }| Suggestion {
                name: dir_name(&path),
                path,
                last_seen_at,
            })
            .filter(|repo| {
                query.is_empty()
                    || repo.name.to_lowercase().contains(&query)
                    || repo.path.to_string_lossy().to_lowercase().contains(&query)
            })
            .collect())
    }

    // Workspaces

    pub fn get(&self, id: &str) -> Result<Workspace> {
        Ok(self.store.workspace(id)?)
    }

    pub fn list(&self) -> Result<Vec<Workspace>> {
        Ok(self.store.workspaces()?)
    }

    /// Creates a workspace: always a new folder `<root>/<slug>` (DMN-020) holding the default `.omfp/views.json`
    /// (DMN-088). With `link`, that repository is checked first (nothing is created unless it is a git root), then
    /// linked into the folder as [`Self::add_repo`] does; the name then defaults to the repository's folder name.
    pub fn create(&self, new: NewWorkspace) -> Result<WorkspaceView> {
        self.store.check_provenance(&new.created_by)?;
        let target = new
            .link
            .as_deref()
            .map(|path| git_root(&*self.git, path))
            .transpose()?;
        let name = if let Some(name) = new.name {
            let name = name.trim();
            if name.is_empty() {
                return Err(WorkspaceError::InvalidName(name.to_owned()));
            }
            if self.store.workspace_name_taken(name)? {
                return Err(
                    StoreError::Conflict(format!("a workspace is already named {name:?}")).into(),
                );
            }
            name.to_owned()
        } else {
            let base = new
                .fallback_name
                .or_else(|| target.as_deref().map(dir_name))
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "workspace".to_owned());
            unique(base.trim(), " ", |name| {
                self.store.workspace_name_taken(name)
            })?
        };
        let ws = Workspace {
            id: new_id(),
            path: self.new_folder(&name)?,
            name,
            created_by: new.created_by,
            created_at: now_ms(),
        };
        let filled = || -> Result<()> {
            if let Some(target) = &target {
                let entry = entry_name(&ws.path, None, &dir_name(target))?;
                std::os::unix::fs::symlink(target, ws.path.join(entry))?;
            }
            crate::widgets::write_default_manifest(&ws.path)?;
            Ok(self.store.insert_workspace(&ws)?)
        };
        if let Err(error) = filled() {
            // The folder is new and holds at most the link (never followed) and `.omfp/views.json`.
            let _ = std::fs::remove_dir_all(&ws.path);
            return Err(error);
        }
        self.view(ws)
    }

    /// Makes `<root>/<slug of name>` (or `-2`, `-3`, … when taken) and returns its canonical path.
    fn new_folder(&self, name: &str) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.root)?;
        let root = std::fs::canonicalize(&self.root)?;
        let slug = unique(&slugify(name), "-", |slug| {
            Ok::<_, WorkspaceError>(std::fs::symlink_metadata(root.join(slug)).is_ok())
        })?;
        let path = root.join(slug);
        std::fs::create_dir(&path)?;
        Ok(path)
    }

    /// Renames a workspace; the folder stays where it is.
    pub fn rename(&self, id: &str, name: &str) -> Result<WorkspaceView> {
        let name = name.trim();
        if name.is_empty() {
            return Err(WorkspaceError::InvalidName(name.to_owned()));
        }
        self.store.rename_workspace(id, name)?;
        let ws = self.get(id)?;
        self.view(ws)
    }

    /// Whether [`Self::remove`] would go ahead (DMN-028): no thread left in the workspace, none being deleted
    /// (DMN-039), and without `force` no worktree or clone with uncommitted changes.
    pub fn check_removal(&self, id: &str, force: bool) -> Result<()> {
        self.checked_removal(id, force).map(drop)
    }

    /// Holds off the removal of workspace `id` while one of its threads is deleted (DMN-039), until the returned
    /// hold drops. Take it before deleting the thread's records: the removal checks the records first, then the holds.
    #[must_use]
    pub fn hold_removal(&self, id: &str) -> RemovalHold<'_> {
        *self
            .deleting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id.to_owned())
            .or_default() += 1;
        RemovalHold {
            workspaces: self,
            id: id.to_owned(),
        }
    }

    /// The workspace and, when its folder exists, its repositories, once the removal checks passed.
    fn checked_removal(
        &self,
        id: &str,
        force: bool,
    ) -> Result<(Workspace, Option<Vec<Discovered>>)> {
        let ws = self.get(id)?;
        // Records before holds: a deletion holds the workspace before it deletes its thread's records.
        if !self.store.workspace_thread_ids(id)?.is_empty()
            || self
                .deleting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(id)
        {
            return Err(WorkspaceError::HasThreads(ws.id));
        }
        if !std::fs::symlink_metadata(&ws.path).is_ok_and(|meta| meta.is_dir()) {
            return Ok((ws, None));
        }
        let repos = scan(&*self.git, &ws.path)?;
        if !force {
            for repo in &repos {
                if !matches!(repo.kind, RepoKind::Link { .. }) && self.git.is_dirty(&repo.path)? {
                    return Err(WorkspaceError::Dirty(repo.path.display().to_string()));
                }
            }
        }
        Ok((ws, Some(repos)))
    }

    /// Removes a workspace without threads (DMN-028): its repositories are removed as [`Self::remove_repo`] does
    /// (links unlinked, worktrees and clones refused with uncommitted changes unless `force`, see
    /// [`Self::check_removal`]), then its folder is deleted. Nothing outside the folder is deleted: symlinks are
    /// never followed.
    pub fn remove(&self, id: &str, force: bool) -> Result<()> {
        let (ws, repos) = self.checked_removal(id, force)?;
        if let Some(repos) = repos {
            for repo in repos.iter().filter(|repo| !repo.is_root) {
                self.remove_entry(repo, true, false)?;
            }
            std::fs::remove_dir_all(&ws.path)?;
        }
        self.store.delete_workspace(&ws.id)?;
        self.scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&ws.id);
        Ok(())
    }

    /// Scans the folder again (DMN-023); `changed` tells whether the repositories differ from the last scan.
    pub fn rescan(&self, id: &str) -> Result<(WorkspaceView, bool)> {
        let ws = self.get(id)?;
        let repos = if ws.path.is_dir() {
            scan(&*self.git, &ws.path)?
        } else {
            Vec::new()
        };
        let previous = self
            .scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ws.id.clone(), repos.clone());
        let changed = previous.as_ref() != Some(&repos);
        let seen: Vec<PathBuf> = repos.iter().map(seen_path).collect();
        self.store.record_seen_repositories(&seen, now_ms())?;
        Ok((
            WorkspaceView {
                workspace: ws,
                repos,
            },
            changed,
        ))
    }

    /// The repositories found by the last scan; scans when there was none.
    pub fn discovered(&self, id: &str) -> Result<Vec<Discovered>> {
        let cached = self
            .scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned();
        match cached {
            Some(repos) => Ok(repos),
            None => Ok(self.rescan(id)?.0.repos),
        }
    }

    fn view(&self, ws: Workspace) -> Result<WorkspaceView> {
        let id = ws.id.clone();
        let (mut view, _) = self.rescan(&id)?;
        view.workspace = ws;
        Ok(view)
    }

    /// Adds a repository to the workspace folder (DMN-024) and rescans it.
    pub fn add_repo(
        &self,
        id: &str,
        name: Option<&str>,
        source: RepoSource,
    ) -> Result<WorkspaceView> {
        let ws = self.get(id)?;
        match source {
            RepoSource::Link(target) => {
                let target = git_root(&*self.git, &target)?;
                let entry = entry_name(&ws.path, name, &dir_name(&target))?;
                std::os::unix::fs::symlink(&target, ws.path.join(entry))?;
            }
            RepoSource::Worktree {
                repo,
                branch,
                base_ref,
            } => self.add_worktree(&ws, name, &repo, branch, base_ref)?,
            RepoSource::Clone(url) => {
                let entry = entry_name(&ws.path, name, &clone_name(&url))?;
                self.git.clone_repo(&url, &ws.path.join(entry))?;
            }
        }
        Ok(self.rescan(id)?.0)
    }

    fn add_worktree(
        &self,
        ws: &Workspace,
        name: Option<&str>,
        repo: &Path,
        branch: Option<String>,
        base_ref: Option<String>,
    ) -> Result<()> {
        let dir = existing_dir(repo)?;
        let toplevel = self
            .git
            .toplevel(&dir)?
            .ok_or(WorkspaceError::NotARepository(dir))?;
        let toplevel = std::fs::canonicalize(toplevel)?;
        let base_ref = match base_ref {
            Some(base) => base,
            None => self
                .git
                .current_branch(&toplevel)?
                .ok_or_else(|| WorkspaceError::DetachedHead(toplevel.clone()))?,
        };
        let repo_name = dir_name(&toplevel);
        let main = std::fs::canonicalize(self.git.main_root(&toplevel)?)?;
        let lock = self.repo_lock(&main);
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = entry_name(&ws.path, name, &repo_name)?;
        let spec = match branch {
            Some(branch) if self.git.branch_exists(&toplevel, &branch)? => {
                WorktreeBranch::Existing(branch)
            }
            Some(branch) => WorktreeBranch::New {
                name: branch,
                base_ref,
            },
            None => WorktreeBranch::New {
                name: self.default_branch(&toplevel, &ws.name)?,
                base_ref,
            },
        };
        self.git
            .worktree_add(&toplevel, &ws.path.join(entry), &spec)?;
        Ok(())
    }

    /// The first `omfp/<slug>-<n>` (n ≥ 1) that is not a branch of `repo` (DMN-024).
    fn default_branch(&self, repo: &Path, workspace_name: &str) -> Result<String> {
        let slug = slugify(workspace_name);
        for n in 1_u32.. {
            let branch = format!("omfp/{slug}-{n}");
            if !self.git.branch_exists(repo, &branch)? {
                return Ok(branch);
            }
        }
        Err(std::io::Error::other("no free branch name").into())
    }

    /// Removes the repository `name` from the workspace (DMN-025) and rescans it: a link is unlinked (its target
    /// is never touched); a worktree is removed with `git worktree remove`, refused with uncommitted changes unless
    /// `force`, its branch kept unless `delete_branch`; a clone is deleted only when clean or forced.
    pub fn remove_repo(
        &self,
        id: &str,
        name: &str,
        force: bool,
        delete_branch: bool,
    ) -> Result<WorkspaceView> {
        let ws = self.get(id)?;
        let repo = scan(&*self.git, &ws.path)?
            .into_iter()
            .find(|repo| repo.name == name)
            .ok_or_else(|| WorkspaceError::NoSuchRepo {
                workspace: ws.id.clone(),
                name: name.to_owned(),
            })?;
        if repo.is_root {
            return Err(WorkspaceError::IsRoot(repo.name));
        }
        self.remove_entry(&repo, force, delete_branch)?;
        Ok(self.rescan(id)?.0)
    }

    fn remove_entry(&self, repo: &Discovered, force: bool, delete_branch: bool) -> Result<()> {
        match &repo.kind {
            RepoKind::Link { .. } => std::fs::remove_file(&repo.path)?,
            RepoKind::Worktree { main } => {
                let lock = self.repo_lock(main);
                let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                if !force && self.git.is_dirty(&repo.path)? {
                    return Err(WorkspaceError::Dirty(repo.path.display().to_string()));
                }
                self.git.worktree_remove(main, &repo.path, force)?;
                if let (true, Some(branch)) = (delete_branch, &repo.branch) {
                    self.git.delete_branch(main, branch)?;
                }
            }
            RepoKind::Clone => {
                if !force && self.git.is_dirty(&repo.path)? {
                    return Err(WorkspaceError::Dirty(repo.path.display().to_string()));
                }
                std::fs::remove_dir_all(&repo.path)?;
            }
        }
        Ok(())
    }

    /// The discovered repository `name` of the workspace.
    pub fn repo(&self, id: &str, name: &str) -> Result<Discovered> {
        self.discovered(id)?
            .into_iter()
            .find(|repo| repo.name == name)
            .ok_or_else(|| WorkspaceError::NoSuchRepo {
                workspace: id.to_owned(),
                name: name.to_owned(),
            })
    }

    /// Git status of a repository against its upstream, with the upstream's name (DMN-026).
    pub fn status(&self, repo: &Discovered) -> Result<(GitStatus, Option<String>)> {
        let upstream = self.git.upstream(&repo.path)?;
        let status = self.git.status(&repo.path, upstream.as_deref())?;
        Ok((status, upstream))
    }

    /// Changes of a repository against its upstream, uncommitted ones included (DMN-026).
    pub fn diff(&self, repo: &Discovered) -> Result<String> {
        let upstream = self.git.upstream(&repo.path)?;
        Ok(self.git.diff(&repo.path, upstream.as_deref())?)
    }
}

/// The entry name to use in `folder`: `name` if valid and free, else the first free variant of `default`.
fn entry_name(folder: &Path, name: Option<&str>, default: &str) -> Result<String> {
    let free = |entry: &str| std::fs::symlink_metadata(folder.join(entry)).is_err();
    match name {
        Some(name) if !is_entry_name(name) => Err(WorkspaceError::InvalidName(name.to_owned())),
        Some(name) if !free(name) => Err(WorkspaceError::EntryExists(name.to_owned())),
        Some(name) => Ok(name.to_owned()),
        None => {
            let base = if is_entry_name(default) {
                default
            } else {
                "repository"
            };
            unique(base, "-", |entry| Ok::<_, WorkspaceError>(!free(entry)))
        }
    }
}

fn existing_dir(path: &Path) -> Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(path) if path.is_dir() => Ok(path),
        _ => Err(WorkspaceError::NotADirectory(path.to_owned())),
    }
}

/// The canonical root of the git repository at `path`, which must be that root (DMN-024).
fn git_root(git: &dyn Git, path: &Path) -> Result<PathBuf> {
    let path = existing_dir(path)?;
    let toplevel = git
        .toplevel(&path)?
        .ok_or_else(|| WorkspaceError::NotARepository(path.clone()))?;
    if std::fs::canonicalize(toplevel)? != path {
        return Err(WorkspaceError::NotARepositoryRoot(path));
    }
    Ok(path)
}

/// The path a discovered repository is suggested by: a worktree's main repository, else the checkout (DMN-021).
fn seen_path(repo: &Discovered) -> PathBuf {
    match &repo.kind {
        RepoKind::Worktree { main } => main.clone(),
        RepoKind::Link { .. } | RepoKind::Clone => repo.canonical.clone(),
    }
}

fn dir_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "repository".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The directory `git clone` would make for `url`: its last path segment without `.git`.
fn clone_name(url: &str) -> String {
    let last = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default();
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}
