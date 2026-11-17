//! SQLite persistence (02-daemon, "Persistence"): workspace folders, threads and their roots, the repositories seen in
//! workspaces (DMN-021), settings, widget state (DMN-086); the activity log and prompt history in `history`, declared
//! jobs' runs in `jobs`.
//!
//! Transcripts are not stored here (DMN-012): omp's session files in each thread's `session_dir` are the source of
//! truth, and the repositories inside a workspace are discovered from its folder (DMN-022). All calls are blocking;
//! the server layer runs them on blocking tasks.

mod history;
mod jobs;
mod migrate;

pub use history::{Activity, ActivityQuery, NewActivity, PromptQuery, SentPrompt};
pub use jobs::{FINISHED_RUNS_KEPT, JobRun, JobState, RunEnd, RunOutput};

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, params};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{entity} {id} not found")]
    NotFound { entity: &'static str, id: String },
    #[error("conflict: {0}")]
    Conflict(String),
    #[error(
        "database schema version {found} is not supported (this daemon knows up to {supported})"
    )]
    UnknownSchemaVersion { found: i64, supported: i64 },
    #[error("path is not valid UTF-8: {}", .0.display())]
    InvalidPath(PathBuf),
    #[error("stored value is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("migration failed: {0}")]
    Migration(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// Who created an entity (ARC-009): the user, the agent of a thread, or an MCP client outside omfp (AGT-051).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    User,
    Thread(String),
    Mcp,
    /// A declared job's prompt run (DMN-334).
    Job(String),
}

/// The stored prefix of a job provenance; thread ids are ULIDs and never start with it.
const JOB_PREFIX: &str = "job:";

impl Provenance {
    /// Stored form: `user`, `mcp`, `job:<name>` or the thread id (a ULID, never one of the others).
    #[must_use]
    pub fn as_str(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::User => "user".into(),
            Self::Mcp => "mcp".into(),
            Self::Thread(id) => id.as_str().into(),
            Self::Job(job) => format!("{JOB_PREFIX}{job}").into(),
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "user" => Self::User,
            "mcp" => Self::Mcp,
            _ => match value.strip_prefix(JOB_PREFIX) {
                Some(job) => Self::Job(job.to_owned()),
                None => Self::Thread(value.to_owned()),
            },
        }
    }
}

/// A repository seen in a workspace (DMN-021).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenRepository {
    /// Canonical path: a link's target or a clone, the main repository of a worktree.
    pub path: PathBuf,
    /// Unix milliseconds.
    pub last_seen_at: i64,
}

/// A workspace folder (DMN-020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    /// Unique.
    pub name: String,
    /// Absolute, canonical path of the folder: one the daemon made under the workspaces root (DMN-020); unique.
    pub path: PathBuf,
    pub created_by: Provenance,
    /// Unix milliseconds.
    pub created_at: i64,
}

/// Where a thread's title comes from (DMN-073).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleSource {
    /// The first words of the first message, or empty before it; replaced by a generated title.
    Placeholder,
    /// Generated from the conversation by the daemon.
    Generated,
    /// Set by the user or an agent; never replaced by a generated title.
    Set,
}

impl TitleSource {
    /// Stored form: `placeholder`, `generated` or `set`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Placeholder => "placeholder",
            Self::Generated => "generated",
            Self::Set => "set",
        }
    }

    /// The stored form back; anything unknown counts as `set`, the source that is never overwritten.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "placeholder" => Self::Placeholder,
            "generated" => Self::Generated,
            _ => Self::Set,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub title_source: TitleSource,
    /// The workspace the thread lives in: its omp cwd (DMN-029).
    pub workspace_id: String,
    pub session_dir: PathBuf,
    pub omp_session_id: Option<String>,
    pub approval_mode: Option<String>,
    /// `provider/id`.
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    pub created_by: Provenance,
    /// Unix milliseconds.
    pub created_at: i64,
    pub archived_at: Option<i64>,
}

/// Where a forked thread comes from (DMN-300): the source thread and the omp session entry the fork ends at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadFork {
    /// The source thread; it may have been deleted since.
    pub source_thread_id: String,
    /// The source's title when it was forked.
    pub source_title: String,
    /// `None`: the whole session was copied.
    pub entry_id: Option<String>,
    /// The first line of the entry's message text.
    pub message: Option<String>,
}

