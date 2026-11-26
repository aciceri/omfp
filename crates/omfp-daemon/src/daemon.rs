//! Starting and stopping a daemon (02-daemon, "Startup and configuration"): in-process with [`start`] (DMN-004), or
//! as `omfp daemon` with [`run`]; one daemon per data directory, published in `daemon.json` (DMN-007); its log
//! (DMN-390, [`log`]); restarting it in place (DMN-391, [`restart`]).

mod log;
mod restart;

use std::ffi::OsString;
use std::fs::{File, Permissions, TryLockError};
use std::io::{IsTerminal as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, bail};
use omfp_protocol::ThreadSummary;
use rustix::io::Errno;
use rustix::process::{Pid, test_kill_process};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

pub use restart::{RESTART_DEADLINE, restart};

use crate::agent::{AgentLauncher, OmpLauncher};
use crate::config::{Cli, Config, default_config_file, web_bundle};
use crate::conventions;
use crate::git::{Git, GitCli};
use crate::jobs::Jobs;
use crate::jobs::declared::{Clock, Declared, SystemClock};
use crate::omp_binary;
use crate::paths::{
    DataDir, create_private_dir, make_private_file, private_file_options, replace_file,
};
use crate::server::{self, AppState, Services};
use crate::store::Store;
use crate::supervisor::{Supervisor, SupervisorConfig};
use crate::theme::default_theme_file;
use crate::threads::Threads;
use crate::token::Token;
use crate::widgets::Widgets;
use crate::widgets::init::default_init_file;
use crate::workspaces::Workspaces;

/// The system dependencies a daemon runs on (ARC-012).
#[derive(Clone)]
pub struct Ports {
    pub launcher: Arc<dyn AgentLauncher>,
    pub git: Arc<dyn Git>,
    /// omp executable given to the launcher (DMN-006).
    pub omp_program: PathBuf,
    /// omp version reported in `system.hello` (PRT-005).
    pub omp_version: String,
    /// Environment overrides for every omp process.
    pub omp_env: Vec<(OsString, Option<OsString>)>,
    /// Overrides of the daemon's own environment that make up the user's own (standalone) omp, whose provider logins
    /// `auth.importable`/`auth.import` read (DMN-229, DMN-230); empty in production: the daemon's environment as
    /// inherited.
    pub user_omp_env: Vec<(OsString, Option<OsString>)>,
    /// Startup warnings shown to clients (DMN-001).
    pub warnings: Vec<String>,
    /// The time declared jobs' schedules and service backoffs follow (DMN-323, DMN-326).
    pub clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for Ports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ports")
            .field("git", &self.git)
            .field("omp_program", &self.omp_program)
            .field("omp_version", &self.omp_version)
            .field("warnings", &self.warnings)
            .finish_non_exhaustive()
    }
}

impl Ports {
    /// The real ports: omp resolved as `--omp-bin`, bundled, then `PATH` (DMN-006) and version-checked (DMN-001);
    /// git on `PATH`; the host's clock.
    pub fn resolve(config: &Config) -> anyhow::Result<Self> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let program = omp_binary::resolve(
            config.omp_bin.as_deref(),
            exe_dir.as_deref(),
            std::env::var_os("PATH").as_deref(),
        )
        .context("cannot find omp: pass --omp-bin or put omp on PATH")?;
        info!(omp = %program.display(), "using omp");
        let data = DataDir::new(std::path::absolute(&config.data_dir)?);
        let omp_env = crate::omp_profile::environment(&data, Vec::new())?;
        let version = omp_binary::version(&program, &omp_env)?;
        let warning = omp_binary::accept(
            &program.display().to_string(),
            &version,
            omfp_core::OMP_VERSION,
            config.allow_omp_mismatch,
        )?;
        if let Some(warning) = &warning {
            warn!("{warning}");
        }
        Ok(Self {
            launcher: Arc::new(OmpLauncher),
            git: Arc::new(GitCli::new("git")),
            omp_program: program,
            omp_version: version,
            omp_env,
            user_omp_env: Vec::new(),
            warnings: warning.into_iter().collect(),
            clock: Arc::new(SystemClock),
        })
    }

    /// The ports of a test daemon: `launcher` and `git` (fakes), omp named `omp` at the pinned version without
    /// environment overrides, `user_omp_env` as the user's own omp, no warnings, the host's clock.
    #[doc(hidden)]
    #[must_use]
    pub fn fake(
        launcher: Arc<dyn AgentLauncher>,
        git: Arc<dyn Git>,
        user_omp_env: Vec<(OsString, Option<OsString>)>,
    ) -> Self {
        Self {
            launcher,
            git,
            omp_program: "omp".into(),
            omp_version: omfp_core::OMP_VERSION.into(),
            omp_env: Vec::new(),
            user_omp_env,
            warnings: Vec::new(),
            clock: Arc::new(SystemClock),
        }
    }
}

