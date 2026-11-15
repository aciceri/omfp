//! L2: reading and writing the files of a workspace (DMN-089, DMN-090) against real git repositories in temp dirs.

use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::Arc;

use omfp_daemon::git::GitCli;
use omfp_daemon::store::Store;
use omfp_daemon::workspaces::{
    IMAGE_CAP, NewWorkspace, RepoSource, TEXT_CAP, WorkspaceError, Workspaces,
};
use omfp_protocol::{FileContent, FileKind};
use omfp_testkit::TempRepo;
use tempfile::TempDir;

/// SHA-1 of `abc`.
const ABC: &str = "a9993e364706816aba3e25717850c26c9cd0d89d";

struct Env {
    dir: TempDir,
    workspaces: Workspaces,
    /// Linked into the workspace as `app`: a repository outside the workspace folder.
    app: TempRepo,
    id: String,
    /// The workspace folder.
    folder: PathBuf,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let workspaces_root = base.join("workspaces");
    let store = Arc::new(Store::open(&base.join("omfp.db"), &workspaces_root).unwrap());
    let workspaces = Workspaces::new(store, Arc::new(GitCli::default()), workspaces_root);
    let app = TempRepo::new().unwrap();
    app.commit_file("src/lib.rs", "fn a() {}\n", "lib").unwrap();
    let ws = workspaces
        .create(NewWorkspace {
            name: Some("files".into()),
            ..NewWorkspace::new()
        })
        .unwrap()
        .workspace;
    workspaces
        .add_repo(&ws.id, Some("app"), RepoSource::Link(app.path().to_owned()))
        .unwrap();
    Env {
        dir,
        workspaces,
        app,
        id: ws.id,
        folder: ws.path,
    }
}

impl Env {
    fn read(&self, path: &str) -> FileContent {
        self.workspaces.read_file(&self.id, path, None).unwrap()
    }

    fn read_error(&self, path: &str) -> WorkspaceError {
        self.workspaces.read_file(&self.id, path, None).unwrap_err()
    }

    fn write(
        &self,
        path: &str,
        content: &str,
        expected: Option<&str>,
    ) -> Result<String, WorkspaceError> {
        self.workspaces
            .write_file(&self.id, path, content, expected)
            .map(|written| written.hash)
    }

    fn app_file(&self, path: &str) -> PathBuf {
        self.app.path().join(path)
    }
}

