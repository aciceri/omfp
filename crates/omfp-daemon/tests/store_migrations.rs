//! L2: the SQLite store on a real file — migrations, pragmas, constraints.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use omfp_daemon::store::{
    ActivityQuery, NewActivity, Provenance, SeenRepository, Store, StoreError, Thread, TitleSource,
    Workspace, new_id, now_ms,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;

fn raw(path: &Path) -> Connection {
    Connection::open(path).unwrap()
}

fn user_version(path: &Path) -> i64 {
    raw(path)
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

/// The `user_version` of a database with every migration applied.
const LATEST: i64 = 14;

fn tables(path: &Path) -> Vec<String> {
    let conn = raw(path);
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn count(db: &Path, table: &str) -> i64 {
    raw(db)
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

/// The sorted entry names of a directory.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn open(dir: &TempDir) -> Store {
    Store::open(&dir.path().join("omfp.db"), &dir.path().join("workspaces")).unwrap()
}

fn workspace(name: &str) -> Workspace {
    Workspace {
        id: new_id(),
        name: name.into(),
        path: PathBuf::from("/work").join(name),
        created_by: Provenance::User,
        created_at: now_ms(),
    }
}

fn thread(workspace_id: &str) -> Thread {
    Thread {
        id: new_id(),
        title: "t".into(),
        title_source: TitleSource::Placeholder,
        workspace_id: workspace_id.into(),
        session_dir: "/data/threads/x/sessions".into(),
        omp_session_id: None,
        approval_mode: None,
        model: None,
        thinking_level: None,
        created_by: Provenance::User,
        created_at: now_ms(),
        archived_at: None,
    }
}

/// Turns a database back into a version-10 one: no order of the left column, no fork lineage, no declared jobs, no
/// interrupted turns.
const BACK_TO_10: &str = "DROP TABLE thread_forks; DROP TABLE job_runs; DROP TABLE job_state;
    DROP TABLE interrupted_turns;
    ALTER TABLE workspaces DROP COLUMN position; ALTER TABLE threads DROP COLUMN position;";

/// Turns a database back into a version-5 one: no widget state, no activity log, no prompt history, no order, no fork
/// lineage, no declared jobs, no interrupted turns, thread tags and `workspaces.owned` again.
const BACK_TO_5: &str =
    "ALTER TABLE workspaces DROP COLUMN position; ALTER TABLE threads DROP COLUMN position;
    DROP TABLE widget_state; DROP TABLE activity; DROP TABLE prompt_history;
    DROP TABLE thread_forks; DROP TABLE job_runs; DROP TABLE job_state; DROP TABLE interrupted_turns;
    CREATE TABLE thread_tags (
    thread_id TEXT NOT NULL REFERENCES threads (id) ON DELETE CASCADE, tag TEXT NOT NULL,
    PRIMARY KEY (thread_id, tag));
    ALTER TABLE workspaces ADD COLUMN owned INTEGER NOT NULL DEFAULT 1 CHECK (owned IN (0, 1));";

// spec: DMN-010, DMN-012
#[test]
fn opening_an_empty_file_applies_every_migration() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    drop(open(&dir));

    assert_eq!(user_version(&db), LATEST);
    // Only metadata lives in the database: no transcript tables, no repositories of workspaces (DMN-022).
    assert_eq!(
        tables(&db),
        [
            "activity",
            "interrupted_turns",
            "job_runs",
            "job_state",
            "prompt_history",
            "seen_repositories",
            "settings",
            "thread_forks",
            "thread_roots",
            "threads",
            "widget_state",
            "workspaces"
        ]
    );
}

// spec: DMN-010
#[test]
fn reopening_is_idempotent_and_keeps_data() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    let ws = workspace("a");
    let t = thread(&ws.id);
    {
        let store = open(&dir);
        store.insert_workspace(&ws).unwrap();
        store.insert_thread(&t).unwrap();
    }
    let store = open(&dir);
    assert_eq!(user_version(&db), LATEST);
    assert_eq!(store.workspaces().unwrap(), vec![ws]);
    assert_eq!(store.threads().unwrap(), vec![t]);
}

