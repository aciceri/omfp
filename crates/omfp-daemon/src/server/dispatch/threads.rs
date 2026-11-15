//! `threads.*` and `thread.*`: creating and listing threads, and the methods acting on a thread's agent.

use std::sync::Arc;

use omfp_core::wire::{ImageContent, ThinkingLevel};
use omfp_protocol::{
    Call, ErrorBody, LiveMuted, PromptBehavior, PromptResult, ThreadGit, ThreadWorkspace,
    ThreadsStopped,
};
use omp_rpc::{
    BashCommand, BtwCommand, CancelSubagentCommand, CompactCommand, FollowUpCommand, ForkCommand,
    GetSubagentMessagesCommand, GoalCommand, NavigateTreeCommand, PromoteQueuedMessageCommand,
    PromptCommand, QueuedMessageQueue, RemoveQueuedMessageCommand, SetLabelCommand,
    SetModelCommand, SetThinkingLevelCommand, SteerCommand, SteerSubagentCommand,
    StreamingBehavior,
};
use serde_json::Value;

use super::workspaces::create_workspace;
use super::{Reply, empty, ok};
use crate::agent::OmpCall;
use crate::server::auth::Principal;
use crate::server::convert::{internal, supervisor_error, thread_error, workspace_error};
use crate::server::prompts;
use crate::server::services::{Services, blocking};
use crate::store::TitleSource;
use crate::supervisor::{QueuedMessage, Restart};
use crate::threads::NewThread;
use crate::titles;

fn images(images: Vec<ImageContent>) -> Option<Vec<ImageContent>> {
    (!images.is_empty()).then_some(images)
}

/// omp's name of a thinking level (`high`, …), as stored on the thread.
fn thinking_name(level: ThinkingLevel) -> Result<String, ErrorBody> {
    match serde_json::to_value(level).map_err(internal)? {
        Value::String(name) => Ok(name),
        other => Err(internal(format!("unexpected thinking level {other}"))),
    }
}

/// `threads.create`: the thread in an existing workspace or a new one named after its title (else its first message),
/// then its first message. Without a title the thread's title comes from the conversation (DMN-073).
async fn create_thread(
    services: &Services,
    principal: &Principal,
    params: omfp_protocol::CreateThreadParams,
) -> Reply {
    let client = principal.client().to_owned();
    let title = params
        .title
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty());
    let (workspace_id, created) = match params.workspace {
        ThreadWorkspace::Id(id) => (id, false),
        ThreadWorkspace::New(new) => {
            let fallback = title.clone().or_else(|| {
                let words = titles::placeholder(params.prompt.as_deref()?);
                Some(words.trim_end_matches('…').to_owned())
            });
            let id = create_workspace(
                services,
                new,
                fallback,
                principal.created_by(),
                client.clone(),
            )
            .await?;
            (id, true)
        }
    };
    let new = NewThread {
        title_source: if title.is_some() {
            TitleSource::Set
        } else {
            TitleSource::Placeholder
        },
        title: title.unwrap_or_default(),
        workspace_id: workspace_id.clone(),
        model: params.model,
        thinking_level: params.thinking_level.map(thinking_name).transpose()?,
        approval_mode: params.approval_mode.map(|mode| mode.as_str().to_owned()),
        created_by: principal.created_by(),
    };
    let threads = Arc::clone(&services.threads);
    let store = Arc::clone(&services.store);
    let created_thread = blocking(move || {
        let thread = threads.create(new).map_err(thread_error)?;
        store.set_created_client(&thread.id, &client)?;
        Ok(thread)
    })
    .await;
    let thread = match created_thread {
        Ok(thread) => thread,
        Err(error) => {
            if created {
                remove_new_workspace(services, workspace_id).await;
            }
            return Err(error);
        }
    };
    if let Some(text) = params.prompt {
        let command = PromptCommand {
            message: text.clone(),
            images: None,
            streaming_behavior: None,
        };
        if let Err(error) = services.supervisor.prompt(&thread.id, command).await {
            if error.is_start_failure() {
                // omp never ran for the thread: the call leaves nothing behind (DMN-211).
                discard_thread(services, &thread.id).await;
                if created {
                    remove_new_workspace(services, workspace_id).await;
                } else {
                    services.refresh_index_quietly().await;
                }
                return Err(supervisor_error(error));
            }
            services.refresh_index_quietly().await;
            // The thread exists; say which one so the client can retry the prompt.
            let mut body = supervisor_error(error);
            let mut details = match body.details.take() {
                Some(Value::Object(details)) => details,
                _ => serde_json::Map::new(),
            };
            details.insert("threadId".to_owned(), Value::String(thread.id));
            return Err(body.with_details(Value::Object(details)));
        }
        prompts::record(
            services,
            principal,
            &thread.id,
            Some(thread.workspace_id.clone()),
            &text,
        )
        .await;
    }
    ok(&services.thread_summary(&thread.id).await?)
}

