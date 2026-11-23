//! Standalone omp's provider logins, offered and copied into omfp's profile on request (ARC-021, DMN-229, DMN-230).
//!
//! The pinned omp's `omp auth export` opens its credential store read-write — it creates a missing one, migrates an
//! older schema and adds what its own schema lacks — so omfp never runs it on standalone omp's store. omfp finds that
//! store the way omp 18.22.0 does ([`credential_store`], from the user's environment: the daemon's with
//! [`SupervisorConfig::user_omp_env`](super::SupervisorConfig::user_omp_env)), copies its files under a SHARED lock
//! taken as SQLite takes it into a private snapshot directory under `<data-dir>/login/` ([`Snapshot`]) without SQLite ever opening
//! them there, and runs the export on that copy with omfp's profile environment; the snapshot is removed right
//! after. omfp's omp then stores the logins with `omp auth import -`, the
//! document going through a pipe only. omfp never reads a credential itself, and MCP server credentials
//! (`mcp:<server>`) stay behind: only model-provider logins are copied.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::future::Future;
use std::io::{Read as _, Seek as _};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use omfp_protocol::{OmpLogin, OmpLogins};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::{info, warn};

use super::{Result, Supervisor, SupervisorError, blocking};
use crate::omp::{LaunchSpec, StartError};

/// The export document version omfp reads (omp 18.22.0's `omp auth export`).
const EXPORT_VERSION: u64 = 1;

/// How long copying the store waits for standalone omp's writers: its lock, or a write-ahead log that moved.
const STORE_BUSY_TIMEOUT: Duration = Duration::from_secs(2);

/// The pause between two tries of the store's lock or of a copy whose write-ahead log moved meanwhile.
const STORE_RETRY: Duration = Duration::from_millis(5);

/// The bytes of a write-ahead log's header: SQLite rewrites them (new salts) whenever it restarts or truncates the
/// log, and only then (`wal.c`, "WAL Header Format").
const WAL_HEADER: u64 = 32;

/// The provider prefix of an MCP server's credential, which is no model-provider login.
const MCP_PREFIX: &str = "mcp:";

/// The name of omp's credential store in its agent directory (omp 18.22.0's `getAgentDbPath`).
const STORE: &str = "agent.db";

/// omp's `auth export` document; fields omfp does not know pass through untouched to `auth import`.
#[derive(Serialize, Deserialize)]
struct Export {
    version: Value,
    #[serde(default)]
    credentials: Vec<Entry>,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

/// One stored credential of the export.
#[derive(Serialize, Deserialize)]
struct Entry {
    provider: String,
    credential: Value,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

impl Export {
    /// Parses omp's document: version 1, without MCP server credentials. Errors never quote the document, which
    /// holds secrets; a version is named only when it is a number.
    fn parse(stdout: &str) -> Result<Self> {
        let unreadable = || {
            omp_error(
                "auth export",
                "its output is not the login export omfp reads".into(),
            )
        };
        let value: Value = serde_json::from_str(stdout).map_err(|_| unreadable())?;
        let version = value.get("version").unwrap_or(&Value::Null);
        if version.as_u64() != Some(EXPORT_VERSION) {
            let version = if version.is_number() {
                format!("version {version}")
            } else {
                "an unknown version".to_owned()
            };
            return Err(omp_error(
                "auth export",
                format!("its login export is {version}; omfp reads version {EXPORT_VERSION}"),
            ));
        }
        let mut export: Self = serde_json::from_value(value).map_err(|_| unreadable())?;
        export
            .credentials
            .retain(|entry| !entry.provider.starts_with(MCP_PREFIX));
        Ok(export)
    }

