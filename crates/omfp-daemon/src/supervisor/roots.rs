//! Root sync (DMN-029): a running omp's roots follow the repositories of its workspace through the local commands
//! `/add-dir` and `/remove-dir`, whose one `command_output` answers the daemon and stays out of the thread's log. The
//! frames carry no id, so an output is the command's only when its text is one omp 19.1.0 writes for that command
//! and path (`slash-commands/builtin-lifecycle.ts`); any other output, such as a background `/mcp reauth`'s, goes on
//! to the log meanwhile (DMN-200, DMN-202).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use omp_rpc::PromptCommand;
use tokio::sync::oneshot;
use tracing::warn;

use super::{Actor, OUTPUT_WAIT, Result, Supervisor, SupervisorError, blocking, lock};
use crate::agent::AgentProcess;
use crate::workspaces::Discovered;

/// One local command of a root sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RootCommand {
    /// `/add-dir <root>`
    Add(PathBuf),
    /// `/remove-dir <root>`
    Remove(PathBuf),
}

impl RootCommand {
    fn message(&self) -> String {
        match self {
            Self::Add(root) => format!("/add-dir {}", root.display()),
            Self::Remove(root) => format!("/remove-dir {}", root.display()),
        }
    }

    /// Whether `text` is omp's answer to this command. omp resolves the path lexically (`path.resolve`, no symlink
    /// resolution), so an absolute, normalized root comes back as given. An unexpected failure (an error persisting
    /// the session) is not recognized: it reaches the log, as a failure should.
    fn answered_by(&self, text: &str) -> bool {
        let (root, verb, done, refusals): (&Path, &str, &str, &[&str]) = match self {
            Self::Add(root) => (
                root,
                "add",
                "Added ",
                &[
                    "Already in the workspace: ",
                    "Directory does not exist: ",
                    "Not a directory: ",
                ],
            ),
            Self::Remove(root) => (root, "remove", "Removed ", &["Not a workspace directory: "]),
        };
        let Some(path) = root.to_str() else {
            return false;
        };
        let after = |prefix: &str| {
            text.strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix(path))
        };
        // `Added <path>.` / `Removed <path>.`, then the workspace directories on the next lines.
        after(done)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('\n'))
            || refusals.iter().any(|prefix| after(prefix) == Some(""))
            || text
                .strip_prefix("Cannot ")
                .and_then(|rest| rest.strip_prefix(verb))
                == Some(" a directory while streaming.")
            || (verb == "remove"
                && text == "Cannot remove the working directory; use /move to change it.")
    }
}

/// A root sync command waiting for its `command_output`.
pub(super) struct PendingRoot {
    command: RootCommand,
    written: oneshot::Sender<()>,
}

impl Supervisor {
    /// Brings the live process's roots in line with the repositories discovered in its workspace (DMN-029):
    /// `/add-dir` for new ones, `/remove-dir` for roots that disappeared, through `prompt` (local commands). omp
    /// refuses them while the agent streams, so a busy thread syncs once its session settles. A process still starting
    /// is waited for (it holds the lifecycle lock), since its flags come from a scan made before. A stopped thread
    /// needs nothing: its next launch passes the roots.
    pub async fn sync_roots(&self, id: &str) -> Result<()> {
        let Some(actor) = self.lookup(id) else {
            return Ok(());
        };
        let _lifecycle = actor.lifecycle.lock().await;
        let Some(process) = actor.process() else {
            actor.roots_pending.store(false, Ordering::SeqCst);
            return Ok(());
        };
        if !actor.status().settled {
            actor.roots_pending.store(true, Ordering::SeqCst);
            return Ok(());
        }
        actor.roots_pending.store(false, Ordering::SeqCst);
        let thread_id = actor.id.clone();
        let (desired, recorded) = self
            .stores(move |inner| {
                let thread = inner.threads.get(&thread_id)?;
                let desired = root_paths(&inner.workspaces.discovered(&thread.workspace_id)?);
                Ok::<_, SupervisorError>((desired, inner.threads.roots(&thread_id)?))
            })
            .await?;
        self.apply_roots(&actor, process, desired, &recorded).await
    }

