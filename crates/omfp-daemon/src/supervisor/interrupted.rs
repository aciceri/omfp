//! Turns a daemon stop interrupted (DMN-370..DMN-374): the shutdown marks the threads whose omp was mid-turn or
//! waiting on async work, the mark outlives the daemon in the store, and `thread.resume` (or the thread's next
//! message) takes it.

use std::sync::atomic::Ordering;
use std::time::Duration;

use omp_rpc::{PromptCommand, StreamingBehavior};
use tracing::{info, warn};

use super::{Actor, Result, Sending, Supervisor, SupervisorError, lock};
use crate::agent::PromptAdmission;
use crate::store::now_ms;

/// What `thread.resume` sends the agent (DMN-372).
pub const RESUME_NOTICE: &str = "[omfp] Your previous turn was interrupted by a daemon restart: omfp stopped your omp \
                                 process while you were working, so the tool calls running then (commands, waits, \
                                 subagents) were cut off and their results are lost. Check what was in progress and \
                                 continue.";

/// How long the shutdown waits for messages on their way to omp before marking the interrupted turns (DMN-370): a
/// hung send must not hold the daemon past its shutdown grace (DMN-005).
pub(super) const MARK_WAIT: Duration = Duration::from_secs(2);

impl Supervisor {
    /// The threads whose omp runs and whose session is not settled — mid-turn, or waiting on async work that can
    /// resume the run — or got a message whose turn omp has not reported yet (DMN-370).
    fn unsettled(&self) -> Vec<String> {
        let actors: Vec<_> = lock(&self.inner.actors).values().cloned().collect();
        actors
            .iter()
            .filter(|actor| {
                lock(&actor.live).is_some()
                    && (!actor.status().settled || actor.turn_requested.load(Ordering::SeqCst))
            })
            .map(|actor| actor.id.clone())
            .collect()
    }

    /// Marks the turns of the unsettled threads interrupted in the store before their omp stops (DMN-370); a failure
    /// is logged, the shutdown goes on. Called once `shutting_down` is set: it waits (at most [`MARK_WAIT`]) for the
    /// sends and clears under way, and later ones see the flag and leave the marks alone (DMN-373). A send that is
    /// still under way then counts anyway: its thread's turn was requested before it went to omp.
    pub(super) async fn mark_interrupted(&self) {
        let marks = tokio::time::timeout(MARK_WAIT, self.inner.interrupted_marks.write()).await;
        if marks.is_err() {
            warn!(
                "messages on their way to omp held the marking of the interrupted turns back; marking anyway"
            );
        }
        let ids = self.unsettled();
        if ids.is_empty() {
            return;
        }
        let marked = ids.clone();
        match self
            .stores(move |inner| inner.threads.mark_interrupted(&marked, now_ms()))
            .await
        {
            Ok(()) => info!(threads = ?ids, "marked the running turns interrupted"),
            Err(error) => {
                warn!(threads = ?ids, %error, "marking the running turns interrupted failed");
            }
        }
    }

    /// Takes the thread's interrupted mark, if any, and publishes the change (DMN-373): `Some` with when its turn
    /// was interrupted.
    async fn take_interrupted(&self, actor: &Actor) -> Result<Option<i64>> {
        let id = actor.id.clone();
        let taken = self
            .stores(move |inner| inner.threads.take_interrupted(&id))
            .await?;
        if taken.is_some() {
            let _ = self.inner.changes.send(actor.id.clone());
        }
        Ok(taken)
    }

    /// A message reached the agent: the turn a daemon stop interrupted is the user's business no more (DMN-373).
    /// Once the daemon shuts down nothing is cleared: the shutdown marks the turns it cuts, this message's included.
    pub(super) async fn clear_interrupted(&self, actor: &Actor) {
        let _marks = self.inner.interrupted_marks.read().await;
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        if let Err(error) = self.take_interrupted(actor).await {
            warn!(thread = %actor.id, %error, "clearing the interrupted mark failed");
        }
    }

    /// `thread.resume` (DMN-372): takes the thread's interrupted mark, starts omp when needed and sends
    /// [`RESUME_NOTICE`] as a prompt, returning at admission like `thread.prompt`. Without a mark (never interrupted,
    /// resumed already, a message sent since) it is refused and sends nothing; when the notice cannot be sent, the
    /// mark comes back. Cancellation-safe ([`Self::detached`]): a caller that goes away (an agent's cancelled call, a
    /// closed MCP stream) never leaves the mark taken without the notice sent.
    pub async fn resume(&self, id: &str) -> Result<PromptAdmission> {
        let id = id.to_owned();
        self.detached(|supervisor| async move { supervisor.resume_now(&id).await })
            .await
    }

    async fn resume_now(&self, id: &str) -> Result<PromptAdmission> {
        let actor = self.actor(id).await?;
        let Some(at) = self.take_interrupted(&actor).await? else {
            return Err(SupervisorError::NotInterrupted);
        };
        let command = PromptCommand {
            message: RESUME_NOTICE.to_owned(),
            images: None,
            // omp needs a behavior while the agent works (02-daemon, "Commands towards omp").
            streaming_behavior: (!actor.status().settled).then_some(StreamingBehavior::FollowUp),
        };
        let sending = Sending::new(&actor.last_prompt);
        match self
            .deliver(&actor, move |process| process.prompt(command))
            .await
        {
            Ok(admission) => {
                if admission.agent_invoked {
                    sending.admitted(admission.prompt_id.clone());
                } else {
                    actor.turn_requested.store(false, Ordering::SeqCst);
                }
                Ok(admission)
            }
            Err(error) => {
                let ids = vec![actor.id.clone()];
                if let Err(error) = self
                    .stores(move |inner| inner.threads.mark_interrupted(&ids, at))
                    .await
                {
                    warn!(thread = %actor.id, %error, "restoring the interrupted mark failed");
                }
                let _ = self.inner.changes.send(actor.id.clone());
                Err(error)
            }
        }
    }
}