// spec: DMN-010, DMN-073
#[test]
fn migration_4_counts_existing_titles_as_set() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    let ws = workspace("a");
    let t = thread(&ws.id);
    {
        let store = open(&dir);
        store.insert_workspace(&ws).unwrap();
        store.insert_thread(&t).unwrap();
    }
    // Back to a version-3 database: no `title_source` column, the registry of known repositories.
    let conn = raw(&db);
    conn.execute_batch(&format!(
        "{BACK_TO_5} ALTER TABLE threads DROP COLUMN title_source; DROP TABLE seen_repositories; {OLD_REGISTRY}
         PRAGMA user_version = 3;"
    ))
    .unwrap();
    drop(conn);

    let store = open(&dir);
    assert_eq!(user_version(&db), LATEST);
    assert_eq!(
        store.threads().unwrap(),
        vec![Thread {
            title_source: TitleSource::Set,
            ..t
        }]
    );
}

/// The known-repositories table before migration 5.
const OLD_REGISTRY: &str = "CREATE TABLE repositories (
    id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE,
    default_base_ref TEXT NOT NULL, remote_url TEXT, created_at INTEGER NOT NULL);";

// spec: DMN-010, DMN-021
#[test]
fn migration_5_keeps_known_repositories_as_seen_ones_and_drops_the_registry() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    drop(open(&dir));
    let conn = raw(&db);
    conn.execute_batch(&format!(
        "{BACK_TO_5} DROP TABLE seen_repositories; {OLD_REGISTRY}
         INSERT INTO repositories VALUES ('01R1', 'app', '/src/app', 'main', NULL, 7);
         INSERT INTO repositories VALUES ('01R2', 'api', '/src/api', 'main', 'git@x:api.git', 9);
         PRAGMA user_version = 4;"
    ))
    .unwrap();
    drop(conn);

    let store = open(&dir);
    assert_eq!(user_version(&db), LATEST);
    assert!(!tables(&db).contains(&"repositories".to_owned()));
    assert_eq!(
        store.seen_repositories().unwrap(),
        [
            SeenRepository {
                path: "/src/api".into(),
                last_seen_at: 9
            },
            SeenRepository {
                path: "/src/app".into(),
                last_seen_at: 7
            }
        ]
    );
}

// spec: DMN-010
#[test]
fn a_database_from_a_newer_daemon_is_refused() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    raw(&db).pragma_update(None, "user_version", 99).unwrap();

    let error = Store::open(&db, &dir.path().join("workspaces")).unwrap_err();
    assert!(
        matches!(
            error,
            StoreError::UnknownSchemaVersion {
                found: 99,
                supported: LATEST
            }
        ),
        "{error:?}"
    );
}

// spec: DMN-011
#[test]
fn the_database_uses_write_ahead_logging() {
    let dir = TempDir::new().unwrap();
    let _store = open(&dir);

    let mode: String = raw(&dir.path().join("omfp.db"))
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

// spec: DMN-011
#[test]
fn foreign_keys_are_enforced() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);

    let error = store
        .insert_thread(&thread("missing-workspace"))
        .unwrap_err();
    assert!(matches!(error, StoreError::Sqlite(_)), "{error:?}");
    assert_eq!(store.threads().unwrap(), Vec::new());
}

// spec: DMN-011
#[test]
fn deleting_a_thread_cascades_to_its_roots() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    let store = open(&dir);
    let ws = workspace("a");
    store.insert_workspace(&ws).unwrap();
    let t = thread(&ws.id);
    store.insert_thread(&t).unwrap();
    store
        .set_thread_roots(&t.id, &[PathBuf::from("/work/a/app")])
        .unwrap();

    store.delete_thread(&t.id).unwrap();

    assert_eq!(
        store.workspace_thread_ids(&ws.id).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(count(&db, "thread_roots"), 0);
    // The workspace itself is untouched.
    assert_eq!(store.workspace(&ws.id).unwrap(), ws);
}

// spec: DMN-010, DMN-086
#[test]
fn migration_7_adds_an_empty_widget_state_table_and_keeps_the_data() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    let ws = workspace("a");
    {
        let store = open(&dir);
        store.insert_workspace(&ws).unwrap();
    }
    raw(&db)
        .execute_batch(&format!(
            "{BACK_TO_10} DROP TABLE widget_state; DROP TABLE activity; DROP TABLE prompt_history;
             PRAGMA user_version = 6;"
        ))
        .unwrap();

    let store = open(&dir);

    assert_eq!(user_version(&db), LATEST);
    assert_eq!(count(&db, "widget_state"), 0);
    assert_eq!(store.workspaces().unwrap(), vec![ws.clone()]);
    assert_eq!(store.widget_state(&ws.id).unwrap(), BTreeMap::new());
}

