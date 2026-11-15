//! L1: the workspace service against the in-memory `FakeGit`, with real folders, files and symlinks in a temp dir
//! and a real SQLite file.

use std::collections::BTreeSet;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use omfp_core::{ThreadState, ToolRun, ToolStatus};
use omfp_daemon::git::{FakeGit, Git, WorktreeBranch};
use omfp_daemon::paths::DataDir;
use omfp_daemon::store::{Provenance, Store, StoreError};
use omfp_daemon::threads::{NewThread, Threads};
use omfp_daemon::workspaces::{
    Discovered, MAX_CANDIDATES, NewWorkspace, RepoKind, RepoSource, WorkspaceError, WorkspaceView,
    Workspaces, named_files, scan,
};
use omfp_protocol::FileMatch;
use serde_json::json;
use tempfile::TempDir;

struct Env {
    _dir: TempDir,
    /// Canonical path of the temp dir.
    root: PathBuf,
    /// `--workspaces-root`: `<root>/workspaces`.
    workspaces_root: PathBuf,
    data: DataDir,
    store: Arc<Store>,
    git: Arc<FakeGit>,
    workspaces: Workspaces,
    /// A fake repository on `main` at `<root>/repos/app`.
    repo: PathBuf,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let workspaces_root = root.join("workspaces");
    let data = DataDir::new(root.join("data"));
    std::fs::create_dir_all(data.root()).unwrap();
    let store = Arc::new(Store::open(&data.db(), &workspaces_root).unwrap());
    let git = Arc::new(FakeGit::new());
    let repo = fake_repo(&git, &root.join("repos").join("app"), "main");
    let workspaces = Workspaces::new(store.clone(), git.clone(), workspaces_root.clone());
    Env {
        _dir: dir,
        root,
        workspaces_root,
        data,
        store,
        git,
        workspaces,
        repo,
    }
}

/// Makes `path` a fake repository with `head` checked out.
fn fake_repo(git: &FakeGit, path: &Path, head: &str) -> PathBuf {
    std::fs::create_dir_all(path).unwrap();
    git.add_repo(path, head);
    path.to_owned()
}

fn named(name: &str) -> NewWorkspace {
    NewWorkspace {
        name: Some(name.into()),
        ..NewWorkspace::new()
    }
}

fn fallback(name: &str) -> NewWorkspace {
    NewWorkspace {
        fallback_name: Some(name.into()),
        ..NewWorkspace::new()
    }
}

fn linking(path: &Path) -> NewWorkspace {
    NewWorkspace {
        link: Some(path.to_owned()),
        ..NewWorkspace::new()
    }
}

fn link(path: &Path) -> RepoSource {
    RepoSource::Link(path.to_owned())
}

fn worktree(repo: &Path, branch: Option<&str>) -> RepoSource {
    RepoSource::Worktree {
        repo: repo.to_owned(),
        branch: branch.map(str::to_owned),
        base_ref: None,
    }
}

fn worktree_of(repo: &Path) -> RepoSource {
    worktree(repo, None)
}

fn names(view: &WorkspaceView) -> Vec<&str> {
    view.repos.iter().map(|repo| repo.name.as_str()).collect()
}

/// Entries of a workspace folder besides omfp's own `.omfp/` (DMN-088), sorted.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".omfp")
        .collect();
    names.sort();
    names
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

impl Env {
    fn create(&self, name: &str) -> WorkspaceView {
        self.workspaces.create(named(name)).unwrap()
    }

    /// What a scan reports for the git root at `path`, found as entry `name`.
    fn discovered(&self, name: &str, path: &Path, kind: RepoKind, branch: &str) -> Discovered {
        Discovered {
            name: name.into(),
            path: path.to_owned(),
            kind,
            branch: Some(branch.into()),
            head: self.git.head(path).unwrap(),
            is_root: false,
            canonical: path.canonicalize().unwrap(),
        }
    }

    fn threads(&self) -> Threads {
        Threads::new(self.store.clone(), self.data.clone())
    }
}

// spec: DMN-020, DMN-088
#[test]
fn a_new_workspace_is_a_folder_under_the_workspaces_root_with_the_default_views() {
    let env = env();

    let view = env.create("Fix login!");

    assert_eq!(view.workspace.name, "Fix login!");
    assert_eq!(view.workspace.path, env.workspaces_root.join("fix-login"));
    assert_eq!(view.workspace.created_by, Provenance::User);
    assert!(view.workspace.path.is_dir());
    assert_eq!(
        std::fs::read_to_string(view.workspace.path.join(".omfp/views.json")).unwrap(),
        omfp_daemon::widgets::DEFAULT_MANIFEST
    );
    assert_eq!(view.repos, Vec::new());
    assert_eq!(
        env.workspaces.get(&view.workspace.id).unwrap(),
        view.workspace
    );
    assert_eq!(env.workspaces.list().unwrap(), vec![view.workspace]);
}

// spec: DMN-020
#[test]
fn names_and_folders_are_unique() {
    let env = env();
    let first = env.create("Fix login");
    std::fs::create_dir_all(env.workspaces_root.join("notes")).unwrap();

    let taken = env.workspaces.create(named("Fix login")).unwrap_err();
    assert!(
        matches!(taken, WorkspaceError::Store(StoreError::Conflict(_))),
        "{taken:?}"
    );
    let same_slug = env.create("fix login");
    let unrecorded_folder = env.create("Notes");
    let blank = env.workspaces.create(named("  ")).unwrap_err();
    assert!(matches!(blank, WorkspaceError::InvalidName(_)), "{blank:?}");
    let renamed = env
        .workspaces
        .rename(&same_slug.workspace.id, "Fix login")
        .unwrap_err();
    assert!(
        matches!(renamed, WorkspaceError::Store(StoreError::Conflict(_))),
        "{renamed:?}"
    );

    assert_eq!(first.workspace.path, env.workspaces_root.join("fix-login"));
    assert_eq!(
        same_slug.workspace.path,
        env.workspaces_root.join("fix-login-2")
    );
    assert_eq!(
        unrecorded_folder.workspace.path,
        env.workspaces_root.join("notes-2")
    );
    assert_eq!(
        entries(&env.workspaces_root),
        vec!["fix-login", "fix-login-2", "notes", "notes-2"]
    );
    // Renaming keeps the folder.
    let view = env
        .workspaces
        .rename(&same_slug.workspace.id, "Other")
        .unwrap();
    assert_eq!(view.workspace.name, "Other");
    assert_eq!(view.workspace.path, same_slug.workspace.path);
}

