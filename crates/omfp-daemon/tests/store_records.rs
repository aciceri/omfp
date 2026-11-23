//! L2: CRUD on the SQLite store.

use std::collections::HashMap;
use std::path::PathBuf;

use omfp_daemon::store::{
    ActivityQuery, NewActivity, SeenRepository, Store, StoreError, Thread, ThreadFork, Workspace,
    new_id, now_ms,
};
use omfp_protocol::{CreatedBy, TitleSource};
use serde_json::json;
use tempfile::TempDir;

fn open() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("omfp.db")).unwrap();
    (dir, store)
}

fn workspace(name: &str, path: &str) -> Workspace {
    Workspace {
        id: new_id(),
        name: name.into(),
        path: path.into(),
        created_by: CreatedBy::User,
        created_at: now_ms(),
    }
}

/// Created by the agent of thread `thread_id`.
fn agent_of(thread_id: &str) -> CreatedBy {
    CreatedBy::Thread {
        thread_id: thread_id.into(),
    }
}

fn thread(created_by: CreatedBy, workspace_id: &str) -> Thread {
    let id = new_id();
    Thread {
        session_dir: format!("/data/threads/{id}/sessions").into(),
        id,
        title: "Fix login".into(),
        title_source: TitleSource::Set,
        workspace_id: workspace_id.into(),
        omp_session_id: None,
        approval_mode: Some("always-ask".into()),
        model: Some("fake/scripted".into()),
        thinking_level: Some("high".into()),
        created_by,
        created_at: now_ms(),
        archived_at: None,
    }
}

/// A store holding one workspace.
fn with_workspace() -> (TempDir, Store, Workspace) {
    let (dir, store) = open();
    let ws = workspace("shop", "/work/shop");
    store.insert_workspace(&ws).unwrap();
    (dir, store, ws)
}

fn assert_conflict<T: std::fmt::Debug>(result: Result<T, StoreError>) {
    let error = result.unwrap_err();
    assert!(matches!(error, StoreError::Conflict(_)), "{error:?}");
}

// spec: DMN-021
#[test]
fn seen_repositories_keep_their_latest_sighting_most_recent_first() {
    let (_dir, store) = open();
    let seen = |path: &str, at: i64| SeenRepository {
        path: path.into(),
        last_seen_at: at,
    };
    store
        .record_seen_repositories(&["/src/app".into(), "/src/api".into()], 10)
        .unwrap();
    store
        .record_seen_repositories(&["/src/lib".into()], 20)
        .unwrap();
    store
        .record_seen_repositories(&["/src/app".into()], 30)
        .unwrap();
    // An older sighting never moves a repository back.
    store
        .record_seen_repositories(&["/src/lib".into()], 5)
        .unwrap();
    store.record_seen_repositories(&[], 40).unwrap();

    assert_eq!(
        store.seen_repositories().unwrap(),
        vec![
            seen("/src/app", 30),
            seen("/src/lib", 20),
            seen("/src/api", 10)
        ]
    );
}

#[test]
fn unknown_ids_are_not_found() {
    let (_dir, store) = open();
    for error in [
        store.workspace("nope").unwrap_err(),
        store.thread("nope").unwrap_err(),
        store.rename_workspace("nope", "x").unwrap_err(),
        store.delete_workspace("nope").unwrap_err(),
        store.delete_thread("nope").unwrap_err(),
        store
            .set_thread_title("nope", "x", TitleSource::Set)
            .unwrap_err(),
        store.set_thread_roots("nope", &[]).unwrap_err(),
        store.set_created_client("nope", "cli").unwrap_err(),
    ] {
        assert!(matches!(error, StoreError::NotFound { .. }), "{error:?}");
    }
}

// spec: DMN-020
#[test]
fn workspaces_round_trip_by_id_path_and_name() {
    let (_dir, store, shop) = with_workspace();
    let made = Workspace {
        created_by: agent_of("01JAGENT"),
        ..workspace("Fix login", "/omfp/workspaces/fix-login")
    };
    store.insert_workspace(&made).unwrap();

    assert_eq!(store.workspace(&made.id).unwrap(), made);
    assert_eq!(
        store.workspace_by_path("/work/shop".as_ref()).unwrap(),
        Some(shop.clone())
    );
    assert_eq!(
        store.workspace_by_path("/work/none".as_ref()).unwrap(),
        None
    );
    assert!(store.workspace_name_taken("Fix login").unwrap());
    assert!(!store.workspace_name_taken("fix login").unwrap());
    assert_eq!(store.workspaces().unwrap(), vec![shop, made]);
}