// spec: DMN-086, AGT-018
#[test]
fn widget_state_is_kept_per_instance_and_key_null_deletes_and_it_goes_with_its_workspace() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    let (one, two) = (workspace("one"), workspace("two"));
    {
        let store = open(&dir);
        store.insert_workspace(&one).unwrap();
        store.insert_workspace(&two).unwrap();
        store
            .set_widget_state(&one.id, "tests", "job:make", &json!("J1"))
            .unwrap();
        store
            .set_widget_state(&one.id, "tests", "open", &json!({ "rows": [1, 2] }))
            .unwrap();
        store
            .set_widget_state(&one.id, "tests", "open", &json!(false))
            .unwrap();
        store
            .set_widget_state(&one.id, "omfp:chat", "draft", &json!("hi"))
            .unwrap();
        store
            .set_widget_state(&two.id, "tests", "x", &json!(1))
            .unwrap();
    }
    // Reopened, as after a daemon restart.
    let store = open(&dir);
    assert_eq!(
        serde_json::to_value(store.widget_state(&one.id).unwrap()).unwrap(),
        json!({ "omfp:chat": { "draft": "hi" }, "tests": { "job:make": "J1", "open": false } })
    );

    store
        .set_widget_state(&one.id, "omfp:chat", "draft", &Value::Null)
        .unwrap();
    assert_eq!(
        serde_json::to_value(store.widget_state(&one.id).unwrap()).unwrap(),
        json!({ "tests": { "job:make": "J1", "open": false } })
    );
    assert!(
        store
            .set_widget_state("missing", "tests", "x", &json!(1))
            .is_err(),
        "foreign key"
    );

    store.delete_workspace(&one.id).unwrap();
    assert_eq!(store.widget_state(&one.id).unwrap(), BTreeMap::new());
    assert_eq!(
        count(&db, "widget_state"),
        1,
        "the other workspace keeps its state"
    );
}

/// A database at `user_version = 2` (the old model) with its directories on disk.
struct OldModel {
    db: PathBuf,
    root: PathBuf,
    /// Created by the user from the old web UI (client `omfp-ui`); managed worktree, adopted dirs, a removed and a
    /// missing workspace.
    first: Thread,
    /// Created by `first`'s agent, archived, no workspace.
    second: Thread,
    worktree: PathBuf,
    notes: PathBuf,
    removed: PathBuf,
    gone: PathBuf,
    other_shop: PathBuf,
}

fn old_thread(base: &Path, title: &str, created_by: Provenance) -> Thread {
    let id = new_id();
    Thread {
        session_dir: base.join("data/threads").join(&id).join("sessions"),
        id,
        title: title.into(),
        title_source: TitleSource::Set,
        workspace_id: String::new(),
        omp_session_id: None,
        approval_mode: None,
        model: None,
        thinking_level: None,
        created_by,
        created_at: now_ms(),
        archived_at: None,
    }
}

fn insert_old_thread(conn: &Connection, t: &Thread, client: Option<&str>) {
    conn.execute(
        "INSERT INTO threads (id, title, session_dir, omp_session_id, approval_mode, model, thinking_level,
             created_by, created_at, archived_at, created_client)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            t.id,
            t.title,
            t.session_dir.to_str().unwrap(),
            t.omp_session_id,
            t.approval_mode,
            t.model,
            t.thinking_level,
            t.created_by.as_str(),
            t.created_at,
            t.archived_at,
            client
        ],
    )
    .unwrap();
}