// spec: DMN-020
#[test]
fn default_and_fallback_names_are_made_unique() {
    let env = env();

    let created: Vec<(String, PathBuf)> = [
        NewWorkspace::new(),
        NewWorkspace::new(),
        fallback("Fix login"),
        fallback("Fix login"),
        fallback("  "),
    ]
    .into_iter()
    .map(|new| {
        let ws = env.workspaces.create(new).unwrap().workspace;
        (ws.name, ws.path)
    })
    .collect();

    let root = &env.workspaces_root;
    assert_eq!(
        created,
        vec![
            ("workspace".into(), root.join("workspace")),
            ("workspace 2".into(), root.join("workspace-2")),
            ("Fix login".into(), root.join("fix-login")),
            ("Fix login 2".into(), root.join("fix-login-2")),
            ("workspace 3".into(), root.join("workspace-3")),
        ]
    );
}

// spec: DMN-020
#[test]
fn a_workspace_created_with_a_link_is_a_new_folder_linking_the_repository() {
    let env = env();
    let alias = env.root.join("alias");
    symlink(&env.repo, &alias).unwrap();

    let view = env.workspaces.create(linking(&alias)).unwrap();

    assert_eq!(view.workspace.name, "app");
    let folder = env.workspaces_root.join("app");
    assert_eq!(view.workspace.path, folder);
    assert_eq!(
        view.repos,
        vec![env.discovered(
            "app",
            &folder.join("app"),
            RepoKind::Link {
                target: env.repo.clone()
            },
            "main"
        )]
    );
    assert_eq!(std::fs::read_link(folder.join("app")).unwrap(), env.repo);

    // Names default to the repository's folder name, made unique; a given name or fallback wins.
    let again = env.workspaces.create(linking(&env.repo)).unwrap();
    assert_eq!(again.workspace.name, "app 2");
    assert_eq!(again.workspace.path, env.workspaces_root.join("app-2"));
    let docs = env
        .workspaces
        .create(NewWorkspace {
            name: Some("Docs".into()),
            ..linking(&env.repo)
        })
        .unwrap();
    let titled = env
        .workspaces
        .create(NewWorkspace {
            fallback_name: Some("Fix login".into()),
            ..linking(&env.repo)
        })
        .unwrap();
    for view in [&again, &docs, &titled] {
        assert_eq!(names(view), vec!["app"]);
    }
    assert_eq!(
        (docs.workspace.name.as_str(), titled.workspace.name.as_str()),
        ("Docs", "Fix login")
    );
    assert_eq!(
        entries(&env.workspaces_root),
        vec!["app", "app-2", "docs", "fix-login"]
    );
}

// spec: DMN-020, DMN-024
#[test]
fn a_bad_link_creates_nothing() {
    let env = env();
    let notes = env.root.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("todo.txt"), "x").unwrap();
    std::fs::create_dir_all(env.repo.join("src")).unwrap();

    for (path, expected) in [
        (env.root.join("missing"), "not a directory"),
        (notes.join("todo.txt"), "not a directory"),
        (notes.clone(), "not in a git repository"),
        (env.repo.join("src"), "not the root"),
    ] {
        let error = env.workspaces.create(linking(&path)).unwrap_err();
        assert!(
            matches!(
                (&error, expected),
                (WorkspaceError::NotADirectory(_), "not a directory")
                    | (WorkspaceError::NotARepository(_), "not in a git repository")
                    | (WorkspaceError::NotARepositoryRoot(_), "not the root")
            ),
            "{}: {error:?}",
            path.display()
        );
    }

    assert_eq!(env.workspaces.list().unwrap(), Vec::new());
    assert!(!env.workspaces_root.exists());
}

// spec: DMN-022
#[test]
fn a_folder_that_became_a_git_root_is_discovered_first_as_its_own_root() {
    let env = env();
    let ws = env.create("mono").workspace;
    let folder = ws.path.clone();
    fake_repo(&env.git, &folder, "main");
    symlink(&env.repo, folder.join("app")).unwrap();

    let (view, _) = env.workspaces.rescan(&ws.id).unwrap();

    assert_eq!(
        view.repos,
        vec![
            Discovered {
                is_root: true,
                ..env.discovered("mono", &folder, RepoKind::Clone, "main")
            },
            env.discovered(
                "app",
                &folder.join("app"),
                RepoKind::Link {
                    target: env.repo.clone()
                },
                "main"
            ),
        ]
    );
}

// spec: ARC-009
#[test]
fn workspace_provenance_is_recorded() {
    let env = env();
    let home = env.create("home");
    let creator = env
        .threads()
        .create(NewThread::new("agent", &home.workspace.id))
        .unwrap();

    let view = env
        .workspaces
        .create(NewWorkspace {
            created_by: Provenance::Thread(creator.id.clone()),
            ..named("made by agent")
        })
        .unwrap();
    assert_eq!(
        env.workspaces.get(&view.workspace.id).unwrap().created_by,
        Provenance::Thread(creator.id)
    );

    let unknown = env
        .workspaces
        .create(NewWorkspace {
            created_by: Provenance::Thread("01JUNKNOWNTHREAD0000000000".into()),
            ..named("orphan")
        })
        .unwrap_err();
    assert!(
        matches!(unknown, WorkspaceError::Store(StoreError::NotFound { .. })),
        "{unknown:?}"
    );
    assert!(!env.workspaces_root.join("orphan").exists());
}

