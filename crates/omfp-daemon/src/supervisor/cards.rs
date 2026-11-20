//! The `omfp_card` calls that wait for an answer (DMN-911, AGT-097): per thread and in memory only. A waiting card
//! makes its thread need the user; starting and ending a wait publish the thread's status like any other change.

use std::collections::hash_map::Entry;
use std::sync::Arc;

use omfp_protocol::CardAnswer;
use tokio::sync::oneshot;

use super::{Actor, Result, Supervisor, SupervisorError, lock};

/// A waiting `omfp_card` call (AGT-097). Dropping it ends the wait: the timeout passed, or omp cancelled the call
/// (DMN-151) and its future was dropped.
pub struct CardWait {
    supervisor: Supervisor,
    actor: Arc<Actor>,
    tool_call_id: String,
    answer: oneshot::Receiver<CardAnswer>,
}

impl std::fmt::Debug for CardWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CardWait")
            .field("thread", &self.actor.id)
            .field("tool_call_id", &self.tool_call_id)
            .finish_non_exhaustive()
    }
}

impl CardWait {
    /// The first answer `card.answer` gave (PRT-224).
    pub async fn answer(&mut self) -> Result<CardAnswer> {
        (&mut self.answer).await.map_err(|_| {
            SupervisorError::Internal(format!(
                "the wait of tool call {} ended without an answer",
                self.tool_call_id
            ))
        })
    }
}

impl Drop for CardWait {
    fn drop(&mut self) {
        // Gone already when an answer ended the wait.
        self.supervisor.publish_status(&self.actor, || {
            lock(&self.actor.cards).remove(&self.tool_call_id);
        });
    }
}

impl Supervisor {
    /// Starts the wait of thread `id`'s `omfp_card` call `tool_call_id` until the returned [`CardWait`] has an answer
    /// or drops (AGT-097); meanwhile the thread needs the user (DMN-911).
    pub async fn wait_card(&self, id: &str, tool_call_id: &str) -> Result<CardWait> {
        let actor = self.actor(id).await?;
        let (sender, answer) = oneshot::channel();
        self.publish_status(&actor, || {
            match lock(&actor.cards).entry(tool_call_id.to_owned()) {
                Entry::Occupied(_) => Err(SupervisorError::Internal(format!(
                    "tool call {tool_call_id} already waits for an answer"
                ))),
                Entry::Vacant(vacant) => {
                    vacant.insert(sender);
                    Ok(())
                }
            }
        })?;
        Ok(CardWait {
            supervisor: self.clone(),
            actor,
            tool_call_id: tool_call_id.to_owned(),
            answer,
        })
    }

    /// Ends the wait of thread `id`'s `omfp_card` call `tool_call_id` with `answer` (PRT-224): the call stops waiting
    /// before it gets the answer, so a later one is [`SupervisorError::CardNotWaiting`] (DMN-911).
    pub async fn answer_card(
        &self,
        id: &str,
        tool_call_id: &str,
        answer: CardAnswer,
    ) -> Result<()> {
        let actor = self.actor(id).await?;
        let not_waiting = || SupervisorError::CardNotWaiting(tool_call_id.to_owned());
        let sender = self
            .publish_status(&actor, || lock(&actor.cards).remove(tool_call_id))
            .ok_or_else(not_waiting)?;
        // The call dropped its wait (cancelled, timed out) while the answer was on its way.
        sender.send(answer).map_err(|_| not_waiting())
    }

    /// Runs `change` on `actor` and publishes the thread's status if it changed (DMN-911).
    fn publish_status<T>(&self, actor: &Actor, change: impl FnOnce() -> T) -> T {
        let before = actor.status();
        let changed = change();
        if actor.status() != before {
            let _ = self.inner.changes.send(actor.id.clone());
        }
        changed
    }
}
