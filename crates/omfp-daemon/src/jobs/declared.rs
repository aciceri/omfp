//! The runtime of declared jobs (DMN-320..DMN-336): each workspace's `.omfp/jobs.json` applied by job name, schedules
//! (DMN-323, DMN-325) on an injectable [`Clock`], services kept running with backoff (DMN-326), command runs on the
//! [`Jobs`] engine (DMN-327), prompt runs through the supervisor (DMN-328), every run in the store (DMN-330), the
//! `jobs/<id>` topic (DMN-336) and the counts of the index (DMN-335).
//!
//! Each workspace has one state behind a mutex, held only for short synchronous steps (events are pushed under it, so
//! they never interleave), and one async lock serializing the applies and writes of its file (DMN-332). Starting a run
//! runs in a task of its own, so cancelling whoever asked never leaves a process or a thread untracked.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use omfp_core::ProcessState;
use omfp_protocol::{
    CreatedBy, ErrorBody, ErrorCode, JobCounts, JobDefinition, JobInfo, JobLine, JobPrompt,
    JobRunStatus, JobRuns, JobStatus, JobTrigger, JobView, JobViewStatus, JobsEvent, JobsState,
    MissedPolicy, PromptThread, RestartPolicy, RunInfo, RunTrigger,
};
use omp_rpc::{PromptCommand, PromptStatus};
use serde_json::Value;
use tokio::sync::{broadcast::error::RecvError, oneshot, watch};
use tokio::task::AbortHandle;
use tracing::warn;

use super::cron::Schedule;
use super::{JobLog, Jobs, RunExit, file, stored_log};
use crate::agent::OmpCall;
use crate::event_log::EventLog;
use crate::paths::replace_file;
use crate::server::convert::{bad_request, internal, supervisor_error};
use crate::store::{self, JobRun, Provenance, RunEnd, Store, new_id};
use crate::supervisor::Supervisor;
use crate::threads::{NewThread, Threads};

/// A `jobs/<workspaceId>` topic (PRT-200).
pub type JobsLog = EventLog<JobsEvent, JobsState>;

const JOBS_CAPACITY: usize = 256;
const LIVE_BUFFER: usize = 1024;
/// A due time the scheduler reaches later than this is missed (DMN-325).
pub const MISSED_AFTER_MS: i64 = 60_000;
/// A service's first restart delay (DMN-326)…
pub const BACKOFF_FIRST_MS: i64 = 1_000;
/// …its longest…
pub const BACKOFF_MAX_MS: i64 = 60_000;
/// …and how long a run must last to reset it.
pub const LONG_RUN_MS: i64 = 60_000;
/// `jobs.runs` answers this many runs without `limit`, and at most [`RUNS_MAX`] (PRT-203).
const RUNS_DEFAULT: u32 = 20;
const RUNS_MAX: u32 = 50;
/// How long the shutdown waits for the runs it killed to be recorded.
const RECORD_WAIT: Duration = Duration::from_secs(5);

/// A boxed future the [`Clock`] returns.
pub type Sleep = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The time the scheduler and the service backoff follow; tests pass one driven by Tokio's paused time (TST-003).
pub trait Clock: Send + Sync + 'static {
    /// Unix milliseconds now.
    fn now_ms(&self) -> i64;
    /// Ends once [`Clock::now_ms`] reached `at_ms`.
    fn sleep_until(&self, at_ms: i64) -> Sleep;
}

/// The host's wall clock. A sleep wakes at least every minute to look at the time again: Tokio's timers follow a
/// monotonic clock, which stands still while the host sleeps, so a time due meanwhile is reached late (missed,
/// DMN-325) rather than hours later.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        store::now_ms()
    }

    fn sleep_until(&self, at_ms: i64) -> Sleep {
        Box::pin(async move {
            loop {
                let left = at_ms - store::now_ms();
                if left <= 0 {
                    break;
                }
                let left = u64::try_from(left.min(60_000)).unwrap_or(0);
                tokio::time::sleep(Duration::from_millis(left)).await;
            }
        })
    }
}

/// A clock on Tokio's time from `origin_ms` on: with `tokio::time::pause()` it only moves when the test advances it.
#[derive(Debug, Clone, Copy)]
pub struct TokioClock {
    origin_ms: i64,
    origin: tokio::time::Instant,
}

impl TokioClock {
    #[must_use]
    pub fn new(origin_ms: i64) -> Self {
        Self {
            origin_ms,
            origin: tokio::time::Instant::now(),
        }
    }
}

impl Clock for TokioClock {
    fn now_ms(&self) -> i64 {
        let elapsed = i64::try_from(self.origin.elapsed().as_millis()).unwrap_or(i64::MAX);
        self.origin_ms.saturating_add(elapsed)
    }

    fn sleep_until(&self, at_ms: i64) -> Sleep {
        let after = u64::try_from(at_ms - self.origin_ms).unwrap_or(0);
        let deadline = self.origin + Duration::from_millis(after);
        Box::pin(tokio::time::sleep_until(deadline))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ErrorBody> + Send + 'static,
) -> Result<T, ErrorBody> {
    tokio::task::spawn_blocking(work).await.map_err(internal)?
}

fn conflict(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::Conflict, message)
}

fn not_found(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::NotFound, message)
}

#[allow(clippy::trivially_copy_pass_by_ref)] // `EventLog` takes `fn(&mut S, &E)`.
fn apply_jobs(state: &mut JobsState, event: &JobsEvent) {
    match event {
        JobsEvent::JobsSet { jobs, error } => {
            state.jobs.clone_from(jobs);
            state.error.clone_from(error);
        }
        JobsEvent::JobUpdated { job } => {
            match state.jobs.binary_search_by(|kept| kept.name.cmp(&job.name)) {
                Ok(at) => state.jobs[at] = job.clone(),
                Err(at) => state.jobs.insert(at, job.clone()),
            }
        }
        JobsEvent::RunStarted { .. } | JobsEvent::RunEnded { .. } => {}
    }
}

// Conversions between the store's rows and the protocol's runs.

fn trigger_name(trigger: RunTrigger) -> &'static str {
    match trigger {
        RunTrigger::Manual => "manual",
        RunTrigger::Schedule => "schedule",
        RunTrigger::Missed => "missed",
        RunTrigger::Service => "service",
        RunTrigger::Start => "start",
    }
}

fn parse_trigger(name: &str) -> RunTrigger {
    match name {
        "schedule" => RunTrigger::Schedule,
        "missed" => RunTrigger::Missed,
        "service" => RunTrigger::Service,
        "start" => RunTrigger::Start,
        _ => RunTrigger::Manual,
    }
}

fn status_name(status: JobRunStatus) -> &'static str {
    match status {
        JobRunStatus::Running => "running",
        JobRunStatus::Succeeded => "succeeded",
        JobRunStatus::Failed => "failed",
        JobRunStatus::Killed => "killed",
        JobRunStatus::Skipped => "skipped",
        JobRunStatus::Interrupted => "interrupted",
    }
}

fn parse_status(name: &str) -> JobRunStatus {
    match name {
        "running" => JobRunStatus::Running,
        "succeeded" => JobRunStatus::Succeeded,
        "killed" => JobRunStatus::Killed,
        "skipped" => JobRunStatus::Skipped,
        "interrupted" => JobRunStatus::Interrupted,
        _ => JobRunStatus::Failed,
    }
}

fn created_by(provenance: &Provenance) -> CreatedBy {
    match provenance {
        Provenance::User => CreatedBy::User,
        Provenance::Mcp => CreatedBy::Mcp,
        Provenance::Thread(thread_id) => CreatedBy::Thread {
            thread_id: thread_id.clone(),
        },
        Provenance::Job(job) => CreatedBy::Job { job: job.clone() },
    }
}

fn provenance(created_by: &CreatedBy) -> Provenance {
    match created_by {
        CreatedBy::User => Provenance::User,
        CreatedBy::Mcp => Provenance::Mcp,
        CreatedBy::Thread { thread_id } => Provenance::Thread(thread_id.clone()),
        CreatedBy::Job { job } => Provenance::Job(job.clone()),
    }
}

fn run_info(run: &JobRun) -> RunInfo {
    RunInfo {
        id: run.id.clone(),
        workspace_id: run.workspace_id.clone(),
        job: Some(run.job.clone()),
        trigger: parse_trigger(&run.trigger),
        started_by: run.started_by.as_ref().map(created_by),
        thread_id: run.thread_id.clone(),
        command: run.command.clone(),
        started_at: run.started_at,
        ended_at: run.ended_at,
        status: parse_status(&run.status),
        reason: run.reason.clone(),
        exit_code: run.exit_code,
        signal: run.signal,
    }
}

fn job_run(run: &RunInfo) -> JobRun {
    JobRun {
        id: run.id.clone(),
        workspace_id: run.workspace_id.clone(),
        job: run.job.clone().unwrap_or_default(),
        trigger: trigger_name(run.trigger).to_owned(),
        started_by: run.started_by.as_ref().map(provenance),
        thread_id: run.thread_id.clone(),
        started_at: run.started_at,
        ended_at: run.ended_at,
        status: status_name(run.status).to_owned(),
        reason: run.reason.clone(),
        exit_code: run.exit_code,
        signal: run.signal,
        command: run.command.clone(),
    }
}

/// `<YYYY-MM-DD HH:MM>` of `at_ms` in `tz` (DMN-328).
fn local_minute(at_ms: i64, tz: &TimeZone) -> String {
    Timestamp::from_millisecond(at_ms)
        .map(|at| {
            at.to_zoned(tz.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// Why the daemon kills a running run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kill {
    /// `jobs.kill`: a service stays stopped (DMN-326).
    Protocol,
    /// The job was removed or disabled (DMN-322).
    Teardown,
    /// A running service changed: it starts again with the new definition (DMN-322).
    Replace,
}

#[derive(Debug, Clone)]
enum RunKind {
    Command,
    Prompt { thread_id: String },
}

/// A run that runs now, of a declared job or of one torn down meanwhile.
#[derive(Debug)]
struct Running {
    job: String,
    info: RunInfo,
    kind: RunKind,
    kill: Option<Kill>,
}

/// A service between runs (DMN-326).
#[derive(Debug)]
struct Service {
    /// Killed through the protocol, or ended under `on-failure` without failing.
    stopped: bool,
    /// When it starts again.
    waiting_until: Option<i64>,
    /// The delay of the next restart after a short run.
    backoff_ms: i64,
    /// The restart timer and its ticket; a timer whose ticket is no longer here does nothing.
    restart: Option<(u64, AbortHandle)>,
    tickets: u64,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            stopped: false,
            waiting_until: None,
            backoff_ms: BACKOFF_FIRST_MS,
            restart: None,
            tickets: 0,
        }
    }
}

impl Service {
    fn cancel(&mut self) {
        self.waiting_until = None;
        if let Some((_, timer)) = self.restart.take() {
            timer.abort();
        }
    }
}

/// A declared job of the last valid file.
#[derive(Debug)]
struct Entry {
    def: JobDefinition,
    /// The running run's id.
    run: Option<String>,
    last_run: Option<RunInfo>,
    /// DMN-325.
    last_due_at: i64,
    next_at: Option<i64>,
    scheduler: Option<AbortHandle>,
    service: Service,
    /// Bumped by every change of the definition, so a run that was starting meanwhile is reconciled (DMN-322).
    generation: u64,
}

impl Entry {
    fn status(&self) -> JobViewStatus {
        if self.run.is_some() {
            JobViewStatus::Running
        } else if !self.def.enabled {
            JobViewStatus::Disabled
        } else if self.service.waiting_until.is_some() {
            JobViewStatus::Waiting
        } else if self.def.trigger == JobTrigger::Service && self.service.stopped {
            JobViewStatus::Stopped
        } else {
            JobViewStatus::Idle
        }
    }

    fn view(&self, name: &str, runs: &HashMap<String, Running>) -> JobView {
        let next_at = match self.def.trigger {
            JobTrigger::Schedule if self.def.enabled => self.next_at,
            JobTrigger::Service => self.service.waiting_until,
            _ => None,
        };
        JobView {
            name: name.to_owned(),
            job: self.def.clone(),
            status: self.status(),
            run: self
                .run
                .as_ref()
                .and_then(|id| runs.get(id))
                .map(|running| running.info.clone()),
            last_run: self.last_run.clone(),
            next_at,
        }
    }

    /// Stops what sets the job up (DMN-322): its timers, and its running command run, killed as `jobs.kill` does.
    fn tear_down(&mut self, runs: &mut HashMap<String, Running>, jobs: &Jobs) {
        if let Some(scheduler) = self.scheduler.take() {
            scheduler.abort();
        }
        self.next_at = None;
        self.service.cancel();
        if let Some(running) = self.run.as_ref().and_then(|id| runs.get_mut(id))
            && matches!(running.kind, RunKind::Command)
        {
            running.kill = Some(Kill::Teardown);
            let _ = jobs.kill(&running.info.id);
        }
    }
}

#[derive(Debug, Default)]
struct SpaceState {
    jobs: BTreeMap<String, Entry>,
    error: Option<String>,
    /// Running runs by id.
    runs: HashMap<String, Running>,
    /// The jobs a run is being started for, by name: the workspace's, not a declaration's, so a job declared again
    /// meanwhile sees its run as running (DMN-324).
    starting: HashSet<String>,
    /// The workspace is being removed: nothing starts or is recorded any more.
    removed: bool,
}

impl SpaceState {
    fn jobs_state(&self) -> JobsState {
        JobsState {
            jobs: self
                .jobs
                .iter()
                .map(|(name, entry)| entry.view(name, &self.runs))
                .collect(),
            error: self.error.clone(),
        }
    }

    /// PRT-210: `None` without jobs and without an error.
    fn counts(&self) -> Option<JobCounts> {
        let mut counts = JobCounts {
            error: self.error.is_some(),
            ..JobCounts::default()
        };
        for entry in self.jobs.values() {
            counts.total += 1;
            match entry.def.trigger {
                JobTrigger::Manual => counts.manual += 1,
                JobTrigger::Schedule => counts.schedule += 1,
                JobTrigger::Service => counts.service += 1,
            }
            if entry.run.is_some() {
                counts.running += 1;
            }
            if entry.def.enabled
                && entry
                    .last_run
                    .as_ref()
                    .is_some_and(|run| run.status == JobRunStatus::Failed)
            {
                counts.failing += 1;
            }
        }
        (counts.total > 0 || counts.error).then_some(counts)
    }
}

/// The jobs file as last applied.
#[derive(Debug, PartialEq, Eq)]
enum Applied {
    /// Not read yet, or unreadable.
    Never,
    /// No file.
    Missing,
    Bytes(Vec<u8>),
}

impl From<Option<Vec<u8>>> for Applied {
    fn from(bytes: Option<Vec<u8>>) -> Self {
        bytes.map_or(Self::Missing, Self::Bytes)
    }
}

/// A jobs file's size and modification time; `None` without a file.
type Stamp = Option<(u64, Option<std::time::SystemTime>)>;

/// One workspace's declared jobs.
struct Space {
    id: String,
    folder: PathBuf,
    log: Arc<JobsLog>,
    /// Serializes applying and writing the file (DMN-332); holds what was applied last.
    file: tokio::sync::Mutex<Applied>,
    /// Set once the file was first read and applied; callers wait for it ([`Declared::ensure`]).
    loaded: tokio::sync::OnceCell<()>,
    state: Mutex<SpaceState>,
}

/// What an apply does once the workspace's state is unlocked.
enum Setup {
    Service(String),
    /// Stores a job's new `last_due_at` (DMN-325).
    LastDue(String, i64),
}

/// How a run ended, before it is recorded.
struct Ended {
    status: JobRunStatus,
    reason: Option<String>,
    exit_code: Option<i32>,
    signal: Option<i32>,
    at: i64,
    output_tail: Option<String>,
    line_count: u64,
}

struct Inner {
    store: Arc<Store>,
    jobs: Jobs,
    supervisor: Supervisor,
    threads: Arc<Threads>,
    clock: Arc<dyn Clock>,
    tz: TimeZone,
    /// Every workspace whose declared jobs were read, by id.
    spaces: Mutex<HashMap<String, Arc<Space>>>,
    counts: Mutex<HashMap<String, JobCounts>>,
    /// Each workspace's jobs file as the last poll round saw it (DMN-322).
    stamps: Mutex<HashMap<String, Stamp>>,
    /// The source of every [`Entry::generation`], unique across declarations, so a job declared again under its old
    /// name never looks like the declaration a starting run began under.
    generations: AtomicU64,
    /// Bumped when the index should be refreshed: counts changed, a thread was made (DMN-335).
    changed: watch::Sender<u64>,
    closed: AtomicBool,
    /// Command runs whose end is not recorded yet.
    recording: watch::Sender<usize>,
    /// Tests hold a registration here, after its store write: told on arrival, released by the receiver.
    #[cfg(test)]
    register_gate: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
    /// Tests hold a workspace's first load here, under its file lock, the same way.
    #[cfg(test)]
    load_gate: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
}

/// The declared jobs of every workspace. Cheap to clone.
#[derive(Clone)]
pub struct Declared {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Declared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Declared").finish_non_exhaustive()
    }
}

