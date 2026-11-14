//! Automatic and explicit thread titles, and their omp session names (DMN-073).

use std::path::PathBuf;
use std::sync::Arc;

use omp_rpc::SetSessionNameCommand;
use tokio::time::Instant;
use tracing::warn;

use super::{Actor, Result, Supervisor, SupervisorError, blocking, lock};
use crate::agent::OmpCall;
use crate::store::{Thread, TitleSource};
use crate::titles::{self, TitleRequest};

impl Supervisor {
    /// Names a new thread immediately, then reconsiders only after meaningful admitted conversation.
    pub(super) async fn consider_title(&self, actor: &Arc<Actor>, message: String) {
        if message.trim().is_empty() {
            return;
        }
        // Serialize admission bookkeeping and task installation, not the model call itself.
        let mut policy = actor.title_policy.lock().await;
        let threads = Arc::clone(&self.inner.threads);
        let workspaces = Arc::clone(&self.inner.workspaces);
        let id = actor.id.clone();
        let placeholder = titles::placeholder(&message);
        let prepared = blocking(move || {
            let mut thread = threads.get(&id)?;
            if thread.title_source == TitleSource::Set {
                return Ok::<_, SupervisorError>(None);
            }
            let unnamed =
                thread.title_source == TitleSource::Placeholder && thread.title.is_empty();
            if unnamed
                && threads.replace_placeholder(&id, &placeholder, TitleSource::Placeholder)?
            {
                thread.title = placeholder;
            }
            let cwd = workspaces.get(&thread.workspace_id)?.path;
            Ok(Some((thread, cwd, unnamed)))
        })
        .await;
        let (thread, cwd, unnamed) = match prepared {
            Ok(Ok(Some(prepared))) => prepared,
            Ok(Ok(None)) => return,
            Ok(Err(error)) | Err(error) => {
                warn!(thread = %actor.id, %error, "cannot prepare the thread title");
                return;
            }
        };
        if unnamed {
            let _ = self.inner.changes.send(actor.id.clone());
        }
        if !self.inner.config.title_generation {
            return;
        }
        policy.observe(&message);
        let initial = thread.title_source == TitleSource::Placeholder;
        let running = lock(&actor.title_task)
            .as_ref()
            .is_some_and(|task| !task.is_finished());
        let now = Instant::now();
        if running || !policy.ready(initial, now) {
            return;
        }
        let context = if unnamed {
            message.chars().take(4000).collect()
        } else {
            actor
                .log
                .with_state(|state| policy.context(&thread.title, state))
        };
        policy.begin(now);
        self.spawn_title_generation(actor, cwd, context, thread);
    }

    /// Reuses the isolated print-mode launcher; publication checks the source and starting title atomically.
    fn spawn_title_generation(
        &self,
        actor: &Arc<Actor>,
        cwd: PathBuf,
        message: String,
        previous: Thread,
    ) {
        let request = TitleRequest {
            program: self.inner.config.omp_program.clone(),
            cwd,
            env: self.inner.config.omp_env.clone(),
            message,
        };
        let supervisor = self.clone();
        let actor = Arc::clone(actor);
        let task_actor = Arc::clone(&actor);
        let task = tokio::spawn(async move {
            let id = &task_actor.id;
            let launcher = Arc::clone(&supervisor.inner.launcher);
            let title = match blocking(move || launcher.generate_title(&request)).await {
                Ok(Ok(title)) => title,
                Ok(Err(error)) => {
                    task_actor.title_policy.lock().await.failed();
                    warn!(thread = %id, %error, "title generation failed; the previous title stays");
                    return;
                }
                Err(error) => {
                    task_actor.title_policy.lock().await.failed();
                    warn!(thread = %id, %error, "title generation failed; the previous title stays");
                    return;
                }
            };
            let publish = task_actor.title_policy.lock().await.publish(
                &previous.title,
                &title,
                previous.title_source == TitleSource::Placeholder,
            );
            if !publish {
                return;
            }
            let threads = Arc::clone(&supervisor.inner.threads);
            let thread_id = id.clone();
            let stored =
                blocking(move || threads.replace_automatic(&thread_id, &previous.title, &title))
                    .await;
            match stored {
                Ok(Ok(true)) => {
                    supervisor.name_session(id).await;
                    let _ = supervisor.inner.changes.send(id.clone());
                }
                Ok(Ok(false)) => {}
                Ok(Err(error)) => warn!(thread = %id, %error, "cannot store the generated title"),
                Err(error) => warn!(thread = %id, %error, "cannot store the generated title"),
            }
        });
        *lock(&actor.title_task) = Some(task);
    }

    /// Waits until the thread's title generation, if any, is over.
    #[cfg(test)]
    pub(super) async fn title_generated(&self, id: &str) {
        let task = self
            .lookup(id)
            .and_then(|actor| lock(&actor.title_task).take());
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    /// Explicit titles disable automatic refresh, including generations already in flight.
    pub async fn set_title(&self, id: &str, title: String, source: TitleSource) -> Result<Thread> {
        let threads = Arc::clone(&self.inner.threads);
        let (thread_id, stored) = (id.to_owned(), title);
        let thread = blocking(move || threads.rename(&thread_id, &stored, source)).await??;
        self.name_session(id).await;
        let _ = self.inner.changes.send(id.to_owned());
        Ok(thread)
    }

    /// Reads the latest title after serializing renames. An old generated-title task or queued host rename
    /// must never leave the session with a stale name after an explicit rename has finished.
    pub(super) async fn name_session(&self, id: &str) {
        let Some(actor) = self.lookup(id) else {
            return;
        };
        let _sync = actor.title_sync.lock().await;
        let Some(process) = actor.process() else {
            return;
        };
        let threads = Arc::clone(&self.inner.threads);
        let thread_id = id.to_owned();
        let named = blocking(move || {
            let thread = threads.get(&thread_id)?;
            if !thread.title.trim().is_empty() {
                process.call(OmpCall::SetSessionName(SetSessionNameCommand {
                    name: thread.title,
                }))?;
            }
            Ok::<_, SupervisorError>(())
        })
        .await;
        match named {
            Ok(Ok(())) => {}
            Ok(Err(error)) | Err(error) => {
                warn!(thread = %id, %error, "cannot name the omp session");
            }
        }
    }
}