/// The paths `query` suggests.
fn suggested(env: &Env, query: &str) -> Vec<PathBuf> {
    env.workspaces
        .suggest(query)
        .unwrap()
        .into_iter()
        .map(|repo| repo.path)
        .collect()
}

// spec: DMN-021
#[test]
fn every_scan_records_its_repositories_for_suggestions() {
    let env = env();
    assert_eq!(suggested(&env, ""), Vec::<PathBuf>::new());
    let api = fake_repo(&env.git, &env.root.join("repos").join("api"), "main");
    let id = env.create("x").workspace.id;
    env.workspaces.add_repo(&id, None, link(&env.repo)).unwrap();
    env.workspaces
        .add_repo(&id, None, worktree_of(&api))
        .unwrap();
    let view = env
        .workspaces
        .add_repo(
            &id,
            None,
            RepoSource::Clone("https://example.invalid/lib.git".into()),
        )
        .unwrap();
    let clone = view.repos[2].canonical.clone();

    // A link by its target, a worktree by its main repository, a clone by itself.
    let mut all = suggested(&env, "");
    all.sort();
    let mut expected = vec![env.repo.clone(), api.clone(), clone.clone()];
    expected.sort();
    assert_eq!(all, expected);
    let first = &env.workspaces.suggest("").unwrap()[0];
    assert_eq!(first.name, dir_name(&first.path));

    // Still suggested once gone from the workspace.
    env.workspaces
        .remove_repo(&id, "app", false, false)
        .unwrap();
    assert!(suggested(&env, "").contains(&env.repo));
}

fn dir_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

// spec: DMN-021
#[test]
fn suggestions_are_filtered_by_path_or_name_ignoring_case() {
    let env = env();
    let api = fake_repo(&env.git, &env.root.join("repos").join("Backend"), "main");
    let id = env.create("x").workspace.id;
    env.workspaces.add_repo(&id, None, link(&env.repo)).unwrap();
    env.workspaces.add_repo(&id, None, link(&api)).unwrap();

    // Queries longer than the temp dir's random suffix, so they cannot match it by chance.
    assert_eq!(suggested(&env, "bACKEND"), vec![api.clone()]);
    assert_eq!(suggested(&env, " REPOS/APP "), vec![env.repo.clone()]);
    assert_eq!(suggested(&env, "repos/").len(), 2);
    // Not a word a temp dir path can hold (GitHub's runner keeps TMPDIR under /var/tmp/nothing-but-nix/).
    assert_eq!(suggested(&env, "zz-no-such-repo"), Vec::<PathBuf>::new());
}

/// Fills `folder` with a link, a worktree and a clone among entries that are no repository.
fn populate_for_scan(env: &Env, folder: &Path) {
    symlink(&env.repo, folder.join("app")).unwrap();
    let branch = WorktreeBranch::New {
        name: "topic".into(),
        base_ref: "main".into(),
    };
    env.git
        .worktree_add(&env.repo, &folder.join("wt"), &branch)
        .unwrap();
    fake_repo(&env.git, &folder.join("lib"), "dev");

    fake_repo(&env.git, &folder.join(".omp"), "main");
    fake_repo(&env.git, &folder.join("group").join("inner"), "main");
    std::fs::create_dir_all(folder.join("notes")).unwrap();
    std::fs::write(folder.join("README.md"), "x").unwrap();
    std::fs::create_dir_all(env.repo.join("src")).unwrap();
    symlink(env.repo.join("src"), folder.join("sub")).unwrap();
    symlink(env.root.join("missing"), folder.join("dangling")).unwrap();
    std::fs::write(env.root.join("file.txt"), "x").unwrap();
    symlink(env.root.join("file.txt"), folder.join("file-link")).unwrap();
}

// spec: DMN-022
#[test]
fn a_scan_finds_links_worktrees_and_clones_at_the_top_level_only() {
    let env = env();
    let ws = env.create("scan").workspace;
    populate_for_scan(&env, &ws.path);

    let (view, changed) = env.workspaces.rescan(&ws.id).unwrap();

    assert!(changed);
    let expected = vec![
        env.discovered(
            "app",
            &ws.path.join("app"),
            RepoKind::Link {
                target: env.repo.clone(),
            },
            "main",
        ),
        env.discovered("lib", &ws.path.join("lib"), RepoKind::Clone, "dev"),
        env.discovered(
            "wt",
            &ws.path.join("wt"),
            RepoKind::Worktree {
                main: env.repo.clone(),
            },
            "topic",
        ),
    ];
    assert_eq!(view.repos, expected);
    assert_eq!(view.repos[0].canonical, env.repo);
    assert_ne!(view.repos[0].head, view.repos[2].head);
    assert_eq!(scan(&*env.git, &ws.path).unwrap(), expected);
    assert_eq!(env.workspaces.discovered(&ws.id).unwrap(), expected);
    assert_eq!(env.workspaces.repo(&ws.id, "lib").unwrap(), expected[1]);
    let missing = env.workspaces.repo(&ws.id, "notes").unwrap_err();
    assert!(
        matches!(missing, WorkspaceError::NoSuchRepo { .. }),
        "{missing:?}"
    );
}

// spec: DMN-023
#[test]
fn a_rescan_reports_a_change_only_when_the_repositories_differ() {
    let env = env();
    let ws = env.create("x").workspace;
    let rescan = || env.workspaces.rescan(&ws.id).unwrap();

    assert_eq!(
        rescan(),
        (
            WorkspaceView {
                workspace: ws.clone(),
                repos: Vec::new()
            },
            false
        )
    );
    symlink(&env.repo, ws.path.join("app")).unwrap();
    // Until rescanned, the last scan is reported.
    assert_eq!(env.workspaces.discovered(&ws.id).unwrap(), Vec::new());
    let (view, changed) = rescan();
    assert!(changed);
    assert_eq!(names(&view), vec!["app"]);
    assert_eq!(env.workspaces.discovered(&ws.id).unwrap(), view.repos);
    assert!(!rescan().1);
    std::fs::write(ws.path.join("notes.txt"), "x").unwrap();
    env.git.set_dirty(&env.repo, true);
    assert!(!rescan().1);
    std::fs::remove_file(ws.path.join("app")).unwrap();
    let (view, changed) = rescan();
    assert!(changed);
    assert_eq!(view.repos, Vec::new());
}

