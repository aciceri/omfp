//! The `Git` port (ARC-012) and its implementation over the `git` CLI.
//!
//! Calls are blocking; the server layer runs them on blocking tasks. Serializing operations on one repository
//! (DMN-025) is the caller's job.

mod fake;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub use fake::FakeGit;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("{} is not in a git repository", .0.display())]
    NotARepository(PathBuf),
    #[error("git {command} failed: {stderr}")]
    Failed { command: String, stderr: String },
    #[error("unexpected git output: {0}")]
    Parse(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = GitError> = std::result::Result<T, E>;

/// Branch to check out in a new worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeBranch {
    /// Create `name` from `base_ref`.
    New { name: String, base_ref: String },
    /// Check out an existing local branch.
    Existing(String),
}

/// Git state of a working tree (DMN-026). Before the first commit (DMN-382): the branch to be born, no ahead/behind and
/// every file as added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatus {
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    /// Commits on HEAD not on the base ref; `None` without a base ref or a first commit.
    pub ahead: Option<u64>,
    /// Commits on the base ref not on HEAD; `None` without a base ref or a first commit.
    pub behind: Option<u64>,
    /// Uncommitted changes, untracked files included.
    pub dirty: bool,
    /// Files changed since the merge base with the base ref (or HEAD without one, the empty tree before the first
    /// commit), uncommitted and untracked changes included.
    pub files: Vec<ChangedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// Relative to the working directory the status was taken in.
    pub path: String,
    /// `None` for binary files.
    pub additions: Option<u64>,
    /// `None` for binary files.
    pub deletions: Option<u64>,
}

/// The git operations the daemon needs.
pub trait Git: Send + Sync + std::fmt::Debug {
    /// Top-level directory of the working tree containing `dir`; `None` when `dir` is not inside a git repository.
    fn toplevel(&self, dir: &Path) -> Result<Option<PathBuf>>;

    /// Root of the main working tree of the repository containing `dir`, shared by all its linked worktrees.
    fn main_root(&self, dir: &Path) -> Result<PathBuf>;

    /// Checked-out branch; `None` when HEAD is detached.
    fn current_branch(&self, dir: &Path) -> Result<Option<String>>;

    /// Commit id of HEAD; `None` before the first commit.
    fn head(&self, dir: &Path) -> Result<Option<String>>;

    /// The upstream of the checked-out branch (`origin/main`); `None` without one or on a detached HEAD.
    fn upstream(&self, dir: &Path) -> Result<Option<String>>;

    /// `git clone <url> <dest>` (DMN-024); `dest` must not exist.
    fn clone_repo(&self, url: &str, dest: &Path) -> Result<()>;

    /// Whether the local branch exists.
    fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool>;

    /// `git worktree add` (DMN-024).
    fn worktree_add(&self, repo: &Path, path: &Path, branch: &WorktreeBranch) -> Result<()>;

    /// `git worktree remove`, with `--force` when asked.
    fn worktree_remove(&self, repo: &Path, path: &Path, force: bool) -> Result<()>;

    /// Deletes a local branch, merged or not.
    fn delete_branch(&self, repo: &Path, branch: &str) -> Result<()>;

    /// Whether the working tree has uncommitted changes or untracked files.
    fn is_dirty(&self, dir: &Path) -> Result<bool>;

    fn status(&self, dir: &Path, base_ref: Option<&str>) -> Result<GitStatus>;

    /// `git diff <base_ref>...HEAD` plus uncommitted and untracked changes, as one patch (DMN-026). Without a base
    /// ref, only the uncommitted and untracked changes; before the first commit, every file as added (DMN-382).
    fn diff(&self, dir: &Path, base_ref: Option<&str>) -> Result<String>;

    /// The bytes of the file `path` (relative to `dir`, inside a working tree) at `HEAD` (DMN-089); `None` when
    /// `HEAD` does not have it as a file or there is no `HEAD` yet.
    fn show_head(&self, dir: &Path, path: &str) -> Result<Option<Vec<u8>>>;

    /// The files of the working tree at `dir` (relative to `dir`): tracked ones and untracked ones that are not
    /// ignored, as `git ls-files --cached --others --exclude-standard` lists them (DMN-092). Tracked files deleted from
    /// the working tree may be listed.
    fn list_files(&self, dir: &Path) -> Result<Vec<String>>;
}

