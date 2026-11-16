//! The local commands of a root sync (DMN-029): `/add-dir` and `/remove-dir`, whose one `command_output` answers the
//! daemon and stays out of the thread's log. The frames carry no id, so an output is the command's only when its text
//! is one omp 18.18.6 writes for that command and path (`slash-commands/builtin-lifecycle.ts`); any other output, such
//! as a background `/mcp reauth`'s, goes on to the log meanwhile (DMN-200, DMN-202).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use omp_rpc::PromptCommand;
use tokio::sync::oneshot;

use super::{Actor, OUTPUT_WAIT, Result, Supervisor, blocking, lock};
use crate::agent::AgentProcess;

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
        let mut pending = lock(&actor.root_sync);
        if !pending
            .as_ref()
            .is_some_and(|pending| pending.command.answered_by(text))
        {
            return false;
        }
        if let Some(pending) = pending.take() {
            let _ = pending.written.send(());
        }
        true
    }
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
