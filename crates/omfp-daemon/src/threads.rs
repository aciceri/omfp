//! Thread records (02-daemon, "Threads in a workspace"): creation in a workspace with a private session directory,
//! metadata, and the roots last given to the thread's omp session. Process supervision lives elsewhere; this module
//! never touches omp.
//!
//! Calls are blocking; the server layer runs them on blocking tasks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::paths::DataDir;
use crate::store::{
    Provenance, Store, StoreError, Thread, ThreadFork, TitleSource, new_id, now_ms,
};

#[derive(Debug, thiserror::Error)]
pub enum ThreadError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = ThreadError> = std::result::Result<T, E>;

/// Parameters of [`Threads::create`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewThread {
    pub title: String,
    /// [`TitleSource::Set`] for a title given at creation; [`TitleSource::Placeholder`] (with an empty title) when the
    /// title is to come from the conversation (DMN-073).
    pub title_source: TitleSource,
    /// The workspace the thread lives in (ARC-008).
    pub workspace_id: String,
    /// `provider/id`.
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    pub approval_mode: Option<String>,
    pub created_by: Provenance,
}

impl NewThread {
    /// A thread created by the user in `workspace_id` with a set title, no setting.
    #[must_use]
    pub fn new(title: impl Into<String>, workspace_id: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            title_source: TitleSource::Set,
            workspace_id: workspace_id.into(),
            model: None,
            thinking_level: None,
            approval_mode: None,
            created_by: Provenance::User,
        }
    }
}

/// omp `--approval-mode` of a new thread when none is given (DMN-074).
pub const DEFAULT_APPROVAL_MODE: &str = "yolo";

#[derive(Debug)]
pub struct Threads {
    store: Arc<Store>,
    data: DataDir,
}

impl Threads {
    #[must_use]
    pub fn new(store: Arc<Store>, data: DataDir) -> Self {
        Self { store, data }
    }

    /// Creates a thread in a workspace, with its private session directory (ARC-020); without an approval mode it
    /// gets [`DEFAULT_APPROVAL_MODE`] (DMN-074).
    pub fn create(&self, new: NewThread) -> Result<Thread> {
        self.store.check_provenance(&new.created_by)?;
        self.store.workspace(&new.workspace_id)?;
        let id = new_id();
        let session_dir = std::path::absolute(self.data.thread_sessions(&id))?;
        std::fs::create_dir_all(&session_dir)?;
        let thread = Thread {
            id,
            title: new.title,
            title_source: new.title_source,
            workspace_id: new.workspace_id,
            session_dir,
            omp_session_id: None,
            approval_mode: Some(
                new.approval_mode
                    .unwrap_or_else(|| DEFAULT_APPROVAL_MODE.to_owned()),
            ),
            model: new.model,
            thinking_level: new.thinking_level,
            created_by: new.created_by,
            created_at: now_ms(),
            archived_at: None,
        };
        if let Err(error) = self.store.insert_thread(&thread) {
            // Best effort: no orphan directory for a thread that does not exist.
            let _ = std::fs::remove_dir_all(self.data.thread(&thread.id));
            return Err(error.into());
        }
        self.get(&thread.id)
    }

    /// Creates a thread like [`Self::create`] with its lineage (DMN-300): neither exists without the other.
    pub fn create_fork(&self, new: NewThread, fork: &ThreadFork) -> Result<Thread> {
        let thread = self.create(new)?;
        if let Err(error) = self.store.insert_thread_fork(&thread.id, fork) {
            // Best effort, as for a failed insert: no thread without the lineage it was made with.
            let _ = self.delete(&thread.id);
            return Err(error.into());
        }
        Ok(thread)
    }

    pub fn get(&self, id: &str) -> Result<Thread> {
        Ok(self.store.thread(id)?)
    }

    /// Every thread, archived ones included, oldest first.
    pub fn list(&self) -> Result<Vec<Thread>> {
        Ok(self.store.threads()?)
    }

    /// The threads `id` spawned, transitively (AGT-070), oldest first; archived ones included.
    pub fn spawned(&self, id: &str) -> Result<Vec<String>> {
        Ok(self.store.spawned_thread_ids(id)?)
    }

    /// Sets the title (DMN-073); a [`TitleSource::Set`] title is never replaced by a generated one.
    pub fn rename(&self, id: &str, title: &str, source: TitleSource) -> Result<Thread> {
        self.store.set_thread_title(id, title, source)?;
        self.get(id)
    }

    /// Sets the title if it is still a placeholder; returns whether it did (a title set meanwhile wins, DMN-073).
    pub fn replace_placeholder(&self, id: &str, title: &str, source: TitleSource) -> Result<bool> {
        Ok(self.store.replace_placeholder_title(id, title, source)?)
    }

