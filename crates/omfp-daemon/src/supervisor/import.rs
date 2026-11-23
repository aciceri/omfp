//! Standalone omp's provider logins, offered and copied into omfp's profile on request (ARC-021, DMN-229, DMN-230).
//!
//! The pinned omp's `omp auth export` opens its credential store read-write — it creates a missing one, migrates an
//! older schema and adds what its own schema lacks — so omfp never runs it on standalone omp's store. omfp finds that
//! store the way omp 19.1.0 does ([`credential_store`], from the user's environment: the daemon's with
//! [`SupervisorConfig::user_omp_env`](super::SupervisorConfig::user_omp_env)), has SQLite write a copy of it into a
//! private snapshot directory under `<data-dir>/login/` with `VACUUM INTO` ([`Snapshot`]), and runs the export on
//! that copy with omfp's profile environment; the snapshot is removed right after. omfp's omp then stores the logins
//! with `omp auth import -`, the document going through a pipe only. omfp never reads a credential itself, and MCP
//! server credentials (`mcp:<server>`) stay behind: only model-provider logins are copied.

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use omfp_protocol::{OmpLogin, OmpLogins};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::{info, warn};

use super::{Result, Supervisor, SupervisorError, blocking};
use crate::omp::{LaunchSpec, StartError};

/// The export document version omfp reads (omp 19.1.0's `omp auth export`).
const EXPORT_VERSION: u64 = 1;

/// How long copying the store waits for standalone omp's writers.
const STORE_BUSY_TIMEOUT: Duration = Duration::from_secs(2);

/// The provider prefix of an MCP server's credential, which is no model-provider login.
const MCP_PREFIX: &str = "mcp:";

/// The name of omp's credential store in its agent directory (omp 19.1.0's `getAgentDbPath`).
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
fn variable(
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
fn normalized(path: &Path) -> PathBuf {
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

/// The directory of the data files (`agent.db`, `sessions/`, …) of the omp whose environment `var` gives,
/// `cwd` being its working directory, mirroring omp 19.1.0's `DirResolver` and its `agentSubdir(…, "data")`
/// (`getAgentDbPath`; `packages/utils/src/dirs.ts`; re-check on an omp upgrade, ENV-020):
/// - the base root is `path.join(HOME, PI_CONFIG_DIR || ".omp")`;
/// - the profile is `OMP_PROFILE` when set (empty: the default), else `PI_PROFILE`; a refused name is the default;
/// - a named profile's root is `<base>/profiles/<name>`, its agent directory `<root>/agent`; the default profile's
///   agent directory is `path.resolve(PI_CODING_AGENT_DIR)` when that is set and not, as a string, `PI_PROFILE`'s own
///   agent directory, else `<root>/agent`;
/// - when the agent directory is the root's own, an existing `$XDG_DATA_HOME/omp` (a named profile's
///   `$XDG_DATA_HOME/omp/profiles/<name>`) holds the data instead.
///
/// omp's `.env` files, which may set these variables too, are not read. `None` without a `HOME`.
fn data_dir(
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
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    /// The store went away since it was found.
    Gone,
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
            Self::Gone => f.write_str("it went away"),
        }
    }
}