// spec: DMN-020
#[test]
fn workspace_names_and_paths_are_unique() {
    let (_dir, store, ws) = with_workspace();

    assert_conflict(store.insert_workspace(&workspace("shop", "/work/other")));
    assert_conflict(store.insert_workspace(&workspace("other", "/work/shop")));
    assert_eq!(store.workspaces().unwrap(), vec![ws]);
}

// spec: DMN-020
#[test]
fn renaming_a_workspace_keeps_names_unique() {
    let (_dir, store, ws) = with_workspace();
    let other = workspace("other", "/work/other");
    store.insert_workspace(&other).unwrap();

    assert_conflict(store.rename_workspace(&ws.id, "other"));
    store.rename_workspace(&ws.id, "Shop v2").unwrap();

    assert_eq!(store.workspace(&ws.id).unwrap().name, "Shop v2");
    assert!(!store.workspace_name_taken("shop").unwrap());
    assert_eq!(store.workspace(&other.id).unwrap(), other);
}

// spec: DMN-028
#[test]
fn a_workspace_with_threads_is_not_deleted() {
    let (_dir, store, ws) = with_workspace();
    let t = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&t).unwrap();
    store.set_thread_archived_at(&t.id, Some(7)).unwrap();

    // Archived threads still live in the workspace.
    assert_conflict(store.delete_workspace(&ws.id));
    assert_eq!(store.workspaces().unwrap(), vec![ws.clone()]);

    store.delete_thread(&t.id).unwrap();
    store.delete_workspace(&ws.id).unwrap();
    assert_eq!(store.workspaces().unwrap(), Vec::new());
}

// spec: ARC-008, ARC-009, DMN-027
#[test]
fn threads_round_trip_with_their_workspace_and_provenance() {
    let (_dir, store, ws) = with_workspace();
    let other = workspace("other", "/work/other");
    store.insert_workspace(&other).unwrap();
    let parent = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&parent).unwrap();
    let child = thread(agent_of(&parent.id), &ws.id);
    store.insert_thread(&child).unwrap();

    assert_eq!(store.thread(&child.id).unwrap(), child);
    assert_eq!(
        store.threads().unwrap(),
        vec![parent.clone(), child.clone()]
    );
    assert_eq!(
        store.workspace_thread_ids(&ws.id).unwrap(),
        vec![parent.id, child.id]
    );
    assert_eq!(
        store.workspace_thread_ids(&other.id).unwrap(),
        Vec::<String>::new()
    );
}

// spec: DMN-073
#[test]
fn only_a_placeholder_title_is_replaced() {
    let (_dir, store, ws) = with_workspace();
    let t = Thread {
        title: String::new(),
        title_source: TitleSource::Placeholder,
        ..thread(CreatedBy::User, &ws.id)
    };
    store.insert_thread(&t).unwrap();

    assert!(
        store
            .replace_placeholder_title(&t.id, "Fix the login…", TitleSource::Placeholder)
            .unwrap()
    );
    store
        .set_thread_title(&t.id, "Mine", TitleSource::Set)
        .unwrap();
    assert!(
        !store
            .replace_placeholder_title(&t.id, "Generated", TitleSource::Generated)
            .unwrap()
    );
    let stored = store.thread(&t.id).unwrap();
    assert_eq!(
        (stored.title.as_str(), stored.title_source),
        ("Mine", TitleSource::Set)
    );
}

// spec: DMN-029
#[test]
fn thread_roots_are_replaced_and_listed_sorted() {
    let (_dir, store, ws) = with_workspace();
    let t = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&t).unwrap();
    assert_eq!(store.thread_roots(&t.id).unwrap(), Vec::<PathBuf>::new());

    let [api, web] = ["/work/shop/api", "/work/shop/web"].map(PathBuf::from);
    store
        .set_thread_roots(&t.id, &[web.clone(), api.clone(), web.clone()])
        .unwrap();
    assert_eq!(store.thread_roots(&t.id).unwrap(), vec![api, web.clone()]);

    store
        .set_thread_roots(&t.id, std::slice::from_ref(&web))
        .unwrap();
    assert_eq!(store.thread_roots(&t.id).unwrap(), vec![web]);
    store.set_thread_roots(&t.id, &[]).unwrap();
    assert_eq!(store.thread_roots(&t.id).unwrap(), Vec::<PathBuf>::new());
}