/// What `<data-dir>/daemon.json` says about the running daemon (DMN-007, D-16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    /// Address the daemon listens on.
    pub addr: SocketAddr,
    pub pid: u32,
    /// When this daemon started (unix ms): a restart in place keeps the pid, not this (DMN-391). `None`: an omfp
    /// older than restarts in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<u64>,
    /// The daemon catches `SIGUSR2` and restarts in place on it (`omfp daemon`, DMN-394). `false`: an omfp older than
    /// restarts in place, or a daemon another program runs in-process ([`start`], [`start_with`]), which the signal
    /// would end.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restarts_in_place: bool,
    /// The PID namespace `pid` belongs to (Linux: the target of `/proc/self/ns/pid`; `None` elsewhere): a client in
    /// another one (a Flatpak sandbox, a container) can neither signal nor watch that pid (DMN-391).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_namespace: Option<String>,
    /// The daemon is restarting in place (DMN-392): it no longer serves, and the restarted one writes this file anew.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restarting: bool,
}

impl DaemonInfo {
    /// The daemon's pid as this process sees it: `None` when the daemon runs in another PID namespace, where the
    /// same number names another process (DMN-391).
    fn local_pid(&self) -> Option<Pid> {
        if self.pid_namespace != pid_namespace() {
            return None;
        }
        i32::try_from(self.pid).ok().and_then(Pid::from_raw)
    }
}

/// The PID namespace of this process (Linux), as [`DaemonInfo::pid_namespace`] names it.
fn pid_namespace() -> Option<String> {
    std::fs::read_link("/proc/self/ns/pid")
        .ok()
        .and_then(|link| link.into_os_string().into_string().ok())
}

const INFO_FILE: &str = "daemon.json";
const LOCK_FILE: &str = "daemon.lock";
/// How often a client checks `daemon.json` while it waits for a daemon.
const POLL: Duration = Duration::from_millis(25);