/// [`Git`] over the `git` executable. Output is parsed in machine formats, so user configuration (colors, pagers,
/// external diff drivers, path quoting) does not change the results.
#[derive(Debug, Clone)]
pub struct GitCli {
    program: PathBuf,
}

impl Default for GitCli {
    fn default() -> Self {
        Self::new("git")
    }
}

impl GitCli {
    #[must_use]
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    fn command<S: AsRef<OsStr>>(&self, dir: &Path, args: &[S]) -> Command {
        let mut command = Command::new(&self.program);
        command
            .arg("-C")
            .arg(dir)
            .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
            .args(args)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0");
        // On the host when the daemon runs in Flatpak (ENV-064).
        crate::flatpak::on_host(command)
    }

    fn output<S: AsRef<OsStr>>(&self, dir: &Path, args: &[S]) -> Result<Output> {
        Ok(self.command(dir, args).output()?)
    }

    /// Runs git and requires success; returns raw stdout.
    fn run<S: AsRef<OsStr>>(&self, dir: &Path, args: &[S]) -> Result<Vec<u8>> {
        let output = self.output(dir, args)?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(failed(args, &output))
        }
    }

    /// Runs git and requires success; returns stdout as text without the trailing newline.
    fn run_line<S: AsRef<OsStr>>(&self, dir: &Path, args: &[S]) -> Result<String> {
        let stdout = self.run(dir, args)?;
        let text = String::from_utf8_lossy(&stdout);
        Ok(text.strip_suffix('\n').unwrap_or(&text).to_owned())
    }

    fn untracked(&self, dir: &Path) -> Result<Vec<String>> {
        let stdout = self.run(dir, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        Ok(split_nul(&stdout)
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect())
    }

    /// The tree changes are measured from: the merge base with `base_ref`, or HEAD; before the first commit (`born`
    /// false), the empty tree, whatever the base ref (DMN-382).
    fn diff_origin(&self, dir: &Path, base_ref: Option<&str>, born: bool) -> Result<String> {
        if !born {
            // Not a constant: the empty tree's id depends on the repository's hash function.
            return self.run_line(dir, &["hash-object", "-t", "tree", "/dev/null"]);
        }
        match base_ref {
            Some(base) => self.run_line(dir, &["merge-base", base, "HEAD"]),
            None => Ok("HEAD".to_owned()),
        }
    }
}

const DIFF_FLAGS: [&str; 4] = ["--no-ext-diff", "--no-color", "--no-renames", "--relative"];