// spec: ARC-009, PRT-033
#[test]
fn the_creating_client_is_recorded_for_threads_and_workspaces() {
    let (_dir, store, ws) = with_workspace();
    let t = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&t).unwrap();
    let silent = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&silent).unwrap();
    assert_eq!(store.created_clients().unwrap(), HashMap::new());

    store.set_created_client(&t.id, "omfp/web").unwrap();
    store.set_created_client(&ws.id, "omfp-cli").unwrap();

    assert_eq!(
        store.created_clients().unwrap(),
        HashMap::from([
            (t.id.clone(), "omfp/web".to_owned()),
            (ws.id.clone(), "omfp-cli".to_owned()),
        ])
    );
}

// spec: ARC-009
#[test]
fn provenance_must_name_an_existing_thread() {
    let (_dir, store, ws) = with_workspace();
    let t = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&t).unwrap();

    store.check_provenance(&CreatedBy::User).unwrap();
    store.check_provenance(&agent_of(&t.id)).unwrap();
    let error = store.check_provenance(&agent_of("01JUNKNOWN")).unwrap_err();
    assert!(matches!(error, StoreError::NotFound { .. }), "{error:?}");
}

// spec: DMN-075
#[test]
fn the_theme_is_a_setting_that_can_be_cleared() {
    let (_dir, store) = open();
    assert_eq!(store.theme().unwrap(), None);

    store
        .set_theme(Some(":root { --omfp-primary: red; }"))
        .unwrap();
    assert_eq!(
        store.theme().unwrap().as_deref(),
        Some(":root { --omfp-primary: red; }")
    );
    store.set_theme(None).unwrap();
    assert_eq!(store.theme().unwrap(), None);
}

// spec: AGT-070, ARC-009
#[test]
fn the_spawned_tree_follows_provenance_transitively_archived_threads_included() {
    let (_dir, store, ws) = with_workspace();
    let root = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&root).unwrap();
    let child = thread(agent_of(&root.id), &ws.id);
    store.insert_thread(&child).unwrap();
    let grandchild = Thread {
        archived_at: Some(now_ms()),
        ..thread(agent_of(&child.id), &ws.id)
    };
    store.insert_thread(&grandchild).unwrap();
    let second = thread(agent_of(&root.id), &ws.id);
    store.insert_thread(&second).unwrap();
    let unrelated = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&unrelated).unwrap();

    assert_eq!(
        store.spawned_thread_ids(&root.id).unwrap(),
        vec![child.id.clone(), grandchild.id.clone(), second.id]
    );
    assert_eq!(
        store.spawned_thread_ids(&child.id).unwrap(),
        vec![grandchild.id.clone()]
    );
    assert_eq!(
        store.spawned_thread_ids(&grandchild.id).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        store.spawned_thread_ids(&unrelated.id).unwrap(),
        Vec::<String>::new()
    );
}

fn action(thread_id: &str, method: &str, error: Option<&str>) -> NewActivity {
    NewActivity {
        at: 1_731_000_000_000,
        thread_id: Some(thread_id.to_owned()),
        client: "omfp-agent".to_owned(),
        method: method.to_owned(),
        params: json!({ "threadId": thread_id, "nested": { "n": [1, 2] } }),
        error: error.map(str::to_owned),
    }
}

fn query(
    thread_id: Option<&str>,
    before: Option<i64>,
    after: Option<i64>,
    limit: u32,
) -> ActivityQuery {
    ActivityQuery {
        thread_id: thread_id.map(str::to_owned),
        before,
        after,
        limit,
    }
}