/// Counts a command run's watcher until it recorded the end.
struct Recording(watch::Sender<usize>);

impl Drop for Recording {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count = count.saturating_sub(1));
    }
}

impl Declared {
    #[must_use]
    pub fn new(
        store: Arc<Store>,
        jobs: Jobs,
        supervisor: Supervisor,
        threads: Arc<Threads>,
        clock: Arc<dyn Clock>,
        tz: TimeZone,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                jobs,
                supervisor,
                threads,
                clock,
                tz,
                spaces: Mutex::new(HashMap::new()),
                counts: Mutex::new(HashMap::new()),
                stamps: Mutex::new(HashMap::new()),
                generations: AtomicU64::new(0),
                changed: watch::Sender::new(0),
                closed: AtomicBool::new(false),
                recording: watch::Sender::new(0),
                #[cfg(test)]
                register_gate: Mutex::new(None),
                #[cfg(test)]
                load_gate: Mutex::new(None),
            }),
        }
    }

    /// At startup: runs the previous daemon left `running` died with it (DMN-330).
    pub fn recover(&self) -> Result<usize, ErrorBody> {
        Ok(self.inner.store.interrupt_job_runs()?)
    }

    /// Each workspace's [`JobCounts`] (PRT-210); a workspace without jobs nor error has none.
    #[must_use]
    pub fn counts(&self) -> HashMap<String, JobCounts> {
        lock(&self.inner.counts).clone()
    }

    /// Changes whenever the index should be refreshed (DMN-335).
    #[must_use]
    pub fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.inner.changed.subscribe()
    }

    fn next_generation(&self) -> u64 {
        self.inner.generations.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    fn space(&self, id: &str) -> Option<Arc<Space>> {
        lock(&self.inner.spaces).get(id).cloned()
    }

    /// The workspace's declared jobs, read at the first use; `not_found` for an unknown workspace. Every caller waits
    /// until that first read is applied, so none sees a workspace's jobs before they are known.
    async fn ensure(&self, id: &str) -> Result<Arc<Space>, ErrorBody> {
        if let Some(space) = self.space(id) {
            self.loaded(&space).await;
            return Ok(space);
        }
        let store = Arc::clone(&self.inner.store);
        let workspace_id = id.to_owned();
        let workspace = blocking(move || Ok(store.workspace(&workspace_id)?)).await?;
        let space = Arc::clone(
            lock(&self.inner.spaces)
                .entry(id.to_owned())
                .or_insert_with(|| {
                    Arc::new(Space {
                        id: workspace.id,
                        folder: workspace.path,
                        log: Arc::new(EventLog::new(
                            new_id(),
                            JobsState::default(),
                            apply_jobs,
                            JOBS_CAPACITY,
                            LIVE_BUFFER,
                        )),
                        file: tokio::sync::Mutex::new(Applied::Never),
                        loaded: tokio::sync::OnceCell::new(),
                        state: Mutex::new(SpaceState::default()),
                    })
                }),
        );
        self.loaded(&space).await;
        Ok(space)
    }

    /// Waits until the workspace's jobs file was read and applied once (the first caller does it).
    async fn loaded(&self, space: &Arc<Space>) {
        space
            .loaded
            .get_or_init(|| async {
                let mut applied = space.file.lock().await;
                #[cfg(test)]
                {
                    let gate = lock(&self.inner.load_gate).take();
                    if let Some((arrived, release)) = gate {
                        let _ = arrived.send(());
                        let _ = release.await;
                    }
                }
                if *applied == Applied::Never {
                    self.read_and_apply(space, &mut applied).await;
                }
            })
            .await;
    }

    /// Reads the workspace's `jobs.json` again and applies it when it changed (DMN-322): at startup, on every rescan
    /// and when a poll round sees its file change ([`Declared::poll_files`]). A workspace gone meanwhile is forgotten.
    pub async fn reload(&self, id: &str) {
        match self.ensure(id).await {
            Ok(space) => {
                let mut applied = space.file.lock().await;
                self.read_and_apply(&space, &mut applied).await;
            }
            Err(error) if error.code == ErrorCode::NotFound => {
                lock(&self.inner.spaces).remove(id);
            }
            Err(error) => warn!(workspace = %id, error = %error.message, "cannot read the jobs"),
        }
    }

    /// A poll round (DMN-322): the jobs file of every workspace, watched or not, is read again and applied when its
    /// size or modification time changed since the previous round (or the workspace is new to the rounds).
    pub async fn poll_files(&self) {
        let store = Arc::clone(&self.inner.store);
        let seen = blocking(move || {
            Ok(store
                .workspaces()?
                .into_iter()
                .map(|ws| {
                    let stamp = std::fs::metadata(file::path(&ws.path))
                        .ok()
                        .map(|metadata| (metadata.len(), metadata.modified().ok()));
                    (ws.id, stamp)
                })
                .collect::<Vec<_>>())
        })
        .await;
        let seen = match seen {
            Ok(seen) => seen,
            Err(error) => {
                warn!(error = %error.message, "cannot look at the jobs files");
                return;
            }
        };
        let changed: Vec<String> = {
            let mut stamps = lock(&self.inner.stamps);
            stamps.retain(|id, _| seen.iter().any(|(seen, _)| seen == id));
            seen.into_iter()
                .filter_map(|(id, stamp)| {
                    (stamps.insert(id.clone(), stamp) != Some(stamp)).then_some(id)
                })
                .collect()
        };
        for id in changed {
            self.reload(&id).await;
        }
    }

    async fn read_and_apply(&self, space: &Arc<Space>, applied: &mut Applied) {
        let path = file::path(&space.folder);
        let read = blocking(move || {
            Ok(match std::fs::read(&path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(format!("cannot read .omfp/jobs.json: {error}")),
            })
        })
        .await;
        match read {
            Ok(Ok(bytes)) => self.apply_bytes(space, applied, bytes).await,
            Ok(Err(error)) => {
                *applied = Applied::Never;
                self.apply(space, Err(error)).await;
            }
            Err(error) => {
                warn!(workspace = %space.id, error = %error.message, "cannot read the jobs");
            }
        }
    }

    /// Applies the file's bytes unless they are the ones applied last.
    async fn apply_bytes(&self, space: &Arc<Space>, applied: &mut Applied, bytes: Option<Vec<u8>>) {
        let read = Applied::from(bytes);
        if *applied == read {
            return;
        }
        let parsed = match &read {
            Applied::Never | Applied::Missing => Ok(BTreeMap::new()),
            Applied::Bytes(bytes) => file::parse_jobs_file(bytes),
        };
        *applied = read;
        self.apply(space, parsed).await;
    }

    /// Applies a valid file by job name, or keeps the last valid jobs with the error of a refused one (DMN-322).
    #[allow(clippy::too_many_lines)] // One pass over the old and new jobs, under one lock.
    async fn apply(
        &self,
        space: &Arc<Space>,
        parsed: Result<BTreeMap<String, JobDefinition>, String>,
    ) {
        let definitions = match parsed {
            Ok(definitions) => definitions,
            Err(error) => {
                let mut state = lock(&space.state);
                if state.error.as_ref() != Some(&error) {
                    state.error = Some(error);
                    self.publish_all(space, &state);
                }
                return;
            }
        };
        // What the store knows of the new jobs, read before locking.
        let new: Vec<String> = {
            let state = lock(&space.state);
            definitions
                .keys()
                .filter(|name| !state.jobs.contains_key(*name))
                .cloned()
                .collect()
        };
        let (store, workspace_id, now) = (
            Arc::clone(&self.inner.store),
            space.id.clone(),
            self.inner.clock.now_ms(),
        );
        let known = blocking(move || {
            new.into_iter()
                .map(|name| {
                    let last = store.last_job_run(&workspace_id, &name)?;
                    let kept = store.job_state(&workspace_id, &name)?;
                    // A job first seen now never catches up on earlier times (DMN-325).
                    let last_due_at = if let Some(at) = kept.last_due_at {
                        at
                    } else {
                        store.set_job_last_due(&workspace_id, &name, now)?;
                        now
                    };
                    Ok((name, (last.as_ref().map(run_info), last_due_at)))
                })
                .collect::<Result<HashMap<_, _>, ErrorBody>>()
        })
        .await;
        let mut known = match known {
            Ok(known) => known,
            Err(error) => {
                warn!(workspace = %space.id, error = %error.message, "cannot read the jobs' runs");
                HashMap::new()
            }
        };
        let mut setups = Vec::new();
        {
            let mut guard = lock(&space.state);
            let state = &mut *guard;
            if state.removed {
                return;
            }
            state.error = None;
            let removed: Vec<String> = state
                .jobs
                .keys()
                .filter(|name| !definitions.contains_key(*name))
                .cloned()
                .collect();
            for name in removed {
                if let Some(mut entry) = state.jobs.remove(&name) {
                    entry.tear_down(&mut state.runs, &self.inner.jobs);
                }
            }
            for (name, def) in definitions {
                match state.jobs.get_mut(&name) {
                    None => {
                        let (last_run, last_due_at) = known.remove(&name).unwrap_or((None, now));
                        // The running run is the workspace's by job name (DMN-324): a run of an earlier declaration
                        // still running (a prompt run goes on, DMN-322) is this job's run.
                        let run = state
                            .runs
                            .iter()
                            .find(|(_, running)| running.job == name)
                            .map(|(id, _)| id.clone());
                        let mut entry = Entry {
                            def,
                            run,
                            last_run,
                            last_due_at,
                            next_at: None,
                            scheduler: None,
                            service: Service::default(),
                            generation: self.next_generation(),
                        };
                        let starting = state.starting.contains(&name);
                        self.set_up(
                            space,
                            &name,
                            &mut entry,
                            (&mut state.runs, starting),
                            &mut setups,
                        );
                        state.jobs.insert(name, entry);
                    }
                    Some(entry) if entry.def != def => {
                        let starting = state.starting.contains(&name);
                        self.change(
                            space,
                            &name,
                            entry,
                            def,
                            (&mut state.runs, starting),
                            &mut setups,
                        );
                    }
                    Some(_) => {}
                }
            }
            self.publish_all(space, state);
        }
        for setup in setups {
            match setup {
                Setup::Service(name) => {
                    self.spawn_start(space, name, RunTrigger::Service, None, false);
                }
                Setup::LastDue(name, at) => {
                    let (store, workspace_id) = (Arc::clone(&self.inner.store), space.id.clone());
                    let stored =
                        blocking(move || Ok(store.set_job_last_due(&workspace_id, &name, at)?))
                            .await;
                    if let Err(error) = stored {
                        warn!(workspace = %space.id, error = %error.message, "cannot record a job's due time");
                    }
                }
            }
        }
    }

    /// Sets up an enabled job (DMN-322): a schedule waits for its next time, a service starts — or, while a run of
    /// it is still being killed (torn down, `jobs.kill`, a change), starts once that run ended (DMN-326).
    fn set_up(
        &self,
        space: &Arc<Space>,
        name: &str,
        entry: &mut Entry,
        (runs, starting): (&mut HashMap<String, Running>, bool),
        setups: &mut Vec<Setup>,
    ) {
        if !entry.def.enabled {
            return;
        }
        match entry.def.trigger {
            JobTrigger::Manual => {}
            JobTrigger::Schedule => {
                let schedule = entry
                    .def
                    .schedule
                    .as_deref()
                    .and_then(|expression| Schedule::parse(expression).ok());
                if let Some(schedule) = schedule {
                    // Known at once, so the `jobs_set` of the apply carries it.
                    entry.next_at = schedule.next_after(entry.last_due_at, &self.inner.tz);
                    let task = tokio::spawn(self.clone().schedule(
                        Arc::clone(space),
                        name.to_owned(),
                        schedule,
                        entry.def.missed,
                        entry.last_due_at,
                    ));
                    entry.scheduler = Some(task.abort_handle());
                }
            }
            JobTrigger::Service => {
                entry.service.stopped = false;
                // A run being killed, whatever its trigger, makes way for the enabled service when it ends.
                if let Some(running) = entry.run.as_ref().and_then(|id| runs.get_mut(id))
                    && running.kill.is_some()
                {
                    running.kill = Some(Kill::Replace);
                }
                if entry.run.is_none() && !starting && entry.service.waiting_until.is_none() {
                    setups.push(Setup::Service(name.to_owned()));
                }
            }
        }
    }

    /// A changed job (DMN-322): its running run goes on under the old definition, except a run of the service
    /// (trigger `service`), stopped and started again with the new definition when the job is still an enabled
    /// service; its timers follow the new definition.
    fn change(
        &self,
        space: &Arc<Space>,
        name: &str,
        entry: &mut Entry,
        def: JobDefinition,
        (runs, starting): (&mut HashMap<String, Running>, bool),
        setups: &mut Vec<Setup>,
    ) {
        entry.generation = self.next_generation();
        if let Some(scheduler) = entry.scheduler.take() {
            scheduler.abort();
        }
        entry.next_at = None;
        if !def.enabled {
            entry.tear_down(runs, &self.inner.jobs);
            entry.def = def;
            return;
        }
        if def.trigger != JobTrigger::Service {
            entry.service.cancel();
        }
        // Only the run's trigger decides: a manual run of a service goes on (DMN-322).
        if let Some(running) = entry.run.as_ref().and_then(|id| runs.get_mut(id))
            && running.info.trigger == RunTrigger::Service
            && matches!(running.kind, RunKind::Command)
        {
            // Its end starts the service again if the job still is one (`after_end`).
            running.kill = Some(Kill::Replace);
            let _ = self.inner.jobs.kill(&running.info.id);
        }
        // Enabled again: times while it was disabled are never run nor caught up (DMN-325).
        if !entry.def.enabled {
            entry.last_due_at = self.inner.clock.now_ms();
            setups.push(Setup::LastDue(name.to_owned(), entry.last_due_at));
        }
        entry.def = def;
        if entry.def.trigger == JobTrigger::Service {
            entry.service.backoff_ms = BACKOFF_FIRST_MS;
        }
        self.set_up(space, name, entry, (runs, starting), setups);
    }

    /// Publishes the whole state as `jobs_set` when it changed (DMN-336), and the counts.
    fn publish_all(&self, space: &Space, state: &SpaceState) {
        let jobs_state = state.jobs_state();
        if space.log.with_state(|current| *current != jobs_state) {
            space.log.push(JobsEvent::JobsSet {
                jobs: jobs_state.jobs,
                error: jobs_state.error,
            });
        }
        self.publish_counts(&space.id, state);
    }

    /// Publishes `job_updated` when the job's view changed (DMN-336), and the counts.
    fn publish(&self, space: &Space, state: &SpaceState, name: &str) {
        if let Some(entry) = state.jobs.get(name) {
            let view = entry.view(name, &state.runs);
            let changed = space.log.with_state(|current| {
                current
                    .jobs
                    .iter()
                    .find(|kept| kept.name == name)
                    .is_none_or(|kept| *kept != view)
            });
            if changed {
                space.log.push(JobsEvent::JobUpdated { job: view });
            }
        }
        self.publish_counts(&space.id, state);
    }

    fn publish_counts(&self, workspace_id: &str, state: &SpaceState) {
        let counts = if state.removed { None } else { state.counts() };
        let changed = {
            let mut all = lock(&self.inner.counts);
            if all.get(workspace_id) == counts.as_ref() {
                false
            } else {
                match counts {
                    Some(counts) => all.insert(workspace_id.to_owned(), counts),
                    None => all.remove(workspace_id),
                };
                true
            }
        };
        if changed {
            self.inner.changed.send_modify(|count| *count += 1);
        }
    }

    // The scheduler (DMN-323, DMN-325).

    async fn schedule(
        self,
        space: Arc<Space>,
        name: String,
        schedule: Schedule,
        missed: MissedPolicy,
        mut last_due_at: i64,
    ) {
        loop {
            let Some(due) = schedule.next_after(last_due_at, &self.inner.tz) else {
                return;
            };
            self.inner.clock.sleep_until(due).await;
            let now = self.inner.clock.now_ms();
            let tz = self.inner.tz.clone();
            let (store, workspace_id, job) = (
                Arc::clone(&self.inner.store),
                space.id.clone(),
                name.clone(),
            );
            // Every time missed meanwhile counts as handled (DMN-325); counted off the async workers.
            let handled = blocking(move || {
                let mut latest = due;
                if now - due > MISSED_AFTER_MS {
                    while let Some(next) = schedule.next_after(latest, &tz)
                        && next <= now
                    {
                        latest = next;
                    }
                }
                store.set_job_last_due(&workspace_id, &job, latest)?;
                Ok(latest)
            })
            .await;
            last_due_at = match handled {
                Ok(latest) => latest,
                Err(error) => {
                    warn!(job = %name, error = %error.message, "cannot record a job's due time");
                    due
                }
            };
            {
                let mut state = lock(&space.state);
                let Some(entry) = state.jobs.get_mut(&name) else {
                    return;
                };
                entry.last_due_at = last_due_at;
                // The next time is published before the run this one starts (DMN-336).
                entry.next_at = schedule.next_after(last_due_at, &self.inner.tz);
                self.publish(&space, &state, &name);
            }
            let trigger = if now - due > MISSED_AFTER_MS {
                (missed == MissedPolicy::Run).then_some(RunTrigger::Missed)
            } else {
                Some(RunTrigger::Schedule)
            };
            if let Some(trigger) = trigger {
                // Its own task: aborting the scheduler never cuts a start short.
                let _ = self
                    .spawn_start(&space, name.clone(), trigger, None, false)
                    .await;
            }
        }
    }

    // Runs.

    fn spawn_start(
        &self,
        space: &Arc<Space>,
        name: String,
        trigger: RunTrigger,
        started_by: Option<Provenance>,
        manual: bool,
    ) -> tokio::task::JoinHandle<Result<RunInfo, ErrorBody>> {
        let (declared, space) = (self.clone(), Arc::clone(space));
        tokio::spawn(async move {
            let result = declared
                .start(&space, &name, trigger, started_by, manual)
                .await;
            if let Err(error) = &result
                && !manual
            {
                warn!(workspace = %space.id, job = %name, error = %error.message, "cannot start a job's run");
            }
            result
        })
    }

    /// Starts a run of the job (DMN-324): `manual` (`jobs.run`) answers `conflict` where a due time records a
    /// `skipped` run.
    async fn start(
        &self,
        space: &Arc<Space>,
        name: &str,
        trigger: RunTrigger,
        started_by: Option<Provenance>,
        manual: bool,
    ) -> Result<RunInfo, ErrorBody> {
        let reserved = {
            let mut guard = lock(&space.state);
            let state = &mut *guard;
            if state.removed || self.closed() {
                return Err(internal(
                    "the daemon is shutting down or the workspace is gone",
                ));
            }
            let Some(entry) = state.jobs.get_mut(name) else {
                return Err(not_found(format!("no job {name} in .omfp/jobs.json")));
            };
            if !entry.def.enabled {
                return Err(conflict(format!("job {name} is disabled")));
            }
            if entry.run.is_some() || state.starting.contains(name) {
                Err(entry.def.command.clone())
            } else {
                state.starting.insert(name.to_owned());
                if trigger == RunTrigger::Manual || trigger == RunTrigger::Service {
                    // A run by hand starts a stopped or waiting service at once, its backoff reset (PRT-204).
                    if manual {
                        entry.service.backoff_ms = BACKOFF_FIRST_MS;
                    }
                    entry.service.cancel();
                    entry.service.stopped = false;
                }
                Ok((entry.def.clone(), entry.generation))
            }
        };
        let (def, generation) = match reserved {
            Ok(reserved) => reserved,
            Err(command) => {
                if manual {
                    return Err(conflict(format!("job {name} is running")));
                }
                return self
                    .skip(space, name, trigger, started_by, None, command, "running")
                    .await;
            }
        };
        let base = RunInfo {
            id: new_id(),
            workspace_id: space.id.clone(),
            job: Some(name.to_owned()),
            trigger,
            started_by: started_by.as_ref().map(created_by),
            thread_id: None,
            command: def.command.clone(),
            started_at: self.inner.clock.now_ms(),
            ended_at: None,
            status: JobRunStatus::Running,
            reason: None,
            exit_code: None,
            signal: None,
        };
        match (&def.command, &def.prompt) {
            (Some(command), _) => {
                self.start_command(space, name, &def, command, base, generation)
                    .await
            }
            (None, Some(prompt)) => {
                self.start_prompt(space, name, prompt, base, manual, generation)
                    .await
            }
            (None, None) => {
                self.release(space, name, &base);
                Err(internal(format!("job {name} has no action")))
            }
        }
    }

    /// Clears a job's `starting` mark when its start gave up, then reconciles the current definition: a job changed
    /// meanwhile into an enabled service starts now (DMN-322, DMN-326).
    fn release(&self, space: &Arc<Space>, name: &str, run: &RunInfo) {
        {
            let mut state = lock(&space.state);
            state.starting.remove(name);
            self.publish(space, &state, name);
        }
        self.after_end(space, name, run, Some(Kill::Replace));
    }

    /// Records a `skipped` run (DMN-324, DMN-328) and publishes its `run_ended`.
    #[allow(clippy::too_many_arguments)] // The run's fields, all known to the caller.
    async fn skip(
        &self,
        space: &Arc<Space>,
        name: &str,
        trigger: RunTrigger,
        started_by: Option<Provenance>,
        thread_id: Option<String>,
        command: Option<String>,
        reason: &str,
    ) -> Result<RunInfo, ErrorBody> {
        let now = self.inner.clock.now_ms();
        let run = RunInfo {
            id: new_id(),
            workspace_id: space.id.clone(),
            job: Some(name.to_owned()),
            trigger,
            started_by: started_by.as_ref().map(created_by),
            thread_id,
            command,
            started_at: now,
            ended_at: Some(now),
            status: JobRunStatus::Skipped,
            reason: Some(reason.to_owned()),
            exit_code: None,
            signal: None,
        };
        self.record_ended(space, name, &run, None).await;
        Ok(run)
    }

    /// Stores a run that ended as it began (skipped, or failed to start) and publishes `run_ended` (DMN-336).
    async fn record_ended(
        &self,
        space: &Arc<Space>,
        name: &str,
        run: &RunInfo,
        starting: Option<()>,
    ) {
        let (store, row) = (Arc::clone(&self.inner.store), job_run(run));
        if let Err(error) = blocking(move || Ok(store.insert_job_run(&row)?)).await {
            warn!(job = %name, error = %error.message, "cannot record a job's run");
        }
        let mut state = lock(&space.state);
        if starting.is_some() {
            state.starting.remove(name);
        }
        if state.removed {
            return;
        }
        space.log.push(JobsEvent::RunEnded { run: run.clone() });
        if run.status != JobRunStatus::Skipped
            && let Some(entry) = state.jobs.get_mut(name)
        {
            entry.last_run = Some(run.clone());
        }
        self.publish(space, &state, name);
    }

    /// Stores a running run and publishes `run_started` then the job (DMN-336); it is the job's run by name, so a job
    /// declared again meanwhile adopts it (DMN-324). A command run started under a definition `generation` that is
    /// no longer the job's is reconciled as a registered run is (DMN-322): killed if the job was removed or disabled
    /// meanwhile; a service run (trigger `service`) is killed and started again only if the job still is an enabled
    /// service; any other run keeps running under the old definition.
    async fn register(
        &self,
        space: &Arc<Space>,
        name: &str,
        run: &RunInfo,
        kind: RunKind,
        generation: u64,
    ) {
        let (store, row) = (Arc::clone(&self.inner.store), job_run(run));
        if let Err(error) = blocking(move || Ok(store.insert_job_run(&row)?)).await {
            warn!(job = %name, error = %error.message, "cannot record a job's run");
        }
        #[cfg(test)]
        {
            let gate = lock(&self.inner.register_gate).take();
            if let Some((arrived, release)) = gate {
                let _ = arrived.send(());
                let _ = release.await;
            }
        }
        let mut guard = lock(&space.state);
        let state = &mut *guard;
        let mut kill = None;
        // The job's run by name (DMN-324): a job declared again meanwhile adopts it.
        state.starting.remove(name);
        match state.jobs.get_mut(name) {
            Some(entry) => {
                entry.run = Some(run.id.clone());
                if !entry.def.enabled || state.removed {
                    kill = Some(Kill::Teardown);
                } else if entry.generation != generation && run.trigger == RunTrigger::Service {
                    // Only the run's trigger decides (DMN-322): its end starts the service again if the job still
                    // is an enabled one (`after_end`).
                    kill = Some(Kill::Replace);
                }
            }
            None => kill = Some(Kill::Teardown),
        }
        if !matches!(kind, RunKind::Command) {
            kill = None;
        }
        state.runs.insert(
            run.id.clone(),
            Running {
                job: name.to_owned(),
                info: run.clone(),
                kind,
                kill,
            },
        );
        if !state.removed {
            space.log.push(JobsEvent::RunStarted { run: run.clone() });
            self.publish(space, state, name);
        }
        drop(guard);
        if kill.is_some() {
            let _ = self.inner.jobs.kill(&run.id);
        }
    }

    /// A command run (DMN-327): a job of the [`Jobs`] engine in its `cwd`, its id the run's.
    async fn start_command(
        &self,
        space: &Arc<Space>,
        name: &str,
        def: &JobDefinition,
        command: &str,
        mut run: RunInfo,
        generation: u64,
    ) -> Result<RunInfo, ErrorBody> {
        let folder = file::cwd_folder(&space.folder, def.cwd.as_deref())
            .ok_or_else(|| bad_request(format!("job {name}: its cwd leaves the workspace folder")));
        let started = folder.and_then(|folder| {
            self.inner
                .jobs
                .start_run(&space.id, &folder, command)
                .map_err(ErrorBody::from)
        });
        let (job, exit) = match started {
            Ok(started) => started,
            Err(error) => {
                run.status = JobRunStatus::Failed;
                run.ended_at = Some(run.started_at);
                run.reason = Some(error.message);
                self.record_ended(space, name, &run, Some(())).await;
                self.after_end(space, name, &run, None);
                return Ok(run);
            }
        };
        run.id = job.id;
        run.started_at = job.started_at;
        let recording = {
            self.inner.recording.send_modify(|count| *count += 1);
            Recording(self.inner.recording.clone())
        };
        self.register(space, name, &run, RunKind::Command, generation)
            .await;
        let declared = self.clone();
        let (space, run_id) = (Arc::clone(space), run.id.clone());
        tokio::spawn(async move {
            let _recording = recording;
            declared.command_ended(&space, &run_id, exit).await;
        });
        Ok(run)
    }

    async fn command_ended(
        &self,
        space: &Arc<Space>,
        run_id: &str,
        exit: oneshot::Receiver<RunExit>,
    ) {
        let ended = match exit.await {
            Ok(exit) => {
                let status = if exit.killed {
                    JobRunStatus::Killed
                } else if exit.code == Some(0) {
                    JobRunStatus::Succeeded
                } else {
                    JobRunStatus::Failed
                };
                Ended {
                    status,
                    reason: None,
                    exit_code: exit.code,
                    signal: exit.signal,
                    at: exit.ended_at,
                    output_tail: serde_json::to_string(&exit.tail).ok(),
                    line_count: exit.line_count,
                }
            }
            Err(_) => Ended {
                status: JobRunStatus::Failed,
                reason: Some("the daemon lost the run".to_owned()),
                exit_code: None,
                signal: None,
                at: self.inner.clock.now_ms(),
                output_tail: None,
                line_count: 0,
            },
        };
        self.finish(space, run_id, ended).await;
    }

    /// Records the end of a running run, publishes `run_ended` then the job (DMN-336), and restarts a service
    /// (DMN-326).
    async fn finish(&self, space: &Arc<Space>, run_id: &str, ended: Ended) {
        let (mut run, job) = {
            let state = lock(&space.state);
            if state.removed {
                return;
            }
            let Some(running) = state.runs.get(run_id) else {
                return;
            };
            (running.info.clone(), running.job.clone())
        };
        run.status = ended.status;
        run.reason = ended.reason;
        run.exit_code = ended.exit_code;
        run.signal = ended.signal;
        run.ended_at = Some(ended.at);
        let (store, id) = (Arc::clone(&self.inner.store), run.id.clone());
        let status = status_name(run.status);
        let reason = run.reason.clone();
        let (exit_code, signal, ended_at) = (run.exit_code, run.signal, ended.at);
        let (output_tail, line_count) = (ended.output_tail, ended.line_count);
        let recorded = blocking(move || {
            Ok(store.finish_job_run(
                &id,
                &RunEnd {
                    ended_at,
                    status,
                    reason: reason.as_deref(),
                    exit_code,
                    signal,
                    output_tail: output_tail.as_deref(),
                    line_count,
                },
            )?)
        })
        .await;
        if let Err(error) = recorded {
            warn!(job = %job, error = %error.message, "cannot record a job's run");
        }
        let kill = {
            let mut guard = lock(&space.state);
            let state = &mut *guard;
            let kill = state.runs.remove(run_id).and_then(|running| running.kill);
            if state.removed {
                return;
            }
            space.log.push(JobsEvent::RunEnded { run: run.clone() });
            if let Some(entry) = state.jobs.get_mut(&job)
                && entry.run.as_deref() == Some(run_id)
            {
                entry.run = None;
                entry.last_run = Some(run.clone());
            }
            self.publish(space, state, &job);
            kill
        };
        self.after_end(space, &job, &run, kill);
    }

    /// What a service does once its run ended (DMN-326): starts again per `restart` after its backoff, at once when
    /// it changed, never after `jobs.kill`.
    fn after_end(&self, space: &Arc<Space>, name: &str, run: &RunInfo, kill: Option<Kill>) {
        if self.closed() {
            return;
        }
        let mut guard = lock(&space.state);
        let state = &mut *guard;
        if state.removed {
            return;
        }
        let starting = state.starting.contains(name);
        let Some(entry) = state.jobs.get_mut(name) else {
            return;
        };
        if entry.def.trigger != JobTrigger::Service
            || !entry.def.enabled
            || entry.run.is_some()
            || starting
        {
            return;
        }
        match kill {
            Some(Kill::Protocol) => {
                entry.service.stopped = true;
                self.publish(space, state, name);
                return;
            }
            Some(Kill::Teardown) => return,
            Some(Kill::Replace) => {
                drop(guard);
                self.spawn_start(space, name.to_owned(), RunTrigger::Service, None, false);
                return;
            }
            None => {}
        }
        let failed = run.status == JobRunStatus::Failed;
        if entry.def.restart == RestartPolicy::OnFailure && !failed {
            entry.service.stopped = true;
            self.publish(space, state, name);
            return;
        }
        let lasted = run.ended_at.unwrap_or(run.started_at) - run.started_at;
        if lasted >= LONG_RUN_MS {
            entry.service.backoff_ms = BACKOFF_FIRST_MS;
        }
        let delay = entry.service.backoff_ms;
        if lasted < LONG_RUN_MS {
            entry.service.backoff_ms = (entry.service.backoff_ms * 2).min(BACKOFF_MAX_MS);
        }
        let at = self.inner.clock.now_ms() + delay;
        entry.service.tickets += 1;
        let ticket = entry.service.tickets;
        entry.service.waiting_until = Some(at);
        let timer =
            tokio::spawn(
                self.clone()
                    .restart_at(Arc::clone(space), name.to_owned(), ticket, at),
            );
        entry.service.restart = Some((ticket, timer.abort_handle()));
        self.publish(space, state, name);
    }

    async fn restart_at(self, space: Arc<Space>, name: String, ticket: u64, at: i64) {
        self.inner.clock.sleep_until(at).await;
        {
            let mut state = lock(&space.state);
            let Some(entry) = state.jobs.get_mut(&name) else {
                return;
            };
            if entry.service.restart.as_ref().map(|(kept, _)| *kept) != Some(ticket) {
                return;
            }
            // Taken, not aborted: this is the timer.
            entry.service.restart = None;
        }
        let _ = self
            .spawn_start(&space, name, RunTrigger::Service, None, false)
            .await;
    }

    /// A prompt run (DMN-328): a new thread with the text as its first message, or the text sent to the job's thread.
    #[allow(clippy::too_many_lines)] // The steps of DMN-328 in order: thread, busy check, prompt, admission.
    async fn start_prompt(
        &self,
        space: &Arc<Space>,
        name: &str,
        prompt: &JobPrompt,
        mut run: RunInfo,
        manual: bool,
        generation: u64,
    ) -> Result<RunInfo, ErrorBody> {
        let reuse = prompt.thread == PromptThread::Reuse;
        let mut thread_id = None;
        if reuse {
            let (store, threads, workspace_id, job) = (
                Arc::clone(&self.inner.store),
                Arc::clone(&self.inner.threads),
                space.id.clone(),
                name.to_owned(),
            );
            let kept = blocking(move || {
                let Some(id) = store.job_state(&workspace_id, &job)?.thread_id else {
                    return Ok(None);
                };
                // A deleted or archived thread, or one moved to another workspace, makes way for a new one
                // (DMN-283: a job never works on another workspace's repositories).
                Ok(threads
                    .get(&id)
                    .ok()
                    .filter(|thread| {
                        thread.archived_at.is_none() && thread.workspace_id == workspace_id
                    })
                    .map(|thread| thread.id))
            })
            .await;
            thread_id = match kept {
                Ok(kept) => kept,
                Err(error) => {
                    self.release(space, name, &run);
                    return Err(error);
                }
            };
        }
        if let Some(id) = &thread_id {
            let status = self.inner.supervisor.status(id);
            if status.process == ProcessState::Starting || !status.settled || status.needs_attention
            {
                if manual {
                    self.release(space, name, &run);
                    return Err(conflict(format!("the thread of job {name} is busy")));
                }
                let skipped = self
                    .skip(
                        space,
                        name,
                        run.trigger,
                        None,
                        Some(id.clone()),
                        None,
                        "busy",
                    )
                    .await;
                self.release(space, name, &run);
                return skipped;
            }
        }
        let created = thread_id.is_none();
        let thread_id = match thread_id {
            Some(id) => id,
            None => match self
                .create_thread(space, name, prompt, run.started_at, reuse)
                .await
            {
                Ok(id) => id,
                Err(error) => {
                    self.release(space, name, &run);
                    return Err(error);
                }
            },
        };
        run.thread_id = Some(thread_id.clone());
        let command = PromptCommand {
            message: prompt.text.clone(),
            images: None,
            streaming_behavior: None,
        };
        let admission = match self.inner.supervisor.prompt(&thread_id, command).await {
            Ok(admission) => admission,
            Err(error) => {
                let start_failure = error.is_start_failure();
                let body = supervisor_error(error);
                if start_failure && created {
                    // A thread that cannot start is not created (DMN-211).
                    self.discard_thread(&thread_id).await;
                    run.thread_id = None;
                }
                run.status = JobRunStatus::Failed;
                run.ended_at = Some(self.inner.clock.now_ms());
                run.reason = Some(
                    match body
                        .details
                        .as_ref()
                        .and_then(|details| details.get("stderrTail"))
                    {
                        Some(Value::String(tail)) if !tail.is_empty() => {
                            format!("{}\n{tail}", body.message)
                        }
                        _ => body.message.clone(),
                    },
                );
                self.record_ended(space, name, &run, Some(())).await;
                // The job may have become a service while omp started: it starts now (DMN-322).
                self.after_end(space, name, &run, Some(Kill::Replace));
                self.inner.changed.send_modify(|count| *count += 1);
                return if manual { Err(body) } else { Ok(run) };
            }
        };
        if created && reuse {
            let (store, workspace_id, job, id) = (
                Arc::clone(&self.inner.store),
                space.id.clone(),
                name.to_owned(),
                thread_id.clone(),
            );
            if let Err(error) =
                blocking(move || Ok(store.set_job_thread(&workspace_id, &job, &id)?)).await
            {
                warn!(job = %name, error = %error.message, "cannot record a job's thread");
            }
        }
        self.inner.changed.send_modify(|count| *count += 1);
        self.register(
            space,
            name,
            &run,
            RunKind::Prompt {
                thread_id: thread_id.clone(),
            },
            generation,
        )
        .await;
        let declared = self.clone();
        let (space, run_id) = (Arc::clone(space), run.id.clone());
        let invoked = admission.agent_invoked.then_some(admission.prompt_id);
        tokio::spawn(async move {
            declared
                .prompt_ended(&space, &run_id, &thread_id, invoked)
                .await;
        });
        Ok(run)
    }

    /// Creates the thread of a prompt run (DMN-328, DMN-334): `created_by` `job:<name>`, no client.
    async fn create_thread(
        &self,
        space: &Space,
        name: &str,
        prompt: &JobPrompt,
        started_at: i64,
        reuse: bool,
    ) -> Result<String, ErrorBody> {
        let title = if reuse {
            name.to_owned()
        } else {
            format!("{name} · {}", local_minute(started_at, &self.inner.tz))
        };
        let new = NewThread {
            model: prompt.model.clone(),
            thinking_level: prompt.thinking_level.clone(),
            approval_mode: prompt.approval_mode.clone(),
            created_by: Provenance::Job(name.to_owned()),
            ..NewThread::new(title, &space.id)
        };
        let threads = Arc::clone(&self.inner.threads);
        blocking(move || {
            threads
                .create(new)
                .map(|thread| thread.id)
                .map_err(crate::server::convert::thread_error)
        })
        .await
    }

    async fn discard_thread(&self, id: &str) {
        if let Err(error) = self.inner.supervisor.forget(id).await {
            warn!(thread = %id, %error, "cannot stop the thread that failed to start");
        }
        let (threads, id) = (Arc::clone(&self.inner.threads), id.to_owned());
        if let Err(error) = blocking(move || {
            threads
                .delete(&id)
                .map_err(crate::server::convert::thread_error)
        })
        .await
        {
            warn!(message = %error.message, "cannot delete the thread that failed to start");
        }
    }

    /// Follows a prompt run until its `prompt_result` (DMN-328): `succeeded` on success, else `failed` (`killed` when
    /// `jobs.kill` aborted it). A run whose omp stops without a result fails; at shutdown it stays `running` and the
    /// next start marks it `interrupted`.
    async fn prompt_ended(
        &self,
        space: &Arc<Space>,
        run_id: &str,
        thread_id: &str,
        prompt_id: Option<String>,
    ) {
        let outcome = match prompt_id {
            None => Ok(()),
            Some(prompt_id) => match self.inner.supervisor.thread_log(thread_id).await {
                Err(error) => Err(error.to_string()),
                Ok(log) => {
                    let mut events = log.listen();
                    loop {
                        let done = log.with_state(|state| match state.prompts.get(&prompt_id) {
                            Some(result) if result.status == PromptStatus::Completed => {
                                Some(Ok(()))
                            }
                            Some(result) => Some(Err(result.error.as_ref().map_or_else(
                                || format!("{:?}", result.status).to_lowercase(),
                                |error| error.message.clone(),
                            ))),
                            None => match &state.process {
                                ProcessState::Stopped | ProcessState::Failed { .. } => {
                                    Some(Err("the thread's omp stopped before the prompt's result"
                                        .to_owned()))
                                }
                                ProcessState::Starting | ProcessState::Running => None,
                            },
                        });
                        if let Some(done) = done {
                            break done;
                        }
                        if let Err(RecvError::Closed) = events.recv().await {
                            break Err("the thread is gone".to_owned());
                        }
                    }
                }
            },
        };
        if self.closed() {
            return;
        }
        let killed = lock(&space.state)
            .runs
            .get(run_id)
            .is_some_and(|running| running.kill == Some(Kill::Protocol));
        let (status, reason) = match outcome {
            Ok(()) => (JobRunStatus::Succeeded, None),
            Err(_) if killed => (JobRunStatus::Killed, None),
            Err(reason) => (JobRunStatus::Failed, Some(reason)),
        };
        self.finish(
            space,
            run_id,
            Ended {
                status,
                reason,
                exit_code: None,
                signal: None,
                at: self.inner.clock.now_ms(),
                output_tail: None,
                line_count: 0,
            },
        )
        .await;
    }

    // The protocol.

    /// The log of `jobs/<id>` (DMN-336); `not_found` for an unknown workspace.
    pub async fn log(&self, workspace_id: &str) -> Result<Arc<JobsLog>, ErrorBody> {
        Ok(Arc::clone(&self.ensure(workspace_id).await?.log))
    }

    /// `jobs.list` (PRT-202).
    pub async fn list(&self, workspace_id: &str) -> Result<JobsState, ErrorBody> {
        Ok(self.log(workspace_id).await?.with_state(Clone::clone))
    }

    /// `jobs.runs` (PRT-203): the kept runs of a job, declared or not, newest first.
    pub async fn runs(
        &self,
        workspace_id: String,
        job: String,
        before: Option<String>,
        limit: Option<u32>,
    ) -> Result<JobRuns, ErrorBody> {
        let limit = limit.unwrap_or(RUNS_DEFAULT).min(RUNS_MAX);
        let store = Arc::clone(&self.inner.store);
        let (runs, more) =
            blocking(move || Ok(store.job_runs(&workspace_id, &job, before.as_deref(), limit)?))
                .await?;
        Ok(JobRuns {
            runs: runs.iter().map(run_info).collect(),
            more,
        })
    }

    /// `jobs.run` (PRT-204): a run by hand, answered once it started.
    pub async fn run(
        &self,
        workspace_id: &str,
        job: &str,
        started_by: Provenance,
    ) -> Result<RunInfo, ErrorBody> {
        let space = self.ensure(workspace_id).await?;
        self.spawn_start(
            &space,
            job.to_owned(),
            RunTrigger::Manual,
            Some(started_by),
            true,
        )
        .await
        .map_err(internal)?
    }

    /// `jobs.kill` of a running declared run (DMN-326, DMN-328): `None` when `run_id` is no such run. A command run is
    /// killed as `jobs.kill` kills a job; a prompt run's turn is aborted.
    pub async fn kill(&self, run_id: &str) -> Option<Result<(), ErrorBody>> {
        let spaces: Vec<Arc<Space>> = lock(&self.inner.spaces).values().cloned().collect();
        for space in spaces {
            let kind = {
                let mut state = lock(&space.state);
                let Some(running) = state.runs.get_mut(run_id) else {
                    continue;
                };
                running.kill = Some(Kill::Protocol);
                running.kind.clone()
            };
            return Some(match kind {
                RunKind::Command => self.inner.jobs.kill(run_id).map_err(ErrorBody::from),
                RunKind::Prompt { thread_id } => self
                    .inner
                    .supervisor
                    .call(&thread_id, OmpCall::AbortAndRestoreQueue)
                    .await
                    .map(|_| ())
                    .map_err(supervisor_error),
            });
        }
        None
    }

    /// The `job/<id>` log of a stored command run the engine no longer holds (DMN-330), if the store has it.
    pub async fn stored_log(&self, run_id: &str) -> Option<Arc<JobLog>> {
        let store = Arc::clone(&self.inner.store);
        let id = run_id.to_owned();
        let (run, output) = blocking(move || Ok(store.job_run(&id)?)).await.ok()??;
        let command = run.command?;
        let lines: Vec<JobLine> = output
            .output_tail
            .and_then(|tail| serde_json::from_str(&tail).ok())
            .unwrap_or_default();
        let dropped = output.line_count.saturating_sub(lines.len() as u64);
        let status = if run.status == "running" {
            JobStatus::Running
        } else {
            JobStatus::Exited {
                code: run.exit_code,
                signal: run.signal,
                ended_at: run.ended_at.unwrap_or(run.started_at),
            }
        };
        Some(stored_log(
            JobInfo {
                id: run.id,
                workspace_id: run.workspace_id,
                command,
                started_at: run.started_at,
                status,
            },
            lines,
            dropped,
        ))
    }

    /// The job's view as published.
    fn published(space: &Space, name: &str) -> Result<JobView, ErrorBody> {
        space
            .log
            .with_state(|state| state.jobs.iter().find(|job| job.name == name).cloned())
            .ok_or_else(|| internal(format!("job {name} was not applied")))
    }

    /// Changes the jobs file one job at a time (DMN-332): read, changed by `change`, checked, written atomically and
    /// applied before answering.
    async fn write(
        &self,
        workspace_id: &str,
        change: impl FnOnce(Option<&[u8]>) -> Result<Vec<u8>, ErrorBody> + Send + 'static,
    ) -> Result<Arc<Space>, ErrorBody> {
        let space = self.ensure(workspace_id).await?;
        let mut applied = space.file.lock().await;
        let path = file::path(&space.folder);
        let bytes = blocking(move || {
            let current = match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(internal(format!("{}: {error}", path.display()))),
            };
            let bytes = change(current.as_deref())?;
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder)
                    .map_err(|error| internal(format!("{}: {error}", folder.display())))?;
            }
            replace_file(&path, &bytes, None)
                .map_err(|error| internal(format!("{}: {error}", path.display())))?;
            Ok(bytes)
        })
        .await?;
        self.apply_bytes(&space, &mut applied, Some(bytes)).await;
        drop(applied);
        Ok(space)
    }

    /// `jobs.define` (PRT-205).
    pub async fn define(
        &self,
        workspace_id: &str,
        name: String,
        job: Value,
        replace: bool,
    ) -> Result<JobView, ErrorBody> {
        let defined = name.clone();
        let space = self
            .write(workspace_id, move |current| {
                let bytes = file::write_job(current, &defined, Some(&job))
                    .map_err(|error| write_error(&error))?;
                let exists = current
                    .and_then(|current| file::parse_jobs_file(current).ok())
                    .is_some_and(|jobs| jobs.contains_key(&defined));
                if exists && !replace {
                    return Err(conflict(format!(
                        "job {defined} exists: pass replace: true to replace it"
                    )));
                }
                Ok(bytes)
            })
            .await?;
        Self::published(&space, &name)
    }

    /// `jobs.remove` (PRT-206).
    pub async fn remove(&self, workspace_id: &str, name: String) -> Result<(), ErrorBody> {
        self.write(workspace_id, move |current| {
            file::write_job(current, &name, None).map_err(|error| write_error(&error))
        })
        .await?;
        Ok(())
    }

    /// `jobs.enable` (PRT-207).
    pub async fn enable(
        &self,
        workspace_id: &str,
        name: String,
        enabled: bool,
    ) -> Result<JobView, ErrorBody> {
        let changed = name.clone();
        let space = self
            .write(workspace_id, move |current| {
                file::set_enabled(current, &changed, enabled).map_err(|error| write_error(&error))
            })
            .await?;
        Self::published(&space, &name)
    }

    /// Tears the workspace's jobs down before its folder is deleted (DMN-329); its runs and job state go with its
    /// store row. Returns once the command runs it killed exited.
    pub async fn remove_workspace(&self, workspace_id: &str) {
        let Some(space) = lock(&self.inner.spaces).remove(workspace_id) else {
            return;
        };
        let _applied = space.file.lock().await;
        let mut guard = lock(&space.state);
        let state = &mut *guard;
        state.removed = true;
        for entry in state.jobs.values_mut() {
            entry.tear_down(&mut state.runs, &self.inner.jobs);
        }
        self.publish_counts(workspace_id, state);
    }

    /// At shutdown, first: no run starts any more, no timer fires.
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        for space in lock(&self.inner.spaces).values() {
            for entry in lock(&space.state).jobs.values_mut() {
                if let Some(scheduler) = entry.scheduler.take() {
                    scheduler.abort();
                }
                entry.service.cancel();
            }
        }
    }

    /// At shutdown, after the jobs were killed: waits (a few seconds at most) until their ends are recorded.
    pub async fn wait_recorded(&self) {
        let mut recording = self.inner.recording.subscribe();
        let _ = tokio::time::timeout(RECORD_WAIT, recording.wait_for(|count| *count == 0)).await;
    }
}