/// Inserts an old workspace attached to thread `old.0` at position `old.1`.
fn insert_old_workspace(
    conn: &Connection,
    old: (&str, i64),
    path: &Path,
    kind: &str,
    repo: Option<&str>,
    removed: bool,
) {
    let id = new_id();
    conn.execute(
        "INSERT INTO workspaces (id, path, kind, repository_id, branch, base_ref, created_by, created_at, removed_at,
             created_client)
         VALUES (?1, ?2, ?3, ?4, 'omfp/fix-login-1', 'main', 'user', ?5, ?6, 'omfp-ui')",
        params![id, path.to_str().unwrap(), kind, repo, now_ms(), removed.then_some(7_i64)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO thread_workspaces (thread_id, workspace_id, position) VALUES (?1, ?2, ?3)",
        params![old.0, id, old.1],
    )
    .unwrap();
}

/// Builds the old-model database in `dir`: migrations 1 and 2 only, then old rows and directories.
fn old_model(dir: &TempDir) -> OldModel {
    let base = dir.path().canonicalize().unwrap();
    let worktree = base.join("data/worktrees/shop/fix-login-1");
    let notes = base.join("src/notes");
    let removed = base.join("src/old");
    let other_shop = base.join("other/shop");
    for path in [&worktree, &notes, &removed, &other_shop] {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("README"), path.to_str().unwrap()).unwrap();
    }
    let mut first = old_thread(&base, "Fix login", Provenance::User);
    first.omp_session_id = Some("sess-1".into());
    first.model = Some("fake/scripted".into());
    first.thinking_level = Some("high".into());
    first.approval_mode = Some("always-ask".into());
    let mut second = old_thread(&base, "Fix login", Provenance::Thread(first.id.clone()));
    second.archived_at = Some(42);

    let db = base.join("omfp.db");
    let conn = raw(&db);
    conn.execute_batch(include_str!("../src/store/0001_initial.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../src/store/0002_created_client.sql"))
        .unwrap();
    conn.pragma_update(None, "user_version", 2).unwrap();
    let repo = new_id();
    conn.execute(
        "INSERT INTO repositories (id, name, path, default_base_ref, remote_url, created_at)
         VALUES (?1, 'shop', ?2, 'main', NULL, 1)",
        params![repo, base.join("src/shop").to_str().unwrap()],
    )
    .unwrap();
    insert_old_thread(&conn, &first, Some("omfp-ui"));
    for tag in ["auth", "backend"] {
        conn.execute(
            "INSERT INTO thread_tags (thread_id, tag) VALUES (?1, ?2)",
            params![first.id, tag],
        )
        .unwrap();
    }
    insert_old_thread(&conn, &second, None);
    let gone = base.join("src/gone");
    let old = |position| (first.id.as_str(), position);
    insert_old_workspace(
        &conn,
        old(0),
        &worktree,
        "managed_worktree",
        Some(&repo),
        false,
    );
    insert_old_workspace(&conn, old(1), &notes, "adopted", None, false);
    insert_old_workspace(&conn, old(2), &removed, "adopted", None, true);
    insert_old_workspace(&conn, old(3), &gone, "adopted", None, false);
    insert_old_workspace(&conn, old(4), &other_shop, "adopted", None, false);
    OldModel {
        db,
        root: base.join("workspaces"),
        first,
        second,
        worktree,
        notes,
        removed,
        gone,
        other_shop,
    }
}

/// The new workspaces of the migrated threads and the threads themselves.
fn assert_migrated_records(store: &Store, old: &OldModel) -> (Workspace, Workspace) {
    let workspaces = store.workspaces().unwrap();
    let [one, two] = <[Workspace; 2]>::try_from(workspaces).unwrap();
    assert_eq!(
        one,
        Workspace {
            id: one.id.clone(),
            name: "Fix login".into(),
            path: old.root.join("fix-login"),
            created_by: Provenance::User,
            created_at: old.first.created_at,
        }
    );
    assert_eq!(
        two,
        Workspace {
            id: two.id.clone(),
            name: "Fix login 2".into(),
            path: old.root.join("fix-login-2"),
            created_by: Provenance::Thread(old.first.id.clone()),
            created_at: old.second.created_at,
        }
    );
    let first = Thread {
        workspace_id: one.id.clone(),
        ..old.first.clone()
    };
    let second = Thread {
        workspace_id: two.id.clone(),
        ..old.second.clone()
    };
    assert_eq!(store.threads().unwrap(), vec![first, second]);
    assert_eq!(
        store.created_clients().unwrap(),
        HashMap::from([
            (old.first.id.clone(), "omfp-ui".to_owned()),
            (one.id.clone(), "omfp-ui".to_owned()),
        ])
    );
    let mut roots = vec![
        old.worktree.clone(),
        old.notes.clone(),
        old.gone.clone(),
        old.other_shop.clone(),
    ];
    roots.sort();
    assert_eq!(store.thread_roots(&old.first.id).unwrap(), roots);
    assert_eq!(
        store.thread_roots(&old.second.id).unwrap(),
        Vec::<PathBuf>::new()
    );
    (one, two)
}