// spec: DMN-160, PRT-086
#[test]
fn the_activity_log_pages_newest_first_and_filters_by_thread() {
    let (_dir, store) = open();
    let first = store
        .record_activity(&action("01JA", "threads.create", None), 100)
        .unwrap();
    let second = store
        .record_activity(
            &action("01JB", "thread.prompt", Some("thread 01JX not found")),
            100,
        )
        .unwrap();
    let third = store
        .record_activity(&action("01JA", "thread.stop", None), 100)
        .unwrap();
    assert!(first.seq < second.seq && second.seq < third.seq);
    assert_eq!(third.entry, action("01JA", "thread.stop", None));

    let seqs = |(entries, more): (Vec<omfp_daemon::store::Activity>, bool)| {
        (entries.iter().map(|a| a.seq).collect::<Vec<_>>(), more)
    };
    let (all, more) = store.activity(&query(None, None, None, 50)).unwrap();
    assert_eq!(all, vec![third.clone(), second.clone(), first.clone()]);
    assert!(!more);
    assert_eq!(
        seqs(store.activity(&query(None, None, None, 2)).unwrap()),
        (vec![third.seq, second.seq], true)
    );
    assert_eq!(
        seqs(
            store
                .activity(&query(None, Some(second.seq), None, 2))
                .unwrap()
        ),
        (vec![first.seq], false)
    );
    assert_eq!(
        seqs(
            store
                .activity(&query(None, None, Some(first.seq), 50))
                .unwrap()
        ),
        (vec![third.seq, second.seq], false)
    );
    assert_eq!(
        seqs(
            store
                .activity(&query(Some("01JA"), None, None, 50))
                .unwrap()
        ),
        (vec![third.seq, first.seq], false)
    );
    assert_eq!(
        store.activity(&query(None, None, None, 50)).unwrap().0[1]
            .entry
            .error
            .as_deref(),
        Some("thread 01JX not found")
    );
}

// spec: DMN-160
#[test]
fn the_activity_log_keeps_the_newest_entries_and_outlives_its_thread() {
    let (_dir, store, ws) = with_workspace();
    let agent = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&agent).unwrap();
    let mut recorded = Vec::new();
    for method in ["a", "b", "c", "d"] {
        recorded.push(
            store
                .record_activity(&action(&agent.id, method, None), 2)
                .unwrap(),
        );
    }
    store.delete_thread(&agent.id).unwrap();

    let (kept, more) = store.activity(&query(None, None, None, 50)).unwrap();
    assert_eq!(kept, vec![recorded[3].clone(), recorded[2].clone()]);
    assert!(!more);
}

/// `ids` in the order `positions` gives them.
fn ordered<'a>(store: &Store, ids: &[&'a String]) -> Vec<&'a String> {
    let positions = store.positions().unwrap();
    let mut ids = ids.to_vec();
    ids.sort_by_key(|id| positions[id.as_str()]);
    ids
}

// spec: DMN-280, DMN-281, DMN-284
#[test]
fn workspaces_are_reordered_before_another_or_last() {
    let (_dir, store) = open();
    let [a, b, c] = ["a", "b", "c"].map(|name| workspace(name, &format!("/work/{name}")));
    for ws in [&a, &b, &c] {
        store.insert_workspace(ws).unwrap();
    }
    let all = [&a.id, &b.id, &c.id];
    assert_eq!(ordered(&store, &all), [&a.id, &b.id, &c.id]);

    store.reorder_workspace(&c.id, Some(&a.id)).unwrap();
    assert_eq!(ordered(&store, &all), [&c.id, &a.id, &b.id]);
    store.reorder_workspace(&c.id, None).unwrap();
    assert_eq!(ordered(&store, &all), [&a.id, &b.id, &c.id]);
    // Positions count from 0 without gaps, whatever was removed.
    store.reorder_workspace(&a.id, None).unwrap();
    store.delete_workspace(&b.id).unwrap();
    let positions = store.positions().unwrap();
    assert_eq!((positions[&c.id], positions[&a.id]), (0, 1));
}

