//! The MCP servers of a workspace and a thread (M13): the configured list (DMN-190), the check of each server by the
//! thread's running omp with `/mcp test` (DMN-191), and the reconnect, a restart of the idle process (DMN-192).
//!
//! omp loads MCP servers only at startup, so the daemon restarts a thread's omp by itself (`restart.rs`) when the
//! servers it would load change: the configs' [`omp_mcp::fingerprint`] changed (DMN-201) or omp said a server needs a
//! new connection (`✓ Reauthorized`, `Added MCP server`, DMN-202).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use omfp_protocol::{McpServer, McpServers, RestartReason};
use omp_rpc::PromptCommand;
use tokio::sync::oneshot;
use tracing::{info, warn};

use super::restart::Restart;
use super::{Actor, OUTPUT_WAIT, Result, Supervisor, SupervisorError, blocking, lock};
use crate::omp_mcp::{self, Check, Homes};

/// The daemon's `/mcp test` of server `name` waiting for its `command_output` (DMN-200).
pub(super) struct PendingCheck {
    name: String,
    answer: oneshot::Sender<Check>,
}

impl Supervisor {
    fn homes(&self) -> Homes {
        Homes {
            home: self.inner.config.home.clone(),
            agent_dir: self.inner.config.agent_dir.clone(),
        }
    }

    /// The servers configured for workspace `workspace_id`, unchecked (PRT-110, DMN-190).
    pub async fn workspace_mcp_servers(&self, workspace_id: &str) -> Result<McpServers> {
        let id = workspace_id.to_owned();
        let path = self
            .stores(move |inner| inner.workspaces.get(&id))
            .await?
            .path;
        let servers = omp_mcp::configured(&path, &self.homes());
        Ok(self.listing(workspace_id, None, false, &path, servers))
    }

    /// The servers of the thread's workspace, each checkable one checked by the thread's omp when it runs (PRT-111,
    /// DMN-191). A stopped thread is not started: its servers run only with it.
    pub async fn thread_mcp_servers(&self, thread_id: &str) -> Result<McpServers> {
        self.thread_servers(thread_id, None).await
    }

    /// Reconnects the thread's MCP servers: omp 18.22.0 reconnects none over RPC (`/mcp reconnect` is TUI-only,
    /// `/mcp reload` does not reconnect), so the idle process restarts and connects them all again; then `name`, or
    /// every checkable server, is checked (PRT-112, DMN-192). A working thread is refused: a restart would abort it.
    pub async fn mcp_reconnect(&self, thread_id: &str, name: Option<&str>) -> Result<McpServers> {
        self.restart(thread_id, Restart::Reconnect).await?;
        self.thread_servers(thread_id, name).await
    }

    /// The thread's servers; with `only`, the others are listed unchecked.
    async fn thread_servers(&self, thread_id: &str, only: Option<&str>) -> Result<McpServers> {
        let id = thread_id.to_owned();
        let (workspace_id, path) = self
            .stores(move |inner| {
                let thread = inner.threads.get(&id)?;
                let workspace = inner.workspaces.get(&thread.workspace_id)?;
                Ok::<_, SupervisorError>((thread.workspace_id, workspace.path))
            })
            .await?;
        let configured = omp_mcp::configured(&path, &self.homes());
        let running = self.is_running(thread_id);
        let mut servers = Vec::with_capacity(configured.len());
        for server in configured {
            let check =
                if running && server.checkable && only.is_none_or(|name| name == server.name) {
                    self.mcp_check(thread_id, &server.name).await?
                } else {
                    None
                };
            servers.push(omp_mcp::checked(server, check));
        }
        Ok(self.listing(&workspace_id, Some(thread_id), running, &path, servers))
    }

