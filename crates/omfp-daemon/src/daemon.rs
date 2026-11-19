//! Starting and stopping a daemon (02-daemon, "Startup and configuration"): in-process with [`start`] (DMN-004), or
//! as `omfp daemon` with [`run`]; one daemon per data directory, published in `daemon.json` (DMN-007).

use std::ffi::OsString;
use std::fs::{File, Permissions, TryLockError};
use std::io::IsTerminal as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use omfp_protocol::ThreadSummary;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

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
            warnings: warning.into_iter().collect(),
            clock: Arc::new(SystemClock),
        })
    }
}

/// What `<data-dir>/daemon.json` says about the running daemon (DMN-007, D-16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    /// Address the daemon listens on.
    pub addr: SocketAddr,
    pub pid: u32,
}

const INFO_FILE: &str = "daemon.json";
const LOCK_FILE: &str = "daemon.lock";

/// The daemon running for `data_dir`, if any: `daemon.json` is read and trusted only while its daemon still holds
/// the data directory's lock (the process is alive).
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
    serde_json::from_str(&text)
        .map(Some)
        .map_err(std::io::Error::other)
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
    addr: SocketAddr,
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
        self.addr
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
        let mut addr = self.addr;
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
        if let Err(error) = std::fs::remove_file(self.data_dir.join(INFO_FILE))
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

/// Starts a daemon in-process on the given ports.
#[allow(clippy::too_many_lines)] // Startup wires every part in order.
pub async fn start_with(config: Config, ports: Ports) -> anyhow::Result<DaemonHandle> {
    let data_dir = std::path::absolute(&config.data_dir)?;
    if let Some(mode) = create_private_dir(&data_dir)
        .with_context(|| format!("cannot create {}", data_dir.display()))?
    {
        warn!(
            data_dir = %data_dir.display(),
            mode = format_args!("{mode:o}"),
            "the data directory was open to other users: its mode is now 0700 (DMN-180)"
        );
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
    let workspaces_root = std::path::absolute(&config.workspaces_root)?;
    // SQLite creates its database (and gives its journal files the database's mode) with the umask's mode.
    private_file_options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data.db())
        .and_then(|_| make_private_file(&data.db()))
        .with_context(|| format!("cannot create {}", data.db().display()))?;
    let store =
        Arc::new(Store::open(&data.db(), &workspaces_root).context("cannot open the database")?);
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
    let widgets = Arc::new(Widgets::new(data.widgets(), config.static_dir.as_deref()));
    let supervisor = Supervisor::new(
        ports.launcher,
        Arc::clone(&threads),
        Arc::clone(&workspaces),
        data.clone(),
        SupervisorConfig {
            omp_env,
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

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot listen on {}", config.bind))?;
    let addr = listener.local_addr()?;
    write_info(
        &data_dir,
        &DaemonInfo {
            addr,
            pid: std::process::id(),
        },
    )
    .context("cannot write daemon.json")?;

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
        addr,
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
/// daemon stops, which a [`StopRequest`] also triggers, and is dropped before the shutdown starts.
pub fn run_with<A>(mut cli: Cli, attach: impl FnOnce(&DaemonHandle) -> A) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        // A started-from-a-terminal daemon outlives its terminal (DMN-355): a line that cannot be written is dropped.
        // Left on, tracing-subscriber reports the failed write with `eprintln!`, which panics the task that logged.
        .log_internal_errors(false)
        .init();
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
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let daemon = start(config).await?;
            // Signals are caught before the URL is printed: whoever waits for it may stop the daemon at once.
            let signal = signal_handler()?;
            println!("{}", daemon.listening_line(std::io::stdout().is_terminal()));
            let attached = attach(&daemon);
            let requested = Arc::clone(&daemon.stop_request);
            tokio::select! {
                result = signal => result?,
                () = requested.notified() => info!("stop requested"),
            }
            drop(attached);
            info!("shutting down");
            daemon.shutdown().await
        })
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
