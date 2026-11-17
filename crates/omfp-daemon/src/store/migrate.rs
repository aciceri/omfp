//! Embedded migrations (DMN-010), applied in order using `PRAGMA user_version`, each in its own transaction with
//! foreign keys checked before commit.
//!
//! Migration 3 (workspace folders, D-11) is not only SQL: every existing thread gets a new workspace folder under the
//! workspaces root, holding one symlink per directory of its old workspaces. Old worktrees stay where they are (the
//! symlinks point at them), and the old roots are recorded so the first start removes them from the session
//! (DMN-029).
//!
//! Migration 6 (DMN-020) drops tags and `workspaces.owned`, then moves every workspace whose folder is not under the
//! workspaces root into a new folder `<root>/<slug of its name>` holding one symlink, named after the old folder, to
//! it (whatever it holds, even nothing). The old folder is never touched; ids, names and threads stay, so sessions
//! resume (omp resumes a session whatever the cwd), and the first start syncs the roots (DMN-029).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, Transaction, params};
use tracing::{info, warn};

use super::{Provenance, Result, StoreError, Thread, TitleSource, Workspace, new_id, path_str};
use crate::names::{is_entry_name, slugify, unique};

/// Every migration's SQL in order: the `n`th brings the schema to `user_version` `n`. Migrations 3 and 6 are run by
/// code around their SQL.
const MIGRATIONS: [&str; 13] = [
    include_str!("0001_initial.sql"),
    include_str!("0002_created_client.sql"),
    include_str!("0003_workspace_folders.sql"),
    include_str!("0004_title_source.sql"),
    include_str!("0005_seen_repositories.sql"),
    include_str!("0006_workspaces_under_root.sql"),
    include_str!("0007_widget_state.sql"),
    include_str!("0008_activity.sql"),
    include_str!("0009_activity_mcp.sql"),
    include_str!("0010_prompt_history.sql"),
    include_str!("0011_order.sql"),
    include_str!("0012_thread_forks.sql"),
    include_str!("0013_jobs.sql"),
];

/// Number of migrations: the highest `user_version` this daemon knows.
const LATEST: i64 = 13;