fn assert_migrated_folders(old: &OldModel, one: &Workspace, two: &Workspace) {
    assert_eq!(entries(&old.root), ["fix-login", "fix-login-2"]);
    assert_eq!(entries(&one.path), ["notes", "shop", "shop-2"]);
    for (entry, target) in [
        ("shop", &old.worktree),
        ("notes", &old.notes),
        ("shop-2", &old.other_shop),
    ] {
        assert_eq!(std::fs::read_link(one.path.join(entry)).unwrap(), *target);
    }
    assert_eq!(entries(&two.path), Vec::<String>::new());
    // The old directories stay where they were, untouched.
    for path in [&old.worktree, &old.notes, &old.removed, &old.other_shop] {
        assert_eq!(entries(path), ["README"]);
        assert_eq!(
            std::fs::read_to_string(path.join("README")).unwrap(),
            path.to_str().unwrap()
        );
    }
    assert!(!old.gone.exists());
}

// spec: DMN-010
#[test]
fn migration_3_moves_every_thread_into_its_own_workspace_folder() {
    let dir = TempDir::new().unwrap();
    let old = old_model(&dir);

    let store = Store::open(&old.db, &old.root).unwrap();

    assert_eq!(user_version(&old.db), LATEST);
    assert_eq!(
        tables(&old.db),
        [
            "activity",
            "interrupted_turns",
            "job_runs",
            "job_state",
            "prompt_history",
            "seen_repositories",
            "settings",
            "thread_forks",
            "thread_roots",
            "threads",
            "widget_state",
            "workspaces"
        ]
    );
    assert_eq!(
        store.seen_repositories().unwrap(),
        [SeenRepository {
            path: old.db.parent().unwrap().join("src/shop"),
            last_seen_at: 1
        }]
    );
    let (one, two) = assert_migrated_records(&store, &old);
    assert_migrated_folders(&old, &one, &two);

    // Reopening is a no-op: same records, no new folders.
    drop(store);
    let store = Store::open(&old.db, &old.root).unwrap();
    assert_eq!(user_version(&old.db), LATEST);
    assert_eq!(
        assert_migrated_records(&store, &old),
        (one.clone(), two.clone())
    );
    assert_migrated_folders(&old, &one, &two);
}