    /// Its providers in first-seen order, each once, `oauth` when any of its logins is an OAuth one.
    fn logins(&self) -> OmpLogins {
        let mut providers: Vec<OmpLogin> = Vec::new();
        for entry in &self.credentials {
            let oauth = entry.credential.get("type").and_then(Value::as_str) == Some("oauth");
            match providers
                .iter_mut()
                .find(|login| login.provider_id == entry.provider)
            {
                Some(login) => login.oauth |= oauth,
                None => providers.push(OmpLogin {
                    provider_id: entry.provider.clone(),
                    oauth,
                }),
            }
        }
        OmpLogins { providers }
    }
}

/// What standalone omp has to copy.
enum Standalone<T> {
    /// No standalone omp: no credential store where it looks, or the store is omfp's own profile's.
    Absent,
    /// Its store cannot be copied (the reason, never its content).
    Unreadable(String),
    /// The `omp auth export` of its snapshot.
    Exported(T),
}

fn omp_error(command: &str, message: String) -> SupervisorError {
    SupervisorError::Omp {
        command: command.to_owned(),
        message,
        code: None,
    }
}

/// A failed `omp auth …`: omp's stderr (never its stdout) as `omp_error`.
fn auth_error(error: StartError) -> SupervisorError {
    match error {
        StartError::Auth { command, stderr } => omp_error(command, stderr),
        other => omp_error("auth", other.to_string()),
    }
}

fn no_logins() -> OmpLogins {
    OmpLogins {
        providers: Vec::new(),
    }
}

/// Runs `operation` as a task of its own and waits for it: a caller that goes away (an MCP stream closed) neither ends
/// it halfway nor releases what it holds before it is done.
async fn detached<T: Send + 'static>(
    operation: impl Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    tokio::spawn(operation)
        .await
        .map_err(|error| SupervisorError::Internal(error.to_string()))?
}

/// The value `key` has in the environment `overrides` make of `inherited` (the last override wins; `None` unsets).
pub(super) fn variable(
    overrides: &[(OsString, Option<OsString>)],
    key: &str,
    inherited: impl FnOnce(&str) -> Option<OsString>,
) -> Option<OsString> {
    match overrides.iter().rev().find(|(name, _)| name == key) {
        Some((_, value)) => value.clone(),
        None => inherited(key),
    }
}

/// `path` with `.` and `..` resolved lexically, as node's `path.normalize` does.
pub(super) fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if out.file_name().is_some() {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// node's `path.join(base, rest)`: `rest` joins `base` even when it is absolute.
fn node_join(base: &OsStr, rest: &OsStr) -> PathBuf {
    let mut joined = base.to_owned();
    joined.push("/");
    joined.push(rest);
    normalized(Path::new(&joined))
}

/// node's `path.resolve(cwd, path)`.
fn resolve(cwd: &Path, path: &Path) -> PathBuf {
    normalized(&cwd.join(path))
}

/// omp's profile name of `value` (`normalizeProfileName`): `None` for the default profile — empty or `default` — and
/// for a name omp refuses: omp's directory resolution falls back to the default for it (its CLI then refuses to
/// start, so the user's omp has no store of that name).
fn profile_name(value: &OsStr) -> Option<String> {
    let name = value.to_str()?.trim();
    let bytes = name.as_bytes();
    let first = bytes.first()?;
    let valid = name != "default"
        && bytes.len() <= 64
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        && !name.ends_with('.');
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    (valid && !reserved).then(|| name.to_owned())
}

/// The directory of the data files (`agent.db`, `blobs/`, `sessions/`, …) of the omp whose environment `var` gives,
/// `cwd` being its working directory, mirroring omp 18.22.0's `DirResolver` and its `agentSubdir(…, "data")`
/// (`getAgentDbPath`, `getBlobsDir`; `packages/utils/src/dirs.ts`; re-check on an omp upgrade, ENV-020):
/// - the base root is `path.join(HOME, PI_CONFIG_DIR || ".omp")`;
/// - the profile is `OMP_PROFILE` when set (empty: the default), else `PI_PROFILE`; a refused name is the default;
/// - a named profile's root is `<base>/profiles/<name>`, its agent directory `<root>/agent`; the default profile's
///   agent directory is `path.resolve(PI_CODING_AGENT_DIR)` when that is set and not, as a string, `PI_PROFILE`'s own
///   agent directory, else `<root>/agent`;
/// - when the agent directory is the root's own, an existing `$XDG_DATA_HOME/omp` (a named profile's
///   `$XDG_DATA_HOME/omp/profiles/<name>`) holds the data instead.
///
/// omp's `.env` files, which may set these variables too, are not read. `None` without a `HOME`.
pub(super) fn data_dir(
    var: impl Fn(&str) -> Option<OsString>,
    cwd: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let set = |key: &str| var(key).filter(|value| !value.is_empty());
    let home = set("HOME")?;
    let config_dir = set("PI_CONFIG_DIR").unwrap_or_else(|| ".omp".into());
    let base_root = node_join(&home, &config_dir);
    let profile_root = |profile: &str| base_root.join("profiles").join(profile);
    let pi_profile = var("PI_PROFILE").as_deref().and_then(profile_name);
    let profile = match var("OMP_PROFILE") {
        Some(value) => profile_name(&value),
        None => pi_profile.clone(),
    };
    let config_root = profile
        .as_deref()
        .map_or_else(|| base_root.clone(), profile_root);
    let default_agent = config_root.join("agent");
    let profile_derived = |dir: &OsStr| {
        pi_profile.as_deref().is_some_and(|pi| {
            dir.as_encoded_bytes()
                == profile_root(pi)
                    .join("agent")
                    .as_os_str()
                    .as_encoded_bytes()
        })
    };
    let agent_dir = match set("PI_CODING_AGENT_DIR") {
        Some(dir) if profile.is_none() && !profile_derived(&dir) => resolve(cwd, Path::new(&dir)),
        _ => default_agent.clone(),
    };
    let data = (agent_dir.as_os_str() == default_agent.as_os_str())
        .then(|| set("XDG_DATA_HOME"))
        .flatten()
        .and_then(|xdg| {
            let app = node_join(&xdg, OsStr::new("omp"));
            let dir = match &profile {
                Some(profile) => app.join("profiles").join(profile),
                None => app,
            };
            exists(&resolve(cwd, &dir)).then_some(dir)
        });
    Some(resolve(cwd, &data.unwrap_or(agent_dir)))
}

/// The credential store (`agent.db`) the user's omp opens: `agent.db` in its [`data_dir`].
fn credential_store(
    var: impl Fn(&str) -> Option<OsString>,
    cwd: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    data_dir(var, cwd, exists).map(|dir| dir.join(STORE))
}

/// Why the store could not be copied: never its content.
#[derive(Debug)]
enum CopyError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    /// Standalone omp kept writing for [`STORE_BUSY_TIMEOUT`].
    Busy,
    /// The store went away since it was found.
    Gone,
    /// Standalone omp left a multi-database transaction to roll back ([`names_super_journal`]).
    SuperJournal,
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Sqlite(error) => error.fmt(f),
            Self::Busy => f.write_str("standalone omp kept writing it; try again"),
            Self::Gone => f.write_str("it went away"),
            Self::SuperJournal => f.write_str(
                "standalone omp left an unfinished multi-database transaction; run omp once, then try again",
            ),
        }
    }
}

impl From<std::io::Error> for CopyError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for CopyError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// `path` with `suffix` appended to its name (`agent.db` → `agent.db-wal`).
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// SQLite's lock bytes in a database file (`os_unix.c`): `PENDING_BYTE`, then `RESERVED_BYTE`, then the
/// `SHARED_SIZE` bytes from `SHARED_FIRST` that readers lock shared and a writer locks exclusively to write the file.
const PENDING_BYTE: i64 = 0x4000_0000;
const SHARED_FIRST: i64 = PENDING_BYTE + 2;
const SHARED_SIZE: i64 = 510;

/// A POSIX record lock's kind: `libc`'s `F_RDLCK` and `F_UNLCK` are `c_int` on Linux and `c_short` on macOS.
#[derive(Clone, Copy)]
enum RecordLock {
    Read,
    Unlock,
}

/// Sets (`kind`) a POSIX record lock on `len` bytes of `file` from `start`, without waiting.
fn record_lock(file: &File, kind: RecordLock, start: i64, len: i64) -> nix::Result<()> {
    use nix::fcntl::{FcntlArg, fcntl};
    use nix::libc;
    use std::os::fd::AsRawFd as _;
    let kind = match kind {
        RecordLock::Read => libc::F_RDLCK,
        RecordLock::Unlock => libc::F_UNLCK,
    };
    let lock = libc::flock {
        l_type: libc::c_short::try_from(kind).map_err(|_| nix::errno::Errno::EINVAL)?,
        l_whence: libc::c_short::try_from(libc::SEEK_SET).map_err(|_| nix::errno::Errno::EINVAL)?,
        l_start: start,
        l_len: len,
        l_pid: 0,
    };
    fcntl(file.as_raw_fd(), FcntlArg::F_SETLK(&lock)).map(drop)
}

/// Takes SQLite's own SHARED lock on `file`, the database file of standalone omp's store, as `unixLock` does: a read
/// lock on `PENDING_BYTE` (refused while a writer waits for EXCLUSIVE), then a read lock on the shared range, then
/// `PENDING_BYTE` released; it waits until `deadline` while a writer holds PENDING or EXCLUSIVE. Held, it lets
/// readers and a writer's RESERVED lock be, but no writer gets EXCLUSIVE: in rollback-journal mode the database file
/// holds a committed state and nobody rolls a hot journal back meanwhile, and the last connection of a write-ahead-log
/// store cannot checkpoint and delete its log. `RESERVED_BYTE` stays free, so another process's check for a hot
/// journal (`unixCheckReservedLock`) is not misled. Like SQLite's, it is a POSIX record lock (`F_SETLK`): it
/// conflicts with other processes' locks only, and a process loses it when it closes any descriptor of the file, so
/// the daemon copies one store at a time ([`COPIES`]) and reads the database file through `file` only. It goes
/// with `file`.
fn lock_shared(file: &File, deadline: Instant) -> Result<(), CopyError> {
    use nix::errno::Errno;
    loop {
        let taken = record_lock(file, RecordLock::Read, PENDING_BYTE, 1).and_then(|()| {
            let shared = record_lock(file, RecordLock::Read, SHARED_FIRST, SHARED_SIZE);
            record_lock(file, RecordLock::Unlock, PENDING_BYTE, 1)?;
            shared
        });
        match taken {
            Ok(()) => return Ok(()),
            Err(Errno::EAGAIN | Errno::EACCES) => {}
            Err(errno) => return Err(CopyError::Io(errno.into())),
        }
        if Instant::now() >= deadline {
            return Err(CopyError::Busy);
        }
        std::thread::sleep(STORE_RETRY);
    }
}