/// The daemon running for `data_dir`, if any: `daemon.json` is read and trusted only while its daemon still holds
/// the data directory's lock (the process is alive) and does not say it is restarting (DMN-392).
pub fn discover(data_dir: &Path) -> std::io::Result<Option<DaemonInfo>> {
    let text = match std::fs::read_to_string(data_dir.join(INFO_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !lock_held(data_dir)? {
        // Nobody holds the lock: `daemon.json` is stale.
        return Ok(None);
    }
    let info: DaemonInfo = serde_json::from_str(&text).map_err(std::io::Error::other)?;
    Ok((!info.restarting).then_some(info))
}

/// Whether the daemon of `data_dir` is restarting in place (DMN-392): `daemon.json` says so and its process lives (a
/// daemon that died while restarting left a stale mark, as does one in another PID namespace, which cannot be watched).
pub fn restarting(data_dir: &Path) -> std::io::Result<bool> {
    let text = match std::fs::read_to_string(data_dir.join(INFO_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let info: DaemonInfo = serde_json::from_str(&text).map_err(std::io::Error::other)?;
    Ok(info.restarting && info.local_pid().is_some_and(process_alive))
}

/// [`discover`] through a restart in place (DMN-392): while the daemon restarts, waits (at most `deadline`) for the
/// restarted one, so a client never starts a daemon of its own meanwhile.
pub fn discover_through_restart(
    data_dir: &Path,
    deadline: Duration,
) -> std::io::Result<Option<DaemonInfo>> {
    let until = Instant::now() + deadline;
    loop {
        if let Some(info) = discover(data_dir)? {
            return Ok(Some(info));
        }
        if !restarting(data_dir)? {
            // The restarted daemon may have written daemon.json since.
            return discover(data_dir);
        }
        if Instant::now() > until {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "the omfp daemon of {} did not restart in {} s",
                    data_dir.display(),
                    deadline.as_secs()
                ),
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// Whether the process `pid` exists (a zombie counts).
fn process_alive(pid: Pid) -> bool {
    test_kill_process(pid) != Err(Errno::SRCH)
}

/// Whether a daemon holds the data directory's lock (DMN-007): its process is alive, even before it wrote
/// `daemon.json`.
pub fn lock_held(data_dir: &Path) -> std::io::Result<bool> {
    let lock = match File::open(data_dir.join(LOCK_FILE)) {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    match lock.try_lock_shared() {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Takes the data directory's exclusive lock (DMN-007).
fn lock_data_dir(data_dir: &Path) -> anyhow::Result<File> {
    let lock = private_file_options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join(LOCK_FILE))
        .with_context(|| format!("cannot open the lock file in {}", data_dir.display()))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => {
            let running = std::fs::read_to_string(data_dir.join(INFO_FILE))
                .ok()
                .and_then(|text| serde_json::from_str::<DaemonInfo>(&text).ok())
                .map(|info| format!(" (pid {}, listening on {})", info.pid, info.addr))
                .unwrap_or_default();
            bail!(
                "another omfp daemon is already running for {}{running}",
                data_dir.display()
            )
        }
        Err(TryLockError::Error(error)) => Err(error).context("cannot lock the data directory"),
    }
}

fn write_info(data_dir: &Path, info: &DaemonInfo) -> std::io::Result<()> {
    let mut text = serde_json::to_string_pretty(info)?;
    text.push('\n');
    replace_file(
        &data_dir.join(INFO_FILE),
        text.as_bytes(),
        Some(Permissions::from_mode(0o600)),
    )
}

/// Asks a running daemon to stop as SIGTERM does (DMN-005); the tray's `Quit daemon` sends it (DMN-130). Cheap to
/// clone, usable from any thread.
#[derive(Debug, Clone)]
pub struct StopRequest(Arc<Notify>);

impl StopRequest {
    /// Requests the stop; [`run_with`] then shuts the daemon down.
    pub fn request(&self) {
        self.0.notify_one();
    }
}

/// A running daemon.
#[derive(Debug)]
pub struct DaemonHandle {
    /// What `daemon.json` says.
    info: DaemonInfo,
    token: Token,
    data_dir: PathBuf,
    supervisor: Supervisor,
    jobs: Jobs,
    declared: Declared,
    terminals: crate::terminals::Terminals,
    stop: watch::Sender<bool>,
    stop_request: Arc<Notify>,
    threads: watch::Receiver<Vec<ThreadSummary>>,
    server: JoinHandle<std::io::Result<()>>,
    background: Vec<JoinHandle<()>>,
    _lock: File,
}

impl DaemonHandle {
    /// The bound address.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.info.addr
    }

    /// The access token clients present in `system.hello`.
    #[must_use]
    pub fn token(&self) -> &Token {
        &self.token
    }

    /// The UI's URL with the token in the fragment (never sent to the server, PRT-002).
    #[must_use]
    pub fn url(&self) -> String {
        format!("{}#token={}", self.base_url(), self.token.expose())
    }

    /// The UI's URL without the token (a daemon on every interface is reached on loopback).
    #[must_use]
    pub fn base_url(&self) -> String {
        let mut addr = self.info.addr;
        if addr.ip().is_unspecified() {
            addr.set_ip(IpAddr::V4(Ipv4Addr::LOCALHOST));
        }
        format!("http://{addr}/")
    }

    /// What `omfp daemon` prints once it listens (UI-004, DMN-181): the URL with the token only to a terminal; to
    /// anything else (`daemon.log`, a pipe) the URL without it and where the token is.
    #[must_use]
    pub fn listening_line(&self, terminal: bool) -> String {
        if terminal {
            format!("omfp daemon listening on {}", self.url())
        } else {
            format!(
                "omfp daemon listening on {} (open it with `omfp desktop`, or add #token=<token> from {})",
                self.base_url(),
                DataDir::new(&self.data_dir).token().display()
            )
        }
    }

    /// The agent supervisor: what the host tools of every thread act through (DMN-060).
    #[must_use]
    pub fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }

    /// The absolute data directory.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The thread summaries of the `index` topic, current and on every change (the tray's status, DMN-129).
    #[must_use]
    pub fn threads(&self) -> watch::Receiver<Vec<ThreadSummary>> {
        self.threads.clone()
    }

    /// What stops this daemon from outside under [`run_with`].
    #[must_use]
    pub fn stop_request(&self) -> StopRequest {
        StopRequest(Arc::clone(&self.stop_request))
    }

    /// Stops the daemon (DMN-005): connections close, every omp process, job and PTY stops (at most 10 s),
    /// `daemon.json` is removed and the data directory unlocked.
    pub async fn shutdown(self) -> anyhow::Result<()> {
        self.close(false).await
    }

    /// Stops the daemon to restart it in place (DMN-391, DMN-392): first `daemon.json` says it restarts, so local
    /// clients wait for the restarted daemon rather than start one of their own, then the shutdown of
    /// [`Self::shutdown`], which leaves that file to the restarted daemon.
    pub async fn shutdown_for_restart(self) -> anyhow::Result<()> {
        let marked = DaemonInfo {
            restarting: true,
            ..self.info.clone()
        };
        if let Err(error) = write_info(&self.data_dir, &marked) {
            warn!(%error, "cannot mark daemon.json as restarting");
        }
        self.close(true).await
    }

    async fn close(self, restarting: bool) -> anyhow::Result<()> {
        let _ = self.stop.send(true);
        self.terminals.shutdown();
        // No declared run starts or restarts while everything stops; the ends of the killed ones are recorded.
        self.declared.close();
        tokio::join!(self.supervisor.shutdown(), self.jobs.shutdown());
        self.declared.wait_recorded().await;
        for task in &self.background {
            task.abort();
        }
        let served = self.server.await;
        if !restarting
            && let Err(error) = std::fs::remove_file(self.data_dir.join(INFO_FILE))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            warn!(%error, "cannot remove daemon.json");
        }
        served.context("the server task failed")??;
        info!("daemon stopped");
        Ok(())
    }
}

/// Starts a daemon in-process with the real ports (DMN-004), serving the web bundle shipped next to the executable
/// unless `--static-dir` names one (DMN-008).
pub async fn start(mut config: Config) -> anyhow::Result<DaemonHandle> {
    let ports = Ports::resolve(&config)?;
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    config.static_dir = web_bundle(config.static_dir.as_deref(), exe_dir.as_deref());
    if let Some(dir) = &config.static_dir {
        info!(web = %dir.display(), "serving the web UI");
    } else {
        warn!("no web UI bundle: pass --static-dir to serve one");
    }
    start_with(config, ports).await
}

/// Creates the data directory private (DMN-180): the mode it had when it was open to other users.
fn create_data_dir(data_dir: &Path) -> anyhow::Result<Option<u32>> {
    create_private_dir(data_dir).with_context(|| format!("cannot create {}", data_dir.display()))
}

fn warn_opened(data_dir: &Path, mode: u32) {
    warn!(
        data_dir = %data_dir.display(),
        mode = format_args!("{mode:o}"),
        "the data directory was open to other users: its mode is now 0700 (DMN-180)"
    );
}

/// Starts a daemon in-process on the given ports.
#[allow(clippy::too_many_lines)] // Startup wires every part in order.
pub async fn start_with(config: Config, ports: Ports) -> anyhow::Result<DaemonHandle> {
    let data_dir = std::path::absolute(&config.data_dir)?;
    if let Some(mode) = create_data_dir(&data_dir)? {
        warn_opened(&data_dir, mode);
    }
    let lock = lock_data_dir(&data_dir)?;
    let data = DataDir::new(&data_dir);
    let token = Token::load_or_create(&data.token()).context("cannot read or create the token")?;
    conventions::write_widget_builder(&data).with_context(|| {
        format!(
            "cannot write the widget-builder agent to {}",
            data.widget_builder().display()
        )
    })?;
    crate::voice_log::write_extension(&data).with_context(|| {
        format!(
            "cannot write omfp's omp extension to {}",
            data.omp_extension_module().display()
        )
    })?;
    let workspaces_root = std::path::absolute(&config.workspaces_root)?;
    // SQLite creates its database (and gives its journal files the database's mode) with the umask's mode.
    private_file_options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data.db())
        .and_then(|_| make_private_file(&data.db()))
        .with_context(|| format!("cannot create {}", data.db().display()))?;
    let store = Arc::new(Store::open(&data.db()).context("cannot open the database")?);
    let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
    let workspaces = Arc::new(Workspaces::new(
        Arc::clone(&store),
        ports.git,
        workspaces_root,
    ));
    let omp_env = crate::omp_profile::environment(&data, ports.omp_env)?;
    let agent_dir = data.omp_profile().join("agent");
    create_private_dir(&data.omp_profile())?;
    create_private_dir(&agent_dir)?;
    let widgets = Arc::new(Widgets::new(config.static_dir.as_deref()));
    let supervisor = Supervisor::new(
        ports.launcher,
        Arc::clone(&threads),
        Arc::clone(&workspaces),
        data.clone(),
        SupervisorConfig {
            omp_env,
            user_omp_env: ports.user_omp_env,
            max_processes: config.max_processes,
            idle_timeout: config.idle_timeout,
            event_capacity: config.event_buffer,
            agent_dir: Some(agent_dir),
            title_generation: config.title_generation,
            builtin_widgets: widgets.builtin_dir().map(Path::to_path_buf),
            ..SupervisorConfig::new(ports.omp_program)
        },
    );
    let services = Services::new(
        store,
        threads,
        workspaces,
        supervisor.clone(),
        widgets,
        ports.clock,
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.message))?;
    let jobs = services.jobs.clone();
    let declared = services.declared.clone();
    let (threads_list, follow_threads) = services.spawn_thread_list();
    let mut background = vec![
        supervisor.spawn_reaper(),
        services.spawn_index_updates(),
        services.spawn_model_updates(),
        services.spawn_rescans(),
        services.spawn_job_counts(),
        services.spawn_polls(),
        follow_threads,
    ];
    if let Some(path) = config.theme_file.clone() {
        info!(theme = %path.display(), "following the theme file");
        background.push(
            services
                .watch_theme_file(path)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?,
        );
    }
    if let Some(path) = config.init_file.clone() {
        info!(init = %path.display(), "following the init file");
        background.push(
            services
                .watch_init_file(path)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?,
        );
    }
    if let Some(path) = config.config_file.clone() {
        info!(config = %path.display(), "following the config file");
        background.push(
            services
                .follow_config_file(path, config.file_options.clone())
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?,
        );
    }
    // Once the config file's `delete_archived_after` is known: its first check runs at once (DMN-395).
    background.push(services.spawn_archive_cleanup(config.delete_archived_after));

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot listen on {}", config.bind))?;
    let addr = listener.local_addr()?;
    let started = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        });
    let info = DaemonInfo {
        addr,
        pid: std::process::id(),
        started: Some(started),
        restarts_in_place: config.restarts_in_place,
        pid_namespace: pid_namespace(),
        restarting: false,
    };
    write_info(&data_dir, &info).context("cannot write daemon.json")?;

    let (stop, stopping) = watch::channel(false);
    // Two secrets: a widget module's URL may show in an error report, which must not open the workspace's files.
    let (mut widget_key, mut file_key) = ([0_u8; 16], [0_u8; 16]);
    for key in [&mut widget_key, &mut file_key] {
        getrandom::fill(key).map_err(|error| anyhow::anyhow!("no randomness: {error}"))?;
    }
    let terminals = services.terminals.clone();
    let state = Arc::new(AppState {
        token: token.clone(),
        omp_version: ports.omp_version,
        warnings: ports.warnings,
        allowed_origins: config.allowed_origins,
        static_dir: config.static_dir,
        widget_base: format!("/widgets/{}", hex::encode(widget_key)),
        file_base: format!("{}{}", server::FILES_PREFIX, hex::encode(file_key)),
        services,
        mcp_sessions: server::McpSessions::default(),
        shutdown: stopping.clone(),
        unauthenticated: Arc::new(tokio::sync::Semaphore::new(
            server::UNAUTHENTICATED_CONNECTIONS,
        )),
    });
    let mut stopped = stopping;
    let server = tokio::spawn(async move {
        axum::serve(listener, server::router(state))
            .with_graceful_shutdown(async move {
                let _ = stopped.wait_for(|stopping| *stopping).await;
            })
            .await
    });
    info!(%addr, data_dir = %data_dir.display(), "daemon listening");
    Ok(DaemonHandle {
        info,
        token,
        data_dir,
        supervisor,
        jobs,
        declared,
        terminals,
        stop,
        stop_request: Arc::new(Notify::new()),
        threads: threads_list,
        server,
        background,
        _lock: lock,
    })
}