/// Applies pending migrations. Foreign keys are off while migrating (tables are rebuilt) and checked before each
/// commit; the caller turns them on afterwards.
pub(super) fn run(conn: &mut Connection, workspaces_root: &Path) -> Result<()> {
    let found: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if !(0..=LATEST).contains(&found) {
        return Err(StoreError::UnknownSchemaVersion {
            found,
            supported: LATEST,
        });
    }
    conn.pragma_update(None, "foreign_keys", false)?;
    let pending = (1..=LATEST)
        .zip(MIGRATIONS)
        .skip_while(|(version, _)| *version <= found);
    for (version, sql) in pending {
        let tx = conn.transaction()?;
        match version {
            3 => to_workspace_folders(&tx, workspaces_root, sql)?,
            6 => to_workspaces_under_root(&tx, workspaces_root, sql)?,
            _ => tx.execute_batch(sql)?,
        }
        check_foreign_keys(&tx)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

fn check_foreign_keys(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        return Err(StoreError::Migration(format!(
            "foreign key violation in table {table}"
        )));
    }
    Ok(())
}

/// A thread as stored before migration 3, with the directories of its live workspaces in order.
struct OldThread {
    thread: Thread,
    client: Option<String>,
    dirs: Vec<OldDir>,
}

/// A directory of an old workspace and the entry name its link gets.
struct OldDir {
    path: PathBuf,
    name: String,
}

/// Migration 3: `sql` holds the new schema, then the swap to it after the rows written in between.
fn to_workspace_folders(tx: &Transaction<'_>, root: &Path, sql: &str) -> Result<()> {
    let old = old_threads(tx)?;
    let (schema, swap) = sql.split_once("-- part: swap").unwrap_or((sql, ""));
    tx.execute_batch(schema)?;
    if !old.is_empty() {
        std::fs::create_dir_all(root).map_err(|error| io(root, &error))?;
    }
    let mut names: Vec<String> = Vec::new();
    for old in &old {
        let ws = new_folder(tx, root, old, &mut names)?;
        insert_thread_v3(tx, old, &ws.id)?;
        info!(thread = %old.thread.id, workspace = %ws.path.display(), "thread moved to a workspace folder");
    }
    tx.execute_batch(swap)?;
    // The old workspaces were the session's roots: recorded, so the first start removes them (DMN-029).
    let mut stmt =
        tx.prepare("INSERT OR IGNORE INTO thread_roots (thread_id, path) VALUES (?1, ?2)")?;
    for old in &old {
        for dir in &old.dirs {
            stmt.execute(params![old.thread.id, path_str(&dir.path)?])?;
        }
    }
    Ok(())
}

fn io(path: &Path, error: &std::io::Error) -> StoreError {
    StoreError::Migration(format!("{}: {error}", path.display()))
}

fn old_threads(tx: &Transaction<'_>) -> Result<Vec<OldThread>> {
    let threads: Vec<(Thread, Option<String>)> = {
        let mut stmt = tx.prepare(
            "SELECT id, title, session_dir, omp_session_id, approval_mode, model, thinking_level, created_by,
                 created_at, archived_at, created_client
             FROM threads ORDER BY id",
        )?;
        stmt.query_map([], |row| {
            Ok((
                Thread {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    title_source: TitleSource::Set,
                    workspace_id: String::new(),
                    session_dir: PathBuf::from(row.get::<_, String>(2)?),
                    omp_session_id: row.get(3)?,
                    approval_mode: row.get(4)?,
                    model: row.get(5)?,
                    thinking_level: row.get(6)?,
                    created_by: Provenance::parse(&row.get::<_, String>(7)?),
                    created_at: row.get(8)?,
                    archived_at: row.get(9)?,
                },
                row.get(10)?,
            ))
        })?
        .collect::<Result<_, _>>()?
    };
    let mut stmt = tx.prepare(
        "SELECT w.path, w.kind, r.name FROM thread_workspaces tw
         JOIN workspaces w ON w.id = tw.workspace_id
         LEFT JOIN repositories r ON r.id = w.repository_id
         WHERE tw.thread_id = ?1 AND w.removed_at IS NULL
         ORDER BY tw.position",
    )?;
    threads
        .into_iter()
        .map(|(thread, client)| {
            let dirs = stmt
                .query_map([&thread.id], |row| {
                    let path = PathBuf::from(row.get::<_, String>(0)?);
                    let kind: String = row.get(1)?;
                    let repository: Option<String> = row.get(2)?;
                    Ok(old_dir(path, &kind, repository))
                })?
                .collect::<Result<_, _>>()?;
            Ok(OldThread {
                thread,
                client,
                dirs,
            })
        })
        .collect()
}

/// The link name of an old workspace directory: a managed worktree (`worktrees/<repo>/<slug>`) is named after its
/// repository, an adopted directory after itself.
fn old_dir(path: PathBuf, kind: &str, repository: Option<String>) -> OldDir {
    let file_name = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
    };
    let name = match (kind, repository) {
        ("managed_worktree", Some(repository)) => Some(repository),
        ("managed_worktree", None) => path.parent().and_then(file_name),
        _ => None,
    }
    .or_else(|| file_name(&path))
    .unwrap_or_else(|| "repository".to_owned());
    OldDir { path, name }
}

