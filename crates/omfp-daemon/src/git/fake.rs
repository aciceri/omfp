//! In-memory [`Git`] for component tests (L1). Repositories are real directories registered with
//! [`FakeGit::add_repo`]; branches, worktrees, dirtiness and the files of `HEAD` ([`FakeGit::commit_file`]) are
//! simulated. `worktree_add`/`worktree_remove` create and delete the worktree directory, like git does.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::{Git, GitError, GitStatus, Result, WorktreeBranch};

type Hook = Arc<dyn Fn(&Path) + Send + Sync>;

#[derive(Debug, Default)]
struct FakeRepo {
    head: Option<String>,
    branches: BTreeSet<String>,
    /// The URL a fake clone was made from.
    cloned_from: Option<String>,
    /// Linked worktree path → checked-out branch.
    worktrees: BTreeMap<PathBuf, String>,
    /// The files of `HEAD`, by path relative to the root; shared by the linked worktrees.
    committed: BTreeMap<PathBuf, Vec<u8>>,
    /// Paths relative to the root that `list_files` leaves out, like a `.gitignore` would.
    ignored: BTreeSet<PathBuf>,
}

#[derive(Debug, Default)]
struct State {
    repos: BTreeMap<PathBuf, FakeRepo>,
    dirty: BTreeSet<PathBuf>,
    /// Checkouts whose `status` fails, with git's message.
    failing: BTreeMap<PathBuf, String>,
}

impl State {
    /// The repository root and, for a linked worktree, its path. Symlinks in `dir` are resolved, like git does.
    fn locate(&self, dir: &Path) -> Option<(&PathBuf, Option<&PathBuf>)> {
        let dir = canonical(dir);
        self.repos.iter().find_map(|(root, repo)| {
            if let Some(worktree) = repo.worktrees.keys().find(|wt| dir.starts_with(wt)) {
                Some((root, Some(worktree)))
            } else {
                dir.starts_with(root).then_some((root, None))
            }
        })
    }

    fn repo(&mut self, root: &Path) -> Result<&mut FakeRepo> {
        self.repos
            .get_mut(root)
            .ok_or_else(|| GitError::NotARepository(root.to_owned()))
    }
}

#[derive(Default)]
pub struct FakeGit {
    state: Mutex<State>,
    on_worktree_add: Mutex<Option<Hook>>,
    on_status: Mutex<Option<Hook>>,
}

impl fmt::Debug for FakeGit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeGit")
            .field("state", &*self.state())
            .finish_non_exhaustive()
    }
}