// spec: DMN-024
#[test]
fn a_link_is_named_after_its_target_unless_named() {
    let env = env();
    let id = env.create("x").workspace.id;
    let add = |name: Option<&str>, target: &Path| env.workspaces.add_repo(&id, name, link(target));

    assert_eq!(names(&add(None, &env.repo).unwrap()), vec!["app"]);
    assert_eq!(names(&add(None, &env.repo).unwrap()), vec!["app", "app-2"]);
    let view = add(Some("frontend"), &env.repo).unwrap();

    assert_eq!(names(&view), vec!["app", "app-2", "frontend"]);
    for repo in &view.repos {
        assert_eq!(
            repo.kind,
            RepoKind::Link {
                target: env.repo.clone()
            }
        );
        assert_eq!(std::fs::read_link(&repo.path).unwrap(), env.repo);
    }
    let duplicate = add(Some("frontend"), &env.repo).unwrap_err();
    assert!(
        matches!(duplicate, WorkspaceError::EntryExists(_)),
        "{duplicate:?}"
    );
    for invalid in ["", ".hidden", "a/b"] {
        let error = add(Some(invalid), &env.repo).unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidName(_)), "{error:?}");
    }
    let plain = env.root.join("plain");
    std::fs::create_dir_all(env.repo.join("src")).unwrap();
    std::fs::create_dir_all(&plain).unwrap();
    let nested = add(None, &env.repo.join("src")).unwrap_err();
    assert!(
        matches!(nested, WorkspaceError::NotARepositoryRoot(_)),
        "{nested:?}"
    );
    let not_git = add(None, &plain).unwrap_err();
    assert!(
        matches!(not_git, WorkspaceError::NotARepository(_)),
        "{not_git:?}"
    );
    let ws = env.workspaces.get(&id).unwrap();
    assert_eq!(entries(&ws.path), vec!["app", "app-2", "frontend"]);
}

// spec: DMN-024
#[test]
fn a_worktree_gets_a_new_default_branch_unique_in_the_repository() {
    let env = env();
    let id = env.create("Fix login!").workspace.id;
    env.git.add_branch(&env.repo, "omfp/fix-login-1");

    env.workspaces
        .add_repo(&id, None, worktree_of(&env.repo))
        .unwrap();
    let view = env
        .workspaces
        .add_repo(&id, None, worktree_of(&env.repo))
        .unwrap();

    let folder = env.workspaces_root.join("fix-login");
    assert_eq!(
        view.repos,
        vec![
            env.discovered(
                "app",
                &folder.join("app"),
                RepoKind::Worktree {
                    main: env.repo.clone()
                },
                "omfp/fix-login-2"
            ),
            env.discovered(
                "app-2",
                &folder.join("app-2"),
                RepoKind::Worktree {
                    main: env.repo.clone()
                },
                "omfp/fix-login-3"
            ),
        ]
    );
    assert_eq!(
        env.git.worktrees(&env.repo),
        vec![
            (folder.join("app"), "omfp/fix-login-2".to_owned()),
            (folder.join("app-2"), "omfp/fix-login-3".to_owned()),
        ]
    );
}

// spec: DMN-024
#[test]
fn a_worktree_checks_out_an_existing_branch_or_creates_the_named_one() {
    let env = env();
    let id = env.create("x").workspace.id;
    env.git.add_branch(&env.repo, "feature/login");
    let repo = || env.repo.as_path();

    let existing = env
        .workspaces
        .add_repo(&id, Some("login"), worktree(repo(), Some("feature/login")))
        .unwrap();
    let created = env
        .workspaces
        .add_repo(&id, Some("topic"), worktree(repo(), Some("topic")))
        .unwrap();
    let bad_base = env
        .workspaces
        .add_repo(
            &id,
            Some("broken"),
            RepoSource::Worktree {
                repo: repo().to_owned(),
                branch: Some("other".into()),
                base_ref: Some("nope".into()),
            },
        )
        .unwrap_err();

    assert_eq!(existing.repos[0].branch.as_deref(), Some("feature/login"));
    assert_eq!(names(&created), vec!["login", "topic"]);
    assert_eq!(created.repos[1].branch.as_deref(), Some("topic"));
    assert!(matches!(bad_base, WorkspaceError::Git(_)), "{bad_base:?}");
    assert_eq!(
        env.git.branches(&env.repo),
        vec!["feature/login", "main", "topic"]
    );
    let ws = env.workspaces.get(&id).unwrap();
    assert_eq!(entries(&ws.path), vec!["login", "topic"]);
}

// spec: DMN-024
#[test]
fn a_worktree_repository_is_given_by_any_path_inside_it() {
    let env = env();
    let id = env.create("x").workspace.id;
    let api = fake_repo(&env.git, &env.root.join("repos").join("api"), "main");
    std::fs::create_dir_all(api.join("src")).unwrap();
    let plain = env.root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();

    env.workspaces
        .add_repo(&id, None, worktree_of(&env.repo))
        .unwrap();
    let view = env
        .workspaces
        .add_repo(&id, None, worktree_of(&api.join("src")))
        .unwrap();

    assert_eq!(names(&view), vec!["api", "app"]);
    assert_eq!(view.repos[0].kind, RepoKind::Worktree { main: api.clone() });
    assert_eq!(env.git.branches(&api), vec!["main", "omfp/x-1"]);
    assert_eq!(env.git.branches(&env.repo), vec!["main", "omfp/x-1"]);
    let missing = env
        .workspaces
        .add_repo(&id, None, worktree_of(&env.root.join("missing")))
        .unwrap_err();
    assert!(
        matches!(missing, WorkspaceError::NotADirectory(_)),
        "{missing:?}"
    );
    let not_git = env
        .workspaces
        .add_repo(&id, None, worktree_of(&plain))
        .unwrap_err();
    assert!(
        matches!(not_git, WorkspaceError::NotARepository(_)),
        "{not_git:?}"
    );
}

