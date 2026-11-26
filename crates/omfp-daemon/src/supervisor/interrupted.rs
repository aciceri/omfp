//! Turns a daemon stop or a crash of omp interrupted (DMN-370..DMN-374, DMN-377): the shutdown marks the threads whose
//! omp was mid-turn or waiting on async work, an omp exiting on its own marks its thread's turn the same way, the mark
//! outlives the daemon in the store, and `thread.resume` (or the thread's next message) takes it.
//!
//! Whether a thread is unsettled is read from its state, never from its process: an omp dying with the daemon (a
//! signal to the whole process group or cgroup) ends before the shutdown gets to it, so the shutdown takes its list of
//! unsettled threads the moment it begins, and an omp's exit marks its own thread's turn before failing the thread.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use omp_rpc::{PromptCommand, StreamingBehavior};
use tracing::{info, warn};

use super::{Actor, Result, Sending, Supervisor, SupervisorError, lock};
use crate::agent::{AgentProcess, PromptAdmission};
use crate::store::{Interruption, now_ms};

/// What `thread.resume` sends the agent after a daemon stop (DMN-372).
pub const RESUME_NOTICE: &str = "[omfp] Your previous turn was interrupted by a daemon restart: omfp stopped your omp \
                                 process while you were working, so the tool calls running then (commands, waits, \
                                 subagents) were cut off and their results are lost. Check what was in progress and \
                                 continue.";

/// What `thread.resume` sends the agent after its omp exited on its own (DMN-372, DMN-377).
pub const CRASH_RESUME_NOTICE: &str = "[omfp] Your previous turn was interrupted because your omp process exited \
                                       unexpectedly (a crash, or a kill for lack of memory) while you were working, \
                                       so the tool calls running then (commands, waits, subagents) were cut off and \
                                       their results are lost. Check what was in progress and continue.";

/// The notice `thread.resume` sends for a turn `cause` interrupted (DMN-372).
fn resume_notice(cause: Interruption) -> &'static str {
    match cause {
        Interruption::DaemonStop => RESUME_NOTICE,
        Interruption::Crash => CRASH_RESUME_NOTICE,
    }
}

/// How long the shutdown waits for messages on their way to omp before marking the interrupted turns (DMN-370): a
/// hung send must not hold the daemon past its shutdown grace (DMN-005).
pub(super) const MARK_WAIT: Duration = Duration::from_secs(2);

impl Actor {
    /// Mid-turn, waiting on async work that can resume the run, or sent agent work omp has not reported yet
    /// (DMN-370), read from the thread's state: an omp that just exited still counts until the thread fails.
    pub(super) fn unsettled(&self) -> bool {
        !self.log.with_state(|state| state.settled) || self.turn_requested.load(Ordering::SeqCst)
    }
}

impl Supervisor {
    /// The unsettled threads (DMN-370): a thread whose omp stopped or failed has settled, and a stop cleared the
    /// message it was sent (DMN-374).
    pub(super) fn unsettled(&self) -> Vec<String> {
        let actors: Vec<_> = lock(&self.inner.actors).values().cloned().collect();
        actors
            .iter()
            .filter(|actor| actor.unsettled())
            .map(|actor| actor.id.clone())
            .collect()
    }

    /// Marks the turns of the unsettled threads interrupted by a daemon stop in the store before their omp stops
    /// (DMN-370); a failure is logged, the shutdown goes on. Called once `shutting_down` is set, with `ids` the
    /// threads unsettled when it was set, before any omp could end: it waits (at most [`MARK_WAIT`]) for the sends
    /// and clears under way, and later ones see the flag and leave the marks alone (DMN-373). The threads unsettled
    /// then count too: a send that is still under way counts anyway, its thread's turn was requested before it went
    /// to omp.
    pub(super) async fn mark_interrupted(&self, mut ids: Vec<String>) {
        let marks = tokio::time::timeout(MARK_WAIT, self.inner.interrupted_marks.write()).await;
        if marks.is_err() {
            warn!(
                "messages on their way to omp held the marking of the interrupted turns back; marking anyway"
            );
        }
        ids.extend(self.unsettled());
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        self.write_marks(&ids, Interruption::DaemonStop).await;
    }