/// Best effort: removes a thread `threads.create` or `threads.fork` made and whose omp could not start (DMN-211,
/// DMN-303).
pub(super) async fn discard_thread(services: &Services, id: &str) {
    if let Err(error) = services.supervisor.forget(id).await {
        tracing::warn!(thread = %id, %error, "cannot stop the thread that failed to start");
    }
    let threads = Arc::clone(&services.threads);
    let id = id.to_owned();
    if let Err(error) = blocking(move || threads.delete(&id).map_err(thread_error)).await {
        tracing::warn!(message = %error.message, "cannot delete the thread that failed to start");
    }
}

/// Best effort: removes the workspace `threads.create` made for a thread it could not create; the new folder had no
/// other use.
async fn remove_new_workspace(services: &Services, workspace_id: String) {
    let workspaces = Arc::clone(&services.workspaces);
    let _ = blocking(move || {
        workspaces
            .remove(&workspace_id, false)
            .map_err(workspace_error)
    })
    .await;
    services.refresh_index_quietly().await;
}

pub(super) async fn threads(services: &Services, principal: &Principal, call: Call) -> Reply {
    let threads = Arc::clone(&services.threads);
    let supervisor = &services.supervisor;
    match call {
        Call::ThreadsCreate(params) => create_thread(services, principal, params).await,
        Call::ThreadsUpdate(params) => {
            let id = params.thread_id.clone();
            if let Some(title) = params.title {
                supervisor
                    .set_title(&id, title.trim().to_owned(), TitleSource::Set)
                    .await
                    .map_err(supervisor_error)?;
            }
            let approval_mode = params.approval_mode;
            let thread_id = id.clone();
            blocking(move || {
                if let Some(mode) = approval_mode {
                    threads
                        .set_approval_mode(&thread_id, mode.as_str())
                        .map_err(thread_error)?;
                }
                threads.get(&thread_id).map_err(thread_error)
            })
            .await?;
            ok(&services.thread_summary(&id).await?)
        }
        Call::ThreadsArchive(params) => {
            supervisor
                .stop(&params.thread_id)
                .await
                .map_err(supervisor_error)?;
            blocking(move || threads.archive(&params.thread_id).map_err(thread_error)).await?;
            services.refresh_index_quietly().await;
            empty()
        }
        Call::ThreadsUnarchive(params) => {
            blocking(move || threads.unarchive(&params.thread_id).map_err(thread_error)).await?;
            services.refresh_index_quietly().await;
            empty()
        }
        Call::ThreadsDelete(params) => {
            let id = params.thread_id.clone();
            let reader = Arc::clone(&threads);
            blocking(move || reader.get(&id).map_err(thread_error)).await?;
            supervisor
                .forget(&params.thread_id)
                .await
                .map_err(supervisor_error)?;
            blocking(move || threads.delete(&params.thread_id).map_err(thread_error)).await?;
            services.refresh_index_quietly().await;
            empty()
        }
        Call::ThreadGit(params) => {
            let id = params.thread_id.clone();
            let thread = blocking(move || threads.get(&id).map_err(thread_error)).await?;
            services.rescan(&thread.workspace_id).await?;
            let repos = services.workspace_status(&thread.workspace_id).await?;
            ok(&ThreadGit {
                workspace_id: thread.workspace_id,
                repos,
            })
        }
        _ => Err(internal("not a threads method")),
    }
}

/// `thread.steer` and `thread.follow_up`: the prompt's bookkeeping (DMN-226), and the text enters the prompt history
/// once omp admitted it, as a prompt's does (DMN-218).
async fn queue(
    services: &Services,
    principal: &Principal,
    thread_id: &str,
    text: &str,
    message: QueuedMessage,
) -> Reply {
    services
        .supervisor
        .queue(thread_id, message)
        .await
        .map_err(supervisor_error)?;
    prompts::record(services, principal, thread_id, None, text).await;
    empty()
}

