//! What request handlers use: the store-backed services, the agent supervisor, and the logs of the `index`,
//! `workspace/<id>` and `views/<id>` topics. Workspace folders are rescanned here (DMN-023), and a change reaches the
//! index, the roots of the live threads in the workspace (DMN-029) and its views (`views`, DMN-082); watched
//! workspaces are polled (`polls`).

mod file_checks;
mod logins;
mod mounts;
mod oauth;
mod polls;
mod views;
mod widget_tools;
mod windows;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use omfp_protocol::{
    ErrorBody, IndexEvent, IndexSnapshot, JobCounts, RepoStatus, SetSettingsParams, Settings,
    Theme, ThreadSummary, Workspace, WorkspaceEvent,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use super::convert::{self, bad_request, internal, workspace_error};
use crate::config_file::{ConfigFile, DaemonOptions, write_back};
use crate::event_log::EventLog;
use crate::jobs::Jobs;
use crate::jobs::declared::{Clock, Declared};
use crate::paths::replace_file;
use crate::store::{Store, StoreError, new_id, now_ms};
use crate::supervisor::Supervisor;
use crate::terminals::Terminals;
use crate::theme;
use crate::threads::Threads;
use crate::watch::{self, WatchedFile};
use crate::widgets::Widgets;
use crate::workspaces::{WorkspaceView, Workspaces};
use views::ViewsLog;
pub(crate) use widget_tools::CHECK_WAIT;
use widget_tools::ServicesWidgetHost;
use windows::Windows;

/// Aborts a task when dropped: a background task's helper ends with it.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The `index` topic: workspaces and thread summaries.
pub(crate) type IndexLog = EventLog<IndexEvent, IndexSnapshot>;

/// A `workspace/<id>` topic: the git status of each repository of the workspace.
pub(crate) type WorkspaceLog = EventLog<WorkspaceEvent, Vec<RepoStatus>>;

const INDEX_CAPACITY: usize = 512;
const WORKSPACE_CAPACITY: usize = 16;
const LIVE_BUFFER: usize = 1024;

#[allow(clippy::trivially_copy_pass_by_ref)] // `EventLog` takes `fn(&mut S, &E)`.
fn apply_index(state: &mut IndexSnapshot, event: &IndexEvent) {
    fn upsert<T: Clone>(items: &mut Vec<T>, item: &T, same: impl Fn(&T) -> bool) {
        match items.iter_mut().find(|existing| same(existing)) {
            Some(existing) => existing.clone_from(item),
            None => items.push(item.clone()),
        }
    }
    match event {
        IndexEvent::WorkspaceUpserted { workspace } => {
            upsert(&mut state.workspaces, workspace, |w| w.id == workspace.id);
        }
        IndexEvent::WorkspaceRemoved { workspace_id } => {
            state.workspaces.retain(|w| w.id != *workspace_id);
        }
        IndexEvent::ThreadUpserted { thread } => {
            upsert(&mut state.threads, thread, |t| t.id == thread.id);
        }
        IndexEvent::ThreadRemoved { thread_id } => {
            state.threads.retain(|t| t.id != *thread_id);
        }
        IndexEvent::ThemeChanged { theme } => state.theme.clone_from(theme),
        IndexEvent::SettingsChanged { settings } => state.settings.clone_from(settings),
        IndexEvent::InitChanged { init } => state.init.clone_from(init),
        IndexEvent::ModelsChanged { models } => state.models = Some(*models),
        IndexEvent::LoginChanged { login } => state.login.clone_from(login),
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)] // `EventLog` takes `fn(&mut S, &E)`.
fn apply_workspace(state: &mut Vec<RepoStatus>, event: &WorkspaceEvent) {
    match event {
        WorkspaceEvent::Status { repos } => state.clone_from(repos),
    }
}

/// The UI settings of the config file and where they come from (DMN-009).
#[derive(Debug, Default)]
struct SettingsState {
    /// The followed config file; `None` keeps the settings in [`SettingsState::memory`] only.
    file: Option<PathBuf>,
    /// Without a file: the TOML `settings.set` wrote so far.
    memory: String,
    /// `[daemon]` as last seen; a change needs a restart.
    daemon: DaemonOptions,
    settings: Settings,
}

/// Shared by every connection. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Services {
    pub store: Arc<Store>,
    pub threads: Arc<Threads>,
    pub workspaces: Arc<Workspaces>,
    pub supervisor: Supervisor,
    pub index: Arc<IndexLog>,
    /// The workspaces' jobs (DMN-080, DMN-081).
    pub jobs: Jobs,
    /// The jobs the workspaces declare (DMN-320..DMN-336).
    pub declared: Declared,
    /// Interactive PTY sessions (TRM-100..105).
    pub terminals: Terminals,
    /// Serializes index refreshes so their events never interleave.
    refresh: Arc<tokio::sync::Mutex<()>>,
    workspace_logs: Arc<Mutex<HashMap<String, Arc<WorkspaceLog>>>>,
    settings: Arc<Mutex<SettingsState>>,
    /// Serializes `settings.set` writes to the config file.
    settings_writes: Arc<tokio::sync::Mutex<()>>,
    /// The widget libraries and the compile cache (DMN-084).
    pub widgets: Arc<Widgets>,
    views_logs: Arc<Mutex<HashMap<String, Arc<ViewsLog>>>>,
    /// Serializes views refreshes and widget state writes so their events never interleave.
    views_refresh: Arc<tokio::sync::Mutex<()>>,
    /// Every window's latest report and its connection (AGT-026, PRT-043, DMN-205).
    windows: Arc<Windows>,
    /// Serializes the mount edits of `views.json` (PRT-080..082), each a read, a change and a write.
    manifest_edits: Arc<tokio::sync::Mutex<()>>,
    /// The followed user init file (DMN-185), served from here; `None` follows none.
    init_file: Arc<Mutex<Option<PathBuf>>>,
    /// The running login every window sees on the `index` topic (DMN-290).
    login_board: Arc<logins::LoginBoard>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `not_found` for `entity` `id`, which the index does not show.
fn not_found(entity: &'static str, id: &str) -> ErrorBody {
    StoreError::NotFound {
        entity,
        id: id.to_owned(),
    }
    .into()
}

/// Runs blocking work (the store, the file system) off the async workers; a panicked task is an internal error.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ErrorBody> + Send + 'static,
) -> Result<T, ErrorBody> {
    tokio::task::spawn_blocking(work).await.map_err(internal)?
}