/// `omfp daemon`: starts the daemon, prints its URL, and stops on SIGINT/SIGTERM (DMN-005).
pub fn run(cli: Cli) -> anyhow::Result<()> {
    run_with(cli, |_| ())
}

/// [`run`] with `attach` called on the started daemon (`omfp daemon --tray`, DMN-128); what it returns lives until the
/// daemon stops, which a [`StopRequest`] also triggers, and is dropped before the shutdown starts. The log goes to
/// stderr and `daemon.log` (DMN-390); `SIGUSR2` with a restart request restarts the daemon in place (DMN-391).
pub fn run_with<A>(mut cli: Cli, attach: impl FnOnce(&DaemonHandle) -> A) -> anyhow::Result<()> {
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    cli.config = cli
        .config
        .or_else(|| default_config_file(xdg_config_home.clone(), home.clone()));
    let mut config = Config::from_cli(
        cli,
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        home.clone(),
    )
    .map_err(anyhow::Error::msg)?;
    config.theme_file = config
        .theme_file
        .or_else(|| default_theme_file(xdg_config_home.clone(), home.clone()));
    config.init_file = config
        .init_file
        .or_else(|| default_init_file(xdg_config_home, home));
    let data_dir = std::path::absolute(&config.data_dir)?;
    let opened = create_data_dir(&data_dir)?;
    let log = DataDir::new(&data_dir).daemon_log();
    log::init(&log).with_context(|| format!("cannot open {}", log.display()))?;
    if let Some(mode) = opened {
        warn_opened(&data_dir, mode);
    }
    let configured_bind = config.bind;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let outcome = runtime.block_on(async {
        // Caught before daemon.json names this process and says so: from then on `omfp daemon restart` may send it
        // (DMN-394).
        let mut restart_requests =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined2())?;
        config.restarts_in_place = true;
        let daemon = start(config).await?;
        // Signals are caught before the URL is printed: whoever waits for it may stop the daemon at once.
        let signal = signal_handler()?;
        tokio::pin!(signal);
        // Dropped when stdout is gone (DMN-355): a restarted daemon prints it again to whatever its first start had.
        let stdout = std::io::stdout();
        let _ = writeln!(
            stdout.lock(),
            "{}",
            daemon.listening_line(stdout.is_terminal())
        );
        let attached = attach(&daemon);
        let requested = Arc::clone(&daemon.stop_request);
        let program = loop {
            tokio::select! {
                result = &mut signal => {
                    result?;
                    break None;
                }
                () = requested.notified() => {
                    info!("stop requested");
                    break None;
                }
                _ = restart_requests.recv() => match restart::take_request(daemon.data_dir()) {
                    Ok(program) => {
                        info!(program = %program.display(), "restart requested");
                        break Some(program);
                    }
                    Err(error) => warn!("restart signal ignored: {error:#}"),
                },
            }
        };
        drop(attached);
        let Some(program) = program else {
            info!("shutting down");
            return daemon.shutdown().await.map(|()| None);
        };
        info!("shutting down to restart");
        let args = restart::exec_args(std::env::args_os().skip(1), configured_bind, daemon.addr());
        daemon.shutdown_for_restart().await?;
        anyhow::Ok(Some((program, args)))
    });
    // Every task and thread of the runtime ends before the process does, or becomes the restarted daemon.
    drop(runtime);
    let Some((program, args)) = outcome? else {
        return Ok(());
    };
    info!(program = %program.display(), "restarting");
    Err(restart::exec(&program, &args))
        .with_context(|| format!("cannot restart as {}", program.display()))
}

/// Installs the SIGINT/SIGTERM handlers now; the future ends at the first signal.
fn signal_handler() -> anyhow::Result<impl Future<Output = anyhow::Result<()>>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        Ok(async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
            Ok(())
        })
    }
    #[cfg(not(unix))]
    Ok(async { Ok(tokio::signal::ctrl_c().await?) })
}
