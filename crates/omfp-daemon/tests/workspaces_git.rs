//! L2: the workspace service and `GitCli` against real git repositories in temp dirs.

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use omfp_daemon::git::{ChangedFile, Git, GitCli, WorktreeBranch};
use omfp_daemon::store::Store;
use omfp_daemon::workspaces::{
    Discovered, NewWorkspace, RepoKind, RepoSource, WorkspaceError, WorkspaceView, Workspaces,
};
use omfp_testkit::TempRepo;
use tempfile::TempDir;

struct Env {
    _dir: TempDir,
    /// Canonical `--workspaces-root`.
    workspaces_root: PathBuf,
    repo: TempRepo,
    /// Canonical path of `repo`.
    root: PathBuf,
    git: GitCli,
    workspaces: Workspaces,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let workspaces_root = base.join("workspaces");
    let store = Arc::new(Store::open(&base.join("omfp.db"), &workspaces_root).unwrap());
    let repo = TempRepo::new().unwrap();
    let root = repo.path().canonicalize().unwrap();
    Env {
        _dir: dir,
        workspaces: Workspaces::new(store, Arc::new(GitCli::default()), workspaces_root.clone()),
        workspaces_root,
        repo,
        root,
        git: GitCli::default(),
    }
}

fn worktree(repo: &Path, branch: Option<&str>) -> RepoSource {
    RepoSource::Worktree {
        repo: repo.to_owned(),
        branch: branch.map(str::to_owned),
        base_ref: None,
    }
}

fn names(view: &WorkspaceView) -> Vec<&str> {
    view.repos.iter().map(|repo| repo.name.as_str()).collect()
}

fn dir_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

impl Env {
    /// Runs git with the test identity in `dir`.
    fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let mut full = vec!["-C", dir.to_str().unwrap()];
        full.extend(args);
        self.repo.git(&full).unwrap()
    }

    fn commit_in(&self, dir: &Path, file: &str, contents: &str) {
        std::fs::write(dir.join(file), contents).unwrap();
        self.git_in(dir, &["add", "--", file]);
        self.git_in(dir, &["commit", "--quiet", "--message", file]);
    }

    fn create(&self, name: &str) -> WorkspaceView {
        self.workspaces
            .create(NewWorkspace {
                name: Some(name.into()),
                ..NewWorkspace::new()
            })
            .unwrap()
    }

    fn head_of(&self, dir: &Path) -> String {
        self.git_in(dir, &["rev-parse", "HEAD"])
    }

    /// Working trees of the repository, main one first, from `git worktree list`.
    fn worktrees(&self) -> Vec<PathBuf> {
        self.repo
            .git(&["worktree", "list", "--porcelain"])
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(PathBuf::from)
            .collect()
    }

    fn branch_list(&self, branch: &str) -> String {
        self.repo.git(&["branch", "--list", branch]).unwrap()
    }
}

// spec: DMN-024, DMN-021
#[test]
fn only_a_real_repository_root_is_linked_and_a_worktree_is_suggested_by_its_main_repository() {
    let env = env();
    let id = env.create("links").workspace.id;
    let plain = TempDir::new().unwrap();
    std::fs::create_dir_all(env.root.join("src")).unwrap();

    let error = env
        .workspaces
        .add_repo(&id, None, RepoSource::Link(plain.path().to_owned()))
        .unwrap_err();
    assert!(
        matches!(error, WorkspaceError::NotARepository(_)),
        "{error:?}"
    );
    let error = env
        .workspaces
        .add_repo(&id, None, RepoSource::Link(env.root.join("src")))
        .unwrap_err();
    assert!(
        matches!(error, WorkspaceError::NotARepositoryRoot(_)),
        "{error:?}"
    );

    env.workspaces
        .add_repo(&id, None, worktree(&env.root.join("src"), None))
        .unwrap();
    let suggested: Vec<PathBuf> = env
        .workspaces
        .suggest("")
        .unwrap()
        .into_iter()
        .map(|repo| repo.path)
        .collect();
    assert_eq!(suggested, vec![env.root.clone()]);
}