/// A new ULID as a string. IDs are monotonic within the process, so ordering by id is ordering by creation.
#[must_use]
pub fn new_id() -> String {
    static GENERATOR: Mutex<ulid::Generator> = Mutex::new(ulid::Generator::new());
    let mut generator = GENERATOR.lock().unwrap_or_else(PoisonError::into_inner);
    generator
        .generate()
        .unwrap_or_else(|_| ulid::Ulid::generate())
        .to_string()
}

/// The current time in Unix milliseconds.
#[must_use]
pub fn now_ms() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

/// The daemon database. One connection behind a mutex; every method is a short blocking call.
#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

const WORKSPACE_COLUMNS: &str = "id, name, path, created_by, created_at";
const THREAD_COLUMNS: &str = "id, title, workspace_id, session_dir, omp_session_id, approval_mode, model, \
     thinking_level, created_by, created_at, archived_at, title_source";
/// The `settings` key of the user's theme CSS (DMN-075): a JSON string, or `null`.
const THEME_SETTING: &str = "theme";

impl Store {
    /// Opens (creating if needed) the database at `path`, enables WAL and foreign keys (DMN-011) and applies pending
    /// migrations (DMN-010). Migrations 3 and 6 create workspace folders under `workspaces_root`.
    pub fn open(path: &Path, workspaces_root: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
        migrate::run(&mut conn, workspaces_root)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves no open transaction behind (rolled back on drop).
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // Seen repositories

    /// Records that the repositories at `paths` were seen at `at` (DMN-021); one transaction.
    pub fn record_seen_repositories(&self, paths: &[PathBuf], at: i64) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO seen_repositories (path, last_seen_at) VALUES (?1, ?2)
                 ON CONFLICT (path) DO UPDATE SET last_seen_at = max(last_seen_at, excluded.last_seen_at)",
            )?;
            for path in paths {
                stmt.execute(params![path_str(path)?, at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every seen repository, most recently seen first, then by path.
    pub fn seen_repositories(&self) -> Result<Vec<SeenRepository>> {
        query_all(
            &self.conn(),
            "SELECT path, last_seen_at FROM seen_repositories ORDER BY last_seen_at DESC, path",
            [],
            |row| {
                Ok(SeenRepository {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    last_seen_at: row.get(1)?,
                })
            },
        )
    }

    // Workspaces

    /// Inserts a workspace last among the workspaces (DMN-280).
    pub fn insert_workspace(&self, ws: &Workspace) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO workspaces (id, name, path, created_by, created_at, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, (SELECT coalesce(max(position) + 1, 0) FROM workspaces))",
                params![
                    ws.id,
                    ws.name,
                    path_str(&ws.path)?,
                    ws.created_by.as_str(),
                    ws.created_at
                ],
            )
            .map_err(|error| {
                unique_conflict(
                    error,
                    &format!(
                        "a workspace is already named {:?} or at {}",
                        ws.name,
                        ws.path.display()
                    ),
                )
            })?;
        Ok(())
    }

    pub fn workspace(&self, id: &str) -> Result<Workspace> {
        self.conn()
            .query_row(
                &format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"),
                [id],
                workspace_from_row,
            )
            .optional()?
            .ok_or_else(|| not_found("workspace", id))
    }

    pub fn workspace_by_path(&self, path: &Path) -> Result<Option<Workspace>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE path = ?1"),
                [path_str(path)?],
                workspace_from_row,
            )
            .optional()?)
    }

    /// Whether a workspace already has this name.
    pub fn workspace_name_taken(&self, name: &str) -> Result<bool> {
        exists(&self.conn(), "workspaces", "name", name)
    }

    /// Every workspace, oldest first.
    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        query_all(
            &self.conn(),
            &format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces ORDER BY id"),
            [],
            workspace_from_row,
        )
    }

    pub fn rename_workspace(&self, id: &str, name: &str) -> Result<()> {
        let updated = self
            .conn()
            .execute(
                "UPDATE workspaces SET name = ?2 WHERE id = ?1",
                params![id, name],
            )
            .map_err(|error| {
                unique_conflict(error, &format!("a workspace is already named {name:?}"))
            })?;
        found(updated, "workspace", id)
    }

    /// Deletes a workspace record; refused while threads live in it (DMN-028).
    pub fn delete_workspace(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let threads: i64 = tx.query_row(
            "SELECT count(*) FROM threads WHERE workspace_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        if threads > 0 {
            return Err(StoreError::Conflict(format!(
                "workspace {id} still has {threads} thread(s)"
            )));
        }
        found(
            tx.execute("DELETE FROM workspaces WHERE id = ?1", [id])?,
            "workspace",
            id,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Ids of the threads living in a workspace (DMN-027), oldest first, archived ones included.
    pub fn workspace_thread_ids(&self, workspace_id: &str) -> Result<Vec<String>> {
        query_all(
            &self.conn(),
            "SELECT id FROM threads WHERE workspace_id = ?1 ORDER BY id",
            [workspace_id],
            |row| row.get(0),
        )
    }

    // Order of the left column

    /// The place of every workspace among the workspaces and of every thread among the threads of its workspace,
    /// archived ones included, from 0 (DMN-284).
    pub fn positions(&self) -> Result<HashMap<String, u32>> {
        query_all(
            &self.conn(),
            "SELECT id, row_number() OVER (ORDER BY position, id) - 1 FROM workspaces
             UNION ALL
             SELECT id, row_number() OVER (PARTITION BY workspace_id ORDER BY position, id) - 1 FROM threads",
            [],
            |row| {
                let position: i64 = row.get(1)?;
                Ok((row.get(0)?, u32::try_from(position).unwrap_or(u32::MAX)))
            },
        )
    }

    /// Puts workspace `id` just before workspace `before`, or last (DMN-281), in one transaction, so reorders from
    /// several windows apply one after the other (DMN-285). A `before` that is no longer a workspace is a conflict;
    /// nothing changes then.
    pub fn reorder_workspace(&self, id: &str, before: Option<&str>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let order: Vec<String> = query_all(
            &tx,
            "SELECT id FROM workspaces ORDER BY position, id",
            [],
            |row| row.get(0),
        )?;
        if !order.iter().any(|other| other == id) {
            return Err(not_found("workspace", id));
        }
        let order = placed(order, id, before).ok_or_else(|| {
            StoreError::Conflict(format!("workspace {} is gone", before.unwrap_or_default()))
        })?;
        write_positions(&tx, "workspaces", &order)?;
        tx.commit()?;
        Ok(())
    }

    /// Puts thread `id` into workspace `workspace_id` (`None`: the one it is in now) just before its thread `before`,
    /// or last (DMN-282, DMN-283), in one transaction (DMN-285); returns whether the thread changed workspace. A
    /// `before` that is not a thread of that workspace (any more) is a conflict; nothing changes then.
    pub fn reorder_thread(
        &self,
        id: &str,
        workspace_id: Option<&str>,
        before: Option<&str>,
    ) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let current: String = tx
            .query_row(
                "SELECT workspace_id FROM threads WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| not_found("thread", id))?;
        let workspace_id = workspace_id.unwrap_or(&current);
        ensure(&tx, "workspaces", "workspace", workspace_id)?;
        let order = query_all(
            &tx,
            "SELECT id FROM threads WHERE workspace_id = ?1 ORDER BY position, id",
            [workspace_id],
            |row| row.get(0),
        )?;
        let order = placed(order, id, before).ok_or_else(|| {
            StoreError::Conflict(format!(
                "thread {} is not in workspace {workspace_id}",
                before.unwrap_or_default()
            ))
        })?;
        let moved = current != workspace_id;
        if moved {
            tx.execute(
                "UPDATE threads SET workspace_id = ?2 WHERE id = ?1",
                params![id, workspace_id],
            )?;
        }
        write_positions(&tx, "threads", &order)?;
        tx.commit()?;
        Ok(moved)
    }

    // Threads

    /// Inserts a thread last among the threads of its workspace (DMN-280).
    pub fn insert_thread(&self, thread: &Thread) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO threads (id, title, workspace_id, session_dir, omp_session_id, approval_mode, model,
                     thinking_level, created_by, created_at, archived_at, title_source, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     (SELECT coalesce(max(position) + 1, 0) FROM threads WHERE workspace_id = ?3))",
                params![
                    thread.id,
                    thread.title,
                    thread.workspace_id,
                    path_str(&thread.session_dir)?,
                    thread.omp_session_id,
                    thread.approval_mode,
                    thread.model,
                    thread.thinking_level,
                    thread.created_by.as_str(),
                    thread.created_at,
                    thread.archived_at,
                    thread.title_source.as_str()
                ],
            )
            .map_err(|error| unique_conflict(error, &format!("thread {} exists", thread.id)))?;
        Ok(())
    }

    pub fn thread(&self, id: &str) -> Result<Thread> {
        self.conn()
            .query_row(
                &format!("SELECT {THREAD_COLUMNS} FROM threads WHERE id = ?1"),
                [id],
                thread_from_row,
            )
            .optional()?
            .ok_or_else(|| not_found("thread", id))
    }

    /// Every thread, archived ones included, oldest first.
    pub fn threads(&self) -> Result<Vec<Thread>> {
        query_all(
            &self.conn(),
            &format!("SELECT {THREAD_COLUMNS} FROM threads ORDER BY id"),
            [],
            thread_from_row,
        )
    }

    pub fn set_thread_title(&self, id: &str, title: &str, source: TitleSource) -> Result<()> {
        let updated = self.conn().execute(
            "UPDATE threads SET title = ?2, title_source = ?3 WHERE id = ?1",
            params![id, title, source.as_str()],
        )?;
        found(updated, "thread", id)
    }

    /// Sets the title only while the thread's title is a placeholder; returns whether it did. A title set meanwhile
    /// is never replaced (DMN-073).
    pub fn replace_placeholder_title(
        &self,
        id: &str,
        title: &str,
        source: TitleSource,
    ) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE threads SET title = ?2, title_source = ?3 WHERE id = ?1 AND title_source = 'placeholder'",
            params![id, title, source.as_str()],
        )?;
        Ok(changed > 0)
    }

    /// Replaces an automatic title only if it still matches the generation's starting title.
    /// The source check is atomic with the write: an explicit rename always wins (DMN-073).
    pub(crate) fn replace_automatic_title(
        &self,
        id: &str,
        expected: &str,
        title: &str,
    ) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE threads SET title = ?3, title_source = 'generated'
             WHERE id = ?1 AND title = ?2 AND title_source != 'set'",
            params![id, expected, title],
        )?;
        Ok(changed > 0)
    }

    /// The user's theme CSS (DMN-075); `None` when unset or `null`.
    pub fn theme(&self) -> Result<Option<String>> {
        Ok(self
            .setting(THEME_SETTING)?
            .and_then(|value| value.as_str().map(str::to_owned)))
    }

    pub fn set_theme(&self, css: Option<&str>) -> Result<()> {
        self.set_setting(THEME_SETTING, &serde_json::json!(css))
    }

    pub fn set_thread_archived_at(&self, id: &str, archived_at: Option<i64>) -> Result<()> {
        self.set_thread_column(id, "archived_at", archived_at)
    }

    pub fn set_thread_omp_session_id(&self, id: &str, session_id: Option<&str>) -> Result<()> {
        self.set_thread_column(id, "omp_session_id", session_id)
    }

    pub fn set_thread_model(&self, id: &str, model: Option<&str>) -> Result<()> {
        self.set_thread_column(id, "model", model)
    }

    pub fn set_thread_thinking_level(&self, id: &str, level: Option<&str>) -> Result<()> {
        self.set_thread_column(id, "thinking_level", level)
    }

    pub fn set_thread_approval_mode(&self, id: &str, mode: &str) -> Result<()> {
        self.set_thread_column(id, "approval_mode", mode)
    }

    fn set_thread_column(&self, id: &str, column: &str, value: impl rusqlite::ToSql) -> Result<()> {
        let sql = format!("UPDATE threads SET {column} = ?2 WHERE id = ?1");
        found(self.conn().execute(&sql, params![id, value])?, "thread", id)
    }

    /// The `--add-dir` roots last given to the thread's omp session (DMN-029), sorted.
    pub fn thread_roots(&self, id: &str) -> Result<Vec<PathBuf>> {
        query_all(
            &self.conn(),
            "SELECT path FROM thread_roots WHERE thread_id = ?1 ORDER BY path",
            [id],
            |row| Ok(PathBuf::from(row.get::<_, String>(0)?)),
        )
    }

    /// Replaces the thread's recorded roots.
    pub fn set_thread_roots(&self, id: &str, roots: &[PathBuf]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        ensure(&tx, "threads", "thread", id)?;
        tx.execute("DELETE FROM thread_roots WHERE thread_id = ?1", [id])?;
        {
            let mut stmt =
                tx.prepare("INSERT OR IGNORE INTO thread_roots (thread_id, path) VALUES (?1, ?2)")?;
            for root in roots {
                stmt.execute(params![id, path_str(root)?])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes a thread with its roots; its workspace stays.
    pub fn delete_thread(&self, id: &str) -> Result<()> {
        let deleted = self
            .conn()
            .execute("DELETE FROM threads WHERE id = ?1", [id])?;
        found(deleted, "thread", id)
    }

    /// Deletes a thread with its roots if it still lives in workspace `workspace_id`: `false` (nothing deleted) when
    /// it moved to another one meanwhile (DMN-039); its workspace stays.
    pub fn delete_thread_in(&self, id: &str, workspace_id: &str) -> Result<bool> {
        let conn = self.conn();
        if conn.execute(
            "DELETE FROM threads WHERE id = ?1 AND workspace_id = ?2",
            [id, workspace_id],
        )? == 1
        {
            return Ok(true);
        }
        ensure(&conn, "threads", "thread", id)?;
        Ok(false)
    }

    /// Checks that a provenance names an existing thread, if it names one (ARC-009).
    pub fn check_provenance(&self, created_by: &Provenance) -> Result<()> {
        match created_by {
            Provenance::User | Provenance::Mcp | Provenance::Job(_) => Ok(()),
            Provenance::Thread(id) => ensure(&self.conn(), "threads", "thread", id),
        }
    }

    /// Records the `client` of the connection that created thread or workspace `id` (PRT-033).
    pub fn set_created_client(&self, id: &str, client: &str) -> Result<()> {
        let conn = self.conn();
        let updated = conn.execute(
            "UPDATE threads SET created_client = ?2 WHERE id = ?1",
            params![id, client],
        )? + conn.execute(
            "UPDATE workspaces SET created_client = ?2 WHERE id = ?1",
            params![id, client],
        )?;
        found(updated, "thread or workspace", id)
    }

    /// The recorded creating clients of threads and workspaces, by id.
    pub fn created_clients(&self) -> Result<HashMap<String, String>> {
        query_all(
            &self.conn(),
            "SELECT id, created_client FROM threads WHERE created_client IS NOT NULL
             UNION ALL
             SELECT id, created_client FROM workspaces WHERE created_client IS NOT NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    /// Records that thread `id` was forked (DMN-300); the row goes with the thread.
    pub fn insert_thread_fork(&self, id: &str, fork: &ThreadFork) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO thread_forks (thread_id, source_thread_id, source_title, entry_id, message)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    fork.source_thread_id,
                    fork.source_title,
                    fork.entry_id,
                    fork.message
                ],
            )
            .map_err(|error| unique_conflict(error, &format!("thread {id} is a fork already")))?;
        Ok(())
    }

    /// The lineage of every forked thread, by thread id (DMN-300).
    pub fn thread_forks(&self) -> Result<HashMap<String, ThreadFork>> {
        query_all(
            &self.conn(),
            "SELECT thread_id, source_thread_id, source_title, entry_id, message FROM thread_forks",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    ThreadFork {
                        source_thread_id: row.get(1)?,
                        source_title: row.get(2)?,
                        entry_id: row.get(3)?,
                        message: row.get(4)?,
                    },
                ))
            },
        )
    }

    // Widget state

    /// Every stored widget value of a workspace (DMN-086): instance → key → value.
    pub fn widget_state(
        &self,
        workspace_id: &str,
    ) -> Result<BTreeMap<String, BTreeMap<String, serde_json::Value>>> {
        let rows: Vec<(String, String, String)> = query_all(
            &self.conn(),
            "SELECT instance, key, value FROM widget_state WHERE workspace_id = ?1 ORDER BY instance, key",
            [workspace_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let mut state: BTreeMap<String, BTreeMap<String, serde_json::Value>> = BTreeMap::new();
        for (instance, key, value) in rows {
            state
                .entry(instance)
                .or_default()
                .insert(key, serde_json::from_str(&value)?);
        }
        Ok(state)
    }

    /// Stores one key of a widget instance's state; `null` deletes it (DMN-086).
    pub fn set_widget_state(
        &self,
        workspace_id: &str,
        instance: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<()> {
        let conn = self.conn();
        if value.is_null() {
            conn.execute(
                "DELETE FROM widget_state WHERE workspace_id = ?1 AND instance = ?2 AND key = ?3",
                params![workspace_id, instance, key],
            )?;
        } else {
            conn.execute(
                "INSERT INTO widget_state (workspace_id, instance, key, value) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (workspace_id, instance, key) DO UPDATE SET value = excluded.value",
                params![workspace_id, instance, key, value.to_string()],
            )?;
        }
        Ok(())
    }

    // Spawned threads (M10)

    /// The threads `id` spawned, transitively (AGT-070): those whose provenance names it or one of them, archived
    /// ones included, oldest first.
    pub fn spawned_thread_ids(&self, id: &str) -> Result<Vec<String>> {
        query_all(
            &self.conn(),
            "WITH RECURSIVE spawned (id) AS (
                 SELECT id FROM threads WHERE created_by = ?1
                 UNION
                 SELECT threads.id FROM threads JOIN spawned ON threads.created_by = spawned.id
             )
             SELECT id FROM spawned ORDER BY id",
            [id],
            |row| row.get(0),
        )
    }

    // Settings

    pub fn setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let value: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(value
            .map(|value| serde_json::from_str(&value))
            .transpose()?)
    }

    pub fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value.to_string()],
        )?;
        Ok(())
    }
}

