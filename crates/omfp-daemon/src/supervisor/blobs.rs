//! The images of threads from before the isolated omp profile (ARC-021, DMN-385): omp keeps a session's images in its
//! content-addressed blob store, `<data dir>/blobs/<sha256>`, and the session file only `blob:sha256:<hash>`
//! references. Sessions omfp's threads wrote before the profile existed have their blobs in standalone omp's store, so
//! once, in the background at daemon start, the daemon links (else copies) the ones omfp's session files reference
//! into omfp's profile.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context as _;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use super::Supervisor;
use super::import::{data_dir, normalized, variable};
use crate::paths::DataDir;
use crate::store::Store;

/// omp's blob reference prefix (`session/blob-store.ts`).
const BLOB_REF: &str = "blob:sha256:";

/// The store setting recording that the migration ran (DMN-385).
pub(super) const MIGRATED_SETTING: &str = "omp_blobs_migrated";

/// What one run of the migration did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Outcome {
    /// Referenced blobs missing from omfp's profile.
    pub missing: usize,
    /// Files (blobs and their extension variants) hard-linked into omfp's profile.
    pub linked: usize,
    /// Files copied because they could not be linked (another file system).
    pub copied: usize,
    /// Missing blobs standalone omp's store does not hold either.
    pub unavailable: usize,
}

impl Supervisor {
    /// Runs the migration of DMN-385 in the background unless the store records it ran: it never delays the start,
    /// and its errors are logged, leaving it to the next start.
    pub fn spawn_blob_migration(&self, store: Arc<Store>) -> JoinHandle<()> {
        let data = self.inner.data.clone();
        let own_env = self.inner.config.omp_env.clone();
        let user_env = self.inner.config.user_omp_env.clone();
        tokio::task::spawn_blocking(move || match migrate(&data, &store, own_env, &user_env) {
            Ok(None) => {}
            Ok(Some(outcome)) => info!(
                missing = outcome.missing,
                linked = outcome.linked,
                copied = outcome.copied,
                unavailable = outcome.unavailable,
                "brought the images of older threads from standalone omp's blob store (DMN-385)"
            ),
            Err(error) => warn!(
                error = format!("{error:#}"),
                "cannot bring the images of older threads from standalone omp's blob store (DMN-385)"
            ),
        })
    }
}

/// The migration of DMN-385: `None` when the store records it ran, else what it did, recorded once it finished.
pub(super) fn migrate(
    data: &DataDir,
    store: &Store,
    own_env: Vec<(OsString, Option<OsString>)>,
    user_env: &[(OsString, Option<OsString>)],
) -> anyhow::Result<Option<Outcome>> {
    if store.setting(MIGRATED_SETTING)?.is_some() {
        return Ok(None);
    }
    let cwd = std::env::current_dir()?;
    let profile = std::path::absolute(data.omp_profile())?;
    // omfp's omp finds its blobs as omp does from the profile environment, which pins every variable involved.
    let own_env = crate::omp_profile::environment(data, own_env)?;
    let own = data_dir(
        |key| variable(&own_env, key, |key| std::env::var_os(key)),
        &cwd,
        Path::exists,
    )
    .context("no HOME for omfp's profile")?
    .join("blobs");
    let standalone = data_dir(
        |key| variable(user_env, key, |key| std::env::var_os(key)),
        &cwd,
        Path::exists,
    );
    let outcome = match standalone {
        // A daemon started from an omfp shell inherits omfp's own profile: no standalone omp.
        Some(dir) if !inside(&dir, &profile) => bring(
            &referenced(&data.root().join("threads"))?,
            &dir.join("blobs"),
            &own,
            link,
        )?,
        _ => Outcome::default(),
    };
    store.set_setting(MIGRATED_SETTING, &serde_json::json!(true))?;
    Ok(Some(outcome))
}

/// Whether `dir` is inside `profile`, compared as canonical paths when they exist.
fn inside(dir: &Path, profile: &Path) -> bool {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| normalized(path));
    canonical(dir).starts_with(canonical(profile))
}

/// The blob hashes the session files under `threads` (`<id>/sessions/…/*.jsonl`, subagent sessions included)
/// reference. Symbolic links are not followed.
pub(super) fn referenced(threads: &Path) -> std::io::Result<BTreeSet<String>> {
    let mut hashes = BTreeSet::new();
    let mut dirs = Vec::new();
    match std::fs::read_dir(threads) {
        Ok(entries) => {
            for entry in entries {
                let sessions = entry?.path().join("sessions");
                if sessions.is_dir() {
                    dirs.push(sessions);
                }
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(hashes),
        Err(error) => return Err(error),
    }
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                dirs.push(path);
            } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
                let bytes = std::fs::read(&path)?;
                collect(&String::from_utf8_lossy(&bytes), &mut hashes);
            }
        }
    }
    Ok(hashes)
}