impl From<rusqlite::Error> for CopyError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for CopyError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
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

    /// Copies the store at `store` into this snapshot's `agent.db` with `VACUUM INTO`, one read transaction of
    /// SQLite's own: it takes standalone omp's locks, replays its write-ahead log or rolls a hot journal back, and
    /// waits up to [`STORE_BUSY_TIMEOUT`] for its writers. The store is opened read-only when omfp cannot write its
    /// directory or one of its files, else read-write, never created; SQLite may leave `-wal`/`-shm` files beside it.
    /// Read-only, SQLite can neither build the index of a log without `-shm` nor roll a hot journal back, and no
    /// writer can be using either then (it could not have created or written them), so the store and those files are
    /// copied into the snapshot first and read from there. Never reads a credential row; an error names the reason
    /// only.
    fn copy(&self, store: &Path) -> Result<(), CopyError> {
        if !store.is_file() {
            return Err(CopyError::Gone);
        }
        let beside = |suffix: &str| store.with_file_name(format!("{STORE}{suffix}"));
        let can_write =
            |path: &Path| nix::unistd::access(path, nix::unistd::AccessFlags::W_OK).is_ok();
        let writable = store.parent().is_some_and(can_write)
            && ["", "-wal", "-journal", "-shm"]
                .into_iter()
                .map(&beside)
                .all(|path| !path.exists() || can_write(&path));
        let staged = !writable
            && ((beside("-wal").is_file() && !beside("-shm").exists())
                || beside("-journal").is_file());
        let source = if staged {
            use std::os::unix::fs::PermissionsExt;
            let staging = self.dir.join("source");
            std::fs::create_dir(&staging)?;
            for suffix in ["", "-wal", "-journal"] {
                if suffix.is_empty() || beside(suffix).is_file() {
                    // `copy` keeps the original's mode: a read-only store would stay read-only here.
                    let copy = staging.join(format!("{STORE}{suffix}"));
                    std::fs::copy(beside(suffix), &copy)?;
                    std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o600))?;
                }
            }
            staging.join(STORE)
        } else {
            store.to_owned()
        };
        let mode = if writable || staged {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let copied = self.vacuum(&source, mode);
        if staged && let Some(staging) = source.parent() {
            std::fs::remove_dir_all(staging)?;
        }
        copied
    }

    /// `VACUUM INTO` this snapshot's `agent.db` from the store at `source`.
    fn vacuum(&self, source: &Path, mode: OpenFlags) -> Result<(), CopyError> {
        let db = Connection::open_with_flags(source, mode | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        db.busy_timeout(STORE_BUSY_TIMEOUT)?;
        let into = self.store();
        let into = into
            .to_str()
            .ok_or_else(|| rusqlite::Error::InvalidPath(into.clone()))?;
        db.execute("VACUUM INTO ?1", [into])?;
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

    /// A directory's entry names.
    fn listing(dir: &Path) -> Vec<OsString> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
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

    /// `<dir>/standalone`, created.
    fn standalone(dir: &Path) -> PathBuf {
        let standalone = dir.join("standalone");
        std::fs::create_dir(&standalone).unwrap();
        standalone
    }

    // spec: DMN-229
    #[test]
    fn a_store_with_an_open_writer_and_a_live_write_ahead_log_is_copied_with_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = standalone(dir.path()).join(STORE);
        let writer = wal_store(&store, 3);
        assert!(
            store.with_file_name("agent.db-wal").exists(),
            "the rows are in the log only"
        );

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        snapshot.copy(&store).unwrap();

        assert_eq!(rows(&snapshot.store()), 3);
        assert_eq!(listing(snapshot.dir()), [OsString::from(STORE)]);
        writer.execute("INSERT INTO t (n) VALUES (3)", []).unwrap();
        assert_eq!(rows(&store), 4, "the writer goes on");
    }

    // spec: DMN-229
    #[test]
    fn a_store_in_a_directory_omfp_cannot_write_is_copied() {
        use std::os::unix::fs::PermissionsExt;
        if rustix::process::geteuid().is_root() {
            // root writes whatever the mode says: nothing to check.
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let standalone = standalone(dir.path());
        let store = standalone.join(STORE);
        let writer = wal_store(&store, 2);
        let before = listing(&standalone);
        std::fs::set_permissions(&standalone, std::fs::Permissions::from_mode(0o555)).unwrap();

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        let copied = snapshot.copy(&store);
        let after = listing(&standalone);
        std::fs::set_permissions(&standalone, std::fs::Permissions::from_mode(0o755)).unwrap();
        drop(writer);

        copied.unwrap();
        assert_eq!(after, before);
        assert_eq!(rows(&snapshot.store()), 2);
    }

    // spec: DMN-229
    #[test]
    fn a_write_ahead_log_without_its_index_in_a_directory_omfp_cannot_write_is_copied() {
        use std::os::unix::fs::PermissionsExt;
        if rustix::process::geteuid().is_root() {
            // root writes whatever the mode says: nothing to check.
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live");
        std::fs::create_dir(&live).unwrap();
        let writer = wal_store(&live.join(STORE), 3);
        // A restored backup: the store and its log, no `-shm`.
        let standalone = standalone(dir.path());
        for name in [STORE.to_owned(), format!("{STORE}-wal")] {
            std::fs::copy(live.join(&name), standalone.join(&name)).unwrap();
        }
        drop(writer);
        let before = listing(&standalone);
        std::fs::set_permissions(&standalone, std::fs::Permissions::from_mode(0o555)).unwrap();

        let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
        let copied = snapshot.copy(&standalone.join(STORE));
        let after = listing(&standalone);
        std::fs::set_permissions(&standalone, std::fs::Permissions::from_mode(0o755)).unwrap();

        copied.unwrap();
        assert_eq!(after, before);
        assert_eq!(rows(&snapshot.store()), 3);
        assert_eq!(listing(snapshot.dir()), [OsString::from(STORE)]);
    }

    // spec: DMN-229
    #[test]
    fn a_writer_that_died_mid_transaction_leaves_its_last_committed_state_in_the_copy() {
        use std::os::unix::fs::PermissionsExt;
        // (files read-only, directory read-only): read-only cases only when not root, who writes whatever the mode
        // says. A restored backup can have read-only files in a writable directory too.
        let cases = if rustix::process::geteuid().is_root() {
            vec![(false, false)]
        } else {
            vec![(false, false), (true, true), (true, false)]
        };
        for (files_read_only, dir_read_only) in cases {
            let dir = tempfile::tempdir().unwrap();
            let live = dir.path().join("live");
            std::fs::create_dir(&live).unwrap();
            let writer = Connection::open(live.join(STORE)).unwrap();
            writer
                .execute_batch(
                    "PRAGMA journal_mode=DELETE; PRAGMA cache_size=5; CREATE TABLE t (n INTEGER, pad BLOB);",
                )
                .unwrap();
            for _ in 0..200 {
                writer
                    .execute("INSERT INTO t (n, pad) VALUES (0, zeroblob(1000))", [])
                    .unwrap();
            }
            // A transaction touching every row spills pages into the database file before it commits; the files as
            // they are now are what a writer that dies here leaves.
            writer.execute_batch("BEGIN; UPDATE t SET n = 1;").unwrap();
            let standalone = standalone(dir.path());
            for name in [STORE.to_owned(), format!("{STORE}-journal")] {
                std::fs::copy(live.join(&name), standalone.join(&name)).unwrap();
            }
            writer.execute_batch("ROLLBACK;").unwrap();
            drop(writer);
            if files_read_only {
                for name in [STORE.to_owned(), format!("{STORE}-journal")] {
                    let file = standalone.join(name);
                    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444))
                        .unwrap();
                }
            }
            if dir_read_only {
                std::fs::set_permissions(&standalone, std::fs::Permissions::from_mode(0o555))
                    .unwrap();
            }

            let snapshot = Snapshot::create(&dir.path().join("login")).unwrap();
            let copied = snapshot.copy(&standalone.join(STORE));
            std::fs::set_permissions(&standalone, std::fs::Permissions::from_mode(0o755)).unwrap();
            copied.unwrap();

            let values: Vec<i64> = Connection::open(snapshot.store())
                .unwrap()
                .prepare("SELECT DISTINCT n FROM t")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert_eq!(
                values,
                [0],
                "the last committed state, files read-only: {files_read_only}, directory read-only: {dir_read_only}"
            );
            assert_eq!(listing(snapshot.dir()), [OsString::from(STORE)]);
        }
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
        assert_eq!(listing(&login), Vec::<OsString>::new());

        let garbage = dir.path().join("garbage.db");
        std::fs::write(&garbage, "not a database, long enough to have a header").unwrap();
        let snapshot = Snapshot::create(&login).unwrap();
        let error = snapshot.copy(&garbage).unwrap_err();
        assert!(
            !error.to_string().contains("not a database, long"),
            "{error}"
        );
        assert!(matches!(
            snapshot.copy(&dir.path().join("missing.db")),
            Err(CopyError::Gone)
        ));
        assert!(!dir.path().join("missing.db").exists(), "nothing created");
        drop(snapshot);
        assert_eq!(listing(&login), Vec::<OsString>::new());
    }
}