/// The byte of a write-ahead log's index (`-shm`) a checkpoint locks exclusively (`os_unix.c` `UNIX_SHM_BASE`,
/// `(22 + SQLITE_SHM_NLOCK) * 4`, plus `wal.c` `WAL_CKPT_LOCK`, 1).
const SHM_CHECKPOINT_BYTE: i64 = (22 + 8) * 4 + 1;

/// Keeps standalone omp from checkpointing its write-ahead log, while the returned file is held: a read lock on the
/// checkpoint byte of `shm` (opened read-only: nothing is created or written), which every checkpoint of another
/// process needs exclusively (`sqlite3WalCheckpoint`: `walLockExclusive(pWal, WAL_CKPT_LOCK, 1)`; an automatic
/// checkpoint that cannot take it is skipped, writers go on). Waits until `deadline` while a checkpoint runs. `None`
/// when there is no `shm`: no connection has the log open (each maps its index there), and none can delete it while
/// [`lock_shared`] is held, so one that opens meanwhile leaves it behind for [`Snapshot::copy_with`] to see.
fn hold_checkpoints(shm: &Path, deadline: Instant) -> Result<Option<File>, CopyError> {
    use nix::errno::Errno;
    let file = match File::open(shm) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(CopyError::Io(error)),
    };
    loop {
        match record_lock(&file, RecordLock::Read, SHM_CHECKPOINT_BYTE, 1) {
            Ok(()) => return Ok(Some(file)),
            Err(Errno::EAGAIN | Errno::EACCES) => {}
            Err(errno) => return Err(CopyError::Io(errno.into())),
        }
        if Instant::now() >= deadline {
            return Err(CopyError::Busy);
        }
        std::thread::sleep(STORE_RETRY);
    }
}

/// Serializes the copies of the daemon: a second copy of the same store closing its descriptor would drop the first
/// one's lock ([`lock_shared`]).
static COPIES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The header of the write-ahead log at `path` (its first [`WAL_HEADER`] bytes, fewer when shorter), `None` when
/// there is none.
fn wal_header(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match File::open(path) {
        Ok(file) => {
            let mut header = Vec::new();
            file.take(WAL_HEADER).read_to_end(&mut header)?;
            Ok(Some(header))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Copies the file at `from`, opened read-only, to `to` when it exists, else removes `to`; answers the bytes copied,
/// `None` without a file.
fn copy_file(from: &Path, to: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let bytes = match File::open(from) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            bytes
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return match std::fs::remove_file(to) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(None),
            };
        }
        Err(error) => return Err(error),
    };
    std::fs::write(to, &bytes)?;
    Ok(Some(bytes))
}

/// The first [`WAL_HEADER`] bytes of `bytes`.
fn header_of(bytes: &[u8]) -> Vec<u8> {
    let header = usize::try_from(WAL_HEADER).map_or(bytes.len(), |header| header.min(bytes.len()));
    bytes[..header].to_vec()
}

/// The 8 bytes every rollback journal header starts with, and a journal ends with when it names a super-journal
/// (`pager.c`, `aJournalMagic`, "Super-journal name").
const JOURNAL_MAGIC: [u8; 8] = [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7];

/// Whether the rollback journal `journal`, one SQLite would play back, ends with a super-journal's name: a multi-database transaction, whose
/// rollback reads, and may delete, the super-journal beside standalone omp's store.
fn names_super_journal(journal: &[u8]) -> bool {
    // A journal SQLite plays back starts with the magic; a persisted one whose header was zeroed is not hot, whatever
    // an older transaction left at its end.
    journal.len() > 2 * JOURNAL_MAGIC.len()
        && journal.starts_with(&JOURNAL_MAGIC)
        && journal.ends_with(&JOURNAL_MAGIC)
}

/// The directory a [`Snapshot`] stages standalone omp's files in, removed with this value.
struct Stage(PathBuf);

impl Stage {
    /// A new, empty `dir`; what an earlier copy left there goes first.
    fn create(dir: PathBuf) -> std::io::Result<Self> {
        match std::fs::remove_dir_all(&dir) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        std::fs::create_dir(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            warn!(%error, dir = %self.0.display(), "cannot remove the staged copy of omp's logins");
        }
    }
}

/// A private copy of standalone omp's credential store: a fresh directory of its own (0700) under `<data-dir>/login/`,
/// holding `agent.db`, removed with this value — after the export, and on every error path.
struct Snapshot {
    dir: PathBuf,
}