// spec: DMN-022
#[test]
fn a_folder_with_a_real_clone_worktree_and_link_is_scanned() {
    let env = env();
    let ws = env.create("scan").workspace;
    let folder = ws.path.clone();
    symlink(&env.root, folder.join("app")).unwrap();
    let branch = WorktreeBranch::New {
        name: "topic".into(),
        base_ref: "main".into(),
    };
    env.git
        .worktree_add(&env.root, &folder.join("wt"), &branch)
        .unwrap();
    env.git
        .clone_repo(env.root.to_str().unwrap(), &folder.join("copy"))
        .unwrap();
    for ignored in [".omp", "notes/inner"] {
        std::fs::create_dir_all(folder.join(ignored)).unwrap();
        env.git_in(&folder.join(ignored), &["init", "--quiet"]);
    }
    std::fs::write(folder.join("README.md"), "x").unwrap();

    let (view, changed) = env.workspaces.rescan(&ws.id).unwrap();

    assert!(changed);
    let head = Some(env.head_of(&env.root));
    let repo = |name: &str, kind: RepoKind, branch: &str, canonical: PathBuf| Discovered {
        name: name.into(),
        path: folder.join(name),
        kind,
        branch: Some(branch.into()),
        head: head.clone(),
        is_root: false,
        canonical,
    };
    assert_eq!(
        view.repos,
        vec![
            repo(
                "app",
                RepoKind::Link {
                    target: env.root.clone()
                },
                "main",
                env.root.clone()
            ),
            repo("copy", RepoKind::Clone, "main", folder.join("copy")),
            repo(
                "wt",
                RepoKind::Worktree {
                    main: env.root.clone()
                },
                "topic",
                folder.join("wt")
            ),
        ]
    );
}

// spec: DMN-020, DMN-024
#[test]
fn a_workspace_created_with_a_real_repository_links_it_in_a_new_folder() {
    let env = env();
    let plain = TempDir::new().unwrap();
    for bad in [plain.path().to_owned(), env.root.join(".git")] {
        env.workspaces
            .create(NewWorkspace {
                link: Some(bad),
                ..NewWorkspace::new()
            })
            .unwrap_err();
    }
    assert!(!env.workspaces_root.exists());

    let view = env
        .workspaces
        .create(NewWorkspace {
            link: Some(env.repo.path().to_owned()),
            ..NewWorkspace::new()
        })
        .unwrap();

    let name = dir_name(&env.root);
    assert_eq!(view.workspace.name, name);
    let folder = view.workspace.path;
    assert_eq!(folder.parent(), Some(env.workspaces_root.as_path()));
    assert_eq!(
        view.repos,
        vec![Discovered {
            name: name.clone(),
            path: folder.join(&name),
            kind: RepoKind::Link {
                target: env.root.clone()
            },
            branch: Some("main".into()),
            head: Some(env.head_of(&env.root)),
            is_root: false,
            canonical: env.root.clone(),
        }]
    );
}