impl Services {
    /// The services over the daemon's stores; every workspace is scanned first (DMN-023) and gets the default
    /// `views.json` when its folder has none (DMN-088). The supervisor's threads get the widget host tools; declared
    /// jobs follow `clock`.
    pub async fn new(
        store: Arc<Store>,
        threads: Arc<Threads>,
        workspaces: Arc<Workspaces>,
        supervisor: Supervisor,
        widgets: Arc<Widgets>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ErrorBody> {
        let jobs = Jobs::default();
        let declared = Declared::new(
            Arc::clone(&store),
            jobs.clone(),
            supervisor.clone(),
            Arc::clone(&threads),
            clock,
            jiff::tz::TimeZone::system(),
        );
        let services = Self {
            store,
            threads,
            workspaces,
            supervisor,
            index: Arc::new(EventLog::new(
                new_id(),
                IndexSnapshot {
                    workspaces: Vec::new(),
                    threads: Vec::new(),
                    theme: Theme::default(),
                    settings: Settings::default(),
                    init: None,
                    models: None,
                    login: None,
                },
                apply_index,
                INDEX_CAPACITY,
                LIVE_BUFFER,
            )),
            jobs,
            declared,
            terminals: Terminals::default(),
            refresh: Arc::new(tokio::sync::Mutex::new(())),
            workspace_logs: Arc::new(Mutex::new(HashMap::new())),
            settings: Arc::new(Mutex::new(SettingsState::default())),
            settings_writes: Arc::new(tokio::sync::Mutex::new(())),
            widgets,
            views_logs: Arc::new(Mutex::new(HashMap::new())),
            views_refresh: Arc::new(tokio::sync::Mutex::new(())),
            windows: Arc::new(Windows::new()),
            manifest_edits: Arc::new(tokio::sync::Mutex::new(())),
            init_file: Arc::new(Mutex::new(None)),
            login_board: Arc::default(),
        };
        let workspaces = Arc::clone(&services.workspaces);
        let declared = services.declared.clone();
        let scanned = blocking(move || {
            declared.recover()?;
            let list = workspaces.list().map_err(workspace_error)?;
            for ws in &list {
                match workspaces.rescan(&ws.id) {
                    Ok((view, _)) if view.repos.iter().any(|repo| repo.is_root) => {}
                    Ok(_) if !ws.path.is_dir() => {}
                    Ok(_) => {
                        if let Err(error) = crate::widgets::write_default_manifest(&ws.path) {
                            warn!(workspace = %ws.id, %error, "cannot write the default views.json");
                        }
                    }
                    Err(error) => {
                        warn!(workspace = %ws.id, %error, "cannot scan the workspace folder");
                    }
                }
            }
            Ok(list)
        })
        .await?;
        services
            .supervisor
            .set_widget_host(Arc::new(ServicesWidgetHost::new(services.clone())));
        services.supervisor.set_agent_host(Arc::new(
            crate::agent_tools::rpc::ServicesAgentHost::new(services.clone()),
        ));
        // Each workspace's jobs once its first scan is done (DMN-322).
        for ws in scanned {
            services.declared.reload(&ws.id).await;
        }
        services.refresh_index().await?;
        Ok(services)
    }

    /// Rescans the workspace of every thread that finishes a tool execution and publishes its git status (DMN-023):
    /// an edit of a tracked file changes nothing the poll's cheap check sees, so the dirty state of `workspace/<id>`
    /// (UI-025) would otherwise wait for the next full scan.
    pub fn spawn_rescans(&self) -> JoinHandle<()> {
        let mut tool_ends = self.supervisor.subscribe_tool_ends();
        let services = self.clone();
        tokio::spawn(async move {
            loop {
                let thread_id = match tool_ends.recv().await {
                    Ok(thread_id) => thread_id,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                let threads = Arc::clone(&services.threads);
                let result = async {
                    let workspace_id = blocking(move || {
                        threads
                            .get(&thread_id)
                            .map(|thread| thread.workspace_id)
                            .map_err(convert::thread_error)
                    })
                    .await?;
                    services.full_scan(&workspace_id).await
                }
                .await;
                if let Err(error) = result {
                    warn!(error = %error.message, "rescan after a tool run failed");
                }
            }
        })
    }

    /// Rescans a workspace folder (DMN-023): a change updates the `index` topic, the live threads in the workspace get
    /// their roots synced (DMN-029), and its views are read again (DMN-082).
    pub async fn rescan(&self, id: &str) -> Result<WorkspaceView, ErrorBody> {
        let workspaces = Arc::clone(&self.workspaces);
        let workspace_id = id.to_owned();
        let (view, changed) =
            blocking(move || workspaces.rescan(&workspace_id).map_err(workspace_error)).await?;
        if changed {
            self.refresh_index_quietly().await;
        }
        self.sync_roots(id).await?;
        self.refresh_views(id).await;
        self.declared.reload(id).await;
        Ok(view)
    }

    /// Syncs the roots of the threads living in workspace `id` with its last scan (DMN-029); a thread whose omp is
    /// still starting is synced once it runs, before this returns.
    pub async fn sync_roots(&self, id: &str) -> Result<(), ErrorBody> {
        let store = Arc::clone(&self.store);
        let workspace_id = id.to_owned();
        let thread_ids = blocking(move || {
            store
                .workspace_thread_ids(&workspace_id)
                .map_err(ErrorBody::from)
        })
        .await?;
        for thread_id in thread_ids {
            if let Err(error) = self.supervisor.sync_roots(&thread_id).await {
                warn!(thread = %thread_id, %error, "root sync failed");
            }
        }
        Ok(())
    }

    /// Keeps thread summaries current when processes start, stop, settle or ask something.
    pub fn spawn_index_updates(&self) -> JoinHandle<()> {
        let mut changes = self.supervisor.subscribe_changes();
        let services = self.clone();
        tokio::spawn(async move {
            while let Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) =
                changes.recv().await
            {
                services.refresh_index_quietly().await;
            }
        })
    }

    /// Refreshes the index whenever the declared jobs' counts change or a job made a thread (DMN-335).
    pub fn spawn_job_counts(&self) -> JoinHandle<()> {
        let mut changes = self.declared.subscribe_changes();
        let services = self.clone();
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                services.refresh_index_quietly().await;
            }
        })
    }

    /// Publishes the count of usable models on the `index` topic whenever the supervisor's changes (DMN-235..DMN-237).
    pub fn spawn_model_updates(&self) -> JoinHandle<()> {
        let mut models = self.supervisor.subscribe_usable_models();
        let services = self.clone();
        tokio::spawn(async move {
            loop {
                let known = *models.borrow_and_update();
                if let Some(models) = known {
                    // Held like a refresh, so the event never interleaves with a refresh's.
                    let _serial = services.refresh.lock().await;
                    if services.index.with_state(|index| index.models != known) {
                        services.index.push(IndexEvent::ModelsChanged { models });
                    }
                }
                if models.changed().await.is_err() {
                    break;
                }
            }
        })
    }

    /// The thread summaries of the `index` topic as a watch (DMN-129): current now, replaced on every thread event.
    pub fn spawn_thread_list(
        &self,
    ) -> (
        tokio::sync::watch::Receiver<Vec<ThreadSummary>>,
        JoinHandle<()>,
    ) {
        use tokio::sync::broadcast::error::RecvError;
        // Subscribed before reading, so no event between the read and the subscription is lost.
        let mut live = self.index.subscribe(None).live;
        let index = Arc::clone(&self.index);
        let (sender, receiver) =
            tokio::sync::watch::channel(index.with_state(|state| state.threads.clone()));
        let task = tokio::spawn(async move {
            loop {
                match live.recv().await {
                    Ok(event)
                        if !matches!(
                            event.event,
                            IndexEvent::ThreadUpserted { .. } | IndexEvent::ThreadRemoved { .. }
                        ) => {}
                    Ok(_) | Err(RecvError::Lagged(_)) => {
                        sender.send_replace(index.with_state(|state| state.threads.clone()));
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });
        (receiver, task)
    }

    /// Reloads workspaces and threads and publishes what changed on the `index` topic.
    pub async fn refresh_index(&self) -> Result<(), ErrorBody> {
        let _serial = self.refresh.lock().await;
        // Only refreshes change the index, and they are serialized: `current` stays current.
        let current = Arc::new(self.index.with_state(Clone::clone));
        let previous = Arc::clone(&current);
        let store = Arc::clone(&self.store);
        let workspaces = Arc::clone(&self.workspaces);
        let supervisor = self.supervisor.clone();
        let settings = self.settings();
        let counts = self.declared.counts();
        let fresh = blocking(move || {
            load_index(
                &store,
                &workspaces,
                &supervisor,
                &previous,
                settings,
                &counts,
            )
        })
        .await?;
        for event in diff_index(&current, fresh) {
            self.index.push(event);
        }
        Ok(())
    }

    /// Refreshes the index, logging instead of failing: the request that changed something already succeeded.
    pub async fn refresh_index_quietly(&self) {
        if let Err(error) = self.refresh_index().await {
            warn!(error = %error.message, "index refresh failed");
        }
    }

    /// The user's theme as published on the `index` topic (DMN-075).
    pub fn theme(&self) -> Theme {
        self.index.with_state(|index| index.theme.clone())
    }

    /// Replaces the user's theme and pushes it to every client (DMN-075); over [`theme::MAX_THEME_BYTES`] it is a
    /// `bad_request`.
    pub async fn set_theme(&self, css: Option<String>) -> Result<(), ErrorBody> {
        if let Some(reason) = css.as_deref().and_then(theme::too_large) {
            return Err(bad_request(reason));
        }
        let store = Arc::clone(&self.store);
        blocking(move || store.set_theme(css.as_deref()).map_err(ErrorBody::from)).await?;
        self.refresh_index().await
    }

    /// Reads the theme file now (startup) and then follows its changes (DMN-075).
    pub async fn watch_theme_file(&self, path: PathBuf) -> Result<JoinHandle<()>, ErrorBody> {
        self.watch_file(path, Self::apply_theme_file).await
    }

    async fn apply_theme_file(self, path: PathBuf, css: Option<String>) {
        info!(path = %path.display(), "theme file changed");
        if let Err(error) = self.set_theme(css).await {
            warn!(error = %error.message, "cannot apply the theme file");
        }
    }

    /// Reads the user's init file now (startup) and then follows its changes, publishing its version
    /// as `init_changed` on the `index` topic (DMN-185).
    pub async fn watch_init_file(&self, path: PathBuf) -> Result<JoinHandle<()>, ErrorBody> {
        *lock(&self.init_file) = Some(path.clone());
        self.watch_file(path, Self::apply_init_file).await
    }

    async fn apply_init_file(self, path: PathBuf, text: Option<String>) {
        info!(path = %path.display(), "init file changed");
        let init = text.map(|text| crate::widgets::init::version(text.as_bytes()));
        // Held like a refresh, so the event never interleaves with a refresh's.
        let _serial = self.refresh.lock().await;
        if self.index.with_state(|index| index.init != init) {
            self.index.push(IndexEvent::InitChanged { init });
        }
    }

    /// The followed user init file, if any (DMN-185).
    pub fn init_file(&self) -> Option<PathBuf> {
        lock(&self.init_file).clone()
    }

    /// The file's first read (startup): it applies only when an immediate second read agrees, else the watcher
    /// applies it once it settled (DMN-091).
    async fn first_read(path: PathBuf) -> Result<(WatchedFile, Option<Option<String>>), ErrorBody> {
        let mut file = WatchedFile::new(path);
        blocking(move || {
            let change = watch::first_read(&mut file);
            Ok((file, change))
        })
        .await
    }

    /// Reads `path` now (startup) and then hands every change of it to `apply`.
    async fn watch_file<F: Future<Output = ()> + Send + 'static>(
        &self,
        path: PathBuf,
        apply: fn(Self, PathBuf, Option<String>) -> F,
    ) -> Result<JoinHandle<()>, ErrorBody> {
        let (file, change) = Self::first_read(path.clone()).await?;
        if let Some(text) = change {
            apply(self.clone(), path.clone(), text).await;
        }
        Ok(self.follow(file, path, apply))
    }

    /// Polls `file`, read from `path`, and hands every change to `apply`, until the returned task is aborted.
    fn follow<F: Future<Output = ()> + Send + 'static>(
        &self,
        file: WatchedFile,
        path: PathBuf,
        apply: fn(Self, PathBuf, Option<String>) -> F,
    ) -> JoinHandle<()> {
        let services = self.clone();
        let (sender, mut changes) = mpsc::unbounded_channel();
        let watcher = AbortOnDrop(watch::watch(file, watch::POLL, sender));
        tokio::spawn(async move {
            let _watcher = watcher;
            while let Some(change) = changes.recv().await {
                apply(services.clone(), path.clone(), change).await;
            }
        })
    }

    /// The UI settings as published on the `index` topic (DMN-009, UI-097).
    pub fn settings(&self) -> Settings {
        lock(&self.settings).settings.clone()
    }

    /// Reads the config file now and then follows its `[ui]` and `[keymap]` (DMN-009); `startup` is its `[daemon]` as
    /// the daemon applied it. An invalid file is an error here, at startup.
    pub async fn follow_config_file(
        &self,
        path: PathBuf,
        startup: DaemonOptions,
    ) -> Result<JoinHandle<()>, ErrorBody> {
        let (file, change) = Self::first_read(path.clone()).await?;
        let parsed = match change.flatten() {
            Some(text) => ConfigFile::parse(&text)
                .map_err(|error| bad_request(format!("{}: {error}", path.display())))?,
            None => ConfigFile::default(),
        };
        {
            let mut state = lock(&self.settings);
            state.file = Some(path.clone());
            state.daemon = startup;
            state.settings = parsed.settings();
        }
        self.refresh_index().await?;
        Ok(self.follow(file, path, Self::reload_config_file))
    }

    /// A change of the config file: `[ui]` / `[keymap]` apply now, `[daemon]` at the next start; an invalid file keeps
    /// the previous settings and reports why in `Settings.error` (DMN-009).
    async fn reload_config_file(self, path: PathBuf, text: Option<String>) {
        let parsed = text.map_or_else(
            || Ok(ConfigFile::default()),
            |text| ConfigFile::parse(&text).map_err(|error| format!("{}: {error}", path.display())),
        );
        {
            let mut state = lock(&self.settings);
            match parsed {
                Ok(file) => {
                    if file.daemon != state.daemon {
                        warn!(path = %path.display(), "[daemon] of the config file changed: restart the daemon to apply it");
                        state.daemon = file.daemon.clone();
                    }
                    info!(path = %path.display(), "config file reloaded");
                    state.settings = file.settings();
                }
                Err(error) => {
                    warn!(%error, "config file invalid; the previous settings stay");
                    state.settings.error = Some(error);
                }
            }
        }
        self.refresh_index_quietly().await;
    }

    /// Merges `update` into the config file, written back with its comments, and pushes the new settings (DMN-009).
    /// Without a config file the settings live in memory.
    pub async fn set_settings(&self, update: SetSettingsParams) -> Result<Settings, ErrorBody> {
        let _write = self.settings_writes.lock().await;
        let (file, memory) = {
            let state = lock(&self.settings);
            (state.file.clone(), state.memory.clone())
        };
        let (text, parsed) = blocking(move || {
            let current = match &file {
                Some(path) => match std::fs::read_to_string(path) {
                    Ok(text) => text,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                    Err(error) => return Err(internal(format!("{}: {error}", path.display()))),
                },
                None => memory,
            };
            let text = write_back(&current, &update).map_err(bad_request)?;
            let parsed = ConfigFile::parse(&text).map_err(bad_request)?;
            if let Some(path) = &file {
                replace_file(path, text.as_bytes(), None)
                    .map_err(|error| internal(format!("{}: {error}", path.display())))?;
            }
            Ok((text, parsed))
        })
        .await?;
        let settings = parsed.settings();
        {
            let mut state = lock(&self.settings);
            if state.file.is_none() {
                state.memory = text;
            }
            state.settings = settings.clone();
        }
        self.refresh_index().await?;
        Ok(settings)
    }

    /// The thread's summary as published on the `index` topic.
    pub async fn thread_summary(&self, id: &str) -> Result<ThreadSummary, ErrorBody> {
        self.refresh_index().await?;
        self.index
            .with_state(|index| index.threads.iter().find(|thread| thread.id == id).cloned())
            .ok_or_else(|| not_found("thread", id))
    }

    /// The workspace as published on the `index` topic.
    pub async fn workspace_summary(&self, id: &str) -> Result<Workspace, ErrorBody> {
        self.refresh_index().await?;
        self.index
            .with_state(|index| index.workspaces.iter().find(|ws| ws.id == id).cloned())
            .ok_or_else(|| not_found("workspace", id))
    }

    /// The log of `workspace/<id>`, its status refreshed.
    pub async fn workspace_log(&self, id: &str) -> Result<Arc<WorkspaceLog>, ErrorBody> {
        self.workspace_status(id).await?;
        Ok(self.workspace_log_entry(id))
    }

    fn workspace_log_entry(&self, id: &str) -> Arc<WorkspaceLog> {
        let mut logs = lock(&self.workspace_logs);
        Arc::clone(logs.entry(id.to_owned()).or_insert_with(|| {
            Arc::new(EventLog::new(
                new_id(),
                Vec::new(),
                apply_workspace,
                WORKSPACE_CAPACITY,
                LIVE_BUFFER,
            ))
        }))
    }

    /// The git status of every repository of the workspace now (DMN-026), published on its topic when it changed.
    pub async fn workspace_status(&self, id: &str) -> Result<Vec<RepoStatus>, ErrorBody> {
        let workspaces = Arc::clone(&self.workspaces);
        let workspace_id = id.to_owned();
        let repos = blocking(move || {
            let repos = workspaces
                .discovered(&workspace_id)
                .map_err(workspace_error)?;
            repos
                .iter()
                .map(|repo| {
                    let (status, upstream) = workspaces.status(repo).map_err(workspace_error)?;
                    let diff = workspaces.diff(repo).map_err(workspace_error)?;
                    Ok(RepoStatus {
                        name: repo.name.clone(),
                        status: convert::git_status(&status, upstream.as_deref(), &diff),
                    })
                })
                .collect::<Result<Vec<_>, ErrorBody>>()
        })
        .await?;
        let log = self.workspace_log_entry(id);
        if log.with_state(|current| *current != repos) {
            log.push(WorkspaceEvent::Status {
                repos: repos.clone(),
            });
        }
        Ok(repos)
    }
}