fn not_found(entity: &'static str, id: &str) -> StoreError {
    StoreError::NotFound {
        entity,
        id: id.to_owned(),
    }
}

/// Maps unique/primary-key violations to [`StoreError::Conflict`]; other errors pass through.
fn unique_conflict(error: rusqlite::Error, message: &str) -> StoreError {
    match &error {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                || failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY =>
        {
            StoreError::Conflict(message.to_owned())
        }
        _ => StoreError::Sqlite(error),
    }
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| StoreError::InvalidPath(path.to_owned()))
}

/// `Ok` when `changed` rows were touched, else entity `id` was not found.
fn found(changed: usize, entity: &'static str, id: &str) -> Result<()> {
    if changed == 0 {
        return Err(not_found(entity, id));
    }
    Ok(())
}

/// Whether `table` has a row whose `column` is `value`.
fn exists(conn: &Connection, table: &str, column: &str, value: &str) -> Result<bool> {
    let sql = format!("SELECT 1 FROM {table} WHERE {column} = ?1");
    Ok(conn
        .query_row(&sql, [value], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Fails unless `table` has the row of `entity` `id`.
fn ensure(conn: &Connection, table: &str, entity: &'static str, id: &str) -> Result<()> {
    exists(conn, table, "id", id)?
        .then_some(())
        .ok_or_else(|| not_found(entity, id))
}

/// Every row `sql` selects, mapped by `row`.
fn query_all<T, C: FromIterator<T>>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    row: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<C> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params, row)?
        .collect::<rusqlite::Result<C>>()?;
    Ok(rows)
}

/// The first `limit` of `rows`, read with one more, and whether more follow.
fn page<T>(mut rows: Vec<T>, limit: u32) -> (Vec<T>, bool) {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let more = rows.len() > limit;
    rows.truncate(limit);
    (rows, more)
}

/// `order` with `id` taken out and put back just before `before`, or last; `None` when `before` is not in it.
fn placed(mut order: Vec<String>, id: &str, before: Option<&str>) -> Option<Vec<String>> {
    order.retain(|other| other != id);
    let at = match before {
        Some(before) => order.iter().position(|other| other == before)?,
        None => order.len(),
    };
    order.insert(at, id.to_owned());
    Some(order)
}

/// Numbers the rows of `table` listed in `order` from 0 (DMN-281).
fn write_positions(conn: &Connection, table: &str, order: &[String]) -> Result<()> {
    let mut stmt = conn.prepare(&format!("UPDATE {table} SET position = ?2 WHERE id = ?1"))?;
    for (position, id) in order.iter().enumerate() {
        stmt.execute(params![id, i64::try_from(position).unwrap_or(i64::MAX)])?;
    }
    Ok(())
}

fn workspace_from_row(row: &Row<'_>) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: row.get(0)?,
        name: row.get(1)?,
        path: PathBuf::from(row.get::<_, String>(2)?),
        created_by: Provenance::parse(&row.get::<_, String>(3)?),
        created_at: row.get(4)?,
    })
}