    /// Publishes a generated title unless the starting automatic title changed meanwhile (DMN-073).
    pub(crate) fn replace_automatic(&self, id: &str, expected: &str, title: &str) -> Result<bool> {
        Ok(self.store.replace_automatic_title(id, expected, title)?)
    }

    /// Changes the omp `--approval-mode`; it applies from the next start of the thread's omp (DMN-074).
    pub fn set_approval_mode(&self, id: &str, mode: &str) -> Result<Thread> {
        self.store.set_thread_approval_mode(id, mode)?;
        self.get(id)
    }

    pub fn archive(&self, id: &str) -> Result<Thread> {
        self.store.set_thread_archived_at(id, Some(now_ms()))?;
        self.get(id)
    }

    pub fn unarchive(&self, id: &str) -> Result<Thread> {
        self.store.set_thread_archived_at(id, None)?;
        self.get(id)
    }

    /// Records the omp session the thread is bound to (DMN-032, DMN-072).
    pub fn set_omp_session_id(&self, id: &str, session_id: &str) -> Result<()> {
        Ok(self.store.set_thread_omp_session_id(id, Some(session_id))?)
    }

    pub fn set_model(&self, id: &str, model: Option<&str>) -> Result<Thread> {
        self.store.set_thread_model(id, model)?;
        self.get(id)
    }

    pub fn set_thinking_level(&self, id: &str, level: Option<&str>) -> Result<Thread> {
        self.store.set_thread_thinking_level(id, level)?;
        self.get(id)
    }

    /// The roots last given to the thread's omp session (DMN-029).
    pub fn roots(&self, id: &str) -> Result<Vec<PathBuf>> {
        Ok(self.store.thread_roots(id)?)
    }

    pub fn set_roots(&self, id: &str, roots: &[PathBuf]) -> Result<()> {
        Ok(self.store.set_thread_roots(id, roots)?)
    }

    /// Puts the thread into `workspace_id` (`None`: its own) before its thread `before`, or last (DMN-282); `true`
    /// when it moved to another workspace (DMN-283). The caller restarts its omp then.
    pub fn reorder(
        &self,
        id: &str,
        workspace_id: Option<&str>,
        before: Option<&str>,
    ) -> Result<bool> {
        Ok(self.store.reorder_thread(id, workspace_id, before)?)
    }

    /// Deletes the thread and its directory (sessions, log). Its workspace is never removed implicitly. The caller
    /// makes sure no omp process runs or starts for it; `Supervisor::delete` does (DMN-039).
    pub fn delete(&self, id: &str) -> Result<()> {
        self.store.delete_thread(id)?;
        self.remove_dir(id)
    }

    /// Deletes the thread's records if it still lives in workspace `workspace_id`, keeping its directory for
    /// [`Self::remove_dir`]: once they are gone no omp can start for the thread (DMN-039). `false` (nothing deleted)
    /// when it moved to another workspace meanwhile (DMN-283).
    pub fn delete_record(&self, id: &str, workspace_id: &str) -> Result<bool> {
        Ok(self.store.delete_thread_in(id, workspace_id)?)
    }

    /// Removes the thread's directory (sessions, log), if there is one.
    pub fn remove_dir(&self, id: &str) -> Result<()> {
        match std::fs::remove_dir_all(self.data.thread(id)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }
}

/// The file of omp session `session_id` in a thread's session directory: omp names it `<timestamp>_<id>.jsonl`, so
/// the newest such name when there are several; `None` while omp has not written it (DMN-213).
#[must_use]
pub fn session_file(session_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let suffix = format!("_{session_id}.jsonl");
    std::fs::read_dir(session_dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(&suffix))
        })
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-213
    #[test]
    fn the_session_file_is_the_newest_file_named_after_the_session() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "2026-11-01T10-00-00-000Z_s1.jsonl",
            "2026-11-02T10-00-00-000Z_s1.jsonl",
            "2026-11-03T10-00-00-000Z_s2.jsonl",
            "2026-11-04T10-00-00-000Z_s1.txt",
        ] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        std::fs::create_dir(dir.path().join("2026-11-05T10-00-00-000Z_s1.jsonl")).unwrap();

        assert_eq!(
            session_file(dir.path(), "s1"),
            Some(dir.path().join("2026-11-02T10-00-00-000Z_s1.jsonl"))
        );
        assert_eq!(session_file(dir.path(), "s3"), None);
        assert_eq!(session_file(&dir.path().join("missing"), "s1"), None);
    }
}