/// The column names of a table.
fn columns(db: &Path, table: &str) -> Vec<String> {
    let conn = raw(db);
    let mut stmt = conn
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn assert_dropped_tags_and_owned(db: &Path) {
    assert_eq!(user_version(db), LATEST);
    assert!(!tables(db).contains(&"thread_tags".to_owned()));
    assert_eq!(
        columns(db, "workspaces"),
        [
            "id",
            "name",
            "path",
            "created_by",
            "created_client",
            "created_at",
            "position"
        ]
    );
}

// spec: DMN-010, DMN-020
#[test]
fn migration_6_moves_workspaces_outside_the_root_into_new_folders_and_drops_tags() {
    let dir = TempDir::new().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let (db, root) = (base.join("omfp.db"), base.join("workspaces"));
    let store = Store::open(&db, &root).unwrap();
    let at = |name: &str, path: PathBuf| Workspace {
        path,
        ..workspace(name)
    };
    // Already under the root: unchanged.
    let inside = at("Fix login", root.join("fix-login"));
    // Outside: a folder with files, a missing one, a hidden one.
    let shop = at("Shop", base.join("src/shop"));
    let gone = at("Fix login!", base.join("src/gone"));
    let hidden = at("Config", base.join("src/.config"));
    for (path, file) in [
        (&inside.path, "notes.txt"),
        (&shop.path, "README"),
        (&hidden.path, "settings"),
    ] {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join(file), path.to_str().unwrap()).unwrap();
    }
    // A stray folder takes the slug `shop`.
    std::fs::create_dir_all(root.join("shop")).unwrap();
    for ws in [&inside, &shop, &gone, &hidden] {
        store.insert_workspace(ws).unwrap();
    }
    let t = thread(&shop.id);
    store.insert_thread(&t).unwrap();
    let roots = vec![shop.path.join("app")];
    store.set_thread_roots(&t.id, &roots).unwrap();
    drop(store);
    let conn = raw(&db);
    conn.execute_batch(&format!("{BACK_TO_5} PRAGMA user_version = 5;"))
        .unwrap();
    conn.execute("UPDATE workspaces SET owned = 0 WHERE id = ?1", [&shop.id])
        .unwrap();
    conn.execute(
        "INSERT INTO thread_tags (thread_id, tag) VALUES (?1, 'auth')",
        [&t.id],
    )
    .unwrap();
    drop(conn);

    let store = Store::open(&db, &root).unwrap();

    assert_dropped_tags_and_owned(&db);
    let moved = |ws: &Workspace, slug: &str| Workspace {
        path: root.join(slug),
        ..ws.clone()
    };
    let expected = vec![
        inside.clone(),
        moved(&shop, "shop-2"),
        moved(&gone, "fix-login-2"),
        moved(&hidden, "config"),
    ];
    assert_eq!(store.workspaces().unwrap(), expected);
    // Threads keep their workspace (and so their session) and their recorded roots, synced at the first start.
    assert_eq!(store.threads().unwrap(), vec![t.clone()]);
    assert_eq!(store.thread_roots(&t.id).unwrap(), roots);
    let assert_folders = || {
        assert_eq!(
            entries(&root),
            ["config", "fix-login", "fix-login-2", "shop", "shop-2"]
        );
        for (folder, entry, target) in [
            ("shop-2", "shop", &shop.path),
            ("fix-login-2", "gone", &gone.path),
            ("config", "config", &hidden.path),
        ] {
            assert_eq!(entries(&root.join(folder)), [entry]);
            assert_eq!(
                std::fs::read_link(root.join(folder).join(entry)).unwrap(),
                *target
            );
        }
        // The old folders are untouched.
        for (path, file) in [
            (&inside.path, "notes.txt"),
            (&shop.path, "README"),
            (&hidden.path, "settings"),
        ] {
            assert_eq!(entries(path), [file]);
            assert_eq!(
                std::fs::read_to_string(path.join(file)).unwrap(),
                path.to_str().unwrap()
            );
        }
        assert!(!gone.path.exists());
        assert_eq!(entries(&root.join("shop")), Vec::<String>::new());
    };
    assert_folders();

    // Reopening is a no-op.
    drop(store);
    let store = Store::open(&db, &root).unwrap();
    assert_eq!(store.workspaces().unwrap(), expected);
    assert_folders();
}

// spec: DMN-010, DMN-160
#[test]
fn migration_8_adds_an_empty_activity_log_and_keeps_the_data() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    let ws = workspace("a");
    {
        let store = open(&dir);
        store.insert_workspace(&ws).unwrap();
    }
    raw(&db)
        .execute_batch(&format!(
            "{BACK_TO_10} DROP TABLE activity; DROP TABLE prompt_history; PRAGMA user_version = 7;"
        ))
        .unwrap();

    let store = open(&dir);

    assert_eq!(user_version(&db), LATEST);
    assert_eq!(count(&db, "activity"), 0);
    assert_eq!(
        columns(&db, "activity"),
        [
            "seq",
            "at",
            "thread_id",
            "client",
            "method",
            "params",
            "error"
        ]
    );
    assert_eq!(store.workspaces().unwrap(), vec![ws]);
}

