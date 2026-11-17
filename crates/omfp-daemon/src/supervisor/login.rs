//! The omp of `auth.*` (DMN-219..DMN-224). omp 18.22.0 runs its OAuth logins over RPC (`get_login_providers`,
//! `login`, `get_logout_accounts`, `logout`), but `omp --mode rpc-ui` exits at startup when no model is usable — no
//! credential and no `--model` — which is exactly the state before the first login. The daemon therefore starts a
//! short-lived omp of its own with an explicit model of omp's bundled catalog ([`LOGIN_MODEL`]): omp starts with it
//! without a credential. omp keeps credentials in the store of its agent directory, which every omp of the daemon
//! shares (same environment), so one login serves every thread; omp loads a thread's models only at startup, so the
//! running threads restart once they are free (DMN-224, DMN-228).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;

use omfp_protocol::RestartReason;
use omp_rpc::Event;
use tracing::{info, warn};

use super::{Result, STDERR_TAIL_LINES, Supervisor, SupervisorError, blocking, lock, stderr_tail};
use crate::agent::{AgentLauncher, AgentProcess};
use crate::omp::{LaunchSpec, StartError};

/// The `--model` of the login omp: any model of omp's bundled catalog lets it start without a credential; it never
/// prompts, so the model is never called.
pub const LOGIN_MODEL: &str = "anthropic/claude-sonnet-4-5";

/// A started login omp: its handle and its frames (the login's `open_url`, `notify` and `input` requests).
pub struct AuthOmp {
    pub process: Arc<dyn AgentProcess>,
    pub events: Receiver<Event>,
}

impl std::fmt::Debug for AuthOmp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthOmp").finish_non_exhaustive()
    }
}

/// The running `auth.login` (DMN-222).
pub(super) struct Running {
    pub(super) id: u64,
    provider_id: String,
    /// The connection whose window shows the flow.
    window: u64,
    process: Arc<dyn AgentProcess>,
    /// The sign-ins of this login waiting for their loopback callback (DMN-340), forgotten with the login.
    pub(super) oauth: crate::oauth_relay::Flows,
}

impl Supervisor {
    fn auth_spec(&self, model: Option<&str>) -> LaunchSpec {
        let dir = self.inner.data.login();
        LaunchSpec {
            program: self.inner.config.omp_program.clone(),
            cwd: dir.join("work"),
            add_dirs: Vec::new(),
            session_dir: dir.join("sessions"),
            fork: None,
            model: model.map(str::to_owned),
            approval_mode: None,
            append_system_prompt: None,
            extension: None,
            stderr_log: dir.join("omp.log"),
            env: self.inner.config.omp_env.clone(),
        }
    }

    /// Starts the login omp (DMN-219): with [`LOGIN_MODEL`], else once more without it (a setup that disables that
    /// provider has a model of its own). A failure carries the last lines of its stderr, as a thread's start does.
    pub async fn auth_omp(&self) -> Result<AuthOmp> {
        self.on_login_omp(|launcher, spec| {
            launcher
                .launch(spec, Vec::new(), Vec::new())
                .map(|launched| AuthOmp {
                    process: launched.process,
                    events: launched.events,
                })
        })
        .await
    }

    /// Runs `start` — something that starts an omp — on the login omp's spec, with [`LOGIN_MODEL`] and else without
    /// it, as [`Supervisor::auth_omp`] does; also the check of the usable models (DMN-235).
    pub(super) async fn on_login_omp<T: Send + 'static>(
        &self,
        start: impl Fn(&dyn AgentLauncher, &LaunchSpec) -> std::result::Result<T, StartError>
        + Send
        + Sync
        + 'static,
    ) -> Result<T> {
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return Err(SupervisorError::ShuttingDown);
        }
        let start = Arc::new(start);
        let mut failure = None;
        for model in [Some(LOGIN_MODEL), None] {
            let spec = self.auth_spec(model);
            let launcher = Arc::clone(&self.inner.launcher);
            let log = spec.stderr_log.clone();
            let start = Arc::clone(&start);
            let started = blocking(move || {
                if let Some(parent) = spec.stderr_log.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                start(launcher.as_ref(), &spec)
            });
            match started.await? {
                Ok(started) => return Ok(started),
                Err(error) => {
                    warn!(?model, %error, "the login omp did not start");
                    let tail = blocking(move || stderr_tail(&log, STDERR_TAIL_LINES))
                        .await
                        .unwrap_or_default();
                    failure = Some(SupervisorError::Start {
                        message: error.to_string(),
                        stderr_tail: tail,
                    });
                }
            }
        }
        Err(failure.unwrap_or_else(|| SupervisorError::Internal("no login omp start".into())))
    }

    /// Makes `process` the running login of `provider_id`, shown by the window of connection `window`; the login that
    /// ran before is cancelled (one at a time: omp's OAuth callback listeners take fixed ports, DMN-222). Returns the
    /// login's number for [`Supervisor::end_login`].
    pub fn begin_login(
        &self,
        provider_id: &str,
        window: u64,
        process: &Arc<dyn AgentProcess>,
    ) -> u64 {
        let id = self.inner.logins.fetch_add(1, Ordering::SeqCst) + 1;
        let previous = lock(&self.inner.login).replace(Running {
            id,
            provider_id: provider_id.to_owned(),
            window,
            process: Arc::clone(process),
            oauth: crate::oauth_relay::Flows::default(),
        });
        if let Some(previous) = previous {
            info!(provider = %previous.provider_id, "a new login cancels the running one");
            previous.process.kill();
        }
        id
    }

    /// Forgets login `id` once it ended; `false` when it was cancelled meanwhile (DMN-222).
    #[must_use]
    pub fn end_login(&self, id: u64) -> bool {
        lock(&self.inner.login)
            .take_if(|running| running.id == id)
            .is_some()
    }

    /// Cancels the running login — with `window`, only the one that window shows (its connection closed) — by
    /// stopping its omp; returns its provider (DMN-222).
    pub fn cancel_login(&self, window: Option<u64>) -> Option<String> {
        let running = lock(&self.inner.login)
            .take_if(|running| window.is_none_or(|window| running.window == window))?;
        info!(provider = %running.provider_id, "login cancelled");
        running.process.kill();
        Some(running.provider_id)
    }

    /// A login stored a credential (DMN-224): every running thread owes a restart for `login` (DMN-228) — at once
    /// when idle, after its turn when working, held back by background work — so its omp loads the models the
    /// credential unlocked.
    pub fn restart_after_login(&self) {
        let actors: Vec<_> = lock(&self.inner.actors).values().cloned().collect();
        for actor in actors {
            if actor.process().is_some() {
                self.restart_for(&actor, RestartReason::Login);
            }
        }
    }
}