fn thread_from_row(row: &Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: row.get(0)?,
        title: row.get(1)?,
        workspace_id: row.get(2)?,
        session_dir: PathBuf::from(row.get::<_, String>(3)?),
        omp_session_id: row.get(4)?,
        approval_mode: row.get(5)?,
        model: row.get(6)?,
        thinking_level: row.get(7)?,
        created_by: Provenance::parse(&row.get::<_, String>(8)?),
        created_at: row.get(9)?,
        archived_at: row.get(10)?,
        title_source: TitleSource::parse(&row.get::<_, String>(11)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: ARC-009, AGT-051
    #[test]
    fn provenance_is_stored_as_user_mcp_or_a_thread_id() {
        assert_eq!(Provenance::User.as_str(), "user");
        assert_eq!(Provenance::Mcp.as_str(), "mcp");
        assert_eq!(Provenance::Thread("01J".into()).as_str(), "01J");
        assert_eq!(Provenance::parse("user"), Provenance::User);
        assert_eq!(Provenance::parse("mcp"), Provenance::Mcp);
        assert_eq!(Provenance::parse("01J"), Provenance::Thread("01J".into()));
    }

    // spec: PRT-209, DMN-334
    #[test]
    fn a_job_provenance_is_stored_as_job_and_its_name() {
        let job = Provenance::Job("nightly".into());
        assert_eq!(job.as_str(), "job:nightly");
        assert_eq!(Provenance::parse("job:nightly"), job);
        assert_eq!(
            Provenance::parse("01JABCDEF"),
            Provenance::Thread("01JABCDEF".into())
        );
    }

    fn sent(text: &str, thread_id: &str, workspace_id: &str, at: i64) -> SentPrompt {
        SentPrompt {
            text: text.into(),
            thread_id: thread_id.into(),
            workspace_id: workspace_id.into(),
            at,
        }
    }

    fn ranked(store: &Store, thread_id: Option<&str>, workspace_id: Option<&str>) -> Vec<String> {
        store
            .prompts(&PromptQuery {
                thread_id: thread_id.map(str::to_owned),
                workspace_id: workspace_id.map(str::to_owned),
                query: None,
                limit: 100,
            })
            .unwrap()
            .into_iter()
            .map(|prompt| prompt.text)
            .collect()
    }

    // spec: DMN-216, DMN-217
    #[test]
    fn prompts_are_kept_once_per_text_bounded_and_ranked_thread_then_workspace_then_newest() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("omfp.db"), &dir.path().join("ws")).unwrap();
        store.record_prompt(&sent("a1", "t1", "w1", 1), 3).unwrap();
        store.record_prompt(&sent("b", "t2", "w1", 2), 3).unwrap();
        store.record_prompt(&sent("c", "t3", "w2", 3), 3).unwrap();
        // The same text again: one row, with the latest submission.
        store.record_prompt(&sent("a1", "t1", "w1", 4), 3).unwrap();
        assert_eq!(ranked(&store, None, None), ["a1", "c", "b"]);
        assert_eq!(ranked(&store, Some("t2"), Some("w2")), ["b", "c", "a1"]);
        assert_eq!(ranked(&store, Some("t9"), Some("w1")), ["a1", "b", "c"]);
        // A fourth text drops the oldest.
        store.record_prompt(&sent("d", "t3", "w2", 5), 3).unwrap();
        assert_eq!(ranked(&store, None, None), ["d", "a1", "c"]);
        let only = store
            .prompts(&PromptQuery {
                thread_id: None,
                workspace_id: None,
                query: Some("A".into()),
                limit: 1,
            })
            .unwrap();
        assert_eq!(only, [sent("a1", "t1", "w1", 4)]);
    }
}
