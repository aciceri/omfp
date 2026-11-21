//! Layout of the daemon's data directory (00-overview, "On-disk data"), and keeping it private to its owner
//! (DMN-180): the directory `0700`, the files the daemon and the desktop write in it `0600`.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, Permissions};
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::{
    DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The daemon's data directory and the paths inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// SQLite database.
    #[must_use]
    pub fn db(&self) -> PathBuf {
        self.root.join("omfp.db")
    }

    /// Access token file (DMN-003).
    #[must_use]
    pub fn token(&self) -> PathBuf {
        self.root.join("token")
    }

    /// Everything owned by one thread.
    #[must_use]
    pub fn thread(&self, thread_id: &str) -> PathBuf {
        self.root.join("threads").join(thread_id)
    }

    /// The thread's omp session directory (ARC-020).
    #[must_use]
    pub fn thread_sessions(&self, thread_id: &str) -> PathBuf {
        self.thread(thread_id).join("sessions")
    }

    /// The thread's omp stderr log (DMN-036).
    #[must_use]
    pub fn thread_log(&self, thread_id: &str) -> PathBuf {
        self.thread(thread_id).join("omp.log")
    }

    /// The `/` commands the thread's omp last announced, kept for a stopped thread (DMN-358).
    #[must_use]
    pub fn thread_commands(&self, thread_id: &str) -> PathBuf {
        self.thread(thread_id).join("commands.json")
    }

    /// The user's widget library, `user:<name>` (AGT-023).
    #[must_use]
    pub fn widgets(&self) -> PathBuf {
        self.root.join("widgets")
    }

    /// The daemon's omp extension root, passed to every omp as `--extension` (DMN-107); it holds only `agents/`.
    #[must_use]
    pub fn omp_extension(&self) -> PathBuf {
        self.root.join("omp")
    }

    /// The isolated omp user configuration and state (ARC-021), never imported from standalone omp except its provider
    /// logins, copied on request (DMN-230).
    #[must_use]
    pub fn omp_profile(&self) -> PathBuf {
        self.root.join("omp-profile")
    }

    /// The `widget-builder` agent definition (AGT-029, DMN-107).
    #[must_use]
    pub fn widget_builder(&self) -> PathBuf {
        self.omp_extension()
            .join("agents")
            .join("widget-builder.md")
    }

    /// The working directory, session directory and stderr log of the short-lived omp of `auth.*` (DMN-219).
    #[must_use]
    pub fn login(&self) -> PathBuf {
        self.root.join("login")
    }

    /// The working directory, session directory and stderr log of the omp answering `prompts.predict_word` (DMN-361).
    #[must_use]
    pub fn predictor(&self) -> PathBuf {
        self.root.join("predict")
    }
}

/// Group and other permission bits.
const SHARED_BITS: u32 = 0o077;

/// Creates `dir` (its missing parents with the default mode) with mode `0700` (DMN-180). An existing directory this
/// user owns that group or others may access is tightened to `0700`; its old mode is returned so the caller can say
/// so. A directory of another user is left as it is.
pub fn create_private_dir(dir: &Path) -> std::io::Result<Option<u32>> {
    let create = || std::fs::DirBuilder::new().mode(0o700).create(dir);
    let created = match create() {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent)?;
            }
            create()
        }
        other => other,
    };
    match created {
        Ok(()) => Ok(None),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let metadata = std::fs::metadata(dir)?;
            if !metadata.is_dir() {
                return Err(std::io::Error::new(
                    ErrorKind::AlreadyExists,
                    format!("{} is not a directory", dir.display()),
                ));
            }
            if metadata.uid() != rustix::process::getuid().as_raw() {
                return Ok(None);
            }
            tighten(dir, &metadata, 0o700)
        }
        Err(error) => Err(error),
    }
}

/// Restricts the file at `path` to this user (`0600`) when group or others may access it, returning its old mode
/// (DMN-003, DMN-180); a file of another user is an error, since only its owner could fix it.
pub fn make_private_file(path: &Path) -> std::io::Result<Option<u32>> {
    let metadata = std::fs::metadata(path)?;
    if metadata.uid() != rustix::process::getuid().as_raw() {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            format!("{} belongs to another user", path.display()),
        ));
    }
    tighten(path, &metadata, 0o600)
}

fn tighten(path: &Path, metadata: &std::fs::Metadata, mode: u32) -> std::io::Result<Option<u32>> {
    let old = metadata.permissions().mode() & 0o7777;
    if old & SHARED_BITS == 0 {
        return Ok(None);
    }
    std::fs::set_permissions(path, Permissions::from_mode(mode))?;
    Ok(Some(old))
}

/// [`OpenOptions`] creating a file with mode `0600` (DMN-180).
#[must_use]
pub fn private_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.mode(0o600);
    options
}