/// Creates the thread's folder (named after the thread, unique on disk and among names) with its links, and inserts
/// the workspace row.
fn new_folder(
    tx: &Transaction<'_>,
    root: &Path,
    old: &OldThread,
    names: &mut Vec<String>,
) -> Result<Workspace> {
    let title = old.thread.title.trim();
    let title = if title.is_empty() { "workspace" } else { title };
    let name = unique(title, " ", |name| {
        Ok::<_, StoreError>(names.iter().any(|n| n == name))
    })?;
    let slug = unique(&slugify(title), "-", |slug| {
        Ok::<_, StoreError>(std::fs::symlink_metadata(root.join(slug)).is_ok())
    })?;
    let path = root.join(slug);
    std::fs::create_dir(&path).map_err(|error| io(&path, &error))?;
    let path = std::fs::canonicalize(&path).map_err(|error| io(&path, &error))?;
    let mut entries: Vec<String> = Vec::new();
    for dir in &old.dirs {
        if !dir.path.is_dir() {
            warn!(thread = %old.thread.id, dir = %dir.path.display(), "old workspace directory is gone; not linked");
            continue;
        }
        let entry = unique(&dir.name, "-", |entry| {
            Ok::<_, StoreError>(entries.iter().any(|e| e == entry))
        })?;
        let link = path.join(&entry);
        std::os::unix::fs::symlink(&dir.path, &link).map_err(|error| io(&link, &error))?;
        entries.push(entry);
    }
    let ws = Workspace {
        id: new_id(),
        name: name.clone(),
        path,
        created_by: old.thread.created_by.clone(),
        created_at: old.thread.created_at,
    };
    tx.execute(
        "INSERT INTO workspaces (id, name, path, owned, created_by, created_client, created_at)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)",
        params![
            ws.id,
            ws.name,
            path_str(&ws.path)?,
            ws.created_by.as_str(),
            old.client,
            ws.created_at
        ],
    )?;
    names.push(name);
    Ok(ws)
}

fn insert_thread_v3(tx: &Transaction<'_>, old: &OldThread, workspace_id: &str) -> Result<()> {
    let thread = &old.thread;
    tx.execute(
        "INSERT INTO threads_v3 (id, title, workspace_id, session_dir, omp_session_id, approval_mode, model,
             thinking_level, created_by, created_client, created_at, archived_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            thread.id,
            thread.title,
            workspace_id,
            path_str(&thread.session_dir)?,
            thread.omp_session_id,
            thread.approval_mode,
            thread.model,
            thread.thinking_level,
            thread.created_by.as_str(),
            old.client,
            thread.created_at,
            thread.archived_at
        ],
    )?;
    Ok(())
}

/// Migration 6: see the module documentation.
fn to_workspaces_under_root(tx: &Transaction<'_>, root: &Path, sql: &str) -> Result<()> {
    tx.execute_batch(sql)?;
    let workspaces: Vec<(String, String, PathBuf)> = {
        let mut stmt = tx.prepare("SELECT id, name, path FROM workspaces ORDER BY id")?;
        stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                PathBuf::from(row.get::<_, String>(2)?),
            ))
        })?
        .collect::<Result<_, _>>()?
    };
    let under = |path: &Path, root: &Path| path != root && path.starts_with(root);
    let canonical_root = std::fs::canonicalize(root).ok();
    let outside: Vec<&(String, String, PathBuf)> = workspaces
        .iter()
        .filter(|(_, _, path)| {
            !canonical_root
                .as_deref()
                .is_some_and(|root| under(path, root))
        })
        .collect();
    if outside.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(root).map_err(|error| io(root, &error))?;
    let root = std::fs::canonicalize(root).map_err(|error| io(root, &error))?;
    let mut paths: Vec<PathBuf> = workspaces.iter().map(|(_, _, path)| path.clone()).collect();
    for (id, name, old) in outside {
        let slug = unique(&slugify(name), "-", |slug| {
            let path = root.join(slug);
            Ok::<_, StoreError>(std::fs::symlink_metadata(&path).is_ok() || paths.contains(&path))
        })?;
        let path = root.join(slug);
        std::fs::create_dir(&path).map_err(|error| io(&path, &error))?;
        let link = path.join(link_name(old));
        std::os::unix::fs::symlink(old, &link).map_err(|error| io(&link, &error))?;
        tx.execute(
            "UPDATE workspaces SET path = ?2 WHERE id = ?1",
            params![id, path_str(&path)?],
        )?;
        info!(workspace = %id, from = %old.display(), to = %path.display(), "workspace moved under the workspaces root");
        paths.push(path);
    }
    Ok(())
}

/// The entry name of the link to an old workspace folder: the folder's name without leading dots (hidden entries are
/// not scanned), else `folder`.
fn link_name(old: &Path) -> String {
    let name = old
        .file_name()
        .map(|name| name.to_string_lossy().trim_start_matches('.').to_owned())
        .unwrap_or_default();
    if is_entry_name(&name) {
        name
    } else {
        "folder".to_owned()
    }
}