// spec: DMN-024
#[test]
fn a_clone_is_named_after_its_url_unless_named() {
    let env = env();
    let id = env.create("x").workspace.id;
    let clone = |name: Option<&str>, url: &str| {
        env.workspaces
            .add_repo(&id, name, RepoSource::Clone(url.into()))
            .unwrap()
    };

    clone(None, "https://example.invalid/org/lib.git");
    clone(None, "https://example.invalid/org/lib.git/");
    clone(None, "git@example.invalid:org/tool");
    let view = clone(Some("docs"), "https://example.invalid/org/site.git");

    assert_eq!(names(&view), vec!["docs", "lib", "lib-2", "tool"]);
    for repo in &view.repos {
        assert_eq!(repo.kind, RepoKind::Clone);
        assert_eq!(repo.branch.as_deref(), Some("main"));
    }
    assert_eq!(
        env.git.cloned_from(&view.repos[1].path).as_deref(),
        Some("https://example.invalid/org/lib.git")
    );
    let duplicate = env
        .workspaces
        .add_repo(
            &id,
            Some("docs"),
            RepoSource::Clone("https://example.invalid/x".into()),
        )
        .unwrap_err();
    assert!(
        matches!(duplicate, WorkspaceError::EntryExists(_)),
        "{duplicate:?}"
    );
}

// spec: DMN-025
#[test]
fn removing_a_link_deletes_only_the_symlink() {
    let env = env();
    let id = env.create("x").workspace.id;
    std::fs::write(env.repo.join("README.md"), "keep").unwrap();
    let path = env
        .workspaces
        .add_repo(&id, None, link(&env.repo))
        .unwrap()
        .repos[0]
        .path
        .clone();

    let view = env
        .workspaces
        .remove_repo(&id, "app", false, false)
        .unwrap();

    assert_eq!(view.repos, Vec::new());
    assert!(!exists(&path));
    assert_eq!(
        std::fs::read_to_string(env.repo.join("README.md")).unwrap(),
        "keep"
    );
    assert_eq!(env.git.toplevel(&env.repo).unwrap(), Some(env.repo.clone()));
}

// spec: DMN-025
#[test]
fn a_dirty_worktree_is_removed_only_when_forced_and_its_branch_is_kept() {
    let env = env();
    let id = env.create("x").workspace.id;
    let view = env
        .workspaces
        .add_repo(&id, None, worktree_of(&env.repo))
        .unwrap();
    let path = view.repos[0].path.clone();
    env.git.set_dirty(&path, true);

    let refused = env
        .workspaces
        .remove_repo(&id, "app", false, false)
        .unwrap_err();
    assert!(matches!(refused, WorkspaceError::Dirty(_)), "{refused:?}");
    assert!(path.is_dir());
    assert_eq!(env.git.worktrees(&env.repo).len(), 1);

    let view = env.workspaces.remove_repo(&id, "app", true, false).unwrap();
    assert_eq!(view.repos, Vec::new());
    assert!(!exists(&path));
    assert_eq!(env.git.worktrees(&env.repo), Vec::new());
    assert_eq!(env.git.branches(&env.repo), vec!["main", "omfp/x-1"]);
}

// spec: DMN-025
#[test]
fn the_worktree_branch_is_deleted_on_request() {
    let env = env();
    let id = env.create("x").workspace.id;
    env.workspaces
        .add_repo(&id, None, worktree_of(&env.repo))
        .unwrap();

    env.workspaces.remove_repo(&id, "app", false, true).unwrap();

    assert_eq!(env.git.worktrees(&env.repo), Vec::new());
    assert_eq!(env.git.branches(&env.repo), vec!["main"]);
}

// spec: DMN-025
#[test]
fn a_clone_is_deleted_only_when_clean_or_forced() {
    let env = env();
    let id = env.create("x").workspace.id;
    for url in [
        "https://example.invalid/lib",
        "https://example.invalid/tool",
    ] {
        env.workspaces
            .add_repo(&id, None, RepoSource::Clone(url.into()))
            .unwrap();
    }
    let folder = env.workspaces.get(&id).unwrap().path;
    env.git.set_dirty(&folder.join("lib"), true);

    assert_eq!(
        names(
            &env.workspaces
                .remove_repo(&id, "tool", false, false)
                .unwrap()
        ),
        vec!["lib"]
    );
    let refused = env
        .workspaces
        .remove_repo(&id, "lib", false, false)
        .unwrap_err();
    assert!(matches!(refused, WorkspaceError::Dirty(_)), "{refused:?}");
    assert!(folder.join("lib").is_dir());
    let view = env.workspaces.remove_repo(&id, "lib", true, false).unwrap();

    assert_eq!(view.repos, Vec::new());
    assert_eq!(entries(&folder), Vec::<String>::new());
}

// spec: DMN-025
#[test]
fn the_workspace_root_and_unknown_entries_are_not_removed() {
    let env = env();
    let view = env.create("mono");
    let (id, mono) = (view.workspace.id, view.workspace.path);
    fake_repo(&env.git, &mono, "main");

    let root = env
        .workspaces
        .remove_repo(&id, "mono", true, false)
        .unwrap_err();
    let unknown = env
        .workspaces
        .remove_repo(&id, "nope", true, false)
        .unwrap_err();

    assert!(matches!(root, WorkspaceError::IsRoot(_)), "{root:?}");
    assert!(
        matches!(unknown, WorkspaceError::NoSuchRepo { .. }),
        "{unknown:?}"
    );
    assert!(mono.is_dir());
    assert_eq!(names(&env.workspaces.rescan(&id).unwrap().0), vec!["mono"]);
}