impl Snapshot {
    /// A new, empty snapshot directory in `parent`.
    fn create(parent: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::fs::create_dir_all(parent)?;
        loop {
            let name = format!(
                "snapshot-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let dir = parent.join(name);
            match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => return Ok(Self { dir }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }

    /// Copies the store at `store` into this snapshot: its state at a commit, with nothing written, created or locked
    /// for writing in standalone omp's directory ([`Snapshot::copy_with`]).
    fn copy(&self, store: &Path) -> Result<(), CopyError> {
        self.copy_with(store, STORE_BUSY_TIMEOUT, || {})
    }

    /// [`Snapshot::copy`], waiting up to `timeout` for standalone omp's writers; `between` runs after each copy of
    /// the database file, before its journal and log are copied (tests race a writer there).
    ///
    /// SQLite never opens standalone omp's files: they are read as bytes, never with `immutable=1` and never through
    /// a connection that would create or update `agent.db-shm` beside them (a read-only connection still does).
    /// Under [`lock_shared`] the database file, then `agent.db-journal` and `agent.db-wal` when they exist, are
    /// staged in this snapshot's own `stage/`:
    /// - rollback journal: no writer gets EXCLUSIVE while the lock is held, so the database file holds the last
    ///   committed transaction; a journal beside it is a live writer's (RESERVED: its pages are not in the database
    ///   file yet) or the hot journal of a writer that died mid-transaction, and either is staged with it and rolled
    ///   back in `stage/` when SQLite opens the copy there, giving that committed state. A journal naming a
    ///   super-journal (a multi-database transaction) is not copied: its rollback would read, and may delete, the
    ///   super-journal beside the store, and under the lock it is a dead writer's (a live one names its super-journal
    ///   only once it holds EXCLUSIVE) that nobody can roll back meanwhile, so the copy fails at once;
    /// - write-ahead log: writers append to the log, and only a checkpoint writes the database file (or the last
    ///   connection closing, which needs the EXCLUSIVE lock [`lock_shared`] excludes). Each try holds
    ///   [`hold_checkpoints`]: no other process checkpoints meanwhile, so the database file is the one state the
    ///   log's frames apply to, and SQLite replays the copied frames over it up to the last commit. Without an index
    ///   (`-shm`) no connection has the log open; the try counts only if none appeared by its end. A log restarted
    ///   meanwhile (a writer may restart it once everything is checkpointed, which leaves the database file alone)
    ///   gets a new header, with a checkpoint sequence and a first salt that grow, so the header read before, the
    ///   one copied and the one read after must be equal.
    ///
    /// SQLite then opens the staged copy in `stage/`, with its own `-shm` there, and `VACUUM INTO` writes the
    /// snapshot's `agent.db`; `stage/` goes. Never reads a credential row; an error names the reason only.
    fn copy_with(
        &self,
        store: &Path,
        timeout: Duration,
        mut between: impl FnMut(),
    ) -> Result<(), CopyError> {
        let copies = COPIES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deadline = Instant::now() + timeout;
        let (journal, wal, shm) = (
            suffixed(store, "-journal"),
            suffixed(store, "-wal"),
            suffixed(store, "-shm"),
        );
        let stage = Stage::create(self.dir.join("stage"))?;
        let staged = stage.0.join(STORE);
        let mut database = match File::open(store) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(CopyError::Gone);
            }
            Err(error) => return Err(CopyError::Io(error)),
        };
        lock_shared(&database, deadline)?;
        let read = |database: &mut File| -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            database.rewind()?;
            database.read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        loop {
            let checkpoints = hold_checkpoints(&shm, deadline)?;
            let before = wal_header(&wal)?;
            let bytes = read(&mut database)?;
            std::fs::write(&staged, &bytes)?;
            between();
            // Under the SHARED lock such a journal is a dead writer's (a live one names its super-journal only
            // once it holds EXCLUSIVE), and nobody can roll it back before the lock goes: no use waiting.
            if copy_file(&journal, &suffixed(&staged, "-journal"))?
                .as_deref()
                .is_some_and(names_super_journal)
            {
                return Err(CopyError::SuperJournal);
            }
            let copied = copy_file(&wal, &suffixed(&staged, "-wal"))?.map(|wal| header_of(&wal));
            let after = wal_header(&wal)?;
            // Without an index at first, a connection that opened the log meanwhile may have checkpointed it.
            let quiet = checkpoints.is_some() || !shm.exists();
            drop(checkpoints);
            if quiet && before == copied && copied == after {
                break;
            }
            if Instant::now() >= deadline {
                return Err(CopyError::Busy);
            }
            std::thread::sleep(STORE_RETRY);
        }
        // The lock goes with the file: everything below happens in omfp's own directory.
        drop(database);
        drop(copies);
        let into = self.store();
        let into = into
            .to_str()
            .ok_or_else(|| rusqlite::Error::InvalidPath(into.clone()))?;
        Connection::open(&staged)?.execute("VACUUM INTO ?1", [into])?;
        drop(stage);
        Ok(())
    }

    /// The agent directory the export runs on.
    fn dir(&self) -> &Path {
        &self.dir
    }

    fn store(&self) -> PathBuf {
        self.dir.join(STORE)
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.dir) {
            warn!(%error, dir = %self.dir.display(), "cannot remove the snapshot of omp's logins");
        }
    }
}

impl Supervisor {
    /// `auth.importable` (DMN-229): the provider logins standalone omp holds, read with `omp auth export` from a
    /// snapshot of its store. Empty without running omp when standalone omp has no credential store, or one that
    /// cannot be copied.
    pub async fn omp_logins(&self) -> Result<OmpLogins> {
        let this = self.clone();
        detached(async move {
            Ok(match this.standalone().await? {
                Standalone::Exported(export) => export.logins(),
                Standalone::Absent | Standalone::Unreadable(_) => no_logins(),
            })
        })
        .await
    }

    /// `auth.import` (DMN-230): exports standalone omp's provider logins afresh and imports them into omfp's profile
    /// with `omp auth import -`; running threads then owe a restart for the login (DMN-224) and the usable models
    /// are checked again (DMN-236). Answers the providers copied; none without running the import. Holds the
    /// credential writes for its whole run, in a task of its own that outlives its caller, and refuses while a login
    /// runs: omp's import replaces a provider's accounts with the ones it read.
    pub async fn import_omp_logins(&self) -> Result<OmpLogins> {
        let this = self.clone();
        detached(async move { this.import_now().await }).await
    }

    async fn import_now(&self) -> Result<OmpLogins> {
        let _writes = self.credential_writes().await;
        if self.login_running() {
            return Err(SupervisorError::LoginRunning);
        }
        let export = match self.standalone().await? {
            Standalone::Absent => return Ok(no_logins()),
            Standalone::Unreadable(error) => {
                return Err(SupervisorError::StandaloneStore(format!(
                    "standalone omp's login store cannot be read: {error}"
                )));
            }
            Standalone::Exported(export) => export,
        };
        let logins = export.logins();
        if logins.providers.is_empty() {
            return Ok(logins);
        }
        let document = serde_json::to_string(&export)
            .map_err(|error| SupervisorError::Internal(error.to_string()))?;
        drop(export);
        let mut spec = self.auth_cli_spec();
        spec.env = self.inner.config.omp_env.clone();
        let launcher = Arc::clone(&self.inner.launcher);
        blocking(move || {
            std::fs::create_dir_all(&spec.cwd).map_err(StartError::Spawn)?;
            launcher.import_logins(&spec, document)
        })
        .await?
        .map_err(auth_error)?;
        let providers: Vec<&str> = logins
            .providers
            .iter()
            .map(|login| login.provider_id.as_str())
            .collect();
        info!(?providers, "copied standalone omp's logins");
        self.restart_after_login();
        self.recheck_usable_models();
        Ok(logins)
    }