fn failed(command: &str, stderr: impl Into<String>) -> GitError {
    GitError::Failed {
        command: command.to_owned(),
        stderr: stderr.into(),
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

impl FakeGit {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes `root` (an existing directory) a repository with `head` checked out.
    pub fn add_repo(&self, root: &Path, head: &str) {
        self.state().repos.insert(
            canonical(root),
            FakeRepo {
                head: Some(head.to_owned()),
                branches: BTreeSet::from([head.to_owned()]),
                ..FakeRepo::default()
            },
        );
    }

    pub fn add_branch(&self, root: &Path, branch: &str) {
        if let Some(repo) = self.state().repos.get_mut(&canonical(root)) {
            repo.branches.insert(branch.to_owned());
        }
    }

    /// The URL the repository at `root` was cloned from by [`Git::clone_repo`].
    #[must_use]
    pub fn cloned_from(&self, root: &Path) -> Option<String> {
        self.state()
            .repos
            .get(&canonical(root))
            .and_then(|repo| repo.cloned_from.clone())
    }

    pub fn set_dirty(&self, dir: &Path, dirty: bool) {
        let dir = canonical(dir);
        let mut state = self.state();
        if dirty {
            state.dirty.insert(dir);
        } else {
            state.dirty.remove(&dir);
        }
    }

    /// Makes `status` in the checkout `dir` fail with `stderr` as git's message; `None` makes it succeed again.
    pub fn fail_status(&self, dir: &Path, stderr: Option<&str>) {
        let dir = canonical(dir);
        let mut state = self.state();
        match stderr {
            Some(stderr) => state.failing.insert(dir, stderr.to_owned()),
            None => state.failing.remove(&dir),
        };
    }

    /// Records `contents` as the file `path` (relative to `root`) at `HEAD` of the repository at `root`; the working
    /// tree is not touched.
    pub fn commit_file(&self, root: &Path, path: &str, contents: &[u8]) {
        if let Some(repo) = self.state().repos.get_mut(&canonical(root)) {
            repo.committed
                .insert(PathBuf::from(path), contents.to_vec());
        }
    }

    /// Makes `list_files` leave out `path` (relative to `root`; a folder hides everything below it), as an entry of
    /// `.gitignore` would.
    pub fn ignore(&self, root: &Path, path: &str) {
        if let Some(repo) = self.state().repos.get_mut(&canonical(root)) {
            repo.ignored.insert(PathBuf::from(path));
        }
    }

    /// Local branches of the repository at `root`.
    #[must_use]
    pub fn branches(&self, root: &Path) -> Vec<String> {
        self.state()
            .repos
            .get(&canonical(root))
            .map(|repo| repo.branches.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Linked worktrees of the repository at `root`, with their branches.
    #[must_use]
    pub fn worktrees(&self, root: &Path) -> Vec<(PathBuf, String)> {
        self.state()
            .repos
            .get(&canonical(root))
            .map(|repo| {
                repo.worktrees
                    .iter()
                    .map(|(path, branch)| (path.clone(), branch.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Calls `hook` with the repository path at the start of every `worktree_add`, outside the fake's own locks.
    pub fn on_worktree_add(&self, hook: impl Fn(&Path) + Send + Sync + 'static) {
        *self
            .on_worktree_add
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(hook));
    }

    /// Calls `hook` with the checkout at the start of every `status` (one per repository in a full scan of a
    /// workspace, DMN-023), outside the fake's own locks.
    pub fn on_status(&self, hook: impl Fn(&Path) + Send + Sync + 'static) {
        *self
            .on_status
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(hook));
    }
}

impl Git for FakeGit {
    fn toplevel(&self, dir: &Path) -> Result<Option<PathBuf>> {
        Ok(self
            .state()
            .locate(dir)
            .map(|(root, worktree)| worktree.unwrap_or(root).clone()))
    }

    fn main_root(&self, dir: &Path) -> Result<PathBuf> {
        self.state()
            .locate(dir)
            .map(|(root, _)| root.clone())
            .ok_or_else(|| GitError::NotARepository(dir.to_owned()))
    }

    fn current_branch(&self, dir: &Path) -> Result<Option<String>> {
        let state = self.state();
        let (root, worktree) = state
            .locate(dir)
            .ok_or_else(|| GitError::NotARepository(dir.to_owned()))?;
        let repo = &state.repos[root];
        Ok(match worktree {
            Some(worktree) => Some(repo.worktrees[worktree].clone()),
            None => repo.head.clone(),
        })
    }

    fn head(&self, dir: &Path) -> Result<Option<String>> {
        // One fake commit per checkout, stable for a branch name.
        Ok(self.current_branch(dir)?.map(|branch| {
            let sum = branch.bytes().fold(0_u64, |sum, byte| {
                sum.wrapping_mul(31).wrapping_add(u64::from(byte))
            });
            format!("{sum:040x}")
        }))
    }

    fn upstream(&self, dir: &Path) -> Result<Option<String>> {
        self.main_root(dir).map(|_| None)
    }

    fn clone_repo(&self, url: &str, dest: &Path) -> Result<()> {
        if dest.exists() {
            return Err(failed(
                "clone",
                format!("destination path '{}' already exists", dest.display()),
            ));
        }
        std::fs::create_dir_all(dest)?;
        self.add_repo(dest, "main");
        if let Some(repo) = self.state().repos.get_mut(&canonical(dest)) {
            repo.cloned_from = Some(url.to_owned());
        }
        Ok(())
    }

    fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool> {
        Ok(self
            .state()
            .repo(&canonical(repo))?
            .branches
            .contains(branch))
    }

    fn worktree_add(&self, repo: &Path, path: &Path, branch: &WorktreeBranch) -> Result<()> {
        let hook = self
            .on_worktree_add
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(repo);
        }
        let mut state = self.state();
        let fake = state.repo(&canonical(repo))?;
        if path.exists() {
            return Err(failed(
                "worktree add",
                format!("'{}' already exists", path.display()),
            ));
        }
        let name = match branch {
            WorktreeBranch::New { name, base_ref } => {
                if fake.branches.contains(name) {
                    return Err(failed(
                        "worktree add",
                        format!("a branch named '{name}' already exists"),
                    ));
                }
                if !fake.branches.contains(base_ref) {
                    return Err(failed(
                        "worktree add",
                        format!("invalid reference: {base_ref}"),
                    ));
                }
                fake.branches.insert(name.clone());
                name
            }
            WorktreeBranch::Existing(name) => {
                let checked_out = fake.head.as_ref() == Some(name)
                    || fake.worktrees.values().any(|branch| branch == name);
                if !fake.branches.contains(name) || checked_out {
                    return Err(failed("worktree add", format!("cannot check out '{name}'")));
                }
                name
            }
        };
        std::fs::create_dir_all(path)?;
        fake.worktrees.insert(canonical(path), name.clone());
        Ok(())
    }

    fn worktree_remove(&self, repo: &Path, path: &Path, force: bool) -> Result<()> {
        let mut state = self.state();
        let path = canonical(path);
        if state.dirty.contains(&path) && !force {
            return Err(failed(
                "worktree remove",
                "contains modified or untracked files, use --force",
            ));
        }
        if state
            .repo(&canonical(repo))?
            .worktrees
            .remove(&path)
            .is_none()
        {
            return Err(failed(
                "worktree remove",
                format!("'{}' is not a working tree", path.display()),
            ));
        }
        state.dirty.remove(&path);
        std::fs::remove_dir_all(&path)?;
        Ok(())
    }

    fn delete_branch(&self, repo: &Path, branch: &str) -> Result<()> {
        let mut state = self.state();
        let fake = state.repo(&canonical(repo))?;
        let checked_out = fake.head.as_deref() == Some(branch)
            || fake.worktrees.values().any(|name| name == branch);
        if checked_out || !fake.branches.remove(branch) {
            return Err(failed(
                "branch --delete",
                format!("cannot delete branch '{branch}'"),
            ));
        }
        Ok(())
    }

    fn is_dirty(&self, dir: &Path) -> Result<bool> {
        Ok(self.state().dirty.contains(&canonical(dir)))
    }

    fn status(&self, dir: &Path, base_ref: Option<&str>) -> Result<GitStatus> {
        let hook = self
            .on_status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(dir);
        }
        if let Some(stderr) = self.state().failing.get(&canonical(dir)) {
            return Err(failed("diff", stderr.clone()));
        }
        Ok(GitStatus {
            branch: self.current_branch(dir)?,
            ahead: base_ref.map(|_| 0),
            behind: base_ref.map(|_| 0),
            dirty: self.is_dirty(dir)?,
            files: Vec::new(),
        })
    }

    fn diff(&self, dir: &Path, _base_ref: Option<&str>) -> Result<String> {
        self.main_root(dir).map(|_| String::new())
    }

    fn show_head(&self, dir: &Path, path: &str) -> Result<Option<Vec<u8>>> {
        let state = self.state();
        let (root, worktree) = state
            .locate(dir)
            .ok_or_else(|| GitError::NotARepository(dir.to_owned()))?;
        let inside = canonical(dir)
            .strip_prefix(worktree.unwrap_or(root))
            .map(Path::to_owned)
            .unwrap_or_default();
        Ok(state.repos[root].committed.get(&inside.join(path)).cloned())
    }

    fn list_files(&self, dir: &Path) -> Result<Vec<String>> {
        let state = self.state();
        let (root, worktree) = state
            .locate(dir)
            .ok_or_else(|| GitError::NotARepository(dir.to_owned()))?;
        let checkout = worktree.unwrap_or(root).clone();
        let ignored = &state.repos[root].ignored;
        // Other repositories and worktrees inside this checkout are theirs, as git sees them.
        let others: BTreeSet<PathBuf> = state
            .repos
            .iter()
            .flat_map(|(other, repo)| std::iter::once(other).chain(repo.worktrees.keys()))
            .filter(|other| **other != checkout)
            .cloned()
            .collect();
        let start = canonical(dir);
        let mut files = Vec::new();
        let mut pending = vec![start.clone()];
        while let Some(folder) = pending.pop() {
            for entry in std::fs::read_dir(&folder)? {
                let path = entry?.path();
                let inside = path.strip_prefix(&checkout).unwrap_or(&path);
                if path.file_name().is_some_and(|name| name == ".git")
                    || ignored.iter().any(|hidden| inside.starts_with(hidden))
                    || others.contains(&canonical(&path))
                {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                } else if let Ok(relative) = path.strip_prefix(&start) {
                    files.push(relative.to_string_lossy().into_owned());
                }
            }
        }
        files.sort_unstable();
        Ok(files)
    }
}
