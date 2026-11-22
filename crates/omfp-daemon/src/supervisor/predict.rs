//! The omp answering the composer's word completions (DMN-360..DMN-363). omp 19.1.0 offers its TUI's word
//! autocomplete over RPC (`predict_word`, `predict_word_feedback`): every omp relays them to one shared predictor of
//! its agent directory (the `predict` unit of omp's per-user daemon), which reads omp's `spelling.autocomplete`
//! setting, learns from the profile's prompt history and Claude Code's and Codex's, and answers within milliseconds
//! once warm. Any omp of omfp's
//! profile reaches the same predictor, so the daemon keeps one omp of its own for it — started like the login omp
//! (DMN-219) on the first request — rather than asking a thread's, which may be stopped (DMN-070) or not exist yet
//! (the draft thread's composer).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use omp_rpc::{PredictWordCommand, PredictWordFeedbackCommand};
use serde_json::Value;
use tokio::time::Instant;
use tracing::{info, warn};

use super::{Result, Supervisor, SupervisorError, blocking, lock};
use crate::agent::{AgentProcess, OmpCall};

/// After a failed start, how long requests get that failure without starting omp again (DMN-362).
pub const PREDICTOR_RETRY_AFTER: Duration = Duration::from_mins(1);

/// The predictor omp and what decides its next start.
#[derive(Default)]
pub(super) struct Predictor {
    /// Held while the omp starts, so concurrent requests wait for one start.
    start: tokio::sync::Mutex<()>,
    /// Never held across an await: the reaper and the daemon's shutdown take it whatever a start does (DMN-363).
    state: std::sync::Mutex<State>,
}

impl Predictor {
    /// Marks a request and answers the running omp, `None` when one must start, or the failure of a recent start.
    fn running(&self) -> Result<Option<Arc<dyn AgentProcess>>> {
        let mut state = lock(&self.state);
        let now = Instant::now();
        state.last_used = Some(now);
        if let Some(process) = &state.process {
            return Ok(Some(Arc::clone(process)));
        }
        match state.failure.as_ref().filter(|failure| now < failure.until) {
            Some(failure) => Err(failure.error()),
            None => Ok(None),
        }
    }
}

#[derive(Default)]
struct State {
    process: Option<Arc<dyn AgentProcess>>,
    /// When a request last used it; the idle reaper stops it `idle_timeout` later (DMN-363).
    last_used: Option<Instant>,
    /// The last failed start, answered until `until` (DMN-362).
    failure: Option<Failure>,
}

struct Failure {
    until: Instant,
    message: String,
    stderr_tail: String,
}

impl Failure {
    fn error(&self) -> SupervisorError {
        SupervisorError::Start {
            message: self.message.clone(),
            stderr_tail: self.stderr_tail.clone(),
        }
    }
}

impl Supervisor {
    /// `prompts.predict_word` (DMN-360): the completion of the word ending at `cursor` (UTF-16 code units of `text`),
    /// `None` when none applies or omp's setting turns word completion off.
    pub async fn predict_word(&self, text: String, cursor: u32) -> Result<Option<String>> {
        let command = PredictWordCommand {
            text,
            cursor: cursor.into(),
        };
        let answer = self.on_predictor(OmpCall::PredictWord(command)).await?;
        Ok(answer
            .get("suffix")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    /// `prompts.predict_word_feedback` (DMN-360): a shown completion the user accepted or typed past.
    pub async fn predict_word_feedback(
        &self,
        text: String,
        cursor: u32,
        suggestion: String,
        accepted: bool,
    ) -> Result<()> {
        let command = PredictWordFeedbackCommand {
            text,
            cursor: cursor.into(),
            suggestion,
            accepted,
        };
        self.on_predictor(OmpCall::PredictWordFeedback(command))
            .await
            .map(drop)
    }

    /// Sends `call` to the predictor omp, started first when none runs (DMN-361); one that went away is forgotten, so
    /// the next request starts another.
    async fn on_predictor(&self, call: OmpCall) -> Result<Value> {
        let process = self.predictor_process().await?;
        let forwarded = Arc::clone(&process);
        match blocking(move || forwarded.call(call)).await? {
            Ok(answer) => Ok(answer),
            Err(error) => {
                if !matches!(error, omp_rpc::Error::Command { .. }) {
                    let mut state = lock(&self.inner.predictor.state);
                    if state
                        .process
                        .as_ref()
                        .is_some_and(|running| Arc::ptr_eq(running, &process))
                    {
                        warn!(%error, "the predictor omp went away");
                        state.process = None;
                    }
                }
                Err(error.into())
            }
        }
    }

    /// The running predictor omp, else one started now; within [`PREDICTOR_RETRY_AFTER`] of a failed start, that
    /// failure (DMN-362). An omp that finished starting after the daemon's shutdown began is stopped at once.
    async fn predictor_process(&self) -> Result<Arc<dyn AgentProcess>> {
        let predictor = &self.inner.predictor;
        if let Some(process) = predictor.running()? {
            return Ok(process);
        }
        let _start = predictor.start.lock().await;
        if let Some(process) = predictor.running()? {
            return Ok(process);
        }
        let started = self
            .on_helper_omp(self.inner.data.predictor(), |launcher, spec| {
                launcher
                    .launch(spec, Vec::new(), Vec::new())
                    .map(|launched| launched.process)
            })
            .await;
        // Shutdown raises its flag before it empties the slot under this lock: one of the two stops the omp.
        let mut state = lock(&predictor.state);
        match started {
            Ok(process) if self.inner.shutting_down.load(Ordering::SeqCst) => {
                process.kill();
                Err(SupervisorError::ShuttingDown)
            }
            Ok(process) => {
                info!("started the predictor omp");
                state.failure = None;
                state.process = Some(Arc::clone(&process));
                Ok(process)
            }
            Err(SupervisorError::Start {
                message,
                stderr_tail,
            }) => {
                let failure = Failure {
                    until: Instant::now() + PREDICTOR_RETRY_AFTER,
                    message,
                    stderr_tail,
                };
                let error = failure.error();
                state.failure = Some(failure);
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    /// Stops the predictor omp once no request used it for `timeout` (DMN-363); one still starting has no process yet.
    pub(super) fn reap_predictor(&self, timeout: Duration) {
        let mut state = lock(&self.inner.predictor.state);
        let idle = state
            .last_used
            .is_none_or(|used| Instant::now().saturating_duration_since(used) >= timeout);
        if idle && let Some(process) = state.process.take() {
            info!("stopping the idle predictor omp");
            process.kill();
        }
    }

    /// Stops the predictor omp at shutdown (DMN-005), after the shutdown flag is up; never waits for a start, whose omp
    /// [`Supervisor::predictor_process`] stops itself.
    pub(super) fn stop_predictor(&self) {
        if let Some(process) = lock(&self.inner.predictor.state).process.take() {
            process.kill();
        }
    }
}