    /// omp exited on its own (DMN-035) while its thread was unsettled: marks the turn interrupted (DMN-377), by a
    /// daemon stop once the shutdown began (omp died with the daemon), by a crash otherwise. The caller took the
    /// process out of the thread first; the marks taken exclusively, at most [`MARK_WAIT`], order the mark after
    /// every clear that found the process still running ([`Self::clear_interrupted`]).
    pub(super) async fn mark_exited(&self, actor: &Actor) {
        let marks = tokio::time::timeout(MARK_WAIT, self.inner.interrupted_marks.write()).await;
        if marks.is_err() {
            warn!(
                thread = %actor.id,
                "messages on their way to omp held the marking of the cut turn back; marking anyway"
            );
        }
        let cause = if self.inner.shutting_down.load(Ordering::SeqCst) {
            Interruption::DaemonStop
        } else {
            Interruption::Crash
        };
        self.write_marks(std::slice::from_ref(&actor.id), cause)
            .await;
    }

    /// Marks the turns of `ids` interrupted by `cause` now; a failure is only logged (DMN-370, DMN-377).
    async fn write_marks(&self, ids: &[String], cause: Interruption) {
        let marked = ids.to_vec();
        match self
            .stores(move |inner| inner.threads.mark_interrupted(&marked, now_ms(), cause))
            .await
        {
            Ok(()) => info!(threads = ?ids, ?cause, "marked the running turns interrupted"),
            Err(error) => {
                warn!(threads = ?ids, ?cause, %error, "marking the running turns interrupted failed");
            }
        }
    }

    /// Takes the thread's interrupted mark, if any, and publishes the change (DMN-373): `Some` with when its turn
    /// was interrupted and by what.
    async fn take_interrupted(&self, actor: &Actor) -> Result<Option<(i64, Interruption)>> {
        let id = actor.id.clone();
        let taken = self
            .stores(move |inner| inner.threads.take_interrupted(&id))
            .await?;
        if taken.is_some() {
            let _ = self.inner.changes.send(actor.id.clone());
        }
        Ok(taken)
    }

    /// A message `admitted_by` the thread's omp reached the agent: the interrupted turn is the user's business no more
    /// (DMN-373). Nothing is cleared once the daemon shuts down (the shutdown marks the turns it cuts, this message's
    /// included), nor once that omp is gone: its exit marks the turn it cut (DMN-377), this message's included.
    pub(super) async fn clear_interrupted(
        &self,
        actor: &Actor,
        admitted_by: &Arc<dyn AgentProcess>,
    ) {
        let _marks = self.inner.interrupted_marks.read().await;
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let running = lock(&actor.live)
            .as_ref()
            .is_some_and(|live| Arc::ptr_eq(&live.process, admitted_by));
        if !running {
            return;
        }
        if let Err(error) = self.take_interrupted(actor).await {
            warn!(thread = %actor.id, %error, "clearing the interrupted mark failed");
        }
    }

    /// `thread.resume` (DMN-372): takes the thread's interrupted mark, starts omp when needed and sends the notice of
    /// what interrupted the turn ([`RESUME_NOTICE`], [`CRASH_RESUME_NOTICE`]) as a prompt, returning at admission like
    /// `thread.prompt`. Without a mark (never interrupted, resumed already, a message sent since) it is refused and
    /// sends nothing; when the notice cannot be sent, the mark comes back. Cancellation-safe ([`Self::detached`]): a
    /// caller that goes away (an agent's cancelled call, a closed MCP stream) never leaves the mark taken without the
    /// notice sent.
    pub async fn resume(&self, id: &str) -> Result<PromptAdmission> {
        let id = id.to_owned();
        self.detached(|supervisor| async move { supervisor.resume_now(&id).await })
            .await
    }

    async fn resume_now(&self, id: &str) -> Result<PromptAdmission> {
        let actor = self.actor(id).await?;
        let Some((at, cause)) = self.take_interrupted(&actor).await? else {
            return Err(SupervisorError::NotInterrupted);
        };
        let command = PromptCommand {
            message: resume_notice(cause).to_owned(),
            images: None,
            // omp needs a behavior while the agent works (02-daemon, "Commands towards omp").
            streaming_behavior: (!actor.status().settled).then_some(StreamingBehavior::FollowUp),
        };
        let sending = Sending::new(&actor.last_prompt);
        match self
            .deliver(&actor, move |process| process.prompt(command))
            .await
        {
            Ok((admission, _)) => {
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
                    .stores(move |inner| inner.threads.mark_interrupted(&ids, at, cause))
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
