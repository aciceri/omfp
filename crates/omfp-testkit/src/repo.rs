use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use tempfile::TempDir;

/// A git repository in a temporary directory, with one initial commit on `main`. Removed when dropped.
#[derive(Debug)]
pub struct TempRepo {
    dir: TempDir,
}

impl TempRepo {
    pub fn new() -> anyhow::Result<Self> {
        let repo = Self {
            dir: tempfile::Builder::new().prefix("omfp-repo-").tempdir()?,
        };
        repo.git(&["init", "--quiet", "--initial-branch=main"])?;
        repo.commit_file("README.md", "test repository\n", "initial commit")?;
        Ok(repo)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Writes `relative_path` (creating parent directories) and commits it.
    pub fn commit_file(
        &self,
        relative_path: &str,
        contents: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let path = self.path().join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, contents)?;
        self.git(&["add", "--", relative_path])?;
        self.git(&["commit", "--quiet", "--message", message])?;
        Ok(())
    }

    /// Runs `git` in the repository with a fixed identity and no user/system config, returning trimmed stdout.
    pub fn git(&self, args: &[&str]) -> anyhow::Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "omfp test")
            .env("GIT_AUTHOR_EMAIL", "test@omfp.invalid")
            .env("GIT_COMMITTER_NAME", "omfp test")
            .env("GIT_COMMITTER_EMAIL", "test@omfp.invalid")
            .output()
            .with_context(|| format!("running git {args:?}"))?;
        if !output.status.success() {
            bail!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_removed_when_dropped() {
        let repo = TempRepo::new().unwrap();
        let path = repo.path().to_path_buf();

        drop(repo);

        assert!(!path.exists());
    }
}
