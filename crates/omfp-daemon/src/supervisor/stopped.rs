//! What a thread answers without starting its omp (DMN-070): showing or keeping a thread never starts it, only what
//! the user sends does. Its `/` commands (DMN-358) and its history (DMN-359) come from what omp last gave it.

use std::path::Path;

use omfp_core::ThreadEvent;
use omp_rpc::{AvailableSlashCommand, GetAvailableCommandsResult};
use serde_json::Value;
use tracing::warn;

use super::{Result, Supervisor, SupervisorError, blocking};
use crate::agent::OmpCall;
use crate::paths::replace_file;

impl Supervisor {
    /// `thread.commands` (DMN-358): the running omp's own list; without a running omp, the list it last announced
    /// (`available_commands_update`), from the thread's state or, after a daemon restart, from the copy kept with the
    /// thread; else none. It never starts omp and is no activity of the thread.
    pub async fn commands(&self, id: &str) -> Result<Value> {
        let actor = self.actor(id).await?;
        if let Some(process) = actor.process() {
            return Ok(blocking(move || process.call(OmpCall::Commands)).await??);
        }
        let mut commands = actor.log.with_state(|state| state.commands.clone());
        if commands.is_empty() {
            let path = self.inner.data.thread_commands(id);
            commands = blocking(move || kept_commands(&path)).await?;
        }
        serde_json::to_value(GetAvailableCommandsResult { commands })
            .map_err(|error| SupervisorError::Internal(error.to_string()))
    }

    /// Keeps the commands the thread's omp announced with the thread (DMN-358), so `thread.commands` knows them after
    /// a daemon restart; a failed write is only logged.
    pub(super) async fn keep_commands(&self, id: &str, commands: &[AvailableSlashCommand]) {
        let path = self.inner.data.thread_commands(id);
        let written = match serde_json::to_vec(commands) {
            Ok(bytes) => blocking(move || replace_file(&path, &bytes, None))
                .await
                .and_then(|written| {
                    written.map_err(|error| SupervisorError::Internal(error.to_string()))
                }),
            Err(error) => Err(SupervisorError::Internal(error.to_string())),
        };
        if let Err(error) = written {
            warn!(thread = %id, %error, "cannot keep the thread's commands");
        }
    }

    /// A shown thread's conversation without omp (DMN-359): a thread whose log has no session since the daemon
    /// started (its omp never ran, no start failed) reads its stored session's current branch from the file, as a
    /// failed start does (DMN-337), and records it as its `history`. A start or stop under way gives the thread its
    /// history itself, so nothing is read then.
    pub async fn read_history(&self, id: &str) -> Result<()> {
        let actor = self.actor(id).await?;
        let Ok(_lifecycle) = actor.lifecycle.try_lock() else {
            return Ok(());
        };
        if actor.process().is_some() || actor.log.with_state(|state| state.session.is_some()) {
            return Ok(());
        }
        let thread = self.thread(id).await?;
        if let Some(read) = self.read_session_file(&actor, &thread).await {
            self.push(
                &actor,
                ThreadEvent::History {
                    session: read.session,
                    messages: read.messages,
                },
            );
        }
        Ok(())
    }
}

/// The commands kept with a thread (DMN-358); none when there is no copy, an unreadable one only logged.
fn kept_commands(path: &Path) -> Vec<AvailableSlashCommand> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            warn!(path = %path.display(), %error, "cannot read the thread's commands");
            return Vec::new();
        }
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        warn!(path = %path.display(), %error, "cannot read the thread's commands");
        Vec::new()
    })
}