// spec: DMN-089
#[test]
fn a_text_file_comes_with_its_hash_its_repository_and_its_head_text() {
    let env = env();
    std::fs::write(env.app_file("src/lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();

    let file = env.read("app/src/lib.rs");

    assert_eq!(
        file,
        FileContent {
            path: "app/src/lib.rs".into(),
            kind: FileKind::Text,
            size: 20,
            hash: file.hash.clone(),
            unchanged: false,
            content: Some("fn a() {}\nfn b() {}\n".into()),
            truncated: false,
            mime: None,
            repo: Some("app".into()),
            head: Some("fn a() {}\n".into()),
            head_hash: None,
            outside: false,
        }
    );
    std::fs::write(env.app_file("abc.txt"), "abc").unwrap();
    assert_eq!(env.read("app/abc.txt").hash, ABC, "hex SHA-1 of the bytes");
}

// spec: DMN-089
#[test]
fn a_relative_path_the_folder_lacks_is_looked_up_in_each_repository_in_scan_order() {
    let env = env();
    let zeta = TempRepo::new().unwrap();
    zeta.commit_file("only-in-zeta.txt", "z\n", "z").unwrap();
    env.workspaces
        .add_repo(
            &env.id,
            Some("zeta"),
            RepoSource::Link(zeta.path().to_owned()),
        )
        .unwrap();

    // Both repositories have README.md: `app` comes first.
    let readme = env.read("README.md");
    assert_eq!(
        (
            readme.path.as_str(),
            readme.repo.as_deref(),
            readme.head.as_deref()
        ),
        ("app/README.md", Some("app"), Some("test repository\n"))
    );
    assert_eq!(env.read("src/lib.rs").path, "app/src/lib.rs");
    assert_eq!(env.read("only-in-zeta.txt").path, "zeta/only-in-zeta.txt");
    // A file of the folder itself is found there first.
    std::fs::write(env.folder.join("README.md"), "the folder's\n").unwrap();
    let own = env.read("README.md");
    assert_eq!((own.path.as_str(), own.repo), ("README.md", None));
    // An absolute path is never looked up.
    let absolute = env.folder.join("only-in-zeta.txt");
    assert!(matches!(
        env.read_error(absolute.to_str().unwrap()),
        WorkspaceError::NoSuchFile(_)
    ));
}

// spec: DMN-089
#[test]
fn an_untracked_file_has_its_repository_but_no_head_and_files_outside_repositories_have_neither() {
    let env = env();
    std::fs::write(env.app_file("new.txt"), "new\n").unwrap();
    let new = env.read("app/new.txt");
    assert_eq!((new.repo.as_deref(), new.head), (Some("app"), None));

    std::fs::write(env.folder.join("notes.md"), "# notes\n").unwrap();
    let notes = env.read("notes.md");
    assert_eq!(
        (notes.repo, notes.head, notes.content.as_deref()),
        (None, None, Some("# notes\n"))
    );
}

// spec: DMN-089
#[test]
fn the_workspace_folder_s_own_repository_holds_the_files_outside_the_other_repositories() {
    let env = env();
    let git = |args: &[&str]| {
        let mut full = vec!["-C", env.folder.to_str().unwrap()];
        full.extend(args);
        env.app.git(&full).unwrap();
    };
    git(&["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(env.folder.join("plan.md"), "v1\n").unwrap();
    git(&["add", "--", "plan.md"]);
    git(&["commit", "--quiet", "--message", "plan"]);
    std::fs::write(env.folder.join("plan.md"), "v2\n").unwrap();
    env.workspaces.rescan(&env.id).unwrap();

    let plan = env.read("plan.md");
    assert_eq!(
        (plan.repo.as_deref(), plan.head.as_deref()),
        (Some("files"), Some("v1\n"))
    );
    assert_eq!(env.read("app/src/lib.rs").repo.as_deref(), Some("app"));
}

// spec: DMN-089
#[test]
fn binaries_carry_no_content_and_images_their_bytes_in_base64_with_their_mime_type() {
    let env = env();
    std::fs::write(env.app_file("data.bin"), [0_u8, 1, 2, 255]).unwrap();
    std::fs::write(env.app_file("latin1.txt"), [b'c', b'a', b'f', 0xe9]).unwrap();
    std::fs::write(env.app_file("logo.PNG"), [0x89, b'P', b'N', b'G']).unwrap();
    std::fs::write(env.app_file("icon.svg"), "<svg/>").unwrap();

    let binary = env.read("app/data.bin");
    assert_eq!(
        (binary.kind, binary.size, binary.content, binary.head),
        (FileKind::Binary, 4, None, None)
    );
    assert_eq!(env.read("app/latin1.txt").kind, FileKind::Binary);
    let png = env.read("app/logo.PNG");
    assert_eq!(
        (
            png.kind,
            png.mime.as_deref(),
            png.content.as_deref(),
            png.head
        ),
        (FileKind::Image, Some("image/png"), Some("iVBORw=="), None)
    );
    let svg = env.read("app/icon.svg");
    assert_eq!(
        (svg.kind, svg.mime.as_deref(), svg.content.as_deref()),
        (FileKind::Image, Some("image/svg+xml"), Some("PHN2Zy8+"))
    );

    std::fs::write(env.app_file("huge.jpg"), vec![0_u8; IMAGE_CAP + 1]).unwrap();
    let huge = env.read("app/huge.jpg");
    assert_eq!(
        (huge.kind, huge.mime.as_deref(), huge.content),
        (FileKind::Image, Some("image/jpeg"), None)
    );
}

// spec: DMN-255
#[test]
fn an_image_comes_with_the_hash_of_its_bytes_at_head_and_read_head_gives_them() {
    let env = env();
    env.app
        .commit_file("logo.png", "PNG at HEAD", "logo")
        .unwrap();
    std::fs::write(env.app_file("logo.png"), "PNG changed").unwrap();
    std::fs::write(env.app_file("new.png"), "PNG new").unwrap();
    std::fs::write(env.folder.join("loose.png"), "PNG loose").unwrap();
    let big = vec![b'x'; IMAGE_CAP + 1];
    env.app
        .commit_file("big.png", std::str::from_utf8(&big).unwrap(), "big")
        .unwrap();

    let logo = env.read("app/logo.png");
    let at_head = env.workspaces.read_head(&env.id, "app/logo.png").unwrap();
    assert_eq!(
        at_head,
        Some((b"PNG at HEAD".to_vec(), PathBuf::from("app/logo.png")))
    );
    // The same hash as `hash` would be for those bytes.
    std::fs::write(env.app_file("same.png"), "PNG at HEAD").unwrap();
    let same_hash = env.read("app/same.png").hash;
    assert_eq!(
        (logo.head_hash.as_deref(), logo.head),
        (Some(same_hash.as_str()), None)
    );
    assert_ne!(logo.head_hash.as_deref(), Some(logo.hash.as_str()));
    // Whatever its size: no content over the cap, the hash still.
    let big_file = env.read("app/big.png");
    assert_eq!(
        (big_file.content, big_file.head_hash.as_deref()),
        (None, Some(big_file.hash.as_str()))
    );
    assert_eq!(env.read("app/new.png").head_hash, None, "not at HEAD");
    assert_eq!(
        env.workspaces.read_head(&env.id, "app/new.png").unwrap(),
        None
    );
    assert_eq!(
        env.read("loose.png").head_hash,
        None,
        "outside repositories"
    );
    assert_eq!(
        env.workspaces.read_head(&env.id, "loose.png").unwrap(),
        None
    );
    assert_eq!(
        env.read("app/src/lib.rs").head_hash,
        None,
        "text has its head"
    );
    let unchanged = env
        .workspaces
        .read_file(&env.id, "app/logo.png", Some(&logo.hash))
        .unwrap();
    assert_eq!(unchanged.head_hash, None);
}

// spec: DMN-089
#[test]
fn long_text_is_cut_at_a_line_end_and_comes_without_head() {
    let env = env();
    let line = "0123456789abcdef\n";
    let text = line.repeat(TEXT_CAP / line.len() + 10);
    env.app.commit_file("big.txt", &text, "big").unwrap();

    let big = env.read("app/big.txt");

    assert!(big.truncated);
    assert_eq!(big.size, text.len() as u64);
    let content = big.content.unwrap();
    assert_eq!(content.len(), TEXT_CAP / line.len() * line.len());
    assert!(content.ends_with('\n'));
    assert_eq!(big.head, None);
}

// spec: DMN-089
#[test]
fn a_known_hash_still_matching_is_unchanged_without_content_or_head() {
    let env = env();
    let first = env.read("app/src/lib.rs");

    let again = env
        .workspaces
        .read_file(&env.id, "app/src/lib.rs", Some(&first.hash))
        .unwrap();

    assert_eq!(
        again,
        FileContent {
            unchanged: true,
            content: None,
            head: None,
            head_hash: None,
            ..first.clone()
        }
    );
    std::fs::write(env.app_file("src/lib.rs"), "changed\n").unwrap();
    let changed = env
        .workspaces
        .read_file(&env.id, "app/src/lib.rs", Some(&first.hash))
        .unwrap();
    assert!(!changed.unchanged);
    assert_eq!(changed.content.as_deref(), Some("changed\n"));
}

// spec: DMN-089, DMN-090, DMN-312, DMN-313
#[test]
fn relative_paths_leaving_the_workspace_folder_are_refused_and_no_write_leaves_it() {
    let env = env();
    let outside = env.dir.path().join("secret.txt");
    std::fs::write(&outside, "secret").unwrap();
    for path in ["../secret.txt", "app/../../secret.txt", "a/../../x"] {
        assert!(
            matches!(env.read_error(path), WorkspaceError::OutsideFolder(_)),
            "{path}"
        );
        assert!(
            matches!(
                env.write(path, "x", None),
                Err(WorkspaceError::OutsideFolder(_))
            ),
            "{path}"
        );
    }
    // Absolute paths outside the folder are read (DMN-310), never written.
    let absolute = [
        outside.to_str().unwrap().to_owned(),
        format!("{}/../../secret.txt", env.folder.display()),
    ];
    for path in &absolute {
        let hash = env.read(path).hash;
        let refused = env.write(path, "x", Some(&hash));
        assert!(
            matches!(&refused, Err(WorkspaceError::ReadOnly(_))),
            "{path}: {refused:?}"
        );
        assert!(
            refused
                .unwrap_err()
                .to_string()
                .ends_with("is outside the workspace folder: it can be read, not written"),
            "{path}"
        );
    }
    let new_file = env.dir.path().join("new.txt");
    assert!(matches!(
        env.write(new_file.to_str().unwrap(), "x", None),
        Err(WorkspaceError::ReadOnly(_))
    ));
    assert!(!new_file.exists());
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "secret");
    // Inside, `..` and absolute paths are fine, and a linked repository is followed.
    let absolute = env.folder.join("app/src/../src/lib.rs");
    assert_eq!(env.read(absolute.to_str().unwrap()).path, "app/src/lib.rs");
    assert_eq!(env.read("app/./src/../README.md").path, "app/README.md");
}

// spec: DMN-310
#[test]
fn an_absolute_path_outside_the_folder_is_read_as_it_is() {
    let env = env();
    let notes = env.dir.path().join("notes/todo.md");
    std::fs::create_dir_all(notes.parent().unwrap()).unwrap();
    std::fs::write(&notes, "abc").unwrap();
    let absolute = notes.to_str().unwrap().to_owned();

    // `..` is resolved; nothing is looked up in the workspace's repositories.
    let dotted = format!(
        "{}/../notes/./todo.md",
        env.dir.path().join("notes").display()
    );
    for path in [&absolute, &dotted] {
        assert_eq!(
            env.read(path),
            FileContent {
                path: absolute.clone(),
                kind: FileKind::Text,
                size: 3,
                hash: ABC.into(),
                unchanged: false,
                content: Some("abc".into()),
                truncated: false,
                mime: None,
                repo: None,
                head: None,
                head_hash: None,
                outside: true,
            },
            "{path}"
        );
    }
    let again = env
        .workspaces
        .read_file(&env.id, &absolute, Some(ABC))
        .unwrap();
    assert!(again.unchanged && again.outside && again.content.is_none());

    let missing = env.dir.path().join("notes/missing.md");
    assert!(matches!(
        env.read_error(missing.to_str().unwrap()),
        WorkspaceError::NoSuchFile(_)
    ));
    assert!(matches!(
        env.read_error(env.dir.path().join("notes").to_str().unwrap()),
        WorkspaceError::IsADirectory(_)
    ));
    let (_, shown) = env.workspaces.open_file(&env.id, &absolute).unwrap();
    assert_eq!(shown, notes);
    assert_eq!(env.workspaces.read_head(&env.id, &absolute).unwrap(), None);
}

// spec: DMN-311
#[test]
fn an_outside_file_in_a_git_working_tree_comes_with_its_head_from_that_repository() {
    let env = env();
    let other = TempRepo::new().unwrap();
    other
        .commit_file("docs/guide.md", "one\n", "guide")
        .unwrap();
    other
        .commit_file("logo.png", "png at head", "logo")
        .unwrap();
    std::fs::write(other.path().join("docs/guide.md"), "two\n").unwrap();
    std::fs::write(other.path().join("docs/new.md"), "new\n").unwrap();
    let name = other
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let at = |path: &str| other.path().join(path).to_str().unwrap().to_owned();

    let guide = env.read(&at("docs/guide.md"));
    assert_eq!(
        (guide.outside, guide.repo.as_deref(), guide.head.as_deref()),
        (true, Some(name.as_str()), Some("one\n"))
    );
    assert_eq!(guide.path, at("docs/guide.md"));
    let untracked = env.read(&at("docs/new.md"));
    assert_eq!(
        (untracked.repo.as_deref(), untracked.head),
        (Some(name.as_str()), None)
    );
    let logo = env.read(&at("logo.png"));
    assert_eq!(logo.head_hash.as_deref(), Some(logo.hash.as_str()));
    assert_eq!(
        env.workspaces
            .read_head(&env.id, &at("docs/guide.md"))
            .unwrap(),
        Some((b"one\n".to_vec(), other.path().join("docs/guide.md")))
    );
    // An unchanged file is not looked up in git again.
    let again = env
        .workspaces
        .read_file(&env.id, &at("docs/guide.md"), Some(&guide.hash))
        .unwrap();
    assert_eq!((again.unchanged, again.repo), (true, None));
}

// spec: DMN-315
#[test]
fn a_file_the_user_may_not_read_is_a_bad_request() {
    let env = env();
    let locked = env.dir.path().join("locked.txt");
    std::fs::write(&locked, "private").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&locked).is_ok() {
        // A superuser reads anything: nothing to check.
        return;
    }
    let error = env.read_error(locked.to_str().unwrap());
    assert!(matches!(&error, WorkspaceError::Unreadable(_)), "{error:?}");
    assert!(
        error
            .to_string()
            .ends_with("cannot be read: permission denied"),
        "{error}"
    );
}

// spec: DMN-089
#[test]
fn a_missing_file_is_not_found_and_a_directory_a_bad_request() {
    let env = env();
    assert!(matches!(
        env.read_error("app/nope.txt"),
        WorkspaceError::NoSuchFile(_)
    ));
    assert!(matches!(
        env.read_error("app/src"),
        WorkspaceError::IsADirectory(_)
    ));
    assert!(matches!(
        env.read_error("."),
        WorkspaceError::IsADirectory(_)
    ));
    assert!(matches!(
        env.write("app/src", "x", None),
        Err(WorkspaceError::IsADirectory(_))
    ));
}

// spec: DMN-090
#[test]
fn a_write_goes_through_the_link_to_the_repository_and_keeps_the_link() {
    let env = env();
    let read = env.read("app/src/lib.rs");

    let written = env
        .workspaces
        .write_file(&env.id, "app/src/lib.rs", "fn c() {}\n", Some(&read.hash))
        .unwrap();

    assert_eq!(
        (written.path.as_str(), written.size),
        ("app/src/lib.rs", 10)
    );
    assert_eq!(
        std::fs::read_to_string(env.app_file("src/lib.rs")).unwrap(),
        "fn c() {}\n"
    );
    let entry = std::fs::symlink_metadata(env.folder.join("app")).unwrap();
    assert!(entry.file_type().is_symlink());
    let again = env.read("app/src/lib.rs");
    assert_eq!(again.hash, written.hash);
    assert_eq!(again.head.as_deref(), Some("fn a() {}\n"));
    // A linked file is written through too.
    symlink(env.app_file("README.md"), env.folder.join("readme")).unwrap();
    let readme = env.read("readme");
    env.write("readme", "new readme\n", Some(&readme.hash))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(env.app_file("README.md")).unwrap(),
        "new readme\n"
    );
    assert!(
        std::fs::symlink_metadata(env.folder.join("readme"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    // Nothing is left behind in the target's folder.
    let mut names: Vec<_> = std::fs::read_dir(env.app_file("src"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, ["lib.rs"]);
}

// spec: DMN-090
#[test]
fn a_stale_hash_is_a_conflict_and_writes_nothing() {
    let env = env();
    let read = env.read("app/src/lib.rs");
    std::fs::write(env.app_file("src/lib.rs"), "changed meanwhile\n").unwrap();

    let refused = env.write("app/src/lib.rs", "mine\n", Some(&read.hash));

    assert!(
        matches!(refused, Err(WorkspaceError::FileChanged(_))),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(env.app_file("src/lib.rs")).unwrap(),
        "changed meanwhile\n"
    );
    std::fs::remove_file(env.app_file("src/lib.rs")).unwrap();
    assert!(matches!(
        env.write("app/src/lib.rs", "mine\n", Some(&read.hash)),
        Err(WorkspaceError::FileChanged(_))
    ));
    assert!(
        !env.app_file("src/lib.rs").exists(),
        "a removed file stays removed"
    );
}

// spec: DMN-090
#[test]
fn without_a_hash_only_a_new_file_is_written() {
    let env = env();
    let hash = env.write("app/abc.txt", "abc", None).unwrap();
    assert_eq!(hash, ABC);
    assert_eq!(
        std::fs::read_to_string(env.app_file("abc.txt")).unwrap(),
        "abc"
    );

    assert!(matches!(
        env.write("app/abc.txt", "other", None),
        Err(WorkspaceError::FileChanged(_))
    ));
    assert_eq!(
        std::fs::read_to_string(env.app_file("abc.txt")).unwrap(),
        "abc"
    );
    env.write("app/abc.txt", "other", Some(ABC)).unwrap();
    assert_eq!(
        std::fs::read_to_string(env.app_file("abc.txt")).unwrap(),
        "other"
    );
}

// spec: DMN-090
#[test]
fn a_write_keeps_the_file_s_permissions() {
    let env = env();
    let script = env.app_file("run.sh");
    std::fs::write(&script, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o750)).unwrap();
    let read = env.read("app/run.sh");

    env.write("app/run.sh", "#!/bin/sh\necho hi\n", Some(&read.hash))
        .unwrap();

    let mode = std::fs::metadata(&script).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o750);
}

// spec: DMN-090
#[test]
fn a_write_creates_no_folder() {
    let env = env();
    let refused = env.write("app/new/dir/file.txt", "x", None);
    assert!(
        matches!(refused, Err(WorkspaceError::NoSuchFolder(_))),
        "{refused:?}"
    );
    assert!(!env.app_file("new").exists());
    // A file where a folder should be is no folder either.
    assert!(matches!(
        env.write("app/README.md/file.txt", "x", None),
        Err(WorkspaceError::NoSuchFolder(_))
    ));
}

// spec: DMN-092
#[test]
fn find_file_lists_tracked_and_untracked_files_but_not_ignored_or_deleted_ones() {
    let env = env();
    env.app
        .commit_file(".gitignore", "target/\n", "ignore")
        .unwrap();
    env.app.commit_file("src/gone.rs", "x\n", "gone").unwrap();
    std::fs::remove_file(env.app_file("src/gone.rs")).unwrap();
    std::fs::create_dir_all(env.app_file("target")).unwrap();
    std::fs::write(env.app_file("target/lib.rs"), "built\n").unwrap();
    std::fs::write(env.app_file("src/new.rs"), "new\n").unwrap();
    // A file of the workspace folder itself, outside every repository.
    std::fs::write(env.folder.join("lib.rs"), "notes\n").unwrap();
    let find = |query: &str| -> Vec<String> {
        env.workspaces
            .find_file(&env.id, query, &std::collections::BTreeSet::new(), false)
            .unwrap()
            .candidates
            .into_iter()
            .map(|candidate| candidate.path)
            .collect()
    };

    assert_eq!(find("lib.rs"), ["lib.rs", "app/src/lib.rs"]);
    assert_eq!(find("src/new.rs"), ["app/src/new.rs"]);
    assert_eq!(find("gone.rs"), Vec::<String>::new());
}
