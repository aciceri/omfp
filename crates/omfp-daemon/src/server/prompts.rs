//! The prompt history (DMN-216..DMN-218): the prompts the user sends from omfp, recorded once omp admitted them, and
//! `prompts.history` (PRT-127). omp's own history (its TUI's `history.db`) does not see RPC prompts.

use std::sync::Arc;

use omfp_protocol::{CreatedBy, ErrorBody, PromptHistory, PromptHistoryEntry, PromptHistoryParams};
use tracing::warn;

use super::auth::Principal;
use super::convert::internal;
use super::services::Services;
use crate::store::{PromptQuery, SentPrompt, now_ms};

/// The newest prompts the history keeps (DMN-216).
pub(crate) const KEPT: u32 = 1000;
/// `prompts.history`'s default and largest `limit` (PRT-127).
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = KEPT;

/// Records `text`, sent to `thread_id` by `principal`, when the user sent it (DMN-218): only the user's principal
/// (agents, MCP clients and the daemon's own prompts are not the user typing) and a text that is not blank. The
/// thread's workspace is read when not given. A failure is only a warning.
pub(crate) async fn record(
    services: &Services,
    principal: &Principal,
    thread_id: &str,
    workspace_id: Option<String>,
    text: &str,
) {
    if principal.created_by() != CreatedBy::User || text.trim().is_empty() {
        return;
    }
    let store = Arc::clone(&services.store);
    let threads = Arc::clone(&services.threads);
    let thread_id = thread_id.to_owned();
    let text = text.to_owned();
    let recorded = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let workspace_id = match workspace_id {
            Some(id) => id,
            None => threads.get(&thread_id)?.workspace_id,
        };
        let prompt = SentPrompt {
            text,
            thread_id,
            workspace_id,
            at: now_ms(),
        };
        store.record_prompt(&prompt, KEPT)?;
        Ok(())
    })
    .await;
    match recorded {
        Ok(Ok(())) => {}
        Ok(Err(error)) => warn!(%error, "cannot record a prompt in the history"),
        Err(error) => warn!(%error, "cannot record a prompt in the history"),
    }
}