// spec: DMN-025
#[test]
fn git_operations_are_serialized_per_repository() {
    let env = env();
    let other = fake_repo(&env.git, &env.root.join("repos").join("api"), "main");
    let [a, b, c] = ["a", "b", "c"].map(|name| env.create(name).workspace.id);
    let (entered_tx, entered) = mpsc::channel::<PathBuf>();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    // What happened, in order: each repository's entry into `worktree_add`, and the test's releases of `app`.
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let app = env.repo.clone();
    let log = Arc::clone(&order);
    env.git.on_worktree_add(move |repo| {
        let name = if repo == app { "app" } else { "api" };
        log.lock().unwrap().push(name.to_owned());
        entered_tx.send(repo.to_owned()).unwrap();
        if repo == app {
            release_rx.lock().unwrap().recv().unwrap();
        }
    });
    let wait = Duration::from_secs(10);
    let add = |id: &str, repo: &Path| env.workspaces.add_repo(id, None, worktree_of(repo));
    let release_app = || {
        order.lock().unwrap().push("release".to_owned());
        release.send(()).unwrap();
    };

    std::thread::scope(|scope| {
        let first = scope.spawn(|| add(&a, &env.repo));
        assert_eq!(entered.recv_timeout(wait), Ok(env.repo.clone()));
        let second = scope.spawn(|| add(&b, &env.repo));
        let parallel = scope.spawn(|| add(&c, &other));

        // Another repository proceeds while `app` is busy; the second `app` operation waits for the first, so it
        // enters only after the first is released (no wall-clock wait: the order is recorded).
        assert_eq!(entered.recv_timeout(wait), Ok(other.clone()));
        release_app();
        assert_eq!(entered.recv_timeout(wait), Ok(env.repo.clone()));
        release_app();

        for handle in [first, second, parallel] {
            handle.join().unwrap().unwrap();
        }
    });
    assert_eq!(
        *order.lock().unwrap(),
        ["app", "api", "release", "app", "release"]
    );
    let mut branches: Vec<String> = env
        .git
        .worktrees(&env.repo)
        .into_iter()
        .map(|(_, branch)| branch)
        .collect();
    branches.sort();
    assert_eq!(branches, vec!["omfp/a-1", "omfp/b-1"]);
}

// spec: DMN-026
#[test]
fn status_and_diff_are_reported_per_repository() {
    let env = env();
    let id = env.create("x").workspace.id;
    let view = env
        .workspaces
        .add_repo(&id, None, worktree_of(&env.repo))
        .unwrap();
    env.git.set_dirty(&view.repos[0].path, true);
    let repo = env.workspaces.repo(&id, "app").unwrap();

    let (status, upstream) = env.workspaces.status(&repo).unwrap();

    assert_eq!(upstream, None);
    assert_eq!(status.branch.as_deref(), Some("omfp/x-1"));
    assert_eq!((status.ahead, status.behind), (None, None));
    assert!(status.dirty);
    assert_eq!(env.workspaces.diff(&repo).unwrap(), "");
}

// spec: DMN-027
#[test]
fn the_same_checkout_is_seen_in_two_workspaces_and_threads_per_workspace() {
    let env = env();
    let a = env.create("a").workspace.id;
    let b = env.create("b").workspace.id;
    let threads = env.threads();
    let first = threads.create(NewThread::new("first", &a)).unwrap();
    let second = threads.create(NewThread::new("second", &a)).unwrap();

    let in_a = env.workspaces.add_repo(&a, None, link(&env.repo)).unwrap();
    let in_b = env
        .workspaces
        .add_repo(&b, Some("mine"), link(&env.repo))
        .unwrap();

    assert_ne!(in_a.repos[0].path, in_b.repos[0].path);
    assert_eq!(in_a.repos[0].canonical, env.repo);
    assert_eq!(in_b.repos[0].canonical, env.repo);
    assert_eq!(
        env.store.workspace_thread_ids(&a).unwrap(),
        vec![first.id, second.id]
    );
    assert_eq!(
        env.store.workspace_thread_ids(&b).unwrap(),
        Vec::<String>::new()
    );
}

// spec: DMN-028
#[test]
fn a_workspace_is_not_removed_while_a_thread_lives_in_it() {
    let env = env();
    let ws = env.create("x").workspace;
    let threads = env.threads();
    let thread = threads.create(NewThread::new("t", &ws.id)).unwrap();
    threads.archive(&thread.id).unwrap();

    let refused = env.workspaces.remove(&ws.id, true).unwrap_err();
    assert!(
        matches!(refused, WorkspaceError::HasThreads(_)),
        "{refused:?}"
    );
    assert!(ws.path.is_dir());
    assert_eq!(env.workspaces.list().unwrap(), vec![ws.clone()]);

    threads.delete(&thread.id).unwrap();
    env.workspaces.remove(&ws.id, false).unwrap();
    assert!(!ws.path.exists());
    let gone = env.workspaces.remove(&ws.id, false).unwrap_err();
    assert!(
        matches!(gone, WorkspaceError::Store(StoreError::NotFound { .. })),
        "{gone:?}"
    );
}

/// A workspace holding a link to `app`, a worktree of `api` and a clone `lib`, plus a plain file.
fn populated(env: &Env) -> (String, PathBuf, PathBuf) {
    let id = env.create("x").workspace.id;
    let api = fake_repo(&env.git, &env.root.join("repos").join("api"), "main");
    std::fs::write(env.repo.join("README.md"), "keep").unwrap();
    env.workspaces.add_repo(&id, None, link(&env.repo)).unwrap();
    env.workspaces
        .add_repo(&id, None, worktree_of(&api))
        .unwrap();
    let view = env
        .workspaces
        .add_repo(
            &id,
            None,
            RepoSource::Clone("https://example.invalid/lib".into()),
        )
        .unwrap();
    std::fs::write(view.workspace.path.join("notes.txt"), "x").unwrap();
    assert_eq!(names(&view), vec!["api", "app", "lib"]);
    (id, view.workspace.path, api)
}

