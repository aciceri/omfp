//! A thread's omp process (DMN-030..DMN-040): its start in the workspace folder with the thread's host tools, the
//! failed start that keeps the transcript readable from the session file, the event pump, the crash and the stop.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use omfp_core::{ProcessState, ThreadEvent};
use omp_rpc::RpcNotification;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use super::roots::root_paths;
use super::{
    Actor, Live, Result, Supervisor, SupervisorError, TitleSource, blocking, conventions, lock,
};
use crate::agent::{AgentProcess, Lease};
use crate::host_tools;
use crate::omp::{LaunchSpec, startup_events, thread_event};
use crate::session_file::SessionRead;
use crate::store::{Thread, new_id};

/// Lines of `omp.log` reported with a crash (DMN-035).
pub(super) const STDERR_TAIL_LINES: usize = 20;

impl Supervisor {
    pub(super) async fn ensure_running(&self, actor: &Arc<Actor>) -> Result<Arc<dyn AgentProcess>> {
        if let Some(process) = actor.process() {
            return Ok(process);
        }
        let _lifecycle = actor.lifecycle.lock().await;
        if let Some(process) = actor.process() {
            return Ok(process);
        }
        self.start_locked(actor).await
    }

    /// Starts omp; the caller holds the lifecycle lock.
    pub(super) async fn start_locked(&self, actor: &Arc<Actor>) -> Result<Arc<dyn AgentProcess>> {
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return Err(SupervisorError::ShuttingDown);
        }
        let max = self.inner.config.max_processes;
        self.inner
            .running
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |running| {
                (running < max).then_some(running + 1)
            })
            .map_err(|_| SupervisorError::LimitReached(max))?;
        let started = self.launch(actor).await;
        if started.is_err() {
            self.inner.running.fetch_sub(1, Ordering::SeqCst);
        }
        started
    }

    /// How to start the thread's omp (DMN-030), from a fresh scan of its workspace (DMN-023); with the roots its
    /// session had and the ones it gets now.
    async fn launch_spec(
        &self,
        actor: &Actor,
    ) -> Result<(Thread, LaunchSpec, Vec<PathBuf>, Vec<PathBuf>)> {
        let id = actor.id.clone();
        let (thread, ws, repos, recorded) = self
            .stores(move |inner| {
                let thread = inner.threads.get(&id)?;
                let (view, _) = inner.workspaces.rescan(&thread.workspace_id)?;
                let recorded = inner.threads.roots(&id)?;
                Ok::<_, SupervisorError>((thread, view.workspace, view.repos, recorded))
            })
            .await?;
        let add_dirs = root_paths(&repos);
        let data = &self.inner.data;
        let spec = LaunchSpec {
            program: self.inner.config.omp_program.clone(),
            cwd: ws.path.clone(),
            add_dirs: add_dirs.clone(),
            session_dir: thread.session_dir.clone(),
            // Only a thread without a session of its own yet starts from a copy (DMN-301).
            fork: lock(&actor.fork_source)
                .take()
                .filter(|_| thread.omp_session_id.is_none()),
            model: thread.model.clone(),
            approval_mode: thread.approval_mode.clone(),
            append_system_prompt: Some(conventions::append_system_prompt(
                &ws.name,
                &ws.id,
                &ws.path,
                self.inner.config.agent_dir.as_deref(),
                &actor.id,
            )),
            extension: Some(data.omp_extension()),
            stderr_log: data.thread_log(&actor.id),
            env: self.inner.config.omp_env.clone(),
        };
        Ok((thread, spec, recorded, add_dirs))
    }

    async fn launch(&self, actor: &Arc<Actor>) -> Result<Arc<dyn AgentProcess>> {
        let (thread, spec, mut recorded, add_dirs) = self.launch_spec(actor).await?;
        // Taken before omp reads the files: a change while it starts reloads it once more (DMN-201). The new process
        // loads everything a due restart was for (DMN-203).
        lock(&actor.restart).clear();
        lock(&actor.oauth).clear();
        self.loading_mcp_configs(actor, spec.cwd.clone()).await;
        self.set_process(actor, ProcessState::Starting);

        self.check_lease(actor, &thread, &spec).await?;

        let agent_tools = lock(&self.inner.agent_host)
            .clone()
            .map(|host| host.tools(&actor.id, &thread.workspace_id))
            .unwrap_or_default();
        let tools = host_tools::thread_tools(
            self.file_opener(&actor.id),
            self.notifier(&actor.id),
            self.title_setter(&actor.id),
            agent_tools,
        );
        let check_host = lock(&self.inner.widget_host)
            .clone()
            .map(|host| (thread.workspace_id.clone(), host));
        let uris =
            match host_tools::host_uris(self.inner.config.builtin_widgets.clone(), check_host) {
                Ok(uris) => uris,
                Err(error) => {
                    return Err(self
                        .failed_start(actor, &thread, error.to_string(), String::new())
                        .await);
                }
            };
        let launcher = Arc::clone(&self.inner.launcher);
        let launch_spec = spec.clone();
        let launched = match blocking(move || launcher.launch(&launch_spec, tools, uris)).await? {
            Ok(launched) => launched,
            Err(error) => {
                let path = spec.stderr_log.clone();
                let tail = blocking(move || stderr_tail(&path, STDERR_TAIL_LINES))
                    .await
                    .unwrap_or_default();
                return Err(self
                    .failed_start(actor, &thread, error.to_string(), tail)
                    .await);
            }
        };

        // A new process starts a new epoch: positions of the old one cannot be replayed (PRT-022).
        actor.log.new_epoch(new_id());
        // omp numbers prompts per process (PRT-030, DMN-152).
        actor.last_prompt.send_modify(|last| last.id = None);
        // omp's own history follows; it knows the session's model (`thread.state`).
        lock(&actor.session_model).take();
        for event in startup_events(&launched.startup) {
            self.push(actor, event);
        }
        let threads = Arc::clone(&self.inner.threads);
        let (id, session_id) = (
            actor.id.clone(),
            launched.startup.session.session_id.clone(),
        );
        if let Err(error) = blocking(move || threads.set_omp_session_id(&id, &session_id)).await? {
            warn!(thread = %actor.id, %error, "cannot store the omp session id");
        }

        let generation = self.inner.generations.fetch_add(1, Ordering::SeqCst);
        let (ended_tx, ended) = watch::channel(false);
        let process = launched.process;
        actor.turn_requested.store(false, Ordering::SeqCst);
        *lock(&actor.live) = Some(Live {
            process: Arc::clone(&process),
            generation,
            ended,
        });
        self.spawn_pump(Arc::clone(actor), launched.events, generation, ended_tx);
        info!(thread = %actor.id, "omp started");
        self.started_with(&spec);
        // The flags added today's roots; the session still has the ones it saved before (S-10).
        recorded.extend(add_dirs.iter().cloned());
        if let Err(error) = self
            .apply_roots(actor, Arc::clone(&process), add_dirs, &recorded)
            .await
        {
            warn!(thread = %actor.id, %error, "cannot remove stale roots");
        }
        Ok(process)
    }

    /// A stored session another omp process owns fails the start (DMN-033); its transcript stays readable (DMN-337).
    async fn check_lease(&self, actor: &Actor, thread: &Thread, spec: &LaunchSpec) -> Result<()> {
        let Some(session_id) = thread.omp_session_id.clone() else {
            return Ok(());
        };
        let launcher = Arc::clone(&self.inner.launcher);
        let probe = spec.clone();
        let lease = blocking(move || launcher.session_lease(&probe, &session_id)).await?;
        match lease {
            Ok(Lease::Held) => {
                let error = SupervisorError::SessionLeased {
                    session_id: thread.omp_session_id.clone().unwrap_or_default(),
                };
                self.history_from_file(actor, thread).await;
                self.fail(actor, error.to_string(), String::new());
                Err(error)
            }
            Ok(Lease::Free) => Ok(()),
            Ok(Lease::Unknown) => {
                warn!(thread = %actor.id, "omp could not probe the session lease; starting anyway");
                Ok(())
            }
            Err(error) => Err(self
                .failed_start(actor, thread, error.to_string(), String::new())
                .await),
        }
    }

    /// What the thread's `omfp_open_file` host tool calls (DMN-060): [`Self::open_file`] on this thread. It holds the
    /// supervisor weakly (the tools live as long as the process it owns).
    fn file_opener(&self, id: &str) -> host_tools::OpenFile {
        let inner = Arc::downgrade(&self.inner);
        let id = id.to_owned();
        Arc::new(move |path: &str, line: Option<u64>| {
            let inner = Weak::upgrade(&inner).ok_or("the daemon is shutting down")?;
            Self { inner }.open_file(&id, path, line)
        })
    }

    /// What the thread's `omfp_notify` host tool calls (DMN-125): [`Self::notify`] on this thread, holding the
    /// supervisor weakly like [`Self::file_opener`].
    fn notifier(&self, id: &str) -> host_tools::Notify {
        let inner = Arc::downgrade(&self.inner);
        let id = id.to_owned();
        Arc::new(move |title: &str, body: &str| {
            let inner = Weak::upgrade(&inner).ok_or("the daemon is shutting down")?;
            Self { inner }.notify(&id, title, body)
        })
    }

    /// What the thread's `set_thread_title` host tool calls (DMN-060): stores a set title, refreshes the index and
    /// names the omp session. It runs on an `omp-rpc` handler thread, so the store call blocks there and the session
    /// rename goes to the runtime; it holds the supervisor weakly (the tools live as long as the process it owns).
    pub(super) fn title_setter(&self, id: &str) -> host_tools::SetTitle {
        let inner = Arc::downgrade(&self.inner);
        let runtime = tokio::runtime::Handle::current();
        let id = id.to_owned();
        Arc::new(move |title: &str| {
            let inner = Weak::upgrade(&inner).ok_or("the daemon is shutting down")?;
            inner
                .threads
                .rename(&id, title, TitleSource::Set)
                .map_err(|error| error.to_string())?;
            let _ = inner.changes.send(id.clone());
            let supervisor = Self { inner };
            let id = id.clone();
            runtime.spawn(async move { supervisor.name_session(&id).await });
            Ok(())
        })
    }

    /// Fails the thread whose omp could not start and returns the error carrying omp's stderr tail (DMN-212); omp's
    /// refusal without a usable model may contradict the known count of usable models (DMN-237). The transcript stays
    /// readable from the session file (DMN-337).
    async fn failed_start(
        &self,
        actor: &Actor,
        thread: &Thread,
        message: String,
        stderr_tail: String,
    ) -> SupervisorError {
        self.failed_with(&stderr_tail);
        self.history_from_file(actor, thread).await;
        self.fail(actor, message.clone(), stderr_tail.clone());
        SupervisorError::Start {
            message,
            stderr_tail,
        }
    }

    /// A thread whose omp could not start shows its stored session's current branch, read from the session file
    /// without omp, as the `history` event omp's start would have given (DMN-337), and knows the model that session
    /// restores (DMN-338). Without a session file nothing changes; an unreadable one is only logged.
    async fn history_from_file(&self, actor: &Actor, thread: &Thread) {
        if let Some(read) = self.read_session_file(actor, thread).await {
            *lock(&actor.session_model) = read.model;
            self.push(
                actor,
                ThreadEvent::History {
                    session: read.session,
                    messages: read.messages,
                },
            );
        }
    }

    /// The stored session's current branch and saved model, read from its file without omp (DMN-337); `None`
    /// without a session file, an unreadable one only logged.
    pub(super) async fn read_session_file(
        &self,
        actor: &Actor,
        thread: &Thread,
    ) -> Option<SessionRead> {
        let dir = thread.session_dir.clone();
        let session_id = thread.omp_session_id.clone();
        let read = blocking(move || crate::session_file::read(&dir, session_id.as_deref()))
            .await
            .and_then(|read| read.map_err(|error| SupervisorError::Internal(error.to_string())));
        read.unwrap_or_else(|error| {
            warn!(thread = %actor.id, %error, "cannot read the session file");
            None
        })
    }

    fn set_process(&self, actor: &Actor, state: ProcessState) {
        self.push(actor, ThreadEvent::Process { state });
    }

    fn fail(&self, actor: &Actor, message: String, stderr_tail: String) {
        let failed = ProcessState::Failed {
            message,
            exit_code: None,
            stderr_tail,
        };
        self.set_process(actor, failed);
    }

    /// Forwards omp's frames into the log: a thread reads the blocking receiver into a channel, a task pushes them
    /// (DMN-040). When the stream ends without a requested stop, the thread fails (DMN-035).
    fn spawn_pump(
        &self,
        actor: Arc<Actor>,
        events: std::sync::mpsc::Receiver<omp_rpc::Event>,
        generation: u64,
        ended: watch::Sender<bool>,
    ) {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            for event in events {
                if sender.send(thread_event(event)).is_err() {
                    break;
                }
            }
        });
        let supervisor = self.clone();
        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                // Kept before the state shows them, so whoever saw them finds them kept (DMN-358).
                if let ThreadEvent::Omp {
                    frame: RpcNotification::AvailableCommandsUpdate(update),
                } = &event
                {
                    supervisor.keep_commands(&actor.id, &update.commands).await;
                }
                // Named before the state shows the line, as a first message names the thread (DMN-912).
                if let Some(line) = super::naming::spoken_line(&event) {
                    supervisor.name_from_voice(&actor, line).await;
                }
                supervisor.push(&actor, event);
            }
            // A live session ends with its process (DMN-103).
            lock(&actor.voice).take();
            supervisor.process_ended(&actor, generation).await;
            let _ = ended.send(true);
        });
    }

    async fn process_ended(&self, actor: &Actor, generation: u64) {
        let Some(crashed) = lock(&actor.live).take_if(|live| live.generation == generation) else {
            return;
        };
        self.inner.running.fetch_sub(1, Ordering::SeqCst);
        // Its OAuth listeners died with it: another process may take their ports (DMN-340).
        lock(&actor.oauth).clear();
        let path = self.inner.data.thread_log(&actor.id);
        let process = crashed.process;
        let tail = blocking(move || {
            drop(process);
            stderr_tail(&path, STDERR_TAIL_LINES)
        })
        .await
        .unwrap_or_default();
        warn!(thread = %actor.id, "omp exited unexpectedly");
        self.fail(actor, "omp exited unexpectedly".into(), tail);
    }

    /// Stops the process; the caller holds the lifecycle lock.
    pub(super) async fn stop_locked(&self, actor: &Actor) -> Result<()> {
        let Some(Live {
            process, mut ended, ..
        }) = lock(&actor.live).take()
        else {
            return Ok(());
        };
        blocking(move || drop(process)).await?;
        let _ = ended.wait_for(|ended| *ended).await;
        self.inner.running.fetch_sub(1, Ordering::SeqCst);
        // The next process loads everything anew (DMN-203).
        lock(&actor.mcp_configs).take();
        lock(&actor.restart).clear();
        lock(&actor.oauth).clear();
        self.set_process(actor, ProcessState::Stopped);
        info!(thread = %actor.id, "omp stopped");
        Ok(())
    }
}

/// The last `lines` lines of omp's stderr log (at most its last 16 KiB).
pub(super) fn stderr_tail(path: &Path, lines: usize) -> String {
    const MAX_BYTES: u64 = 16 * 1024;
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let length = file.metadata().map_or(0, |metadata| metadata.len());
    if file
        .seek(SeekFrom::Start(length.saturating_sub(MAX_BYTES)))
        .is_err()
    {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