    fn listing(
        &self,
        workspace_id: &str,
        thread_id: Option<&str>,
        running: bool,
        path: &Path,
        servers: Vec<McpServer>,
    ) -> McpServers {
        McpServers {
            workspace_id: workspace_id.to_owned(),
            thread_id: thread_id.map(str::to_owned),
            running,
            project_config: omp_mcp::project_config(path).to_string_lossy().into_owned(),
            user_config: omp_mcp::user_config(&self.homes())
                .map(|path| path.to_string_lossy().into_owned()),
            servers,
        }
    }

    /// Checks server `name` with the thread's running omp: `/mcp test <name>` runs as a local command (no model turn)
    /// that connects anew, lists the tools and disconnects, and answers with one `command_output` frame (DMN-191).
    /// `None` when the thread does not run or omp sent no answer for this server. Checks of one thread run one at a
    /// time so their outputs cannot cross.
    async fn mcp_check(&self, thread_id: &str, name: &str) -> Result<Option<Check>> {
        let Some(actor) = self.lookup(thread_id) else {
            return Ok(None);
        };
        let _one_at_a_time = actor.local_commands.lock().await;
        let Some(process) = actor.process() else {
            return Ok(None);
        };
        let (answer, answered) = oneshot::channel();
        *lock(&actor.mcp_check) = Some(PendingCheck {
            name: name.to_owned(),
            answer,
        });
        actor.touch();
        let command = PromptCommand {
            message: format!("/mcp test {name}"),
            images: None,
            streaming_behavior: None,
        };
        let check = match blocking(move || process.prompt(command)).await {
            Ok(Ok(_)) => Ok(tokio::time::timeout(OUTPUT_WAIT, answered)
                .await
                .ok()
                .and_then(std::result::Result::ok)),
            Ok(Err(error)) => {
                warn!(thread = %thread_id, server = %name, %error, "MCP check failed");
                Ok(Some(Check::Failed(format!(
                    "omp did not answer the check: {error}"
                ))))
            }
            Err(error) => Err(error),
        };
        lock(&actor.mcp_check).take();
        check
    }

    /// Hands a `command_output` that answers the daemon's pending `/mcp test` to it; `true` when it did, so the
    /// output stays out of the thread's transcript (DMN-200).
    pub(super) fn take_check_output(actor: &Actor, text: &str) -> bool {
        let mut pending = lock(&actor.mcp_check);
        let Some(check) = pending
            .as_ref()
            .and_then(|pending| omp_mcp::parse_check(&pending.name, text))
        else {
            return false;
        };
        if let Some(pending) = pending.take() {
            let _ = pending.answer.send(check);
        }
        true
    }

    /// Records the configs the thread's omp is about to read (DMN-201).
    pub(super) async fn loading_mcp_configs(&self, actor: &Actor, workspace: PathBuf) {
        let homes = self.homes();
        let folder = workspace.clone();
        match blocking(move || omp_mcp::fingerprint(&folder, &homes)).await {
            Ok(fingerprint) => *lock(&actor.mcp_configs) = Some((workspace, fingerprint)),
            Err(error) => warn!(thread = %actor.id, %error, "cannot read the MCP configs"),
        }
    }

    /// Compares the MCP configs of every running thread with the ones its omp read; a change makes a restart due
    /// (DMN-201, DMN-228). The workspace polls call it every round.
    pub async fn check_mcp_configs(&self) {
        let actors: Vec<Arc<Actor>> = lock(&self.inner.actors).values().cloned().collect();
        for actor in actors {
            let Some((workspace, _)) = lock(&actor.mcp_configs).clone() else {
                continue;
            };
            if actor.process().is_none() {
                continue;
            }
            let homes = self.homes();
            let Ok(now) = blocking(move || omp_mcp::fingerprint(&workspace, &homes)).await else {
                continue;
            };
            let changed = match &mut *lock(&actor.mcp_configs) {
                Some((_, fingerprint)) if *fingerprint != now => {
                    *fingerprint = now;
                    true
                }
                _ => false,
            };
            if changed {
                info!(thread = %actor.id, "MCP configs changed");
                self.restart_for(&actor, RestartReason::McpConfig);
            }
        }
    }
}