// spec: DMN-028
#[test]
fn removing_a_workspace_removes_its_repositories_then_its_folder() {
    let env = env();
    let (id, folder, api) = populated(&env);

    env.workspaces.remove(&id, false).unwrap();

    assert!(!folder.exists());
    assert_eq!(
        std::fs::read_to_string(env.repo.join("README.md")).unwrap(),
        "keep"
    );
    assert_eq!(env.git.worktrees(&api), Vec::new());
    assert_eq!(env.git.branches(&api), vec!["main", "omfp/x-1"]);
    assert_eq!(env.workspaces.list().unwrap(), Vec::new());
    let gone = env.workspaces.get(&id).unwrap_err();
    assert!(
        matches!(gone, WorkspaceError::Store(StoreError::NotFound { .. })),
        "{gone:?}"
    );
}

// spec: DMN-028
#[test]
fn a_dirty_worktree_or_clone_keeps_the_workspace_unless_forced() {
    let env = env();
    let (id, folder, api) = populated(&env);
    let before = entries(&folder);

    for dirty in ["api", "lib"] {
        env.git.set_dirty(&folder.join("api"), dirty == "api");
        env.git.set_dirty(&folder.join("lib"), dirty == "lib");
        let refused = env.workspaces.remove(&id, false).unwrap_err();
        assert!(
            matches!(refused, WorkspaceError::Dirty(_)),
            "{dirty}: {refused:?}"
        );
        assert_eq!(entries(&folder), before);
        assert_eq!(env.git.worktrees(&api).len(), 1);
    }
    env.workspaces.remove(&id, true).unwrap();

    assert!(!folder.exists());
    assert!(env.repo.join("README.md").is_file());
    assert_eq!(env.git.worktrees(&api), Vec::new());
    assert_eq!(env.workspaces.list().unwrap(), Vec::new());
}

// spec: DMN-028
#[test]
fn nothing_outside_the_folder_is_deleted() {
    let env = env();
    let notes = env.root.join("notes");
    fake_repo(&env.git, &notes.join("lib"), "main");
    std::fs::write(notes.join("todo.txt"), "x").unwrap();
    let file = env.root.join("todo.txt");
    std::fs::write(&file, "x").unwrap();
    let ws = env.create("x").workspace;
    symlink(&env.repo, ws.path.join("app")).unwrap();
    symlink(&notes, ws.path.join("notes")).unwrap();
    symlink(&file, ws.path.join("todo.txt")).unwrap();
    // A link is never checked for changes: its target is not removed.
    env.git.set_dirty(&env.repo, true);

    env.workspaces.remove(&ws.id, false).unwrap();

    assert!(!exists(&ws.path));
    assert_eq!(env.workspaces.list().unwrap(), Vec::new());
    assert_eq!(entries(&notes), vec!["lib", "todo.txt"]);
    assert!(env.repo.is_dir());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
}