/// Methods acting on a thread's agent; every omp command starts omp when needed (DMN-070).
#[allow(clippy::too_many_lines)] // One arm per method.
pub(super) async fn thread(services: &Services, principal: &Principal, call: Call) -> Reply {
    let supervisor = &services.supervisor;
    let forward = |id: String, call: OmpCall| async move {
        supervisor.call(&id, call).await.map_err(supervisor_error)
    };
    match call {
        Call::ThreadPrompt(params) => {
            let behavior = match params.behavior {
                Some(behavior) => Some(behavior),
                // omp needs a behavior while the agent works (02-daemon, "Commands towards omp").
                None if !supervisor.status(&params.thread_id).settled => {
                    Some(PromptBehavior::FollowUp)
                }
                None => None,
            };
            let text = params.text;
            let command = PromptCommand {
                message: text.clone(),
                images: images(params.images),
                streaming_behavior: behavior.map(|behavior| match behavior {
                    PromptBehavior::Steer => StreamingBehavior::Steer,
                    PromptBehavior::FollowUp => StreamingBehavior::FollowUp,
                }),
            };
            let admission = supervisor
                .prompt(&params.thread_id, command)
                .await
                .map_err(supervisor_error)?;
            prompts::record(services, principal, &params.thread_id, None, &text).await;
            ok(&PromptResult {
                prompt_id: admission.prompt_id,
                agent_invoked: admission.agent_invoked,
            })
        }
        Call::ThreadSteer(params) => {
            let message = QueuedMessage::Steer(SteerCommand {
                message: params.text.clone(),
                images: images(params.images),
            });
            queue(
                services,
                principal,
                &params.thread_id,
                &params.text,
                message,
            )
            .await
        }
        Call::ThreadFollowUp(params) => {
            let message = QueuedMessage::FollowUp(FollowUpCommand {
                message: params.text.clone(),
                images: images(params.images),
            });
            queue(
                services,
                principal,
                &params.thread_id,
                &params.text,
                message,
            )
            .await
        }
        Call::ThreadAbort(params) => forward(params.thread_id, OmpCall::AbortAndRestoreQueue).await,
        Call::ThreadQueueRemove(params) => {
            let command = RemoveQueuedMessageCommand {
                message: params.message,
                queue: params.queue.unwrap_or(QueuedMessageQueue::FollowUp),
            };
            forward(params.thread_id, OmpCall::RemoveQueued(command)).await
        }
        Call::ThreadQueuePromote(params) => {
            let command = PromoteQueuedMessageCommand {
                message: params.message,
            };
            forward(params.thread_id, OmpCall::PromoteQueued(command)).await
        }
        Call::ThreadSetModel(params) => {
            let model = format!("{}/{}", params.provider, params.model_id);
            let command = SetModelCommand {
                provider: params.provider,
                model_id: params.model_id,
            };
            forward(params.thread_id.clone(), OmpCall::SetModel(command)).await?;
            let threads = Arc::clone(&services.threads);
            blocking(move || {
                threads
                    .set_model(&params.thread_id, Some(&model))
                    .map_err(thread_error)
            })
            .await?;
            empty()
        }
        Call::ThreadSetThinking(params) => {
            let name = thinking_name(params.level)?;
            let command = SetThinkingLevelCommand {
                level: params.level,
            };
            forward(params.thread_id.clone(), OmpCall::SetThinkingLevel(command)).await?;
            let threads = Arc::clone(&services.threads);
            blocking(move || {
                threads
                    .set_thinking_level(&params.thread_id, Some(&name))
                    .map_err(thread_error)
            })
            .await?;
            empty()
        }
        Call::ThreadModels(params) => forward(params.thread_id, OmpCall::AvailableModels).await,
        Call::ThreadThinkingLevels(params) => {
            forward(params.thread_id, OmpCall::ThinkingLevels).await
        }
        Call::ThreadCommands(params) => forward(params.thread_id, OmpCall::Commands).await,
        Call::ThreadCompact(params) => {
            let command = CompactCommand {
                custom_instructions: params.instructions,
            };
            forward(params.thread_id, OmpCall::Compact(command)).await
        }
        Call::ThreadTree(params) => forward(params.thread_id, OmpCall::Tree).await,
        Call::ThreadNavigate(params) => {
            let command = NavigateTreeCommand {
                entry_id: params.entry_id,
                summarize: params.summarize,
                custom_instructions: params.instructions,
            };
            forward(params.thread_id, OmpCall::NavigateTree(command)).await
        }
        Call::ThreadFork(params) => {
            let command = ForkCommand {
                entry_id: params.entry_id,
            };
            forward(params.thread_id, OmpCall::Fork(command)).await
        }
        Call::ThreadLabel(params) => {
            let command = SetLabelCommand {
                entry_id: params.entry_id,
                label: params.label,
            };
            forward(params.thread_id, OmpCall::SetLabel(command)).await?;
            empty()
        }
        Call::ThreadSubagents(params) => forward(params.thread_id, OmpCall::Subagents).await,
        Call::ThreadSubagentMessages(params) => {
            let command = GetSubagentMessagesCommand {
                subagent_id: Some(params.subagent_id),
                session_file: None,
                from_byte: params.from_byte,
            };
            forward(params.thread_id, OmpCall::SubagentMessages(command)).await
        }
        Call::ThreadSubagentSteer(params) => {
            let command = SteerSubagentCommand {
                subagent_id: params.subagent_id,
                message: params.text,
            };
            forward(params.thread_id, OmpCall::SteerSubagent(command)).await?;
            empty()
        }
        Call::ThreadSubagentCancel(params) => {
            let command = CancelSubagentCommand {
                subagent_id: params.subagent_id,
            };
            forward(params.thread_id, OmpCall::CancelSubagent(command)).await?;
            empty()
        }
        Call::ThreadGoal(params) => {
            let command = GoalCommand {
                op: params.op,
                objective: params.objective,
                token_budget: params.token_budget,
            };
            forward(params.thread_id, OmpCall::Goal(command)).await
        }
        Call::ThreadBtw(params) => {
            let command = BtwCommand {
                question: params.question,
                record_id: params.record_id,
            };
            forward(params.thread_id, OmpCall::Btw(command)).await
        }
        Call::ThreadBash(params) => {
            let command = BashCommand {
                command: params.command,
                exclude_from_context: params.exclude_from_context,
            };
            forward(params.thread_id, OmpCall::Bash(command)).await
        }
        Call::ThreadState(params) => forward(params.thread_id, OmpCall::State).await,
        Call::ThreadSession(params) => {
            ok(&super::session::thread_session(services, &params.thread_id).await?)
        }
        Call::ThreadRestart(params) => {
            supervisor
                .restart(&params.thread_id, Restart::User)
                .await
                .map_err(supervisor_error)?;
            empty()
        }
        Call::ThreadMcpServers(params) => ok(&supervisor
            .thread_mcp_servers(&params.thread_id)
            .await
            .map_err(supervisor_error)?),
        Call::ThreadMcpReconnect(params) => ok(&supervisor
            .mcp_reconnect(&params.thread_id, params.name.as_deref())
            .await
            .map_err(supervisor_error)?),
        Call::ThreadStop(params) => {
            // The cascade also kills jobs and PTYs started by those threads' agents (AGT-071, DMN-162, TRM-104).
            let thread_ids = if params.cascade {
                let stopped = supervisor.stop_tree(&params.thread_id).await;
                let stopped = stopped.map_err(supervisor_error)?;
                services.jobs.kill_started_by(&stopped);
                services.terminals.kill_started_by(&stopped);
                stopped
            } else {
                supervisor
                    .stop(&params.thread_id)
                    .await
                    .map_err(supervisor_error)?;
                vec![params.thread_id]
            };
            ok(&ThreadsStopped { thread_ids })
        }
        Call::ThreadLiveStop(params) => {
            supervisor
                .live_stop(&params.thread_id)
                .await
                .map_err(supervisor_error)?;
            empty()
        }
        Call::ThreadLiveMute(params) => {
            let muted = supervisor
                .live_mute(&params.thread_id, params.muted)
                .await
                .map_err(supervisor_error)?;
            ok(&LiveMuted { muted })
        }
        Call::InteractionRespond(params) => {
            supervisor
                .respond(
                    &params.thread_id,
                    &params.interaction_id,
                    params.response,
                    principal.client(),
                )
                .await
                .map_err(supervisor_error)?;
            empty()
        }
        other => Err(internal(format!(
            "`{}` is not a thread method",
            other.method()
        ))),
    }
}