// spec: DMN-282, DMN-283, DMN-284
#[test]
fn threads_are_reordered_within_their_workspace_and_moved_to_another() {
    let (_dir, store, shop) = with_workspace();
    let other = workspace("other", "/work/other");
    store.insert_workspace(&other).unwrap();
    let [t1, t2, t3] = [(); 3].map(|()| thread(CreatedBy::User, &shop.id));
    let archived = thread(CreatedBy::User, &shop.id);
    let there = thread(CreatedBy::User, &other.id);
    for t in [&t1, &t2, &t3, &archived, &there] {
        store.insert_thread(t).unwrap();
    }
    store
        .set_thread_archived_at(&archived.id, Some(now_ms()))
        .unwrap();
    let ids = [&t1.id, &t2.id, &t3.id, &archived.id];

    assert!(!store.reorder_thread(&t3.id, None, Some(&t1.id)).unwrap());
    assert_eq!(
        ordered(&store, &ids),
        [&t3.id, &t1.id, &t2.id, &archived.id]
    );
    // Archived threads have their place too.
    assert!(
        !store
            .reorder_thread(&archived.id, Some(&shop.id), Some(&t1.id))
            .unwrap()
    );
    assert_eq!(
        ordered(&store, &ids),
        [&t3.id, &archived.id, &t1.id, &t2.id]
    );

    // Into another workspace: before one of its threads, or last.
    assert!(
        store
            .reorder_thread(&t1.id, Some(&other.id), Some(&there.id))
            .unwrap()
    );
    assert_eq!(store.thread(&t1.id).unwrap().workspace_id, other.id);
    assert_eq!(ordered(&store, &[&there.id, &t1.id]), [&t1.id, &there.id]);
    assert!(store.reorder_thread(&t2.id, Some(&other.id), None).unwrap());
    let positions = store.positions().unwrap();
    assert_eq!(
        [&t1.id, &there.id, &t2.id].map(|id| positions[id]),
        [0, 1, 2]
    );
    // What stays behind keeps its order.
    assert_eq!(
        ordered(&store, &[&t3.id, &archived.id]),
        [&t3.id, &archived.id]
    );
}

// spec: DMN-285
#[test]
fn a_reorder_before_something_gone_changes_nothing() {
    let (_dir, store, shop) = with_workspace();
    let other = workspace("other", "/work/other");
    store.insert_workspace(&other).unwrap();
    let [t1, t2] = [(); 2].map(|()| thread(CreatedBy::User, &shop.id));
    for t in [&t1, &t2] {
        store.insert_thread(t).unwrap();
    }
    // Another window moved `t1` away meanwhile: placing `t2` before it in `shop` is a conflict.
    store.reorder_thread(&t1.id, Some(&other.id), None).unwrap();
    assert_conflict(store.reorder_thread(&t2.id, None, Some(&t1.id)));
    assert_eq!(store.thread(&t2.id).unwrap().workspace_id, shop.id);
    assert_conflict(store.reorder_thread(&t2.id, Some(&other.id), Some("gone")));
    assert_eq!(store.thread(&t2.id).unwrap().workspace_id, shop.id);
    assert_conflict(store.reorder_workspace(&shop.id, Some("gone")));
    for error in [
        store.reorder_workspace("nope", None).unwrap_err(),
        store.reorder_thread("nope", None, None).unwrap_err(),
        store
            .reorder_thread(&t2.id, Some("nope"), None)
            .unwrap_err(),
    ] {
        assert!(matches!(error, StoreError::NotFound { .. }), "{error:?}");
    }
}

// spec: DMN-300, DMN-305
#[test]
fn a_forks_lineage_outlives_its_source_and_goes_with_the_fork() {
    let (_dir, store, ws) = with_workspace();
    let source = thread(CreatedBy::User, &ws.id);
    let fork = thread(CreatedBy::User, &ws.id);
    store.insert_thread(&source).unwrap();
    store.insert_thread(&fork).unwrap();
    let lineage = ThreadFork {
        source_thread_id: source.id.clone(),
        source_title: "Fix login".into(),
        entry_id: Some("u1".into()),
        message: Some("Fix the login form".into()),
    };
    assert_eq!(store.thread_forks().unwrap(), HashMap::new());

    store.insert_thread_fork(&fork.id, &lineage).unwrap();
    assert_conflict(store.insert_thread_fork(&fork.id, &lineage));
    assert!(store.insert_thread_fork("01NOTHREAD", &lineage).is_err());

    store.delete_thread(&source.id).unwrap();
    assert_eq!(
        store.thread_forks().unwrap(),
        HashMap::from([(fork.id.clone(), lineage)])
    );
    store.delete_thread(&fork.id).unwrap();
    assert_eq!(store.thread_forks().unwrap(), HashMap::new());
}