    /// Sends `/add-dir` for `desired` roots not `recorded` and `/remove-dir` for recorded ones no longer desired,
    /// then records `desired`. Housekeeping, not the thread's doing: it is no activity of the thread and its output
    /// stays out of the transcript (DMN-029).
    pub(super) async fn apply_roots(
        &self,
        actor: &Actor,
        process: Arc<dyn AgentProcess>,
        desired: Vec<PathBuf>,
        recorded: &[PathBuf],
    ) -> Result<()> {
        let added = desired
            .iter()
            .filter(|root| !recorded.contains(root))
            .map(|root| RootCommand::Add(root.clone()));
        let removed = recorded
            .iter()
            .filter(|root| !desired.contains(root))
            .map(|root| RootCommand::Remove(root.clone()));
        let commands: Vec<RootCommand> = added.chain(removed).collect();
        if !commands.is_empty() {
            let _one_at_a_time = actor.local_commands.lock().await;
            for command in commands {
                Self::root_command(actor, &process, command).await?;
            }
        }
        let thread_id = actor.id.clone();
        self.stores(move |inner| inner.threads.set_roots(&thread_id, &desired))
            .await
    }

    /// Sends one root sync command and waits for its `command_output`, which [`Self::take_root_output`] hands here
    /// instead of the log. omp writes it before its answer, so after the answer it is only in transit through the
    /// event pump; no output recognized within [`OUTPUT_WAIT`] gives up waiting.
    pub(super) async fn root_command(
        actor: &Actor,
        process: &Arc<dyn AgentProcess>,
        command: RootCommand,
    ) -> Result<()> {
        let (written, output) = oneshot::channel();
        let message = command.message();
        *lock(&actor.root_sync) = Some(PendingRoot { command, written });
        let process = Arc::clone(process);
        let sent = blocking(move || {
            process.prompt(PromptCommand {
                message,
                images: None,
                streaming_behavior: None,
            })
        })
        .await;
        if matches!(sent, Ok(Ok(_))) {
            let _ = tokio::time::timeout(OUTPUT_WAIT, output).await;
        }
        lock(&actor.root_sync).take();
        sent??;
        Ok(())
    }

    /// Hands a `command_output` that answers the pending root sync command to it; `true` when it did, so the output
    /// stays out of the thread's log (DMN-029).
    pub(super) fn take_root_output(actor: &Actor, text: &str) -> bool {
        let Some(pending) =
            lock(&actor.root_sync).take_if(|pending| pending.command.answered_by(text))
        else {
            return false;
        };
        let _ = pending.written.send(());
        true
    }

    /// A root change made before the prompt reaches omp first (DMN-029): a sync still pending runs now if the session
    /// settled, and one in flight (it holds the lifecycle lock) finishes before the prompt is sent.
    pub(super) async fn roots_before_prompt(&self, actor: &Actor) {
        if actor.roots_pending.load(Ordering::SeqCst) && actor.status().settled {
            if let Err(error) = self.sync_roots(&actor.id).await {
                warn!(thread = %actor.id, %error, "root sync before the prompt failed");
            }
        } else {
            drop(actor.lifecycle.lock().await);
        }
    }
}

/// The `--add-dir` roots for a workspace's repositories: every one but the folder itself, by its path inside the
/// workspace (DMN-029).
pub(super) fn root_paths(repos: &[Discovered]) -> Vec<PathBuf> {
    repos
        .iter()
        .filter(|repo| !repo.is_root)
        .map(|repo| repo.path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(root: &str) -> RootCommand {
        RootCommand::Add(PathBuf::from(root))
    }

    fn remove(root: &str) -> RootCommand {
        RootCommand::Remove(PathBuf::from(root))
    }

    // spec: DMN-029
    #[test]
    fn a_root_command_is_answered_only_by_omps_outputs_for_its_path() {
        let listing = "Workspace directories:\n  /w (working directory)\n  /w/app";
        assert!(add("/w/app").answered_by(&format!("Added /w/app.\n{listing}")));
        assert!(add("/w/app").answered_by("Already in the workspace: /w/app"));
        assert!(add("/w/app").answered_by("Directory does not exist: /w/app"));
        assert!(add("/w/app").answered_by("Not a directory: /w/app"));
        assert!(add("/w/app").answered_by("Cannot add a directory while streaming."));
        assert!(remove("/w/app").answered_by(&format!("Removed /w/app.\n{listing}")));
        assert!(remove("/w/app").answered_by("Not a workspace directory: /w/app"));
        assert!(remove("/w/app").answered_by("Cannot remove a directory while streaming."));

        assert!(!add("/w/app").answered_by(&format!("Added /w/app2.\n{listing}")));
        assert!(!add("/w/app").answered_by("Added /w/app.extra"));
        assert!(!add("/w/app").answered_by(&format!("Removed /w/app.\n{listing}")));
        assert!(!add("/w/app").answered_by("Cannot remove a directory while streaming."));
        assert!(!remove("/w/app").answered_by("Already in the workspace: /w/app"));
        assert!(!add("/w/app").answered_by(
            "✓ Reauthorized \"gh\" (user config). Credentials apply on the next connection."
        ));
        assert!(!add("/w/app").answered_by(listing));
    }
}