/// Writes `contents` to `path`, creating its folders.
fn put(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn found(
    env: &Env,
    id: &str,
    query: &str,
    mentioned: &[&str],
) -> Vec<(String, Option<String>, FileMatch, bool)> {
    let mentioned = mentioned.iter().map(|&path| path.to_owned()).collect();
    env.workspaces
        .find_file(id, query, &mentioned, false)
        .unwrap()
        .candidates
        .into_iter()
        .map(|candidate| {
            (
                candidate.path,
                candidate.repo,
                candidate.matched,
                candidate.mentioned,
            )
        })
        .collect()
}

fn candidate(
    path: &str,
    repo: Option<&str>,
    matched: FileMatch,
    mentioned: bool,
) -> (String, Option<String>, FileMatch, bool) {
    (path.into(), repo.map(Into::into), matched, mentioned)
}

/// A workspace linking `app` (the env's repository) and `lib` (another one), with files of its own.
fn find_env() -> (Env, String, PathBuf) {
    let env = env();
    let ws = env.create("find").workspace;
    let lib = fake_repo(&env.git, &env.root.join("repos").join("lib"), "main");
    put(&env.repo.join("src/theme.ts"), "app\n");
    put(&env.repo.join("src/app.ts"), "app\n");
    put(&lib.join("ui/theme.ts"), "lib\n");
    put(&lib.join("build/theme.ts"), "ignored\n");
    env.git.ignore(&lib, "build");
    put(&ws.path.join("notes/theme.ts"), "notes\n");
    put(&ws.path.join(".hidden/theme.ts"), "hidden\n");
    env.workspaces
        .add_repo(&ws.id, Some("app"), link(&env.repo))
        .unwrap();
    env.workspaces
        .add_repo(&ws.id, Some("lib"), link(&lib))
        .unwrap();
    (env, ws.id, ws.path)
}

// spec: DMN-092, UI-731
#[test]
fn workspace_folder_search_reaches_nested_matches_beyond_twenty_thousand_entries() {
    let (env, id, folder) = find_env();
    let large = folder.join("large");
    std::fs::create_dir_all(large.join("tail")).unwrap();
    for index in 0..20_001 {
        std::fs::File::create(large.join(format!("entry-{index:05}"))).unwrap();
    }
    put(&large.join("tail/needle.md"), "needle\n");
    let result = env
        .workspaces
        .find_file(&id, "lrg/tl/ndl", &BTreeSet::new(), true)
        .unwrap();
    assert_eq!(
        result
            .candidates
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["large/tail/needle.md"]
    );
}

// spec: DMN-092
#[test]
fn fuzzy_queries_search_every_repository_and_the_folder_without_changing_strict_lookup() {
    let (env, id, _) = find_env();
    let named = BTreeSet::new();
    let result = env
        .workspaces
        .find_file(&id, "THEM TS", &named, true)
        .unwrap();
    let paths: Vec<_> = result
        .candidates
        .iter()
        .map(|file| (file.path.as_str(), file.repo.as_deref()))
        .collect();
    assert_eq!(
        paths,
        [
            ("notes/theme.ts", None),
            ("app/src/theme.ts", Some("app")),
            ("lib/ui/theme.ts", Some("lib"))
        ]
    );
    assert!(
        result
            .candidates
            .iter()
            .all(|file| file.matched == FileMatch::Fuzzy)
    );
    let app = env
        .workspaces
        .find_file(&id, "APP THMTS", &named, true)
        .unwrap();
    assert_eq!(app.candidates[0].path, "app/src/theme.ts");
    assert_eq!(
        env.workspaces
            .find_file(&id, "THEM TS", &named, false)
            .unwrap()
            .candidates,
        Vec::new()
    );
}

// spec: DMN-092
#[test]
fn fuzzy_filtering_precedes_the_result_limit_so_later_repositories_remain_searchable() {
    let (env, id, _) = find_env();
    for index in 0..70 {
        put(&env.repo.join(format!("filler-{index:02}.ts")), "filler\n");
    }
    let named = BTreeSet::new();
    let initial = env.workspaces.find_file(&id, "", &named, true).unwrap();
    assert_eq!(initial.candidates.len(), MAX_CANDIDATES);
    assert!(initial.truncated);
    assert!(
        !initial
            .candidates
            .iter()
            .any(|file| file.repo.as_deref() == Some("lib"))
    );
    let narrowed = env
        .workspaces
        .find_file(&id, "LB U THMTS", &named, true)
        .unwrap();
    assert_eq!(
        narrowed
            .candidates
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["lib/ui/theme.ts"]
    );
    assert!(!narrowed.truncated);
}

// spec: DMN-092, DMN-312
#[test]
fn bare_and_partial_names_find_files_of_the_repositories_and_the_folder() {
    let (env, id, _) = find_env();

    // A bare name: every file of that name (git-ignored and hidden ones left out), shorter paths first.
    assert_eq!(
        found(&env, &id, "theme.ts", &[]),
        [
            candidate("notes/theme.ts", None, FileMatch::Basename, false),
            candidate("app/src/theme.ts", Some("app"), FileMatch::Basename, false),
            candidate("lib/ui/theme.ts", Some("lib"), FileMatch::Basename, false),
        ]
    );
    // A path relative to a repository is exact.
    assert_eq!(
        found(&env, &id, "ui/theme.ts", &[])[0],
        candidate("lib/ui/theme.ts", Some("lib"), FileMatch::Exact, false)
    );
    // A path relative to a repository is exact; the other files of that name follow.
    assert_eq!(
        found(&env, &id, "src/theme.ts", &[])[0],
        candidate("app/src/theme.ts", Some("app"), FileMatch::Exact, false)
    );
    // The end of a path matches at a component boundary.
    assert_eq!(
        found(&env, &id, "pp/src/app.ts", &[]),
        [candidate(
            "app/src/app.ts",
            Some("app"),
            FileMatch::Basename,
            false
        )]
    );
    assert_eq!(
        found(&env, &id, "./app/src/../src/app.ts", &[]),
        [candidate(
            "app/src/app.ts",
            Some("app"),
            FileMatch::Exact,
            false
        )]
    );
    assert_eq!(found(&env, &id, "missing.ts", &[]), []);
    let outside = env
        .workspaces
        .find_file(&id, "../x.ts", &BTreeSet::new(), false);
    assert!(
        matches!(outside, Err(WorkspaceError::OutsideFolder(_))),
        "{outside:?}"
    );
    // An absolute path outside the folder names no file of the workspace, even one named like them.
    assert_eq!(found(&env, &id, "/elsewhere/src/theme.ts", &[]), []);
}

// spec: DMN-092
#[test]
fn a_suffix_match_ranks_before_name_matches() {
    let (env, id, _) = find_env();
    put(&env.repo.join("deep/src/theme.ts"), "deep\n");
    assert_eq!(
        found(&env, &id, "deep/src/theme.ts", &[])[0].2,
        FileMatch::Exact
    );
    let ranked = found(&env, &id, "x/src/theme.ts", &[]);
    assert_eq!(ranked.len(), 4);
    let suffixed = found(&env, &id, "src/theme.ts", &[]);
    assert_eq!(
        suffixed[..2],
        [
            candidate("app/src/theme.ts", Some("app"), FileMatch::Exact, false),
            candidate(
                "app/deep/src/theme.ts",
                Some("app"),
                FileMatch::Suffix,
                false
            ),
        ]
    );
}

// spec: DMN-092
#[test]
fn files_the_thread_names_rank_first_among_equal_matches() {
    let (env, id, folder) = find_env();
    // Relative to the folder, to a repository, or absolute through the link's target.
    for mentioned in [
        "app/src/theme.ts".to_owned(),
        "src/theme.ts".to_owned(),
        env.repo.join("src/theme.ts").to_string_lossy().into_owned(),
        folder
            .join("app/src/theme.ts")
            .to_string_lossy()
            .into_owned(),
    ] {
        assert_eq!(
            found(&env, &id, "theme.ts", &[&mentioned])[0],
            candidate("app/src/theme.ts", Some("app"), FileMatch::Basename, true),
            "{mentioned}"
        );
    }

    let mut state = ThreadState::default();
    state.tools.push(ToolRun {
        tool_call_id: "c1".into(),
        tool_name: "read".into(),
        args: Some(json!({ "path": "app/src/theme.ts:3-9" })),
        intent: None,
        status: ToolStatus::Succeeded,
        partial_result: None,
        preview: None,
        result: None,
        background: None,
    });
    let named = named_files(&state);
    assert_eq!(
        env.workspaces
            .find_file(&id, "theme.ts", &named, false)
            .unwrap()
            .candidates[0]
            .path,
        "app/src/theme.ts"
    );
}

// spec: DMN-092
#[test]
fn at_most_fifty_candidates_are_returned() {
    let (env, id, _) = find_env();
    for index in 0..60 {
        put(&env.repo.join(format!("many/{index:02}/x.ts")), "x\n");
    }
    let result = env
        .workspaces
        .find_file(&id, "x.ts", &BTreeSet::new(), false)
        .unwrap();
    assert_eq!(result.candidates.len(), MAX_CANDIDATES);
    assert_eq!(MAX_CANDIDATES, 50);
    assert!(result.truncated);
    assert_eq!(result.candidates[0].path, "app/many/00/x.ts");
}