// spec: DMN-024, DMN-025
#[test]
fn a_real_worktree_is_added_and_removed() {
    let env = env();
    let id = env.create("Fix login").workspace.id;
    let folder = env.workspaces_root.join("fix-login");
    std::fs::create_dir_all(env.root.join("src")).unwrap();

    let view = env
        .workspaces
        .add_repo(&id, Some("app"), worktree(&env.root, None))
        .unwrap();
    let taken = env
        .workspaces
        .add_repo(&id, Some("app"), worktree(&env.root, None))
        .unwrap_err();
    let view_2 = env
        .workspaces
        .add_repo(&id, Some("app2"), worktree(&env.root.join("src"), None))
        .unwrap();

    assert_eq!(names(&view), vec!["app"]);
    assert!(matches!(taken, WorkspaceError::EntryExists(_)), "{taken:?}");
    assert_eq!(names(&view_2), vec!["app", "app2"]);
    assert_eq!(
        view_2.repos[1].kind,
        RepoKind::Worktree {
            main: env.root.clone()
        }
    );
    assert!(folder.join("app").join("README.md").is_file());
    let checked_out =
        |dir: &str| env.git_in(&folder.join(dir), &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(checked_out("app"), "omfp/fix-login-1");
    assert_eq!(checked_out("app2"), "omfp/fix-login-2");
    assert_eq!(
        env.worktrees(),
        vec![env.root.clone(), folder.join("app"), folder.join("app2")]
    );

    // Dirty: refused unless forced; the branch is kept.
    std::fs::write(folder.join("app").join("scratch.txt"), "wip\n").unwrap();
    let dirty = env
        .workspaces
        .remove_repo(&id, "app", false, false)
        .unwrap_err();
    assert!(matches!(dirty, WorkspaceError::Dirty(_)), "{dirty:?}");
    assert!(folder.join("app").join("scratch.txt").is_file());
    env.workspaces.remove_repo(&id, "app", true, false).unwrap();
    assert!(!folder.join("app").exists());
    assert_eq!(env.worktrees(), vec![env.root.clone(), folder.join("app2")]);
    assert_eq!(env.branch_list("omfp/fix-login-1"), "omfp/fix-login-1");

    // An unmerged branch is deleted on request.
    env.commit_in(&folder.join("app2"), "unmerged.txt", "work\n");
    let view = env
        .workspaces
        .remove_repo(&id, "app2", false, true)
        .unwrap();
    assert_eq!(view.repos, Vec::new());
    assert_eq!(env.branch_list("omfp/fix-login-2"), "");
}

// spec: DMN-024
#[test]
fn an_existing_branch_is_checked_out_in_a_real_worktree() {
    let env = env();
    let id = env.create("x").workspace.id;
    env.repo.git(&["branch", "feature"]).unwrap();

    let view = env
        .workspaces
        .add_repo(&id, Some("app"), worktree(&env.root, Some("feature")))
        .unwrap();

    assert_eq!(view.repos[0].branch.as_deref(), Some("feature"));
    assert_eq!(
        env.git_in(&view.repos[0].path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature"
    );
}

// spec: DMN-024, DMN-025
#[test]
fn a_local_repository_is_cloned_by_path_and_by_url() {
    let env = env();
    let id = env.create("x").workspace.id;
    let name = dir_name(&env.root);

    env.workspaces
        .add_repo(&id, None, RepoSource::Clone(env.root.display().to_string()))
        .unwrap();
    let view = env
        .workspaces
        .add_repo(
            &id,
            None,
            RepoSource::Clone(format!("file://{}", env.root.display())),
        )
        .unwrap();

    assert_eq!(names(&view), vec![name.clone(), format!("{name}-2")]);
    for repo in &view.repos {
        assert_eq!(repo.kind, RepoKind::Clone);
        assert_eq!(repo.branch.as_deref(), Some("main"));
        assert_eq!(repo.head, Some(env.head_of(&env.root)));
        assert!(repo.path.join("README.md").is_file());
        assert_eq!(
            env.git.upstream(&repo.path).unwrap().as_deref(),
            Some("origin/main")
        );
    }
    let view = env
        .workspaces
        .remove_repo(&id, &name, false, false)
        .unwrap();
    assert_eq!(names(&view), vec![format!("{name}-2")]);
    assert!(env.root.join("README.md").is_file());
}

// spec: DMN-026
#[test]
fn status_and_diff_cover_commits_against_the_upstream_and_uncommitted_changes() {
    let env = env();
    let id = env.create("x").workspace.id;
    env.workspaces
        .add_repo(
            &id,
            Some("copy"),
            RepoSource::Clone(env.root.display().to_string()),
        )
        .unwrap();
    let copy = env.workspaces_root.join("x").join("copy");
    env.commit_in(&copy, "feature.txt", "a\nb\n");
    env.repo
        .commit_file("other.txt", "main only\n", "advance main")
        .unwrap();
    env.git_in(&copy, &["fetch", "--quiet"]);
    std::fs::write(copy.join("README.md"), "test repository\nmore\n").unwrap();
    std::fs::write(copy.join("new.txt"), "x\n").unwrap();
    let repo = env.workspaces.repo(&id, "copy").unwrap();

    let (status, upstream) = env.workspaces.status(&repo).unwrap();

    assert_eq!(upstream.as_deref(), Some("origin/main"));
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!((status.ahead, status.behind), (Some(1), Some(1)));
    assert!(status.dirty);
    let file = |path: &str, additions| ChangedFile {
        path: path.into(),
        additions: Some(additions),
        deletions: Some(0),
    };
    assert_eq!(
        status.files,
        vec![
            file("README.md", 1),
            file("feature.txt", 2),
            file("new.txt", 1)
        ]
    );
    let diff = env.workspaces.diff(&repo).unwrap();
    for expected in ["+++ b/feature.txt", "+more", "+++ b/new.txt"] {
        assert!(
            diff.contains(expected),
            "{expected:?} missing from:\n{diff}"
        );
    }
    assert!(!diff.contains("other.txt"), "{diff}");
}

// spec: DMN-382
#[test]
fn a_repository_without_commits_reports_its_branch_and_every_file_as_added() {
    let env = env();
    let id = env.create("x").workspace.id;
    let fresh = env.workspaces_root.join("x").join("fresh");
    std::fs::create_dir_all(&fresh).unwrap();
    env.git_in(&fresh, &["init", "--quiet", "--initial-branch=trunk"]);
    std::fs::write(fresh.join("staged.txt"), "a\nb\n").unwrap();
    env.git_in(&fresh, &["add", "--", "staged.txt"]);
    std::fs::write(fresh.join("draft.txt"), "x\n").unwrap();
    env.workspaces.rescan(&id).unwrap();
    let repo = env.workspaces.repo(&id, "fresh").unwrap();

    let (status, upstream) = env.workspaces.status(&repo).unwrap();

    assert_eq!(upstream, None);
    assert_eq!(status.branch.as_deref(), Some("trunk"));
    assert_eq!((status.ahead, status.behind), (None, None));
    assert!(status.dirty);
    let file = |path: &str, additions| ChangedFile {
        path: path.into(),
        additions: Some(additions),
        deletions: Some(0),
    };
    assert_eq!(
        status.files,
        vec![file("draft.txt", 1), file("staged.txt", 2)]
    );
    let diff = env.workspaces.diff(&repo).unwrap();
    for expected in [
        "+++ b/staged.txt\n@@ -0,0 +1,2 @@\n+a\n+b\n",
        "+++ b/draft.txt\n@@ -0,0 +1 @@\n+x\n",
    ] {
        assert!(
            diff.contains(expected),
            "{expected:?} missing from:\n{diff}"
        );
    }
    assert_eq!(diff.matches("new file mode").count(), 2, "{diff}");

    // A base ref changes nothing before the first commit: there is nothing to count against.
    let against_base = env.git.status(&fresh, Some("origin/trunk")).unwrap();
    assert_eq!(against_base, status);
    assert_eq!(env.git.diff(&fresh, Some("origin/trunk")).unwrap(), diff);
}

// spec: DMN-028
#[test]
fn removing_a_workspace_removes_real_worktrees_and_keeps_link_targets() {
    let env = env();
    let id = env.create("x").workspace.id;
    env.workspaces
        .add_repo(&id, Some("link"), RepoSource::Link(env.root.clone()))
        .unwrap();
    let view = env
        .workspaces
        .add_repo(&id, Some("wt"), worktree(&env.root, None))
        .unwrap();
    let folder = view.workspace.path;

    env.workspaces.remove(&id, false).unwrap();

    assert!(!folder.exists());
    assert!(env.root.join("README.md").is_file());
    assert_eq!(env.worktrees(), vec![env.root.clone()]);
    assert_eq!(env.branch_list("omfp/x-1"), "omfp/x-1");
}

#[test]
fn git_cli_reports_toplevel_main_root_branch_head_and_upstream() {
    let env = env();
    let plain = TempDir::new().unwrap();
    let wt = env.workspaces_root.join("wt");
    std::fs::create_dir_all(&env.workspaces_root).unwrap();
    let branch = WorktreeBranch::New {
        name: "topic".into(),
        base_ref: "main".into(),
    };
    env.git.worktree_add(&env.root, &wt, &branch).unwrap();

    assert_eq!(env.git.toplevel(plain.path()).unwrap(), None);
    assert_eq!(env.git.toplevel(&env.root).unwrap(), Some(env.root.clone()));
    assert_eq!(env.git.toplevel(&wt).unwrap(), Some(wt.clone()));
    assert_eq!(env.git.main_root(&wt).unwrap(), env.root);
    assert_eq!(
        env.git.current_branch(&env.root).unwrap().as_deref(),
        Some("main")
    );
    assert_eq!(env.git.head(&wt).unwrap(), Some(env.head_of(&env.root)));
    assert_eq!(env.git.upstream(&env.root).unwrap(), None);
    assert_eq!(env.git.upstream(&wt).unwrap(), None);
    assert!(env.git.branch_exists(&env.root, "topic").unwrap());
    assert!(!env.git.branch_exists(&env.root, "nope").unwrap());
    assert!(!env.git.is_dirty(&wt).unwrap());
    std::fs::write(wt.join("README.md"), "changed\n").unwrap();
    assert!(env.git.is_dirty(&wt).unwrap());

    let empty = plain.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    env.git_in(&empty, &["init", "--quiet", "--initial-branch=main"]);
    assert_eq!(env.git.head(&empty).unwrap(), None);
    assert_eq!(
        env.git.current_branch(&empty).unwrap().as_deref(),
        Some("main")
    );

    env.git_in(&env.root, &["checkout", "--quiet", "--detach"]);
    assert_eq!(env.git.current_branch(&env.root).unwrap(), None);
    assert_eq!(env.git.upstream(&env.root).unwrap(), None);
    assert_eq!(env.git.head(&env.root).unwrap(), Some(env.head_of(&wt)));
}
