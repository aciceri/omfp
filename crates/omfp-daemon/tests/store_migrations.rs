//! L2: the SQLite store on a real file — migrations, pragmas, constraints.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use omfp_daemon::store::{Interruption, Store, StoreError, Thread, Workspace, new_id, now_ms};
use omfp_protocol::{CreatedBy, TitleSource};
use rusqlite::Connection;
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
const LATEST: i64 = 15;

/// Every table of a database with every migration applied. Only metadata lives in the database: no transcript
/// tables, no repositories of workspaces (DMN-022).
const TABLES: [&str; 12] = [
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
    "workspaces",
];

/// Turns a database back into a version-12 one (the first release): no declared jobs, no interrupted turns.
const BACK_TO_12: &str = "DROP TABLE job_runs; DROP TABLE job_state; DROP TABLE interrupted_turns; PRAGMA user_version = 12;";

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

fn open(dir: &TempDir) -> Store {
    Store::open(&dir.path().join("omfp.db")).unwrap()
}

fn workspace(name: &str) -> Workspace {
    Workspace {
        id: new_id(),
        name: name.into(),
        path: PathBuf::from("/work").join(name),
        created_by: CreatedBy::User,
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
        created_by: CreatedBy::User,
        created_at: now_ms(),
        archived_at: None,
    }
}

// spec: DMN-010, DMN-012
#[test]
fn opening_an_empty_file_applies_every_migration() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    drop(open(&dir));

    assert_eq!(user_version(&db), LATEST);
    assert_eq!(tables(&db), TABLES);
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

// spec: DMN-010
#[test]
fn a_database_of_the_first_release_takes_the_later_migrations_and_keeps_its_data() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    let ws = workspace("a");
    let t = thread(&ws.id);
    {
        let store = open(&dir);
        store.insert_workspace(&ws).unwrap();
        store.insert_thread(&t).unwrap();
        store
            .set_widget_state(&ws.id, "tests", "open", &json!(true))
            .unwrap();
    }
    raw(&db).execute_batch(BACK_TO_12).unwrap();

    let store = open(&dir);

    assert_eq!(user_version(&db), LATEST);
    assert_eq!(tables(&db), TABLES);
    assert_eq!(store.workspaces().unwrap(), vec![ws.clone()]);
    assert_eq!(store.threads().unwrap(), vec![t]);
    assert_eq!(
        serde_json::to_value(store.widget_state(&ws.id).unwrap()).unwrap(),
        json!({ "tests": { "open": true } })
    );
    assert_eq!(count(&db, "job_runs"), 0);
}

// spec: DMN-010
#[test]
fn a_database_from_before_the_first_release_is_refused_untouched() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    raw(&db).pragma_update(None, "user_version", 11).unwrap();

    let error = Store::open(&db).unwrap_err();
    assert!(
        matches!(&error, StoreError::Migration(message) if message.contains("pre-release")),
        "{error:?}"
    );
    assert_eq!(user_version(&db), 11);
    assert_eq!(tables(&db), Vec::<String>::new());
}

// spec: DMN-010
#[test]
fn a_database_from_a_newer_daemon_is_refused() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    raw(&db).pragma_update(None, "user_version", 99).unwrap();

    let error = Store::open(&db).unwrap_err();
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
        .mark_interrupted(
            &[busy.id.clone(), other.id.clone(), new_id()],
            10,
            Interruption::DaemonStop,
        )
        .unwrap();
    store
        .mark_interrupted(std::slice::from_ref(&busy.id), 20, Interruption::DaemonStop)
        .unwrap();
    assert_eq!(
        store.interrupted_turns().unwrap(),
        HashMap::from([(busy.id.clone(), 20), (other.id.clone(), 10)])
    );
    assert_eq!(
        store.take_interrupted(&busy.id).unwrap(),
        Some((20, Interruption::DaemonStop))
    );
    assert_eq!(store.take_interrupted(&busy.id).unwrap(), None);
    store.delete_thread(&other.id).unwrap();
    assert_eq!(count(&db, "interrupted_turns"), 0);
}

// spec: DMN-010, DMN-377
#[test]
fn migration_15_keeps_the_marks_as_daemon_stops_and_stores_what_interrupted_a_turn() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("omfp.db");
    let ws = workspace("a");
    let (stopped, crashed) = (thread(&ws.id), thread(&ws.id));
    {
        let store = open(&dir);
        store.insert_workspace(&ws).unwrap();
        store.insert_thread(&stopped).unwrap();
        store.insert_thread(&crashed).unwrap();
    }
    // The table as migration 14 left it, with a mark of a daemon of that version.
    raw(&db)
        .execute_batch(&format!(
            "DROP TABLE interrupted_turns; {} INSERT INTO interrupted_turns VALUES ('{}', 10); PRAGMA user_version = 14;",
            include_str!("../src/store/0014_interrupted_turns.sql"),
            stopped.id
        ))
        .unwrap();

    let store = open(&dir);

    assert_eq!(user_version(&db), LATEST);
    store
        .mark_interrupted(
            std::slice::from_ref(&crashed.id),
            20,
            Interruption::DaemonStop,
        )
        .unwrap();
    // Marking again takes the later cause with the later time.
    store
        .mark_interrupted(std::slice::from_ref(&crashed.id), 30, Interruption::Crash)
        .unwrap();
    assert!(
        raw(&db)
            .execute(
                "UPDATE interrupted_turns SET cause = 'unknown' WHERE thread_id = ?1",
                [&crashed.id],
            )
            .is_err(),
        "only known causes"
    );
    assert_eq!(
        store.take_interrupted(&stopped.id).unwrap(),
        Some((10, Interruption::DaemonStop))
    );
    assert_eq!(
        store.take_interrupted(&crashed.id).unwrap(),
        Some((30, Interruption::Crash))
    );
}
