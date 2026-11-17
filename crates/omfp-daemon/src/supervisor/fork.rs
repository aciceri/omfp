//! A forked thread's first start (DMN-301, DMN-302). omp copies the source thread's session file into the new
//! thread's session directory (`--fork <file>`: a new session id, the source as its parent, its artifacts copied);
//! for a fork at an entry the new omp then moves onto the branch up to that entry (`fork { entryId }`: a new session
//! file of the same directory, extended through the tool results answering the entry) and the full copy is deleted.
//! The source thread's omp gets no command and its session files are only read.

use std::path::PathBuf;

use omp_rpc::ForkCommand;
use serde_json::Value;
use tracing::warn;

use super::{Result, Supervisor, SupervisorError, blocking, lock};
use crate::agent::OmpCall;
use crate::threads::session_file;

impl Supervisor {
    /// Starts thread `id`, which has no session yet, on a copy of session file `source`, cut after `entry_id` when
    /// given (DMN-301, DMN-302).
    pub async fn start_fork(
        &self,
        id: &str,
        source: PathBuf,
        entry_id: Option<String>,
    ) -> Result<()> {
        let actor = self.actor(id).await?;
        *lock(&actor.fork_source) = Some(source);
        self.ensure_running(&actor).await?;
        let Some(entry_id) = entry_id else {
            return Ok(());
        };
        let copy = self.thread(id).await?;
        let answer = self
            .call(
                id,
                OmpCall::Fork(ForkCommand {
                    entry_id: Some(entry_id),
                }),
            )
            .await?;
        if answer.get("cancelled").and_then(Value::as_bool) == Some(true) {
            return Err(SupervisorError::Internal(
                "omp cancelled the fork at the entry".into(),
            ));
        }
        // The full copy goes on past the entry: the thread's search must not find that text (DMN-302).
        let Some(copied) = copy.omp_session_id else {
            return Ok(());
        };
        let removed = blocking(move || match session_file(&copy.session_dir, &copied) {
            Some(file) => std::fs::remove_file(file),
            None => Ok(()),
        })
        .await?;
        if let Err(error) = removed {
            warn!(thread = %id, %error, "cannot delete the full copy of the forked session");
        }
        Ok(())
    }
}