/// Whether `bytes` is a blob hash as omp's `BLOB_HASH_RE` reads one: 64 lowercase hex digits.
fn is_hash(bytes: &[u8]) -> bool {
    bytes.len() == 64
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

/// Adds the hashes of the well-formed blob references in `text`.
fn collect(text: &str, hashes: &mut BTreeSet<String>) {
    for (at, _) in text.match_indices(BLOB_REF) {
        let rest = &text.as_bytes()[at + BLOB_REF.len()..];
        if let Some(hash) = rest.get(..64)
            && is_hash(hash)
            && !rest.get(64).is_some_and(u8::is_ascii_hexdigit)
        {
            hashes.insert(String::from_utf8_lossy(hash).into_owned());
        }
    }
}

/// Brings into `to` every blob of `hashes` that `to` lacks, with its extension variants (`<hash>.<ext>`), from
/// `from`: read only, never written. `place` links a file, [`link`] in production.
fn bring(
    hashes: &BTreeSet<String>,
    from: &Path,
    to: &Path,
    place: impl Fn(&Path, &Path) -> std::io::Result<Placed>,
) -> std::io::Result<Outcome> {
    let mut outcome = Outcome::default();
    let wanted: Vec<&String> = hashes
        .iter()
        .filter(|hash| !to.join(hash.as_str()).exists())
        .collect();
    outcome.missing = wanted.len();
    if wanted.is_empty() {
        return Ok(outcome);
    }
    let files = blob_files(from)?;
    for hash in wanted {
        let Some(names) = files.get(hash.as_str()) else {
            outcome.unavailable += 1;
            continue;
        };
        std::fs::create_dir_all(to)?;
        // The canonical blob last: an interrupted run leaves it missing, so the next one brings the variants too.
        for name in names.iter().rev() {
            let target = to.join(name);
            if target.exists() {
                continue;
            }
            match place(&from.join(name), &target) {
                Ok(Placed::Linked) => outcome.linked += 1,
                Ok(Placed::Copied) => outcome.copied += 1,
                Ok(Placed::Present) => {}
                // Standalone omp's garbage collection took it since it was listed.
                Err(error) if error.kind() == ErrorKind::NotFound && !from.join(name).exists() => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(outcome)
}

/// The files of the blob store `dir` by hash: the canonical `<hash>` first, then its `<hash>.<ext>` variants; omp's
/// staging files (`.<hash>….tmp`) and anything else are left out. An absent store holds none.
fn blob_files(dir: &Path) -> std::io::Result<BTreeMap<String, Vec<OsString>>> {
    let mut files: BTreeMap<String, Vec<OsString>> = BTreeMap::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(files),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(text) = name.to_str() else { continue };
        if text.len() < 64 || !is_hash(&text.as_bytes()[..64]) {
            continue;
        }
        // The first 64 bytes are ASCII: a character boundary.
        let (hash, ext) = text.split_at(64);
        if !(ext.is_empty() || (ext.starts_with('.') && ext.len() > 1)) {
            continue;
        }
        let names = files.entry(hash.to_owned()).or_default();
        if ext.is_empty() {
            names.insert(0, name);
        } else {
            names.push(name);
        }
    }
    files.retain(|hash, names| names.first().is_some_and(|name| *name == **hash));
    Ok(files)
}

/// How a file reached omfp's profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placed {
    Linked,
    Copied,
    /// Another writer (omp) put it there first: content-addressed, the same bytes.
    Present,
}

/// Hard-links `from` as `to`, else (another file system, a file system without hard links) copies it through a
/// staging file renamed into place, so `to` is never seen half written.
fn link(from: &Path, to: &Path) -> std::io::Result<Placed> {
    link_with(from, to, |from, to| std::fs::hard_link(from, to))
}

fn link_with(
    from: &Path,
    to: &Path,
    hard_link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<Placed> {
    /// Numbers the staging files of this process, which with its id makes omp's 16 hex digits.
    static STAGED: AtomicU64 = AtomicU64::new(0);
    match hard_link(from, to) {
        Ok(()) => return Ok(Placed::Linked),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(Placed::Present),
        Err(_) => {}
    }
    let staging = to.with_file_name(format!(
        ".{}.{:08x}{:08x}.tmp",
        to.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        STAGED.fetch_add(1, Ordering::Relaxed)
    ));
    let copied = std::fs::copy(from, &staging).and_then(|_| std::fs::rename(&staging, to));
    if copied.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    copied.map(|()| Placed::Copied)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    use std::path::PathBuf;

    use super::*;

    fn hash(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// Every file under `dir` with its bytes, mode, inode and modification time.
    fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>, u32, u64, i64, i64)> {
        let mut out = Vec::new();
        let mut dirs = vec![dir.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            let meta = std::fs::metadata(&dir).unwrap();
            out.push((
                dir.clone(),
                Vec::new(),
                meta.mode(),
                meta.ino(),
                meta.mtime(),
                meta.mtime_nsec(),
            ));
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let meta = std::fs::metadata(&path).unwrap();
                if meta.is_dir() {
                    dirs.push(path);
                } else {
                    let bytes = std::fs::read(&path).unwrap();
                    out.push((
                        path,
                        bytes,
                        meta.mode(),
                        meta.ino(),
                        meta.mtime(),
                        meta.mtime_nsec(),
                    ));
                }
            }
        }
        out.sort();
        out
    }

    /// A data directory whose threads' session files reference blobs 1..=4 (a subagent's session included), and a
    /// standalone omp at `home` holding blobs 1 (with a `.png` variant) and 2.
    struct Setup {
        dir: tempfile::TempDir,
        data: DataDir,
        store: Store,
        home: PathBuf,
    }

    impl Setup {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let data = DataDir::new(dir.path().join("data"));
            std::fs::create_dir_all(data.root()).unwrap();
            let store = Store::open(&data.db(), &dir.path().join("workspaces")).unwrap();
            let sessions = data.thread_sessions("01OLD");
            write(
                &sessions.join("2026-11-14_abc.jsonl"),
                &format!(
                    "{{\"type\":\"message\",\"content\":[{{\"type\":\"image\",\"data\":\"blob:sha256:{}\"}}]}}\n\
                     {{\"data\":\"blob:sha256:{}\"}}\n{{\"data\":\"blob:sha256:{}\"}}\n",
                    hash(1),
                    hash(3),
                    hash(4),
                ),
            );
            write(
                &sessions.join("2026-11-14_abc").join("0-Scout.jsonl"),
                &format!("{{\"data\":\"blob:sha256:{}\"}}\n", hash(2)),
            );
            let home = dir.path().join("home");
            let blobs = home.join(".omp").join("agent").join("blobs");
            write(&blobs.join(hash(1)), "one");
            std::fs::hard_link(blobs.join(hash(1)), blobs.join(format!("{}.png", hash(1))))
                .unwrap();
            write(&blobs.join(hash(2)), "two");
            write(&blobs.join(hash(9)), "unreferenced");
            // Blob 4 is in omfp's profile already.
            write(
                &data.omp_profile().join("agent").join("blobs").join(hash(4)),
                "four",
            );
            Self {
                dir,
                data,
                store,
                home,
            }
        }

        fn standalone_blobs(&self) -> PathBuf {
            self.home.join(".omp").join("agent").join("blobs")
        }

        fn own_blobs(&self) -> PathBuf {
            self.data.omp_profile().join("agent").join("blobs")
        }

        fn migrate(&self) -> Option<Outcome> {
            let own_env = vec![(
                "HOME".into(),
                Some(self.dir.path().join("omfp-home").into_os_string()),
            )];
            migrate(
                &self.data,
                &self.store,
                own_env,
                &omfp_testkit::standalone_omp_env(&self.home),
            )
            .unwrap()
        }
    }

    // spec: DMN-385
    #[test]
    fn the_session_files_of_omfps_threads_give_the_referenced_blobs() {
        let setup = Setup::new();
        let sessions = setup.data.thread_sessions("01NEW");
        write(
            &sessions.join("x.jsonl"),
            &format!(
                "blob:sha256:{} blob:sha256:{}0 blob:sha256:{} blob:sha256:ABC",
                hash(5),
                hash(6),
                hash(0xab).to_uppercase(),
            ),
        );
        write(
            &sessions.join("notes.txt"),
            &format!("blob:sha256:{}", hash(8)),
        );
        assert_eq!(
            referenced(&setup.data.root().join("threads")).unwrap(),
            [1, 2, 3, 4, 5]
                .map(hash)
                .into_iter()
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(
            referenced(&setup.dir.path().join("nothing")).unwrap(),
            BTreeSet::new()
        );
    }

    // spec: DMN-385, ARC-021
    #[test]
    fn missing_blobs_are_linked_from_standalone_omp_once_without_writing_there() {
        let setup = Setup::new();
        let standalone = setup.standalone_blobs();
        let before = tree(&setup.home);
        // Read only: nothing can be written in standalone omp's store.
        let mode = std::fs::metadata(&standalone).unwrap().permissions();
        std::fs::set_permissions(&standalone, std::fs::Permissions::from_mode(0o555)).unwrap();

        let outcome = setup.migrate();

        std::fs::set_permissions(&standalone, mode).unwrap();
        assert_eq!(
            outcome,
            Some(Outcome {
                missing: 3,
                linked: 3,
                copied: 0,
                unavailable: 1,
            })
        );
        let own = setup.own_blobs();
        for name in [hash(1), format!("{}.png", hash(1)), hash(2)] {
            assert_eq!(
                std::fs::metadata(own.join(&name)).unwrap().ino(),
                std::fs::metadata(standalone.join(&name)).unwrap().ino(),
                "{name} is linked"
            );
        }
        assert!(!own.join(hash(3)).exists());
        assert!(!own.join(hash(9)).exists(), "only referenced blobs");
        assert_eq!(std::fs::read(own.join(hash(4))).unwrap(), b"four");
        assert_eq!(
            tree(&setup.home),
            before,
            "standalone omp's tree is unchanged"
        );
        assert_eq!(
            setup.store.setting(MIGRATED_SETTING).unwrap(),
            Some(serde_json::json!(true))
        );

        // Recorded: a later start skips the scan, even with a blob gone again.
        std::fs::remove_file(own.join(hash(2))).unwrap();
        assert_eq!(setup.migrate(), None);
        assert!(!own.join(hash(2)).exists());
    }

    // spec: DMN-385
    #[test]
    fn bringing_again_places_nothing() {
        let setup = Setup::new();
        let hashes = referenced(&setup.data.root().join("threads")).unwrap();
        let (from, to) = (setup.standalone_blobs(), setup.own_blobs());
        assert_eq!(bring(&hashes, &from, &to, link).unwrap().linked, 3);
        assert_eq!(
            bring(&hashes, &from, &to, link).unwrap(),
            Outcome {
                missing: 1,
                linked: 0,
                copied: 0,
                unavailable: 1,
            }
        );
    }

    // spec: DMN-385
    #[test]
    fn a_blob_that_cannot_be_linked_is_copied_through_a_staging_file() {
        let setup = Setup::new();
        let (from, to) = (setup.standalone_blobs(), setup.own_blobs());
        let cross_device = |_: &Path, _: &Path| -> std::io::Result<()> {
            Err(std::io::Error::from(ErrorKind::CrossesDevices))
        };
        let place = |from: &Path, to: &Path| link_with(from, to, cross_device);
        let hashes = [hash(1)].into_iter().collect();
        assert_eq!(
            bring(&hashes, &from, &to, place).unwrap(),
            Outcome {
                missing: 1,
                linked: 0,
                copied: 2,
                unavailable: 0,
            }
        );
        assert_eq!(std::fs::read(to.join(hash(1))).unwrap(), b"one");
        assert_ne!(
            std::fs::metadata(to.join(hash(1))).unwrap().ino(),
            std::fs::metadata(from.join(hash(1))).unwrap().ino()
        );
        let mut names: Vec<_> = std::fs::read_dir(&to)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [hash(1), format!("{}.png", hash(1)), hash(4)].map(OsString::from)
        );
        // Another writer was first: the same bytes, nothing to do.
        assert_eq!(
            link(&from.join(hash(2)), &to.join(hash(1))).unwrap(),
            Placed::Present
        );
    }

    // spec: DMN-385
    #[test]
    fn without_a_standalone_store_or_inside_omfps_profile_nothing_is_brought_and_the_run_is_recorded()
     {
        let setup = Setup::new();
        std::fs::remove_dir_all(setup.home.join(".omp")).unwrap();
        assert_eq!(
            setup.migrate(),
            Some(Outcome {
                missing: 3,
                linked: 0,
                copied: 0,
                unavailable: 3,
            })
        );

        // A daemon started from an omfp shell: "standalone" omp is omfp's own profile.
        let setup = Setup::new();
        let mut user_env = omfp_testkit::standalone_omp_env(&setup.home);
        user_env.push((
            "PI_CODING_AGENT_DIR".into(),
            Some(setup.data.omp_profile().join("agent").into_os_string()),
        ));
        let own_env = vec![("HOME".into(), Some(setup.home.clone().into_os_string()))];
        assert_eq!(
            migrate(&setup.data, &setup.store, own_env, &user_env).unwrap(),
            Some(Outcome::default())
        );
        assert!(!setup.own_blobs().join(hash(1)).exists());
    }
}