impl Git for GitCli {
    fn toplevel(&self, dir: &Path) -> Result<Option<PathBuf>> {
        if !dir.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} is not a directory", dir.display()),
            )
            .into());
        }
        let args = ["rev-parse", "--show-toplevel"];
        let output = self.output(dir, &args)?;
        if output.status.success() {
            Ok(Some(path_from_line(&output.stdout)))
        } else if String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
            Ok(None)
        } else {
            Err(failed(&args, &output))
        }
    }

    fn main_root(&self, dir: &Path) -> Result<PathBuf> {
        let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
        let output = self.output(dir, &args)?;
        if !output.status.success() {
            if String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
                return Err(GitError::NotARepository(dir.to_owned()));
            }
            return Err(failed(&args, &output));
        }
        let common_dir = path_from_line(&output.stdout);
        // `<root>/.git` for a normal repository; the repository itself when bare.
        Ok(match common_dir.parent() {
            Some(parent) if common_dir.file_name() == Some(OsStr::new(".git")) => parent.to_owned(),
            _ => common_dir,
        })
    }

    fn current_branch(&self, dir: &Path) -> Result<Option<String>> {
        let args = ["symbolic-ref", "--quiet", "--short", "HEAD"];
        let output = self.output(dir, &args)?;
        match output.status.code() {
            Some(0) => Ok(Some(line(&output.stdout))),
            Some(1) => Ok(None),
            _ => Err(failed(&args, &output)),
        }
    }

    fn head(&self, dir: &Path) -> Result<Option<String>> {
        let args = ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"];
        let output = self.output(dir, &args)?;
        match output.status.code() {
            Some(0) => Ok(Some(line(&output.stdout))),
            Some(1) => Ok(None),
            _ => Err(failed(&args, &output)),
        }
    }

    fn upstream(&self, dir: &Path) -> Result<Option<String>> {
        let args = [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ];
        let output = self.output(dir, &args)?;
        // git exits with 128 both without an upstream and on a detached HEAD.
        Ok(output.status.success().then(|| line(&output.stdout)))
    }

    fn clone_repo(&self, url: &str, dest: &Path) -> Result<()> {
        let parent = dest.parent().unwrap_or(dest);
        let args: [&OsStr; 5] = [
            "clone".as_ref(),
            "--quiet".as_ref(),
            "--".as_ref(),
            url.as_ref(),
            dest.as_os_str(),
        ];
        self.run(parent, &args).map(drop)
    }

    fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool> {
        let reference = format!("refs/heads/{branch}");
        let args = ["show-ref", "--verify", "--quiet", reference.as_str()];
        let output = self.output(repo, &args)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(failed(&args, &output)),
        }
    }

    fn worktree_add(&self, repo: &Path, path: &Path, branch: &WorktreeBranch) -> Result<()> {
        let path = path.as_os_str();
        let args: Vec<&OsStr> = match branch {
            WorktreeBranch::New { name, base_ref } => vec![
                "worktree".as_ref(),
                "add".as_ref(),
                "--quiet".as_ref(),
                "--no-track".as_ref(),
                "-b".as_ref(),
                name.as_ref(),
                path,
                base_ref.as_ref(),
            ],
            WorktreeBranch::Existing(name) => vec![
                "worktree".as_ref(),
                "add".as_ref(),
                "--quiet".as_ref(),
                path,
                name.as_ref(),
            ],
        };
        self.run(repo, &args).map(drop)
    }

    fn worktree_remove(&self, repo: &Path, path: &Path, force: bool) -> Result<()> {
        let mut args: Vec<&OsStr> = vec!["worktree".as_ref(), "remove".as_ref()];
        if force {
            args.push("--force".as_ref());
        }
        args.push(path.as_os_str());
        self.run(repo, &args).map(drop)
    }

    fn delete_branch(&self, repo: &Path, branch: &str) -> Result<()> {
        self.run(repo, &["branch", "--delete", "--force", "--", branch])
            .map(drop)
    }

    fn is_dirty(&self, dir: &Path) -> Result<bool> {
        let stdout = self.run(dir, &["status", "--porcelain", "--untracked-files=all"])?;
        Ok(!stdout.is_empty())
    }

    fn status(&self, dir: &Path, base_ref: Option<&str>) -> Result<GitStatus> {
        let born = self.head(dir)?.is_some();
        let (ahead, behind) = match base_ref {
            Some(base) if born => {
                let range = format!("{base}...HEAD");
                let counts = self.run_line(
                    dir,
                    &["rev-list", "--left-right", "--count", range.as_str()],
                )?;
                let (behind, ahead) = counts
                    .split_once('\t')
                    .and_then(|(behind, ahead)| Some((behind.parse().ok()?, ahead.parse().ok()?)))
                    .ok_or_else(|| GitError::Parse(format!("rev-list --count: {counts:?}")))?;
                (Some(ahead), Some(behind))
            }
            _ => (None, None),
        };
        let origin = self.diff_origin(dir, base_ref, born)?;
        let mut args = vec!["diff"];
        args.extend(DIFF_FLAGS);
        args.extend(["--numstat", "-z", origin.as_str()]);
        let numstat = self.run(dir, &args)?;
        let mut files = split_nul(&numstat)
            .map(parse_numstat)
            .collect::<Result<Vec<_>>>()?;
        for path in self.untracked(dir)? {
            let contents = std::fs::read(dir.join(&path))?;
            let lines = (!contents.contains(&0)).then(|| count_lines(&contents));
            files.push(ChangedFile {
                path,
                additions: lines,
                deletions: lines.map(|_| 0),
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(GitStatus {
            branch: self.current_branch(dir)?,
            ahead,
            behind,
            dirty: self.is_dirty(dir)?,
            files,
        })
    }

    fn diff(&self, dir: &Path, base_ref: Option<&str>) -> Result<String> {
        let born = self.head(dir)?.is_some();
        let origin = self.diff_origin(dir, base_ref, born)?;
        let mut args = vec!["diff"];
        args.extend(DIFF_FLAGS);
        args.push(origin.as_str());
        let mut patch = String::from_utf8_lossy(&self.run(dir, &args)?).into_owned();
        for path in self.untracked(dir)? {
            let args = [
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-color",
                "--",
                "/dev/null",
                path.as_str(),
            ];
            let output = self.output(dir, &args)?;
            // `--no-index` exits with 1 when the files differ.
            if !matches!(output.status.code(), Some(0 | 1)) {
                return Err(failed(&args, &output));
            }
            patch.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        Ok(patch)
    }

    fn show_head(&self, dir: &Path, path: &str) -> Result<Option<Vec<u8>>> {
        // `./` makes the path relative to `dir` rather than to the top of the working tree.
        let object = format!("HEAD:./{path}");
        let args = ["rev-parse", "--verify", "--quiet", object.as_str()];
        let output = self.output(dir, &args)?;
        let id = match output.status.code() {
            Some(0) => line(&output.stdout),
            Some(1) => return Ok(None),
            _ => return Err(failed(&args, &output)),
        };
        if self.run_line(dir, &["cat-file", "-t", id.as_str()])? != "blob" {
            return Ok(None);
        }
        self.run(dir, &["cat-file", "blob", id.as_str()]).map(Some)
    }

    fn list_files(&self, dir: &Path) -> Result<Vec<String>> {
        let stdout = self.run(
            dir,
            &[
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ],
        )?;
        let mut files: Vec<String> = split_nul(&stdout)
            // An untracked nested repository shows as its folder (`name/`).
            .filter(|path| !path.ends_with(b"/"))
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect();
        // A conflicted file is listed once per stage.
        files.sort_unstable();
        files.dedup();
        Ok(files)
    }
}

fn failed<S: AsRef<OsStr>>(args: &[S], output: &Output) -> GitError {
    GitError::Failed {
        command: args
            .iter()
            .map(|arg| arg.as_ref().to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn line(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout.strip_suffix(b"\n").unwrap_or(stdout)).into_owned()
}

fn path_from_line(stdout: &[u8]) -> PathBuf {
    PathBuf::from(OsStr::from_bytes(
        stdout.strip_suffix(b"\n").unwrap_or(stdout),
    ))
}

fn split_nul(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|&byte| byte == 0)
        .filter(|part| !part.is_empty())
}

/// One `--numstat -z` record without renames: `<added>\t<deleted>\t<path>` (`-` counts for binary files).
fn parse_numstat(record: &[u8]) -> Result<ChangedFile> {
    let text = String::from_utf8_lossy(record);
    let mut fields = text.splitn(3, '\t');
    let (Some(additions), Some(deletions), Some(path)) =
        (fields.next(), fields.next(), fields.next())
    else {
        return Err(GitError::Parse(format!("numstat record {text:?}")));
    };
    let count = |field: &str| match field {
        "-" => Ok(None),
        _ => field
            .parse()
            .map(Some)
            .map_err(|_| GitError::Parse(format!("numstat record {text:?}"))),
    };
    Ok(ChangedFile {
        path: path.to_owned(),
        additions: count(additions)?,
        deletions: count(deletions)?,
    })
}

fn count_lines(contents: &[u8]) -> u64 {
    let lines = contents.split_inclusive(|&byte| byte == b'\n').count();
    u64::try_from(lines).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numstat_records_are_parsed() {
        assert_eq!(
            parse_numstat(b"3\t1\tsrc/a b.rs").unwrap(),
            ChangedFile {
                path: "src/a b.rs".into(),
                additions: Some(3),
                deletions: Some(1)
            }
        );
        assert_eq!(
            parse_numstat(b"-\t-\timage.png").unwrap(),
            ChangedFile {
                path: "image.png".into(),
                additions: None,
                deletions: None
            }
        );
        assert!(matches!(parse_numstat(b"garbage"), Err(GitError::Parse(_))));
    }

    #[test]
    fn lines_are_counted_with_or_without_a_final_newline() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2);
    }
}