// spec: DMN-010, DMN-160, AGT-053
#[test]
fn migration_9_lets_an_activity_entry_have_no_thread_and_keeps_the_log() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    drop(open(&dir));
    raw(&db)
        .execute_batch(&format!(
            "{BACK_TO_10} DROP TABLE activity; DROP TABLE prompt_history;
             CREATE TABLE activity (seq INTEGER PRIMARY KEY, at INTEGER NOT NULL, thread_id TEXT NOT NULL,
                 client TEXT NOT NULL, method TEXT NOT NULL, params TEXT NOT NULL, error TEXT);
             CREATE INDEX activity_by_thread ON activity (thread_id, seq);
             INSERT INTO activity VALUES (41, 7, '01JA', 'omp', 'thread.stop', '{{\"threadId\":\"01JB\"}}', NULL);
             PRAGMA user_version = 8;"
        ))
        .unwrap();

    let store = open(&dir);

    assert_eq!(user_version(&db), LATEST);
    let mcp = store
        .record_activity(
            &NewActivity {
                at: 8,
                thread_id: None,
                client: "claude-code".to_owned(),
                method: "threads.create".to_owned(),
                params: serde_json::json!({}),
                error: None,
            },
            100,
        )
        .unwrap();
    assert_eq!(mcp.seq, 42, "seq keeps increasing after the rebuild");
    let (entries, more) = store
        .activity(&ActivityQuery {
            thread_id: None,
            before: None,
            after: None,
            limit: 10,
        })
        .unwrap();
    assert!(!more);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.seq, entry.entry.thread_id.clone()))
            .collect::<Vec<_>>(),
        [(42, None), (41, Some("01JA".to_owned()))]
    );
    assert_eq!(
        entries[1].entry.params,
        serde_json::json!({ "threadId": "01JB" })
    );
}

// spec: DMN-010, DMN-280
#[test]
fn migration_11_keeps_the_order_the_column_showed() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    // Inserted in creation order; the column showed them by name, ignoring case.
    let zeta = workspace("zeta");
    let alpha = workspace("Alpha");
    let beta = workspace("beta");
    let (first, second) = (thread(&zeta.id), thread(&zeta.id));
    let other = thread(&beta.id);
    {
        let store = open(&dir);
        for ws in [&zeta, &alpha, &beta] {
            store.insert_workspace(ws).unwrap();
        }
        for t in [&first, &second, &other] {
            store.insert_thread(t).unwrap();
        }
    }
    raw(&db)
        .execute_batch(&format!("{BACK_TO_10} PRAGMA user_version = 10;"))
        .unwrap();

    let store = open(&dir);

    assert_eq!(user_version(&db), LATEST);
    let positions = store.positions().unwrap();
    let at = |id: &str| positions[id];
    assert_eq!((at(&alpha.id), at(&beta.id), at(&zeta.id)), (0, 1, 2));
    assert_eq!((at(&first.id), at(&second.id), at(&other.id)), (0, 1, 0));
    // A new workspace and a new thread go last.
    let late = workspace("aaa");
    store.insert_workspace(&late).unwrap();
    let third = thread(&zeta.id);
    store.insert_thread(&third).unwrap();
    let positions = store.positions().unwrap();
    assert_eq!((positions[&late.id], positions[&third.id]), (3, 2));
}

// spec: DMN-010, DMN-370, DMN-371, DMN-372
#[test]
fn migration_14_adds_interrupted_turns_kept_until_taken_and_gone_with_their_thread() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    let ws = workspace("a");
    let (busy, other) = (thread(&ws.id), thread(&ws.id));
    {
        let store = open(&dir);
        store.insert_workspace(&ws).unwrap();
        store.insert_thread(&busy).unwrap();
        store.insert_thread(&other).unwrap();
    }
    raw(&db)
        .execute_batch("DROP TABLE interrupted_turns; PRAGMA user_version = 13;")
        .unwrap();

    let store = open(&dir);

    assert_eq!(user_version(&db), LATEST);
    assert_eq!(store.interrupted_turns().unwrap(), HashMap::new());
    // A thread deleted meanwhile is skipped; marking again takes the later time.
    store
        .mark_interrupted(&[busy.id.clone(), other.id.clone(), new_id()], 10)
        .unwrap();
    store
        .mark_interrupted(std::slice::from_ref(&busy.id), 20)
        .unwrap();
    assert_eq!(
        store.interrupted_turns().unwrap(),
        HashMap::from([(busy.id.clone(), 20), (other.id.clone(), 10)])
    );
    assert_eq!(store.take_interrupted(&busy.id).unwrap(), Some(20));
    assert_eq!(store.take_interrupted(&busy.id).unwrap(), None);
    store.delete_thread(&other.id).unwrap();
    assert_eq!(count(&db, "interrupted_turns"), 0);
}