/// `prompts.history` (PRT-127).
pub(crate) async fn history(
    services: &Services,
    params: PromptHistoryParams,
) -> Result<PromptHistory, ErrorBody> {
    let query = PromptQuery {
        thread_id: params.thread_id,
        workspace_id: params.workspace_id,
        query: params.query,
        limit: params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT),
    };
    let store = Arc::clone(&services.store);
    let prompts = tokio::task::spawn_blocking(move || store.prompts(&query))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    Ok(PromptHistory {
        prompts: prompts
            .into_iter()
            .map(|prompt| PromptHistoryEntry {
                text: prompt.text,
                thread_id: prompt.thread_id,
                workspace_id: prompt.workspace_id,
                at: prompt.at,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use omfp_protocol::{Call, MessageParams, PromptBehavior, PromptParams};
    use omp_rpc::PromptCommand;
    use tempfile::TempDir;

    use super::*;
    use crate::agent::FakeLauncher;
    use crate::git::{FakeGit, Git};
    use crate::paths::DataDir;
    use crate::server::dispatch::dispatch;
    use crate::store::Store;
    use crate::supervisor::{Supervisor, SupervisorConfig};
    use crate::threads::{NewThread, Threads};
    use crate::widgets::Widgets;
    use crate::workspaces::{NewWorkspace, Workspaces};

    async fn services(dir: &TempDir) -> Services {
        let data = DataDir::new(dir.path().join("data"));
        std::fs::create_dir_all(data.root()).unwrap();
        let root = dir.path().join("workspaces");
        let store = Arc::new(Store::open(&data.db()).unwrap());
        let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
        let workspaces = Arc::new(Workspaces::new(
            Arc::clone(&store),
            Arc::new(FakeGit::new()) as Arc<dyn Git>,
            root,
        ));
        let widgets = Arc::new(Widgets::new(None));
        let supervisor = Supervisor::new(
            Arc::new(FakeLauncher::new()),
            Arc::clone(&threads),
            Arc::clone(&workspaces),
            data,
            SupervisorConfig::new("omp"),
        );
        Services::new(
            store,
            threads,
            workspaces,
            supervisor,
            widgets,
            Arc::new(crate::jobs::declared::SystemClock),
        )
        .await
        .unwrap()
    }

    fn prompt(thread_id: &str, text: &str) -> Call {
        Call::ThreadPrompt(PromptParams {
            thread_id: thread_id.to_owned(),
            text: text.to_owned(),
            images: Vec::new(),
            behavior: None,
        })
    }

    async fn texts(services: &Services, params: PromptHistoryParams) -> Vec<String> {
        history(services, params)
            .await
            .unwrap()
            .prompts
            .into_iter()
            .map(|prompt| prompt.text)
            .collect()
    }

    // spec: DMN-218, PRT-127, PRT-128
    #[tokio::test(flavor = "multi_thread")]
    async fn only_the_users_prompts_are_recorded_not_agents_nor_the_daemons_own() {
        let dir = tempfile::tempdir().unwrap();
        let services = services(&dir).await;
        let ws = services
            .workspaces
            .create(NewWorkspace::new())
            .unwrap()
            .workspace
            .id;
        let thread = services
            .threads
            .create(NewThread::new("t", &ws))
            .unwrap()
            .id;
        let owner = Principal::Owner {
            client: "omfp/web".into(),
        };
        let agent = Principal::Agent {
            thread_id: thread.clone(),
            client: "omp".into(),
        };

        dispatch(&services, &owner, prompt(&thread, "fix the build"))
            .await
            .unwrap();
        dispatch(&services, &agent, prompt(&thread, "agent says"))
            .await
            .unwrap();
        dispatch(&services, &owner, prompt(&thread, "   "))
            .await
            .unwrap();
        // What the daemon sends itself (`/mcp test`, `/add-dir`) goes to the supervisor, not through `thread.prompt`.
        services
            .supervisor
            .prompt(
                &thread,
                PromptCommand {
                    message: "/mcp test docs".into(),
                    images: None,
                    streaming_behavior: None,
                },
            )
            .await
            .unwrap();

        let entries = history(&services, PromptHistoryParams::default())
            .await
            .unwrap()
            .prompts;
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (
                entries[0].text.as_str(),
                entries[0].thread_id.as_str(),
                entries[0].workspace_id.as_str()
            ),
            ("fix the build", thread.as_str(), ws.as_str())
        );
        assert_eq!(
            texts(
                &services,
                PromptHistoryParams {
                    query: Some("BUILD".into()),
                    ..PromptHistoryParams::default()
                }
            )
            .await,
            ["fix the build"]
        );
        assert_eq!(
            texts(
                &services,
                PromptHistoryParams {
                    query: Some("nothing".into()),
                    ..PromptHistoryParams::default()
                }
            )
            .await,
            Vec::<String>::new()
        );
    }

    // spec: DMN-218, DMN-226
    #[tokio::test(flavor = "multi_thread")]
    async fn steered_and_queued_messages_enter_the_history_like_prompts() {
        let dir = tempfile::tempdir().unwrap();
        let services = services(&dir).await;
        let ws = services
            .workspaces
            .create(NewWorkspace::new())
            .unwrap()
            .workspace
            .id;
        let thread = services
            .threads
            .create(NewThread::new("t", &ws))
            .unwrap()
            .id;
        let owner = Principal::Owner {
            client: "omfp/web".into(),
        };
        let message = |text: &str| MessageParams {
            thread_id: thread.clone(),
            text: text.to_owned(),
            images: Vec::new(),
        };

        dispatch(&services, &owner, Call::ThreadSteer(message("steer it")))
            .await
            .unwrap();
        dispatch(
            &services,
            &owner,
            Call::ThreadFollowUp(message("then this")),
        )
        .await
        .unwrap();
        dispatch(
            &services,
            &owner,
            // The composer's steer and queue (UI-061): `thread.prompt` with a behavior.
            Call::ThreadPrompt(PromptParams {
                thread_id: thread.clone(),
                text: "the composer's steer".into(),
                images: Vec::new(),
                behavior: Some(PromptBehavior::Steer),
            }),
        )
        .await
        .unwrap();

        assert_eq!(
            texts(&services, PromptHistoryParams::default()).await,
            ["the composer's steer", "then this", "steer it"]
        );
    }
}