/// Opens the log file `path` for appending, created `0600`; an existing one others may read is tightened first
/// (DMN-180): `daemon.log`, a thread's `omp.log`.
pub fn open_private_log(path: &Path) -> std::io::Result<File> {
    let file = private_file_options()
        .create(true)
        .append(true)
        .open(path)?;
    let metadata = file.metadata()?;
    if metadata.permissions().mode() & SHARED_BITS != 0 {
        file.set_permissions(Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// Replaces the file `target` with `bytes` in one step (DMN-225): a temporary file in its folder, named after it,
/// this process and a per-call counter (so concurrent writes never share one), flushed to disk and renamed over it; a
/// failure removes the temporary file. The new file takes `permissions` when given, else those of the file it
/// replaces (a new file: the default mode). A missing folder is created.
pub fn replace_file(
    target: &Path,
    bytes: &[u8],
    permissions: Option<Permissions>,
) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let (Some(folder), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(std::io::Error::other(format!(
            "{} has no folder",
            target.display()
        )));
    };
    if !folder.as_os_str().is_empty() {
        std::fs::create_dir_all(folder)?;
    }
    let permissions = match permissions {
        Some(permissions) => Some(permissions),
        None => match std::fs::metadata(target) {
            Ok(metadata) => Some(metadata.permissions()),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        },
    };
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(
        ".omfp-{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = folder.join(temporary);
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_on_disk_layout() {
        let data = DataDir::new("/data/omfp");
        assert_eq!(data.root(), Path::new("/data/omfp"));
        assert_eq!(data.db(), Path::new("/data/omfp/omfp.db"));
        assert_eq!(data.token(), Path::new("/data/omfp/token"));
        assert_eq!(data.thread("T1"), Path::new("/data/omfp/threads/T1"));
        assert_eq!(
            data.thread_sessions("T1"),
            Path::new("/data/omfp/threads/T1/sessions")
        );
        assert_eq!(
            data.thread_log("T1"),
            Path::new("/data/omfp/threads/T1/omp.log")
        );
        assert_eq!(data.widgets(), Path::new("/data/omfp/widgets"));
        assert_eq!(data.omp_extension(), Path::new("/data/omfp/omp"));
        assert_eq!(
            data.widget_builder(),
            Path::new("/data/omfp/omp/agents/widget-builder.md")
        );
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    // spec: DMN-180
    #[test]
    fn the_data_directory_is_created_private_and_an_open_one_is_tightened() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("share/omfp");

        assert_eq!(create_private_dir(&data).unwrap(), None);
        assert_eq!(mode(&data), 0o700);
        assert_eq!(create_private_dir(&data).unwrap(), None);

        std::fs::set_permissions(&data, Permissions::from_mode(0o755)).unwrap();
        assert_eq!(create_private_dir(&data).unwrap(), Some(0o755));
        assert_eq!(mode(&data), 0o700);

        std::fs::write(dir.path().join("file"), "").unwrap();
        assert!(create_private_dir(&dir.path().join("file")).is_err());
    }

    // spec: DMN-180
    #[test]
    fn logs_are_opened_private_and_an_open_one_is_tightened() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("daemon.log");
        open_private_log(&log).unwrap();
        assert_eq!(mode(&log), 0o600);

        std::fs::set_permissions(&log, Permissions::from_mode(0o644)).unwrap();
        open_private_log(&log).unwrap();
        assert_eq!(mode(&log), 0o600);

        let other = dir.path().join("other");
        std::fs::write(&other, "").unwrap();
        std::fs::set_permissions(&other, Permissions::from_mode(0o640)).unwrap();
        assert_eq!(make_private_file(&other).unwrap(), Some(0o640));
        assert_eq!(make_private_file(&other).unwrap(), None);
        assert_eq!(mode(&other), 0o600);
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // spec: DMN-225
    #[test]
    fn a_replaced_file_keeps_its_mode_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config/config.toml");

        replace_file(&file, b"a = 1\n", None).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a = 1\n");

        std::fs::set_permissions(&file, Permissions::from_mode(0o640)).unwrap();
        replace_file(&file, b"a = 2\n", None).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a = 2\n");
        assert_eq!(mode(&file), 0o640);

        replace_file(&file, b"a = 3\n", Some(Permissions::from_mode(0o600))).unwrap();
        assert_eq!(mode(&file), 0o600);
        assert_eq!(entries(&dir.path().join("config")), vec!["config.toml"]);
    }

    // spec: DMN-225
    #[test]
    fn a_failed_replace_removes_its_temporary_file_and_keeps_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("views.json");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("inside"), "kept").unwrap();

        assert!(replace_file(&target, b"{}", None).is_err());
        assert_eq!(entries(dir.path()), vec!["views.json"]);
        assert_eq!(entries(&target), vec!["inside"]);
    }

    // spec: DMN-225
    #[test]
    fn concurrent_replaces_of_one_file_never_share_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("views.json");
        let texts: Vec<String> = (0..16).map(|n| format!("{{\"n\":{n}}}")).collect();
        std::thread::scope(|scope| {
            for text in &texts {
                let file = &file;
                scope.spawn(move || replace_file(file, text.as_bytes(), None).unwrap());
            }
        });
        assert!(texts.contains(&std::fs::read_to_string(&file).unwrap()));
        assert_eq!(entries(dir.path()), vec!["views.json"]);
    }
}