fn write_error(error: &file::WriteError) -> ErrorBody {
    match error {
        file::WriteError::NotFound(_) => not_found(error.to_string()),
        file::WriteError::InvalidFile(_) | file::WriteError::InvalidJob(_) => {
            bad_request(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::agent::{FakeCall, FakeLauncher, FakeRun};
    use crate::git::{FakeGit, Git};
    use crate::paths::DataDir;
    use crate::store::PromptQuery;
    use crate::supervisor::SupervisorConfig;
    use crate::workspaces::{NewWorkspace, Workspaces};

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;

    /// 2026-11-16 08:00 UTC, the clock's start in every test.
    fn origin() -> i64 {
        "2026-11-16T08:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_millisecond()
    }

    fn fixture(name: &str) -> FakeRun {
        FakeRun::from_fixture(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/threads")
                .join(omfp_core::OMP_VERSION)
                .join(format!("{name}.jsonl")),
        )
        .unwrap()
    }

    struct Harness {
        _dir: tempfile::TempDir,
        declared: Declared,
        store: Arc<Store>,
        threads: Arc<Threads>,
        supervisor: Supervisor,
        launcher: FakeLauncher,
        workspace: String,
        folder: PathBuf,
    }

    fn harness() -> Harness {
        harness_with(Arc::new(TokioClock::new(origin())))
    }

    fn harness_with(clock: Arc<dyn Clock>) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().join("data"));
        std::fs::create_dir_all(data.root()).unwrap();
        let root = dir.path().join("workspaces");
        let store = Arc::new(Store::open(&data.db(), &root).unwrap());
        let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
        let workspaces = Arc::new(Workspaces::new(
            Arc::clone(&store),
            Arc::new(FakeGit::new()) as Arc<dyn Git>,
            root,
        ));
        let view = workspaces.create(NewWorkspace::new()).unwrap();
        let launcher = FakeLauncher::new();
        let supervisor = Supervisor::new(
            Arc::new(launcher.clone()),
            Arc::clone(&threads),
            workspaces,
            data,
            SupervisorConfig::new("omp"),
        );
        let declared = Declared::new(
            Arc::clone(&store),
            Jobs::default(),
            supervisor.clone(),
            Arc::clone(&threads),
            clock,
            TimeZone::UTC,
        );
        Harness {
            _dir: dir,
            declared,
            store,
            threads,
            supervisor,
            launcher,
            workspace: view.workspace.id,
            folder: view.workspace.path,
        }
    }

    impl Harness {
        async fn write(&self, jobs: &Value) {
            let path = file::path(&self.folder);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_vec(jobs).unwrap()).unwrap();
            self.declared.reload(&self.workspace).await;
        }

        fn state(&self) -> JobsState {
            self.declared
                .space(&self.workspace)
                .unwrap()
                .log
                .with_state(Clone::clone)
        }

        fn view(&self, name: &str) -> JobView {
            self.state()
                .jobs
                .into_iter()
                .find(|job| job.name == name)
                .unwrap()
        }

        fn runs(&self, job: &str) -> Vec<RunInfo> {
            let (mut runs, _) = self.store.job_runs(&self.workspace, job, None, 50).unwrap();
            runs.reverse();
            runs.iter().map(run_info).collect()
        }

        /// Lets the runtime's tasks run until `done`, Tokio's paused time advancing when they all wait.
        async fn until(&self, done: impl Fn(&Self) -> bool) {
            for _ in 0..3000 {
                if done(self) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("the condition never held");
        }
    }

    // spec: DMN-323, DMN-325, DMN-330, DMN-336, PRT-200
    #[tokio::test(start_paused = true)]
    async fn a_schedule_runs_at_its_times_and_publishes_its_next_time() {
        let h = harness();
        // A missing cwd fails each run at once, without a process: the test needs no real time.
        h.write(&json!({ "jobs": { "tick": {
            "trigger": "schedule", "schedule": "*/5 * * * *", "command": "true", "cwd": "missing"
        } } }))
        .await;
        let log = h.declared.log(&h.workspace).await.unwrap();
        let mut events = log.listen();
        assert_eq!(h.view("tick").next_at, Some(origin() + 5 * MINUTE));
        assert_eq!(h.view("tick").status, JobViewStatus::Idle);

        tokio::time::sleep(Duration::from_millis(u64::try_from(11 * MINUTE).unwrap())).await;
        h.until(|h| h.runs("tick").len() == 2).await;

        let runs = h.runs("tick");
        assert_eq!(
            runs.iter()
                .map(|run| (run.trigger, run.started_at, run.status))
                .collect::<Vec<_>>(),
            [
                (
                    RunTrigger::Schedule,
                    origin() + 5 * MINUTE,
                    JobRunStatus::Failed
                ),
                (
                    RunTrigger::Schedule,
                    origin() + 10 * MINUTE,
                    JobRunStatus::Failed
                ),
            ]
        );
        assert!(
            runs[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("cannot start the job")
        );
        assert_eq!(runs[0].started_by, None);
        let view = h.view("tick");
        assert_eq!(view.next_at, Some(origin() + 15 * MINUTE));
        assert_eq!(
            view.last_run.as_ref().map(|run| run.id.clone()),
            Some(runs[1].id.clone())
        );
        // Each due time publishes the next one, then the run's `run_ended` before the job showing it (DMN-336).
        let mut kinds = Vec::new();
        while let Ok(event) = events.try_recv() {
            kinds.push(match &event.event {
                JobsEvent::RunEnded { run } => format!("ended {}", run.started_at - origin()),
                JobsEvent::JobUpdated { job } => format!(
                    "next {:?}, last run {:?}",
                    job.next_at.map(|at| at - origin()),
                    job.last_run.as_ref().map(|run| run.started_at - origin())
                ),
                other => format!("{other:?}"),
            });
        }
        assert_eq!(
            kinds,
            [
                "next Some(600000), last run None".to_owned(),
                format!("ended {}", 5 * MINUTE),
                "next Some(600000), last run Some(300000)".to_owned(),
                "next Some(900000), last run Some(300000)".to_owned(),
                format!("ended {}", 10 * MINUTE),
                "next Some(900000), last run Some(600000)".to_owned(),
            ]
        );
    }

    // spec: DMN-325
    #[tokio::test(start_paused = true)]
    async fn a_time_reached_late_is_missed_and_runs_once_with_missed_run() {
        let h = harness();
        for job in ["catch-up", "drop"] {
            h.store
                .set_job_last_due(&h.workspace, job, origin() - 3 * HOUR - MINUTE)
                .unwrap();
        }
        h.write(&json!({ "jobs": {
            "catch-up": { "trigger": "schedule", "schedule": "0 * * * *", "missed": "run", "command": "true", "cwd": "missing" },
            "drop": { "trigger": "schedule", "schedule": "0 * * * *", "command": "true", "cwd": "missing" }
        } }))
        .await;

        h.until(|h| {
            h.runs("catch-up").len() == 1 && h.view("drop").next_at == Some(origin() + HOUR)
        })
        .await;

        let runs = h.runs("catch-up");
        assert_eq!(
            (runs[0].trigger, runs[0].started_at),
            (RunTrigger::Missed, origin())
        );
        assert_eq!(h.runs("drop"), []);
        for job in ["catch-up", "drop"] {
            assert_eq!(
                h.store.job_state(&h.workspace, job).unwrap().last_due_at,
                Some(origin()),
                "{job}: every missed time counts as handled"
            );
            assert_eq!(h.view(job).next_at, Some(origin() + HOUR));
        }
    }

    // spec: DMN-325
    #[tokio::test(start_paused = true)]
    async fn a_new_job_never_runs_for_times_before_it_was_seen() {
        let h = harness();
        h.write(&json!({ "jobs": { "hourly": {
            "trigger": "schedule", "schedule": "@hourly", "missed": "run", "command": "true", "cwd": "missing"
        } } }))
        .await;

        h.until(|h| h.view("hourly").next_at.is_some()).await;

        assert_eq!(
            h.store
                .job_state(&h.workspace, "hourly")
                .unwrap()
                .last_due_at,
            Some(origin())
        );
        assert_eq!(h.view("hourly").next_at, Some(origin() + HOUR));
        assert_eq!(h.runs("hourly"), []);
    }

    // spec: DMN-326, PRT-200
    #[tokio::test(start_paused = true)]
    async fn a_failing_service_restarts_after_a_doubling_delay() {
        let h = harness();
        h.write(&json!({ "jobs": { "watch": { "trigger": "service", "command": "true", "cwd": "missing" } } }))
            .await;

        h.until(|h| h.runs("watch").len() == 4).await;

        let starts: Vec<i64> = h
            .runs("watch")
            .iter()
            .map(|run| run.started_at - origin())
            .collect();
        assert_eq!(starts, [0, 1_000, 3_000, 7_000]);
        assert!(
            h.runs("watch")
                .iter()
                .all(|run| run.trigger == RunTrigger::Service)
        );
        let view = h.view("watch");
        assert_eq!(view.status, JobViewStatus::Waiting);
        assert_eq!(view.next_at, Some(origin() + 15_000));
    }

    /// The wall clock, but every wait (a service's restart delay) lasts at most a few milliseconds.
    struct QuickClock;

    impl Clock for QuickClock {
        fn now_ms(&self) -> i64 {
            store::now_ms()
        }

        fn sleep_until(&self, _at_ms: i64) -> Sleep {
            Box::pin(tokio::time::sleep(Duration::from_millis(5)))
        }
    }

    /// Reads `jobs/<id>` events until `done` saw what it waits for; at most 30 s.
    async fn until_event(
        events: &mut tokio::sync::broadcast::Receiver<Arc<crate::event_log::Sequenced<JobsEvent>>>,
        mut done: impl FnMut(&JobsEvent) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match events.recv().await {
                    Ok(event) if done(&event.event) => return,
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => panic!("the jobs topic closed"),
                }
            }
        })
        .await
        .expect("the awaited jobs events never came");
    }

    // spec: DMN-326, PRT-200
    #[tokio::test]
    async fn an_on_failure_service_stops_after_a_clean_exit_and_restarts_after_a_failing_one() {
        let h = harness_with(Arc::new(QuickClock));
        let log = h.declared.log(&h.workspace).await.unwrap();
        let mut events = log.listen();
        h.write(&json!({ "jobs": {
            "clean": { "trigger": "service", "command": "exit 0", "restart": "on-failure" },
            "failing": { "trigger": "service", "command": "exit 3", "restart": "on-failure" }
        } }))
        .await;

        let (mut clean_stopped, mut failing_ended) = (false, 0);
        until_event(&mut events, |event| {
            match event {
                JobsEvent::JobUpdated { job } if job.name == "clean" => {
                    clean_stopped = job.status == JobViewStatus::Stopped;
                }
                JobsEvent::RunEnded { run } if run.job.as_deref() == Some("failing") => {
                    failing_ended += 1;
                }
                _ => {}
            }
            clean_stopped && failing_ended >= 2
        })
        .await;

        let clean = h.runs("clean");
        assert_eq!(clean.len(), 1);
        assert_eq!(
            (clean[0].status, clean[0].exit_code),
            (JobRunStatus::Succeeded, Some(0))
        );
        let view = h.view("clean");
        assert_eq!((view.run, view.next_at), (None, None));
        // Stopped for good: no restart is pending, so nothing starts it again.
        let space = h.declared.space(&h.workspace).unwrap();
        assert!(lock(&space.state).jobs["clean"].service.restart.is_none());
        let failing = h.runs("failing");
        assert_eq!(
            (failing[0].status, failing[0].exit_code),
            (JobRunStatus::Failed, Some(3))
        );
        assert_eq!(failing[1].trigger, RunTrigger::Service);
    }

    // spec: DMN-322, PRT-201
    #[tokio::test]
    async fn a_service_changed_while_its_run_starts_starts_again_with_the_new_definition() {
        let h = harness();
        let log = h.declared.log(&h.workspace).await.unwrap();
        let mut events = log.listen();
        let (arrived, held) = oneshot::channel();
        let (release, released) = oneshot::channel();
        *lock(&h.declared.inner.register_gate) = Some((arrived, released));
        let old = "echo old; while true; do sleep 0.01; done";
        let new = "echo new; while true; do sleep 0.01; done";
        h.write(&json!({ "jobs": { "watch": { "trigger": "service", "command": old } } }))
            .await;
        tokio::time::timeout(Duration::from_secs(30), held)
            .await
            .unwrap()
            .unwrap();

        // The run is stored but not registered yet when the definition changes.
        h.write(&json!({ "jobs": { "watch": { "trigger": "service", "command": new } } }))
            .await;
        release.send(()).unwrap();

        let mut ended = None;
        until_event(&mut events, |event| {
            if let JobsEvent::RunEnded { run } = event {
                ended = Some(run.clone());
            }
            ended.is_some()
        })
        .await;
        let ended = ended.unwrap();
        assert_eq!(
            (ended.command.as_deref(), ended.status),
            (Some(old), JobRunStatus::Killed)
        );
        let mut started = None;
        until_event(&mut events, |event| {
            if let JobsEvent::RunStarted { run } = event {
                started = Some(run.clone());
            }
            started.is_some()
        })
        .await;
        let started = started.unwrap();
        assert_eq!(
            (started.command.as_deref(), started.trigger),
            (Some(new), RunTrigger::Service)
        );
        h.declared.kill(&started.id).await.unwrap().unwrap();
        until_event(
            &mut events,
            |event| matches!(event, JobsEvent::RunEnded { run } if run.id == started.id),
        )
        .await;
    }

    // spec: DMN-322
    #[tokio::test]
    async fn a_manual_run_starting_while_its_job_changes_into_a_service_keeps_running() {
        let h = harness();
        let log = h.declared.log(&h.workspace).await.unwrap();
        let mut events = log.listen();
        let old = "echo old; while true; do sleep 0.01; done";
        let new = "echo new; while true; do sleep 0.01; done";
        h.write(&json!({ "jobs": { "build": { "command": old } } }))
            .await;
        let (arrived, held) = oneshot::channel();
        let (release, released) = oneshot::channel();
        *lock(&h.declared.inner.register_gate) = Some((arrived, released));
        let declared = h.declared.clone();
        let workspace = h.workspace.clone();
        let asked = tokio::spawn(async move {
            declared
                .run(&workspace, "build", Provenance::User)
                .await
                .unwrap()
        });
        tokio::time::timeout(Duration::from_secs(30), held)
            .await
            .unwrap()
            .unwrap();

        // The run is stored but not registered yet when the job becomes a service.
        h.write(&json!({ "jobs": { "build": { "trigger": "service", "command": new } } }))
            .await;
        release.send(()).unwrap();
        let run = tokio::time::timeout(Duration::from_secs(30), asked)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            (run.command.as_deref(), run.trigger),
            (Some(old), RunTrigger::Manual)
        );
        let view = h.view("build");
        assert_eq!(view.status, JobViewStatus::Running);
        assert_eq!(view.run.map(|running| running.id), Some(run.id.clone()));
        let space = h.declared.space(&h.workspace).unwrap();
        assert_eq!(lock(&space.state).runs[&run.id].kill, None, "not killed");
        h.declared.kill(&run.id).await.unwrap().unwrap();
        let mut ended = None;
        until_event(&mut events, |event| {
            if let JobsEvent::RunEnded { run: ended_run } = event
                && ended_run.id == run.id
            {
                ended = Some(ended_run.clone());
            }
            ended.is_some()
        })
        .await;
        assert_eq!(ended.unwrap().status, JobRunStatus::Killed);
    }

    /// Holds the next registration (after its store write): returns the arrival and the release.
    fn gate(h: &Harness) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (arrived, held) = oneshot::channel();
        let (release, released) = oneshot::channel();
        *lock(&h.declared.inner.register_gate) = Some((arrived, released));
        (held, release)
    }

    async fn arrived(held: oneshot::Receiver<()>) {
        tokio::time::timeout(Duration::from_secs(30), held)
            .await
            .expect("the registration never came")
            .unwrap();
    }

    // spec: DMN-322
    #[tokio::test]
    async fn a_service_run_stops_for_good_when_its_job_becomes_manual_running_or_starting() {
        let h = harness();
        let log = h.declared.log(&h.workspace).await.unwrap();
        let mut events = log.listen();
        let hold = "while true; do sleep 0.01; done";
        let service = json!({ "jobs": { "watch": { "trigger": "service", "command": hold } } });
        let manual = json!({ "jobs": { "watch": { "command": hold } } });
        let ended = async |events: &mut tokio::sync::broadcast::Receiver<_>| {
            let mut ended = None;
            until_event(events, |event| {
                if let JobsEvent::RunEnded { run } = event {
                    ended = Some(run.clone());
                }
                ended.is_some()
            })
            .await;
            ended.unwrap()
        };

        // Running: the service run is killed, and nothing starts again.
        h.write(&service).await;
        until_event(&mut events, |event| {
            matches!(event, JobsEvent::RunStarted { .. })
        })
        .await;
        h.write(&manual).await;
        let run = ended(&mut events).await;
        assert_eq!(
            (run.trigger, run.status),
            (RunTrigger::Service, JobRunStatus::Killed)
        );
        let view = h.view("watch");
        assert_eq!((view.status, view.run), (JobViewStatus::Idle, None));

        // Starting: the same, though the change landed before the run registered.
        let (held, release) = gate(&h);
        h.write(&service).await;
        arrived(held).await;
        h.write(&manual).await;
        release.send(()).unwrap();
        let run = ended(&mut events).await;
        assert_eq!(
            (run.trigger, run.status),
            (RunTrigger::Service, JobRunStatus::Killed)
        );
        let view = h.view("watch");
        assert_eq!((view.status, view.run), (JobViewStatus::Idle, None));
        let space = h.declared.space(&h.workspace).unwrap();
        let state = lock(&space.state);
        assert!(state.runs.is_empty() && state.starting.is_empty());
    }

    // spec: DMN-324
    #[tokio::test]
    async fn a_job_declared_again_while_its_run_starts_sees_it_running_and_adopts_it() {
        let h = harness();
        let hold = "while true; do sleep 0.01; done";
        h.write(&json!({ "jobs": { "build": { "command": hold } } }))
            .await;
        let (held, release) = gate(&h);
        let declared = h.declared.clone();
        let workspace = h.workspace.clone();
        let asked = tokio::spawn(async move {
            declared
                .run(&workspace, "build", Provenance::User)
                .await
                .unwrap()
        });
        arrived(held).await;

        // Removed and declared again while the run is stored but not registered.
        h.write(&json!({ "jobs": {} })).await;
        h.write(&json!({ "jobs": { "build": { "command": hold } } }))
            .await;

        let error = h
            .declared
            .run(&h.workspace, "build", Provenance::User)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        let space = h.declared.space(&h.workspace).unwrap();
        let due = h
            .declared
            .start(&space, "build", RunTrigger::Schedule, None, false)
            .await
            .unwrap();
        assert_eq!(
            (due.status, due.reason.as_deref()),
            (JobRunStatus::Skipped, Some("running"))
        );
        release.send(()).unwrap();
        let run = tokio::time::timeout(Duration::from_secs(30), asked)
            .await
            .unwrap()
            .unwrap();

        let view = h.view("build");
        assert_eq!(view.status, JobViewStatus::Running);
        assert_eq!(view.run.map(|running| running.id), Some(run.id.clone()));
        assert_eq!(
            lock(&space.state).runs[&run.id].kill,
            None,
            "adopted, not killed"
        );
        h.declared.kill(&run.id).await.unwrap().unwrap();
    }

    // spec: DMN-326, DMN-322
    #[tokio::test]
    async fn a_service_enabled_again_while_its_run_is_killed_starts_once_that_run_ended() {
        let h = harness();
        let log = h.declared.log(&h.workspace).await.unwrap();
        let mut events = log.listen();
        // SIGTERM makes the shell wait for the gate file: the kill is still under way while the job changes.
        let gate_file = h.folder.join("gate");
        let command = format!(
            "trap 'while [ ! -e {} ]; do sleep 0.01; done; exit 0' TERM; echo ready; while true; do sleep 0.01; done",
            gate_file.display()
        );
        let job = |trigger: &str, enabled: bool| json!({ "jobs": { "watch": { "trigger": trigger, "command": command, "enabled": enabled } } });
        h.write(&job("manual", true)).await;
        let run = h
            .declared
            .run(&h.workspace, "watch", Provenance::User)
            .await
            .unwrap();
        // The trap is set once the shell printed `ready`.
        let output = h.declared.inner.jobs.log(&run.id).unwrap();
        let mut lines = output.listen();
        tokio::time::timeout(Duration::from_secs(30), async {
            while !output.with_state(|topic| topic.lines.iter().any(|line| line.text == "ready")) {
                let _ = lines.recv().await;
            }
        })
        .await
        .expect("the run never got ready");

        // A service now (its manual run goes on), disabled (the run is killed), enabled again before it ended.
        h.write(&job("service", true)).await;
        h.write(&job("service", false)).await;
        h.write(&job("service", true)).await;

        let space = h.declared.space(&h.workspace).unwrap();
        assert_eq!(lock(&space.state).runs[&run.id].kill, Some(Kill::Replace));
        std::fs::write(&gate_file, "").unwrap();
        let mut ended = false;
        let mut started = None;
        until_event(&mut events, |event| {
            match event {
                JobsEvent::RunEnded { run: ended_run } if ended_run.id == run.id => ended = true,
                JobsEvent::RunStarted { run: started_run } if ended => {
                    started = Some(started_run.clone());
                }
                _ => {}
            }
            started.is_some()
        })
        .await;
        let started = started.unwrap();
        assert_eq!(started.trigger, RunTrigger::Service);
        h.declared.kill(&started.id).await.unwrap().unwrap();
        until_event(
            &mut events,
            |event| matches!(event, JobsEvent::RunEnded { run } if run.id == started.id),
        )
        .await;
    }

    // spec: DMN-322, PRT-204
    #[tokio::test]
    async fn a_caller_during_the_first_load_waits_for_the_declared_jobs() {
        let h = harness();
        let path = file::path(&h.folder);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{ "jobs": { "build": { "command": "true" } } }"#).unwrap();
        let (arrived_tx, held) = oneshot::channel();
        let (release, released) = oneshot::channel();
        *lock(&h.declared.inner.load_gate) = Some((arrived_tx, released));
        let declared = h.declared.clone();
        let workspace = h.workspace.clone();
        let first = tokio::spawn(async move { declared.list(&workspace).await.unwrap() });
        arrived(held).await;

        let declared = h.declared.clone();
        let workspace = h.workspace.clone();
        let second =
            tokio::spawn(async move { declared.run(&workspace, "build", Provenance::User).await });
        // On this single-threaded runtime, a few yields let the second caller run as far as it can before the load
        // ends: without waiting for it, it would already have answered `not_found`.
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert!(
            !second.is_finished(),
            "the second caller waits for the first load"
        );
        release.send(()).unwrap();

        let state = first.await.unwrap();
        assert_eq!(state.jobs.len(), 1);
        let run = second.await.unwrap().unwrap();
        assert_eq!(run.job.as_deref(), Some("build"));
    }

    // spec: DMN-325
    #[tokio::test(start_paused = true)]
    async fn a_schedule_enabled_again_never_runs_for_times_while_it_was_disabled() {
        let h = harness();
        let job = |enabled: bool| {
            json!({ "jobs": { "hourly": {
                "trigger": "schedule", "schedule": "0 * * * *", "missed": "run",
                "command": "true", "cwd": "missing", "enabled": enabled
            } } })
        };
        h.write(&job(true)).await;
        assert_eq!(h.view("hourly").next_at, Some(origin() + HOUR));
        h.write(&job(false)).await;

        // 09:00 passes while it is disabled; it is enabled again at 09:00:10.
        tokio::time::sleep(Duration::from_secs(3610)).await;
        h.write(&job(true)).await;

        let enabled_at = origin() + HOUR + 10_000;
        h.until(|h| {
            h.store
                .job_state(&h.workspace, "hourly")
                .unwrap()
                .last_due_at
                == Some(enabled_at)
        })
        .await;
        assert_eq!(h.view("hourly").next_at, Some(origin() + 2 * HOUR));
        assert_eq!(h.runs("hourly"), []);
    }

    // spec: DMN-322, DMN-326
    #[tokio::test(flavor = "multi_thread")]
    async fn a_job_changed_into_a_service_while_its_prompt_run_fails_to_start_starts_the_service() {
        let h = harness();
        let log = h.declared.log(&h.workspace).await.unwrap();
        let mut events = log.listen();
        h.write(&json!({ "jobs": { "flex": { "prompt": { "text": "hi" } } } }))
            .await;
        h.launcher.fail_next_launch("no model configured");
        let launches = h.launcher.gate_launches();
        let declared = h.declared.clone();
        let workspace = h.workspace.clone();
        let asked =
            tokio::spawn(async move { declared.run(&workspace, "flex", Provenance::User).await });
        tokio::time::timeout(Duration::from_secs(30), launches.arrived(1))
            .await
            .expect("omp never launched");

        // omp is starting for the prompt when the job becomes a command service.
        h.write(&json!({ "jobs": { "flex": {
            "trigger": "service", "command": "while true; do sleep 0.01; done"
        } } }))
        .await;
        launches.open();

        let error = tokio::time::timeout(Duration::from_secs(30), asked)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ThreadFailed);
        let mut started = None;
        until_event(&mut events, |event| {
            if let JobsEvent::RunStarted { run } = event {
                started = Some(run.clone());
            }
            started.is_some()
        })
        .await;
        let started = started.unwrap();
        assert_eq!(started.trigger, RunTrigger::Service);
        h.declared.kill(&started.id).await.unwrap().unwrap();
        until_event(
            &mut events,
            |event| matches!(event, JobsEvent::RunEnded { run } if run.id == started.id),
        )
        .await;
    }

    // spec: DMN-321, DMN-322, DMN-335, PRT-200, PRT-210
    #[tokio::test(start_paused = true)]
    async fn a_refused_file_keeps_the_last_valid_jobs_and_a_removed_one_tears_them_down() {
        let h = harness();
        let mut changes = h.declared.subscribe_changes();
        h.write(&json!({ "jobs": {
            "build": { "command": "make" },
            "nightly": { "trigger": "schedule", "schedule": "0 3 * * *", "command": "make", "enabled": false }
        } }))
        .await;
        assert!(changes.has_changed().unwrap());
        assert_eq!(
            h.declared.counts().get(&h.workspace).copied(),
            Some(JobCounts {
                total: 2,
                manual: 1,
                schedule: 1,
                ..JobCounts::default()
            })
        );
        assert_eq!(h.view("nightly").status, JobViewStatus::Disabled);
        assert_eq!(h.view("nightly").next_at, None);
        changes.mark_unchanged();

        std::fs::write(
            file::path(&h.folder),
            r#"{ "jobs": { "build": { "command": "" } } }"#,
        )
        .unwrap();
        h.declared.reload(&h.workspace).await;

        let state = h.state();
        assert_eq!(
            state
                .jobs
                .iter()
                .map(|job| job.name.as_str())
                .collect::<Vec<_>>(),
            ["build", "nightly"]
        );
        assert_eq!(
            state.error.as_deref(),
            Some("jobs.build.command: expected a non-blank shell command")
        );
        assert!(h.declared.counts()[&h.workspace].error);
        assert!(changes.has_changed().unwrap());

        std::fs::remove_file(file::path(&h.folder)).unwrap();
        h.declared.reload(&h.workspace).await;

        assert_eq!(h.state(), JobsState::default());
        assert_eq!(h.declared.counts().get(&h.workspace), None);
    }

    // spec: DMN-332, PRT-205, PRT-206, PRT-207
    #[tokio::test(start_paused = true)]
    async fn define_remove_and_enable_write_the_file_one_job_at_a_time() {
        let h = harness();
        let view = h
            .declared
            .define(
                &h.workspace,
                "test".into(),
                json!({ "command": "just test", "enabled": true }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(view.status, JobViewStatus::Idle);
        assert_eq!(view.job.command.as_deref(), Some("just test"));
        assert_eq!(
            std::fs::read_to_string(file::path(&h.folder)).unwrap(),
            "{\n  \"jobs\": {\n    \"test\": {\n      \"command\": \"just test\"\n    }\n  }\n}\n"
        );

        let error = h
            .declared
            .define(
                &h.workspace,
                "test".into(),
                json!({ "command": "other" }),
                false,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        let error = h
            .declared
            .define(
                &h.workspace,
                "test".into(),
                json!({ "command": "x", "cwd": "../up" }),
                true,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::BadRequest);
        assert!(error.message.starts_with("job.cwd: "), "{}", error.message);

        let view = h
            .declared
            .enable(&h.workspace, "test".into(), false)
            .await
            .unwrap();
        assert_eq!(view.status, JobViewStatus::Disabled);
        assert!(
            std::fs::read_to_string(file::path(&h.folder))
                .unwrap()
                .contains("\"enabled\": false")
        );

        h.declared
            .remove(&h.workspace, "test".into())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(file::path(&h.folder)).unwrap(),
            "{\n  \"jobs\": {}\n}\n"
        );
        assert_eq!(h.state().jobs, []);
        let error = h
            .declared
            .remove(&h.workspace, "test".into())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);

        // An invalid current file is never overwritten.
        std::fs::write(file::path(&h.folder), "{ nope").unwrap();
        let error = h
            .declared
            .define(&h.workspace, "test".into(), json!({ "command": "x" }), true)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::BadRequest);
        assert_eq!(
            std::fs::read_to_string(file::path(&h.folder)).unwrap(),
            "{ nope"
        );
    }

    // spec: DMN-324, PRT-204
    #[tokio::test(start_paused = true)]
    async fn unknown_and_disabled_jobs_do_not_run() {
        let h = harness();
        h.write(&json!({ "jobs": { "off": { "command": "true", "enabled": false } } }))
            .await;

        let error = h
            .declared
            .run(&h.workspace, "nope", Provenance::User)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
        let error = h
            .declared
            .run(&h.workspace, "off", Provenance::User)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(h.runs("off"), []);
    }

    // spec: DMN-328, DMN-334, DMN-218, PRT-201, PRT-209
    #[tokio::test(start_paused = true)]
    async fn a_prompt_run_makes_a_thread_of_the_job_and_ends_with_its_prompt_result() {
        let h = harness();
        h.write(&json!({ "jobs": { "ask": {
            "prompt": { "text": "Say hi", "model": "anthropic/claude", "approvalMode": "write" }
        } } }))
        .await;
        h.launcher.push_run(fixture("text_reply"));

        let run = h
            .declared
            .run(&h.workspace, "ask", Provenance::User)
            .await
            .unwrap();

        assert_eq!(
            (run.trigger, run.status),
            (RunTrigger::Manual, JobRunStatus::Running)
        );
        assert_eq!(run.started_by, Some(CreatedBy::User));
        let thread = h.threads.get(run.thread_id.as_deref().unwrap()).unwrap();
        assert_eq!(thread.title, "ask · 2026-11-16 08:00");
        assert_eq!(thread.title_source, store::TitleSource::Set);
        assert_eq!(thread.created_by, Provenance::Job("ask".into()));
        assert_eq!(thread.model.as_deref(), Some("anthropic/claude"));
        assert_eq!(thread.approval_mode.as_deref(), Some("write"));
        assert!(!h.store.created_clients().unwrap().contains_key(&thread.id));
        h.until(|h| h.view("ask").last_run.is_some()).await;
        let last = h.view("ask").last_run.unwrap();
        assert_eq!((last.id, last.status), (run.id, JobRunStatus::Succeeded));
        let history = h
            .store
            .prompts(&PromptQuery {
                thread_id: None,
                workspace_id: None,
                query: None,
                limit: 100,
            })
            .unwrap();
        assert!(
            history.is_empty(),
            "a job's prompt stays out of the history"
        );
    }

    // spec: DMN-328
    #[tokio::test(start_paused = true)]
    async fn a_reuse_prompt_sends_to_the_jobs_thread_and_skips_it_while_busy() {
        let h = harness();
        h.write(
            &json!({ "jobs": { "daily": { "prompt": { "text": "Go on", "thread": "reuse" } } } }),
        )
        .await;
        h.launcher.push_run(fixture("text_reply"));

        let first = h
            .declared
            .run(&h.workspace, "daily", Provenance::User)
            .await
            .unwrap();
        let thread_id = first.thread_id.clone().unwrap();
        assert_eq!(h.threads.get(&thread_id).unwrap().title, "daily");
        assert_eq!(
            h.store
                .job_state(&h.workspace, "daily")
                .unwrap()
                .thread_id
                .as_deref(),
            Some(thread_id.as_str())
        );
        h.until(|h| h.view("daily").last_run.is_some()).await;

        let second = h
            .declared
            .run(&h.workspace, "daily", Provenance::User)
            .await
            .unwrap();
        assert_eq!(second.thread_id.as_deref(), Some(thread_id.as_str()));
        assert_eq!(h.launcher.launches().len(), 1);
        // The fake has no second answer: the run ends once killed and omp gone.
        h.declared.kill(&second.id).await.unwrap().unwrap();
        h.launcher.process(0).exit();
        h.until(|h| h.view("daily").run.is_none()).await;

        // The thread is busy: someone else's prompt waits on a dialog.
        let busy = h
            .threads
            .create(NewThread::new("mine", &h.workspace))
            .unwrap();
        h.store
            .set_job_thread(&h.workspace, "daily", &busy.id)
            .unwrap();
        h.launcher.push_run(fixture("approval_granted"));
        h.supervisor
            .prompt(
                &busy.id,
                PromptCommand {
                    message: "edit it".into(),
                    images: None,
                    streaming_behavior: None,
                },
            )
            .await
            .unwrap();
        h.until(|h| h.supervisor.status(&busy.id).needs_attention)
            .await;

        let error = h
            .declared
            .run(&h.workspace, "daily", Provenance::User)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        let space = h.declared.space(&h.workspace).unwrap();
        let skipped = h
            .declared
            .start(&space, "daily", RunTrigger::Schedule, None, false)
            .await
            .unwrap();
        assert_eq!(
            (
                skipped.status,
                skipped.reason.as_deref(),
                skipped.thread_id.as_deref()
            ),
            (JobRunStatus::Skipped, Some("busy"), Some(busy.id.as_str()))
        );

        // An archived thread makes way for a new one.
        h.threads.archive(&busy.id).unwrap();
        h.launcher.push_run(fixture("text_reply"));
        let fresh = h
            .declared
            .run(&h.workspace, "daily", Provenance::User)
            .await
            .unwrap();
        assert_ne!(fresh.thread_id.as_deref(), Some(busy.id.as_str()));
    }

    // spec: DMN-324, DMN-322
    #[tokio::test(start_paused = true)]
    async fn a_job_declared_again_adopts_the_run_its_removed_declaration_left_running() {
        let h = harness();
        h.write(&json!({ "jobs": { "ask": { "prompt": { "text": "hi" } } } }))
            .await;
        // The fake omp never answers: the prompt run keeps running.
        let run = h
            .declared
            .run(&h.workspace, "ask", Provenance::User)
            .await
            .unwrap();
        h.declared.remove(&h.workspace, "ask".into()).await.unwrap();

        let view = h
            .declared
            .define(
                &h.workspace,
                "ask".into(),
                json!({ "trigger": "schedule", "schedule": "* * * * *", "prompt": { "text": "hi" } }),
                false,
            )
            .await
            .unwrap();

        assert_eq!(view.status, JobViewStatus::Running);
        assert_eq!(view.run.map(|running| running.id), Some(run.id.clone()));
        let error = h
            .declared
            .run(&h.workspace, "ask", Provenance::User)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        // The next due time is 08:01, a minute after the clock's start.
        tokio::time::sleep(Duration::from_secs(61)).await;
        h.until(|h| h.runs("ask").len() == 2).await;
        let due = &h.runs("ask")[1];
        assert_eq!(
            (due.trigger, due.status, due.reason.as_deref()),
            (RunTrigger::Schedule, JobRunStatus::Skipped, Some("running"))
        );
        assert_eq!(h.launcher.launches().len(), 1, "no second thread started");
    }

    // spec: DMN-328
    #[tokio::test(start_paused = true)]
    async fn a_reuse_thread_moved_to_another_workspace_makes_way_for_a_new_one() {
        let h = harness();
        h.write(
            &json!({ "jobs": { "daily": { "prompt": { "text": "Go on", "thread": "reuse" } } } }),
        )
        .await;
        h.launcher.push_run(fixture("text_reply"));
        let first = h
            .declared
            .run(&h.workspace, "daily", Provenance::User)
            .await
            .unwrap();
        let moved = first.thread_id.unwrap();
        h.until(|h| h.view("daily").last_run.is_some()).await;
        let other = store::Workspace {
            id: new_id(),
            name: "other".into(),
            path: h.folder.with_file_name("other"),
            created_by: Provenance::User,
            created_at: 0,
        };
        h.store.insert_workspace(&other).unwrap();
        h.store
            .reorder_thread(&moved, Some(&other.id), None)
            .unwrap();
        h.launcher.push_run(fixture("text_reply"));

        let second = h
            .declared
            .run(&h.workspace, "daily", Provenance::User)
            .await
            .unwrap();

        let replacement = second.thread_id.unwrap();
        assert_ne!(replacement, moved);
        let thread = h.threads.get(&replacement).unwrap();
        assert_eq!(
            (thread.workspace_id, thread.title),
            (h.workspace.clone(), "daily".to_owned())
        );
        assert_eq!(
            h.store
                .job_state(&h.workspace, "daily")
                .unwrap()
                .thread_id
                .as_deref(),
            Some(replacement.as_str())
        );
    }

    // spec: DMN-328, DMN-211, PRT-204
    #[tokio::test(start_paused = true)]
    async fn a_prompt_run_whose_thread_cannot_start_fails_and_leaves_no_thread() {
        let h = harness();
        h.write(&json!({ "jobs": { "ask": { "prompt": { "text": "hi" } } } }))
            .await;
        h.launcher.fail_next_launch("no model configured");

        let error = h
            .declared
            .run(&h.workspace, "ask", Provenance::User)
            .await
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::ThreadFailed);
        assert_eq!(h.store.threads().unwrap(), []);
        let runs = h.runs("ask");
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].status, runs[0].thread_id.as_deref()),
            (JobRunStatus::Failed, None)
        );
        assert!(
            runs[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("no model configured")
        );
    }

    // spec: DMN-328
    #[tokio::test(start_paused = true)]
    async fn killing_a_prompt_run_aborts_the_threads_turn() {
        let h = harness();
        h.write(&json!({ "jobs": { "ask": { "prompt": { "text": "hi" } } } }))
            .await;
        let run = h
            .declared
            .run(&h.workspace, "ask", Provenance::User)
            .await
            .unwrap();

        h.declared.kill(&run.id).await.unwrap().unwrap();

        let process = h.launcher.process(0);
        assert!(
            process
                .calls()
                .iter()
                .any(|call| matches!(call, FakeCall::Call(OmpCall::AbortAndRestoreQueue)))
        );
        // omp stops without a result: the run ends killed.
        process.exit();
        h.until(|h| h.view("ask").last_run.is_some()).await;
        assert_eq!(h.view("ask").last_run.unwrap().status, JobRunStatus::Killed);
        assert!(h.declared.kill("unknown").await.is_none());
    }

    // spec: DMN-330
    #[tokio::test(start_paused = true)]
    async fn a_stored_command_run_serves_its_kept_lines() {
        let h = harness();
        let run = JobRun {
            id: new_id(),
            workspace_id: h.workspace.clone(),
            job: "test".into(),
            trigger: "manual".into(),
            started_by: None,
            thread_id: None,
            started_at: 10,
            ended_at: None,
            status: "running".into(),
            reason: None,
            exit_code: None,
            signal: None,
            command: Some("make".into()),
        };
        h.store.insert_job_run(&run).unwrap();
        assert_eq!(h.declared.recover().unwrap(), 1);
        h.store
            .finish_job_run(
                &run.id,
                &RunEnd {
                    ended_at: 20,
                    status: "failed",
                    reason: None,
                    exit_code: Some(2),
                    signal: None,
                    output_tail: Some(r#"[{"stream":"stderr","text":"boom"}]"#),
                    line_count: 300,
                },
            )
            .unwrap();

        let log = h.declared.stored_log(&run.id).await.unwrap();

        let state = serde_json::to_value(log.with_state(Clone::clone)).unwrap();
        assert_eq!(
            state,
            json!({
                "job": { "id": run.id, "workspaceId": h.workspace, "command": "make", "startedAt": 10,
                         "status": { "state": "exited", "code": 2, "signal": null, "endedAt": 20 } },
                "lines": [{ "stream": "stderr", "text": "boom" }],
                "dropped": 299
            })
        );
        assert!(h.declared.stored_log("unknown").await.is_none());
    }
}
