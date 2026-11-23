//! L2: the thread service on a real SQLite file and data directory.

use std::path::PathBuf;
use std::sync::Arc;

use omfp_daemon::paths::DataDir;
use omfp_daemon::store::{Store, StoreError, Workspace, new_id, now_ms};
use omfp_daemon::threads::{DEFAULT_APPROVAL_MODE, NewThread, ThreadError, Threads};
use omfp_protocol::CreatedBy;
use tempfile::TempDir;

struct Env {
    _dir: TempDir,
    base: PathBuf,
    data: DataDir,
    store: Arc<Store>,
    threads: Threads,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let data = DataDir::new(base.join("data"));
    std::fs::create_dir_all(data.root()).unwrap();
    let store = Arc::new(Store::open(&data.db()).unwrap());
    let threads = Threads::new(store.clone(), data.clone());
    Env {
        _dir: dir,
        base,
        data,
        store,
        threads,
    }
}

/// The sorted entry names of a directory.
fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

impl Env {
    /// Records a workspace on an empty folder; the thread service never touches it.
    fn workspace(&self, name: &str) -> Workspace {
        let path = self.base.join("work").join(name);
        std::fs::create_dir_all(&path).unwrap();
        let ws = Workspace {
            id: new_id(),
            name: name.into(),
            path,
            created_by: CreatedBy::User,
            created_at: now_ms(),
        };
        self.store.insert_workspace(&ws).unwrap();
        ws
    }
}

fn assert_not_found<T: std::fmt::Debug>(result: Result<T, ThreadError>) {
    let error = result.unwrap_err();
    assert!(
        matches!(error, ThreadError::Store(StoreError::NotFound { .. })),
        "{error:?}"
    );
}

// spec: ARC-008, ARC-020
#[test]
fn a_new_thread_lives_in_its_workspace_with_its_own_session_directory() {
    let env = env();
    let ws = env.workspace("shop");

    let thread = env
        .threads
        .create(NewThread::new("Fix login", &ws.id))
        .unwrap();

    assert_eq!(thread.workspace_id, ws.id);
    assert_eq!(thread.session_dir, env.data.thread_sessions(&thread.id));
    assert!(thread.session_dir.is_dir());
    // The private directory holds sessions only: no home, nothing in the workspace.
    assert_eq!(entries(&env.data.thread(&thread.id)), ["sessions"]);
    assert_eq!(entries(&ws.path), Vec::<String>::new());
    assert_eq!(thread.title, "Fix login");
    assert_eq!(thread.omp_session_id, None);
    assert_eq!(thread.created_by, CreatedBy::User);
    assert_eq!(env.threads.list().unwrap(), vec![thread]);
}

// spec: ARC-009
#[test]
fn a_thread_records_settings_and_provenance() {
    let env = env();
    let ws = env.workspace("shop");
    let parent = env
        .threads
        .create(NewThread::new("parent", &ws.id))
        .unwrap();

    let thread = env
        .threads
        .create(NewThread {
            model: Some("fake/scripted".into()),
            thinking_level: Some("high".into()),
            approval_mode: Some("always-ask".into()),
            created_by: CreatedBy::Thread {
                thread_id: parent.id.clone(),
            },
            ..NewThread::new("child", &ws.id)
        })
        .unwrap();

    assert_eq!(thread.model.as_deref(), Some("fake/scripted"));
    assert_eq!(thread.thinking_level.as_deref(), Some("high"));
    assert_eq!(thread.approval_mode.as_deref(), Some("always-ask"));
    assert_eq!(
        thread.created_by,
        CreatedBy::Thread {
            thread_id: parent.id,
        }
    );
    assert_eq!(env.threads.get(&thread.id).unwrap(), thread);
}