/// The index as stored and scanned now; thread summaries keep their `updated_at` unless something in them changed
/// besides their place in the column (DMN-284).
fn load_index(
    store: &Store,
    workspaces: &Workspaces,
    supervisor: &Supervisor,
    previous: &IndexSnapshot,
    settings: Settings,
    counts: &HashMap<String, JobCounts>,
) -> Result<IndexSnapshot, ErrorBody> {
    let clients = store.created_clients()?;
    let forks = store.thread_forks()?;
    let threads = store.threads()?;
    let positions = store.positions()?;
    let position = |id: &str| positions.get(id).copied().unwrap_or_default();
    let mut scanned = Vec::new();
    for ws in workspaces.list().map_err(workspace_error)? {
        let repos = workspaces.discovered(&ws.id).map_err(workspace_error)?;
        scanned.push((ws, repos));
    }
    // Which workspaces hold each checkout (DMN-027).
    let mut holders: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for (ws, repos) in &scanned {
        for repo in repos {
            holders
                .entry(repo.canonical.clone())
                .or_default()
                .push(ws.id.clone());
        }
    }
    let workspaces = scanned
        .iter()
        .map(|(ws, repos)| {
            let thread_ids = threads
                .iter()
                .filter(|thread| thread.workspace_id == ws.id)
                .map(|thread| thread.id.clone())
                .collect();
            convert::workspace(
                ws,
                repos,
                thread_ids,
                &holders,
                &clients,
                position(&ws.id),
                counts.get(&ws.id).copied(),
            )
        })
        .collect();
    let threads = threads
        .iter()
        .map(|thread| {
            let before = previous
                .threads
                .iter()
                .find(|summary| summary.id == thread.id);
            let updated_at = before.map_or(thread.created_at, |before| before.updated_at);
            let mut summary = convert::thread_summary(
                thread,
                supervisor.status(&thread.id),
                &clients,
                &forks,
                updated_at,
                position(&thread.id),
            );
            // A new place, in its workspace or another one, is the user's own doing: not news (DMN-284).
            if before.is_some_and(|before| {
                *before
                    != ThreadSummary {
                        position: before.position,
                        workspace_id: before.workspace_id.clone(),
                        ..summary.clone()
                    }
            }) {
                summary.updated_at = now_ms();
            }
            summary
        })
        .collect();
    Ok(IndexSnapshot {
        workspaces,
        threads,
        theme: Theme {
            css: store.theme()?,
        },
        settings,
        init: previous.init.clone(),
        models: previous.models,
        login: previous.login.clone(),
    })
}

/// The events turning `current` into `fresh`.
fn diff_index(current: &IndexSnapshot, fresh: IndexSnapshot) -> Vec<IndexEvent> {
    let mut events = Vec::new();
    for workspace in &current.workspaces {
        if !fresh.workspaces.iter().any(|w| w.id == workspace.id) {
            events.push(IndexEvent::WorkspaceRemoved {
                workspace_id: workspace.id.clone(),
            });
        }
    }
    for workspace in fresh.workspaces {
        if !current.workspaces.contains(&workspace) {
            events.push(IndexEvent::WorkspaceUpserted { workspace });
        }
    }
    for thread in &current.threads {
        if !fresh.threads.iter().any(|t| t.id == thread.id) {
            events.push(IndexEvent::ThreadRemoved {
                thread_id: thread.id.clone(),
            });
        }
    }
    for thread in fresh.threads {
        if !current.threads.contains(&thread) {
            events.push(IndexEvent::ThreadUpserted { thread });
        }
    }
    if current.theme != fresh.theme {
        events.push(IndexEvent::ThemeChanged { theme: fresh.theme });
    }
    if current.settings != fresh.settings {
        events.push(IndexEvent::SettingsChanged {
            settings: fresh.settings,
        });
    }
    events
}