    /// Finds standalone omp's credential store ([`credential_store`]) with the user's environment and, when it is
    /// another than omfp's profile's (compared as canonical paths), copies it into a [`Snapshot`] and runs
    /// `omp auth export` (DMN-229) on that copy: omfp's omp with omfp's profile environment, its agent directory the
    /// snapshot's (`PI_CODING_AGENT_DIR`, non-empty, so no `.env` file moves it and no XDG directory replaces it), and
    /// no log below errors. Nothing it writes reaches standalone omp; the snapshot goes once the export ended.
    async fn standalone(&self) -> Result<Standalone<Export>> {
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return Err(SupervisorError::ShuttingDown);
        }
        let overrides = self.inner.config.user_omp_env.clone();
        let own_profile = self.inner.data.omp_profile();
        let snapshots = self.inner.data.login();
        let mut spec = self.auth_cli_spec();
        spec.env = self.inner.config.omp_env.clone();
        let launcher = Arc::clone(&self.inner.launcher);
        let found = blocking(move || -> std::result::Result<_, StartError> {
            let cwd = std::env::current_dir()?;
            let user_env = |key: &str| variable(&overrides, key, |key| std::env::var_os(key));
            let Some(store) = credential_store(user_env, &cwd, Path::exists) else {
                return Ok(Standalone::Absent);
            };
            let store = match store.canonicalize() {
                Ok(store) if store.is_file() => store,
                Ok(_) => return Ok(Standalone::Absent),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Standalone::Absent);
                }
                Err(error) => return Ok(Standalone::Unreadable(error.to_string())),
            };
            // A daemon started from an omfp shell inherits omfp's own profile: no standalone omp.
            let own_profile = own_profile
                .canonicalize()
                .or_else(|_| std::path::absolute(&own_profile).map(|path| normalized(&path)))?;
            if store.starts_with(&own_profile) {
                return Ok(Standalone::Absent);
            }
            let snapshot = Snapshot::create(&snapshots)?;
            match snapshot.copy(&store) {
                Ok(()) => {}
                // Standalone omp's store went away since it was found.
                Err(CopyError::Gone) => return Ok(Standalone::Absent),
                Err(error) => return Ok(Standalone::Unreadable(error.to_string())),
            }
            let agent_dir = snapshot.dir().as_os_str().to_owned();
            for key in ["PI_CODING_AGENT_DIR", "OMP_CODING_AGENT_DIR"] {
                spec.env.push((key.into(), Some(agent_dir.clone())));
            }
            spec.env
                .push(("OMP_LOG_LEVEL".into(), Some(OsStr::new("error").to_owned())));
            std::fs::create_dir_all(&spec.cwd)?;
            launcher.export_logins(&spec).map(Standalone::Exported)
        })
        .await?
        .map_err(auth_error)?;
        Ok(match found {
            Standalone::Exported(stdout) => Standalone::Exported(Export::parse(&stdout)?),
            Standalone::Absent => Standalone::Absent,
            Standalone::Unreadable(error) => Standalone::Unreadable(error),
        })
    }

    /// The spec of a one-shot `omp auth …`: omfp's program, in the login omp's working directory (DMN-219); the
    /// caller sets its environment.
    fn auth_cli_spec(&self) -> LaunchSpec {
        let dir = self.inner.data.login();
        LaunchSpec {
            program: self.inner.config.omp_program.clone(),
            cwd: dir.join("work"),
            add_dirs: Vec::new(),
            session_dir: dir.join("sessions"),
            fork: None,
            model: None,
            approval_mode: None,
            append_system_prompt: None,
            extension: None,
            stderr_log: dir.join("omp.log"),
            env: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export(document: &Value) -> Result<Export> {
        Export::parse(&document.to_string())
    }

    // spec: DMN-229
    #[test]
    fn the_export_groups_providers_in_order_without_mcp_servers() {
        let parsed = export(&serde_json::json!({ "version": 1, "credentials": [
            { "provider": "openai", "credential": { "type": "api_key", "key": "k" } },
            { "provider": "mcp:fixture", "credential": { "type": "oauth", "access": "a" } },
            { "provider": "anthropic", "credential": { "type": "api_key", "key": "k" } },
            { "provider": "openai", "credential": { "type": "oauth", "access": "a" } },
        ] }))
        .unwrap();
        assert_eq!(
            parsed.logins(),
            OmpLogins {
                providers: vec![
                    OmpLogin {
                        provider_id: "openai".into(),
                        oauth: true
                    },
                    OmpLogin {
                        provider_id: "anthropic".into(),
                        oauth: false
                    },
                ],
            }
        );
        let document = serde_json::to_value(&parsed).unwrap();
        assert_eq!(document["credentials"].as_array().unwrap().len(), 3);
        assert!(!document.to_string().contains("mcp:"));
    }

    // spec: DMN-229
    #[test]
    fn another_version_or_unreadable_output_is_refused_without_quoting_it() {
        let error = export(&serde_json::json!({ "version": 2, "credentials": [] }))
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "omp `auth export` failed: its login export is version 2; omfp reads version 1"
        );
        let error = Export::parse("secret-key not json").err().unwrap();
        assert!(!error.to_string().contains("secret-key"), "{error}");
        let error = export(
            &serde_json::json!({ "version": 1, "credentials": [{ "provider": "secret-key" }] }),
        )
        .err()
        .unwrap();
        assert!(!error.to_string().contains("secret-key"), "{error}");
        for version in [
            serde_json::json!("sk-secret"),
            serde_json::json!({ "key": "sk-secret" }),
        ] {
            let error = export(&serde_json::json!({ "version": version, "credentials": [] }))
                .err()
                .unwrap();
            assert_eq!(
                error.to_string(),
                "omp `auth export` failed: its login export is an unknown version; omfp reads version 1"
            );
        }
    }

    fn store(env: &[(&str, &str)], exists: &[&str]) -> Option<PathBuf> {
        let var = |key: &str| {
            env.iter()
                .rev()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| OsString::from(value))
        };
        credential_store(var, Path::new("/cwd"), |dir| {
            exists.iter().any(|existing| dir == Path::new(existing))
        })
    }

    // spec: DMN-229
    #[test]
    fn the_store_is_found_where_omp_opens_it() {
        let at = |path: &str| Some(PathBuf::from(path));
        assert_eq!(store(&[("HOME", "/h")], &[]), at("/h/.omp/agent/agent.db"));
        assert_eq!(store(&[], &[]), None, "no HOME");
        assert_eq!(
            store(&[("HOME", "/h"), ("PI_CONFIG_DIR", "/abs/../dir")], &[]),
            at("/h/dir/agent/agent.db"),
            "joined to HOME even when absolute"
        );
        assert_eq!(
            store(&[("HOME", "/h"), ("PI_CONFIG_DIR", "")], &[]),
            at("/h/.omp/agent/agent.db")
        );
        // Profiles: OMP_PROFILE wins, empty or `default` is the default, a refused name is the default.
        assert_eq!(
            store(&[("HOME", "/h"), ("OMP_PROFILE", "work")], &[]),
            at("/h/.omp/profiles/work/agent/agent.db")
        );
        assert_eq!(
            store(&[("HOME", "/h"), ("PI_PROFILE", "work")], &[]),
            at("/h/.omp/profiles/work/agent/agent.db")
        );
        assert_eq!(
            store(
                &[("HOME", "/h"), ("OMP_PROFILE", ""), ("PI_PROFILE", "work")],
                &[]
            ),
            at("/h/.omp/agent/agent.db")
        );
        for refused in ["Work", "a.", "../x", "con", "lpt1.txt", "default"] {
            assert_eq!(
                store(&[("HOME", "/h"), ("OMP_PROFILE", refused)], &[]),
                at("/h/.omp/agent/agent.db"),
                "{refused}"
            );
        }
        // The agent directory override: the default profile only, resolved against omp's working directory.
        assert_eq!(
            store(&[("HOME", "/h"), ("PI_CODING_AGENT_DIR", "/agent")], &[]),
            at("/agent/agent.db")
        );
        assert_eq!(
            store(&[("HOME", "/h"), ("PI_CODING_AGENT_DIR", "rel")], &[]),
            at("/cwd/rel/agent.db")
        );
        assert_eq!(
            store(
                &[
                    ("HOME", "/h"),
                    ("OMP_PROFILE", "work"),
                    ("PI_CODING_AGENT_DIR", "/agent")
                ],
                &[]
            ),
            at("/h/.omp/profiles/work/agent/agent.db")
        );
        assert_eq!(
            store(
                &[
                    ("HOME", "/h"),
                    ("OMP_PROFILE", ""),
                    ("PI_PROFILE", "work"),
                    ("PI_CODING_AGENT_DIR", "/h/.omp/profiles/work/agent")
                ],
                &[]
            ),
            at("/h/.omp/agent/agent.db"),
            "PI_PROFILE's own agent directory is no override"
        );
        assert_eq!(
            store(
                &[
                    ("HOME", "/h"),
                    ("OMP_PROFILE", ""),
                    ("PI_PROFILE", "work"),
                    ("PI_CODING_AGENT_DIR", "/h/.omp/profiles/work/agent/")
                ],
                &[]
            ),
            at("/h/.omp/profiles/work/agent/agent.db"),
            "spelled otherwise, as a string, it is an override"
        );
    }

    // spec: DMN-229
    #[test]
    fn an_existing_xdg_data_directory_holds_the_store_of_the_roots_own_agent_directory() {
        let at = |path: &str| Some(PathBuf::from(path));
        let xdg = [("HOME", "/h"), ("XDG_DATA_HOME", "/x")];
        assert_eq!(store(&xdg, &[]), at("/h/.omp/agent/agent.db"));
        assert_eq!(store(&xdg, &["/x/omp"]), at("/x/omp/agent.db"));
        assert_eq!(
            store(
                &[xdg[0], xdg[1], ("PI_CODING_AGENT_DIR", "/agent")],
                &["/x/omp"]
            ),
            at("/agent/agent.db")
        );
        assert_eq!(
            store(
                &[
                    xdg[0],
                    xdg[1],
                    ("PI_CODING_AGENT_DIR", "/h/other/../.omp/agent")
                ],
                &["/x/omp"]
            ),
            at("/x/omp/agent.db"),
            "an override resolving to the root's own agent directory is none"
        );
        let named = [xdg[0], xdg[1], ("OMP_PROFILE", "work")];
        assert_eq!(
            store(&named, &["/x/omp"]),
            at("/h/.omp/profiles/work/agent/agent.db")
        );
        assert_eq!(
            store(&named, &["/x/omp/profiles/work"]),
            at("/x/omp/profiles/work/agent.db")
        );
    }

    /// A directory's entries and their bytes.
    fn listing(dir: &Path) -> Vec<(OsString, Vec<u8>)> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name(), std::fs::read(entry.path()).unwrap())
            })
            .collect();
        entries.sort();
        entries
    }

    /// A WAL-mode store at `path` with `rows` rows in a table `t`, its connection left open.
    fn wal_store(path: &Path, rows: i64) -> Connection {
        let db = Connection::open(path).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE t (n INTEGER);",
        )
        .unwrap();
        for n in 0..rows {
            db.execute("INSERT INTO t (n) VALUES (?1)", [n]).unwrap();
        }
        db
    }

    fn rows(store: &Path) -> i64 {
        Connection::open(store)
            .unwrap()
            .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
            .unwrap()
    }

    // spec: DMN-229
    #[test]
    fn a_store_with_a_live_write_ahead_log_is_copied_with_its_rows_without_writing_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let standalone = dir.path().join("standalone");
        std::fs::create_dir(&standalone).unwrap();
        let store = standalone.join(STORE);
        let writer = wal_store(&store, 3);
        assert!(
            standalone.join("agent.db-wal").exists(),
            "the rows are in the log only"
        );
        let before = listing(&standalone);

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        snapshot.copy(&store).unwrap();

        assert_eq!(rows(&snapshot.store()), 3);
        assert_eq!(listing(&standalone), before);
        assert_eq!(listing(snapshot.dir()).len(), 1, "only agent.db is left");
        drop(writer);
    }

    // spec: DMN-229
    #[test]
    fn a_store_without_a_write_ahead_log_is_copied_without_writing_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let standalone = dir.path().join("standalone");
        std::fs::create_dir(&standalone).unwrap();
        let store = standalone.join(STORE);
        drop(wal_store(&store, 2));
        let before = listing(&standalone);
        assert_eq!(before.len(), 1, "a closed WAL store has no log");

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        snapshot.copy(&store).unwrap();

        assert_eq!(listing(&standalone), before);
        assert_eq!(rows(&snapshot.store()), 2);
    }

    /// A WAL-mode store at `<dir>/standalone/agent.db` whose `rows` rows are in its log only, with no `-shm` and no
    /// connection: what a crashed omp leaves.
    fn wal_without_shm(dir: &Path, rows: i64) -> PathBuf {
        let live = dir.join("live");
        std::fs::create_dir(&live).unwrap();
        let writer = wal_store(&live.join(STORE), rows);
        let standalone = dir.join("standalone");
        std::fs::create_dir(&standalone).unwrap();
        for name in [STORE.to_owned(), format!("{STORE}-wal")] {
            std::fs::copy(live.join(&name), standalone.join(&name)).unwrap();
        }
        drop(writer);
        standalone.join(STORE)
    }

    // spec: DMN-229
    #[test]
    fn a_write_ahead_log_without_its_shm_is_copied_and_the_standalone_tree_stays_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let store = wal_without_shm(dir.path(), 4);
        let standalone = store.parent().unwrap().to_owned();
        let before = listing(&standalone);
        assert_eq!(before.len(), 2, "agent.db and agent.db-wal, no -shm");

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        snapshot.copy(&store).unwrap();

        assert_eq!(
            listing(&standalone),
            before,
            "no -shm, nothing replayed there"
        );
        assert_eq!(rows(&snapshot.store()), 4, "the log's rows are in the copy");
    }

    // spec: DMN-229
    #[test]
    fn a_store_with_a_write_ahead_log_in_a_directory_omfp_cannot_write_is_copied() {
        use std::os::unix::fs::PermissionsExt;
        if rustix::process::geteuid().is_root() {
            // root writes whatever the mode says: nothing to check.
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = wal_without_shm(dir.path(), 2);
        let standalone = store.parent().unwrap().to_owned();
        let before = listing(&standalone);
        std::fs::set_permissions(&standalone, std::fs::Permissions::from_mode(0o555)).unwrap();

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        let copied = snapshot.copy(&store);
        std::fs::set_permissions(&standalone, std::fs::Permissions::from_mode(0o755)).unwrap();

        copied.unwrap();
        assert_eq!(listing(&standalone), before);
        assert_eq!(rows(&snapshot.store()), 2);
    }

    /// A rollback-journal store at `path` of `rows` rows of 0, each about a kilobyte, so a transaction touching them
    /// all spills pages into the database file before it commits.
    fn journal_store(path: &Path, rows: i64) -> Connection {
        let db = Connection::open(path).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA cache_size=5; CREATE TABLE t (n INTEGER, pad BLOB);",
        )
        .unwrap();
        for _ in 0..rows {
            db.execute("INSERT INTO t (n, pad) VALUES (0, zeroblob(1000))", [])
                .unwrap();
        }
        db
    }

    /// The distinct values of `t.n` in the store at `store`: one at every transaction boundary.
    fn values(store: &Path) -> Vec<i64> {
        let db = Connection::open(store).unwrap();
        let mut statement = db.prepare("SELECT DISTINCT n FROM t ORDER BY n").unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    // spec: DMN-229
    #[test]
    fn a_writer_that_died_mid_transaction_leaves_a_hot_journal_rolled_back_in_the_copy_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = hot_journal_store(dir.path());
        let standalone = store.parent().unwrap().to_owned();
        let before = listing(&standalone);
        assert_eq!(before.len(), 2, "agent.db and its hot journal");
        let torn = Connection::open_with_flags(
            format!("file:{}?immutable=1", store.display()),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap()
        .query_row("SELECT count(DISTINCT n) FROM t", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
        assert_eq!(
            torn, 2,
            "the database file alone holds half the transaction"
        );

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        snapshot.copy(&store).unwrap();

        assert_eq!(values(&snapshot.store()), [0], "the last committed state");
        assert_eq!(
            listing(&standalone),
            before,
            "the hot journal stays where it is"
        );
    }

    /// Names the store [`journal_writer_process`] writes, in a child of this test binary: SQLite's POSIX locks
    /// conflict only across processes.
    const WRITER_STORE: &str = "OMFP_TEST_JOURNAL_WRITER_STORE";

    /// No test of its own: the body of the writer process
    /// [`a_rollback_journal_writer_mid_transaction_is_waited_for_never_copied_halfway`] starts. It sets every row of
    /// `t` to 1 in one transaction, spilling pages into the database file, says `in-transaction`, waits for a line on
    /// stdin, commits and says `committed`.
    #[test]
    fn journal_writer_process() {
        let Some(store) = std::env::var_os(WRITER_STORE) else {
            return;
        };
        let db = Connection::open(store).unwrap();
        db.execute_batch("PRAGMA cache_size=5; BEGIN; UPDATE t SET n = 1;")
            .unwrap();
        println!("in-transaction");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        db.execute_batch("COMMIT;").unwrap();
        println!("committed");
    }

    /// Reads `lines` until one is `expected`.
    fn wait_for(lines: &mut impl Iterator<Item = std::io::Result<String>>, expected: &str) {
        for line in lines.by_ref() {
            if line.unwrap().trim_end().ends_with(expected) {
                return;
            }
        }
        panic!("the writer process ended before saying {expected}");
    }

    // spec: DMN-229
    #[test]
    fn a_rollback_journal_writer_mid_transaction_is_waited_for_never_copied_halfway() {
        use std::io::{BufRead as _, Write as _};
        let dir = tempfile::tempdir().unwrap();
        let standalone = dir.path().join("standalone");
        std::fs::create_dir(&standalone).unwrap();
        let store = standalone.join(STORE);
        drop(journal_store(&store, 200));
        let mut writer = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "supervisor::import::tests::journal_writer_process",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(WRITER_STORE, &store)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = std::io::BufReader::new(writer.stdout.take().unwrap()).lines();
        wait_for(&mut lines, "in-transaction");
        assert!(
            standalone.join("agent.db-journal").exists(),
            "the writer is mid-transaction"
        );

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        let error = snapshot
            .copy_with(&store, Duration::from_millis(50), || {})
            .unwrap_err();
        assert!(matches!(error, CopyError::Busy), "{error}");
        assert!(!snapshot.store().exists(), "no half-written copy");

        writeln!(writer.stdin.take().unwrap()).unwrap();
        wait_for(&mut lines, "committed");
        assert!(writer.wait().unwrap().success());
        snapshot.copy(&store).unwrap();
        assert_eq!(values(&snapshot.store()), [1], "the committed transaction");
    }

    /// Names the store [`checkpoint_process`] checkpoints, in a child of this test binary.
    const CHECKPOINT_STORE: &str = "OMFP_TEST_CHECKPOINT_STORE";

    /// No test of its own: the body of the process
    /// [`another_process_cannot_checkpoint_the_log_while_the_store_is_copied`] starts. It checkpoints the log and
    /// empties it (`TRUNCATE`) without waiting, and says whether that was refused: `checkpoint busy=<0|1>`.
    #[test]
    fn checkpoint_process() {
        let Some(store) = std::env::var_os(CHECKPOINT_STORE) else {
            return;
        };
        let db = Connection::open(store).unwrap();
        db.busy_timeout(Duration::ZERO).unwrap();
        let busy: i64 = db
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
            .unwrap();
        println!("checkpoint busy={busy}");
    }

    /// Runs the test `name` of this binary in a child process with `key` naming `store`, and answers its stdout.
    fn child(name: &str, key: &str, store: &Path) -> String {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("supervisor::import::tests::{name}"),
                "--nocapture",
                "--test-threads=1",
            ])
            .env(key, store)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    // spec: DMN-229
    #[test]
    fn another_process_cannot_checkpoint_the_log_while_the_store_is_copied() {
        let dir = tempfile::tempdir().unwrap();
        let standalone = dir.path().join("standalone");
        std::fs::create_dir(&standalone).unwrap();
        let store = standalone.join(STORE);
        // A live omp: its rows in the log, the log's index beside it.
        let writer = wal_store(&store, 3);
        assert!(standalone.join("agent.db-shm").exists());

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        let mut during = None;
        snapshot
            .copy_with(&store, STORE_BUSY_TIMEOUT, || {
                if during.is_none() {
                    // A checkpoint now would write the database file while it is being copied: in two reads of an
                    // empty log nothing would show it.
                    during = Some(child("checkpoint_process", CHECKPOINT_STORE, &store));
                }
            })
            .unwrap();

        let during = during.unwrap();
        assert!(
            during
                .lines()
                .any(|line| line.trim_end().ends_with("checkpoint busy=1")),
            "the checkpoint must wait for the copy: {during}"
        );
        assert_eq!(rows(&snapshot.store()), 3);
        let after = child("checkpoint_process", CHECKPOINT_STORE, &store);
        assert!(
            after
                .lines()
                .any(|line| line.trim_end().ends_with("checkpoint busy=0")),
            "the copy left nothing behind: {after}"
        );
        drop(writer);
    }

    /// Names the store [`hot_journal_reader_process`] reads, in a child of this test binary.
    const READER_STORE: &str = "OMFP_TEST_HOT_JOURNAL_READER_STORE";

    /// No test of its own: the body of the reader process
    /// [`a_reader_of_standalone_omp_still_sees_a_hot_journal_while_the_store_is_copied`] starts. It reads the
    /// distinct values of `t` without waiting and says them, or `busy`.
    #[test]
    fn hot_journal_reader_process() {
        let Some(store) = std::env::var_os(READER_STORE) else {
            return;
        };
        let db = Connection::open(store).unwrap();
        db.busy_timeout(Duration::ZERO).unwrap();
        match db.query_row("SELECT count(DISTINCT n) FROM t", [], |row| {
            row.get::<_, i64>(0)
        }) {
            Ok(values) => println!("values {values}"),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::DatabaseBusy =>
            {
                println!("busy");
            }
            Err(error) => panic!("{error}"),
        }
    }

    /// The files a writer that died mid-transaction leaves at `<dir>/standalone`: `agent.db`, half updated, and its
    /// hot journal.
    fn hot_journal_store(dir: &Path) -> PathBuf {
        let live = dir.join("live");
        std::fs::create_dir(&live).unwrap();
        let writer = journal_store(&live.join(STORE), 200);
        // A transaction setting every row to 1 spills pages into the database file before it commits; the files as
        // they are now are what a writer that dies here leaves.
        writer.execute_batch("BEGIN; UPDATE t SET n = 1;").unwrap();
        let standalone = dir.join("standalone");
        std::fs::create_dir(&standalone).unwrap();
        for name in [STORE.to_owned(), format!("{STORE}-journal")] {
            std::fs::copy(live.join(&name), standalone.join(&name)).unwrap();
        }
        writer.execute_batch("ROLLBACK;").unwrap();
        drop(writer);
        standalone.join(STORE)
    }

    // spec: DMN-229
    #[test]
    fn a_reader_of_standalone_omp_still_sees_a_hot_journal_while_the_store_is_copied() {
        let dir = tempfile::tempdir().unwrap();
        let store = hot_journal_store(dir.path());
        let standalone = store.parent().unwrap().to_owned();
        let before = listing(&standalone);

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        let mut read = None;
        snapshot
            .copy_with(&store, STORE_BUSY_TIMEOUT, || {
                if read.is_none() {
                    // Another process of standalone omp reads while the copy holds its lock.
                    read = Some(child("hot_journal_reader_process", READER_STORE, &store));
                }
            })
            .unwrap();

        let read = read.unwrap();
        assert!(
            read.lines().any(|line| line.trim_end().ends_with("busy")),
            "the reader must not take the hot journal for a live writer's and read half a transaction: {read}"
        );
        assert_eq!(values(&snapshot.store()), [0]);
        assert_eq!(listing(&standalone), before);
    }

    // spec: DMN-229
    #[test]
    fn a_hot_journal_naming_a_super_journal_is_not_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = hot_journal_store(dir.path());
        let standalone = store.parent().unwrap().to_owned();
        // What a multi-database transaction appends to its journal: the super-journal's name, its length and checksum,
        // and the journal magic (`pager.c`, `writeSuperJournal`).
        // A name SQLite takes for a super-journal (`pagerIsSuperJrnlName`: `-mj`, nine hex digits, the seventh `9`).
        let name = standalone.join("agent.db-mj000000900");
        std::fs::write(&name, b"").unwrap();
        let mut record = Vec::new();
        record.extend_from_slice(&(u32::try_from(PENDING_BYTE / 4096).unwrap() + 1).to_be_bytes());
        record.extend_from_slice(name.as_os_str().as_encoded_bytes());
        let length = u32::try_from(name.as_os_str().len()).unwrap();
        record.extend_from_slice(&length.to_be_bytes());
        let checksum = name
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .fold(0_u32, |sum, byte| sum.wrapping_add(u32::from(*byte)));
        record.extend_from_slice(&checksum.to_be_bytes());
        record.extend_from_slice(&JOURNAL_MAGIC);
        let journal = standalone.join("agent.db-journal");
        let mut bytes = std::fs::read(&journal).unwrap();
        bytes.extend_from_slice(&record);
        std::fs::write(&journal, bytes).unwrap();
        let before = listing(&standalone);

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        let error = snapshot
            .copy_with(&store, STORE_BUSY_TIMEOUT, || {})
            .unwrap_err();

        assert!(matches!(error, CopyError::SuperJournal), "{error}");
        // The same journal persisted with its header zeroed is no hot journal: nothing to refuse.
        let mut persisted = std::fs::read(&journal).unwrap();
        persisted[..JOURNAL_MAGIC.len()].fill(0);
        assert!(!names_super_journal(&persisted));
        assert_eq!(listing(&standalone), before, "the super-journal stays");
        assert!(!snapshot.store().exists());
    }

    /// The header check alone: the writer runs in this process, whose locks never conflict with the copy's, so the
    /// checkpoint before its restart is not excluded here as it is for another process
    /// ([`another_process_cannot_checkpoint_the_log_while_the_store_is_copied`]).
    // spec: DMN-229
    #[test]
    fn a_write_ahead_log_restarted_while_the_database_file_is_copied_is_copied_again() {
        let dir = tempfile::tempdir().unwrap();
        let standalone = dir.path().join("standalone");
        std::fs::create_dir(&standalone).unwrap();
        let store = standalone.join(STORE);
        let writer = wal_store(&store, 3);

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        let mut copies = 0;
        snapshot
            .copy_with(&store, STORE_BUSY_TIMEOUT, || {
                copies += 1;
                if copies == 1 {
                    // omp checkpoints everything, truncates the log and writes again: its new frames replace
                    // the ones the database file copied before lacked.
                    writer
                        .execute_batch(
                            "PRAGMA wal_checkpoint(TRUNCATE); INSERT INTO t (n) VALUES (3);",
                        )
                        .unwrap();
                }
            })
            .unwrap();

        assert_eq!(copies, 2, "the first copy was dropped");
        assert_eq!(rows(&snapshot.store()), 4);
        drop(writer);
    }

    // spec: DMN-229
    #[test]
    fn a_snapshot_is_private_and_removed_after_a_copy_or_a_failed_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let login = dir.path().join("login");
        let store = dir.path().join(STORE);
        drop(wal_store(&store, 1));

        let snapshot = Snapshot::create(&login).unwrap();
        let other = Snapshot::create(&login).unwrap();
        assert_ne!(
            snapshot.dir(),
            other.dir(),
            "each snapshot has its own directory"
        );
        drop(other);
        let mode = std::fs::metadata(snapshot.dir())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        snapshot.copy(&store).unwrap();
        drop(snapshot);
        assert_eq!(listing(&login), []);

        let garbage = dir.path().join("garbage.db");
        std::fs::write(&garbage, "not a database, long enough to have a header").unwrap();
        let snapshot = Snapshot::create(&login).unwrap();
        let error = snapshot.copy(&garbage).unwrap_err();
        assert!(
            !error.to_string().contains("not a database, long"),
            "{error}"
        );
        assert!(snapshot.copy(&dir.path().join("missing.db")).is_err());
        assert!(!dir.path().join("missing.db").exists(), "nothing created");
        drop(snapshot);
        assert_eq!(listing(&login), []);
    }
}