// spec: DMN-074
#[test]
fn a_new_thread_runs_in_yolo_mode_unless_another_mode_is_given() {
    let env = env();
    let ws = env.workspace("shop");

    let thread = env.threads.create(NewThread::new("t", &ws.id)).unwrap();

    assert_eq!(DEFAULT_APPROVAL_MODE, "yolo");
    assert_eq!(thread.approval_mode.as_deref(), Some("yolo"));
    let changed = env
        .threads
        .set_approval_mode(&thread.id, "always-ask")
        .unwrap();
    assert_eq!(changed.approval_mode.as_deref(), Some("always-ask"));
}

// spec: ARC-009
#[test]
fn creation_is_refused_for_unknown_creators_or_workspaces() {
    let env = env();
    let ws = env.workspace("shop");

    assert_not_found(env.threads.create(NewThread {
        created_by: CreatedBy::Thread {
            thread_id: "01JUNKNOWN0000000000000000".into(),
        },
        ..NewThread::new("x", &ws.id)
    }));
    assert_not_found(env.threads.create(NewThread::new("x", "missing")));

    assert_eq!(env.threads.list().unwrap(), Vec::new());
    assert!(!env.data.root().join("threads").exists());
}

// spec: DMN-027
#[test]
fn a_workspace_can_be_shared_by_threads() {
    let env = env();
    let ws = env.workspace("shared");
    let other = env.workspace("other");

    let one = env.threads.create(NewThread::new("one", &ws.id)).unwrap();
    let two = env.threads.create(NewThread::new("two", &ws.id)).unwrap();

    assert_eq!(
        env.store.workspace_thread_ids(&ws.id).unwrap(),
        vec![one.id, two.id]
    );
    assert_eq!(
        env.store.workspace_thread_ids(&other.id).unwrap(),
        Vec::<String>::new()
    );
}

// spec: DMN-029
#[test]
fn the_session_roots_are_recorded() {
    let env = env();
    let ws = env.workspace("shop");
    let thread = env.threads.create(NewThread::new("t", &ws.id)).unwrap();
    assert_eq!(
        env.threads.roots(&thread.id).unwrap(),
        Vec::<PathBuf>::new()
    );

    let [api, web] = ["api", "web"].map(|name| ws.path.join(name));
    env.threads
        .set_roots(&thread.id, &[web.clone(), api.clone()])
        .unwrap();
    assert_eq!(
        env.threads.roots(&thread.id).unwrap(),
        vec![api, web.clone()]
    );
    env.threads
        .set_roots(&thread.id, std::slice::from_ref(&web))
        .unwrap();
    assert_eq!(env.threads.roots(&thread.id).unwrap(), vec![web]);

    assert_not_found(env.threads.set_roots("missing", &[]));
}

// spec: DMN-028
#[test]
fn deleting_a_thread_removes_its_directory_but_never_its_workspace() {
    let env = env();
    let ws = env.workspace("shop");
    std::fs::write(ws.path.join("README"), "keep").unwrap();
    let thread = env.threads.create(NewThread::new("t", &ws.id)).unwrap();

    env.threads.delete(&thread.id).unwrap();

    assert!(!env.data.thread(&thread.id).exists());
    assert_eq!(env.threads.list().unwrap(), Vec::new());
    assert_eq!(env.store.workspaces().unwrap(), vec![ws.clone()]);
    assert_eq!(entries(&ws.path), ["README"]);
    assert_not_found(env.threads.delete(&thread.id));
}

// spec: DMN-039, DMN-283
#[test]
fn a_deletion_names_the_workspace_and_deletes_nothing_once_the_thread_moved_away() {
    let env = env();
    let (shop, blog) = (env.workspace("shop"), env.workspace("blog"));
    let thread = env.threads.create(NewThread::new("t", &shop.id)).unwrap();
    assert!(
        env.threads
            .reorder(&thread.id, Some(&blog.id), None)
            .unwrap()
    );

    assert!(!env.threads.delete_record(&thread.id, &shop.id).unwrap());
    assert_eq!(env.threads.get(&thread.id).unwrap().workspace_id, blog.id);

    assert!(env.threads.delete_record(&thread.id, &blog.id).unwrap());
    assert_not_found(env.threads.get(&thread.id));
    assert_not_found(env.threads.delete_record(&thread.id, &blog.id));
}
