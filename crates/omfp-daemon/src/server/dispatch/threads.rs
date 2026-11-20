//! `threads.*` and `thread.*`: creating and listing threads, and the methods acting on a thread's agent.

use std::sync::Arc;

use omfp_core::wire::{ImageContent, ThinkingLevel};
use omfp_protocol::{
    Call, CloseJobs, CreatedBy, ErrorBody, ErrorCode, JobRunStatus, LiveMuted, MessageParams,
    PromptBehavior, PromptResult, RunInfo, RunTrigger, ThreadGit, ThreadWorkspace, ThreadsStopped,
};
use omp_rpc::{
    BashCommand, BtwCommand, CancelSubagentCommand, CompactCommand, FollowUpCommand, ForkCommand,
    GetSubagentMessagesCommand, GoalCommand, NavigateTreeCommand, PromoteQueuedMessageCommand,
    PromptCommand, QueuedMessageQueue, RemoveQueuedMessageCommand, SetLabelCommand,
    SetModelCommand, SetThinkingLevelCommand, SteerCommand, SteerSubagentCommand,
    StreamingBehavior,
};
use serde_json::{Value, json};

use super::workspaces::create_workspace;
use super::{Reply, empty, ok, on_threads, on_workspaces};
use crate::agent::OmpCall;
use crate::server::auth::Principal;
use crate::server::convert::{internal, supervisor_error, thread_error};
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
    let first_images = images(params.images);
    if params.prompt.is_some() || first_images.is_some() {
        let text = params.prompt.unwrap_or_default();
        let command = PromptCommand {
            message: text.clone(),
            images: first_images,
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
    if let Err(error) = services.supervisor.delete(id).await {
        tracing::warn!(thread = %id, %error, "cannot delete the thread that failed to start");
    }
}

/// Best effort: removes the workspace `threads.create` made for a thread it could not create; the new folder had no
/// other use.
async fn remove_new_workspace(services: &Services, workspace_id: String) {
    let _ = on_workspaces(services, move |all| all.remove(&workspace_id, false)).await;
    services.refresh_index_quietly().await;
}

/// Before `threads.archive` or `threads.delete` (PRT-208, DMN-333): the transient runs the thread's agent started and
/// that still run. Without `jobs` they make the call a `conflict` listing them; `kill` kills them as `jobs.kill` does,
/// without waiting for their end; `keep` leaves them running.
fn close_jobs(
    services: &Services,
    thread_id: &str,
    jobs: Option<CloseJobs>,
) -> Result<(), ErrorBody> {
    let running = services.jobs.running_started_by(thread_id);
    if running.is_empty() {
        return Ok(());
    }
    match jobs {
        Some(CloseJobs::Keep) => Ok(()),
        Some(CloseJobs::Kill) => {
            for job in running {
                // A run that ended and was forgotten meanwhile needs no kill.
                let _ = services.jobs.kill(&job.id);
            }
            Ok(())
        }
        None => {
            let runs: Vec<RunInfo> = running
                .into_iter()
                .map(|job| RunInfo {
                    id: job.id,
                    workspace_id: job.workspace_id,
                    job: None,
                    trigger: RunTrigger::Start,
                    started_by: Some(CreatedBy::Thread {
                        thread_id: thread_id.to_owned(),
                    }),
                    thread_id: None,
                    command: Some(job.command),
                    started_at: job.started_at,
                    ended_at: None,
                    status: JobRunStatus::Running,
                    reason: None,
                    exit_code: None,
                    signal: None,
                })
                .collect();
            Err(ErrorBody::new(
                ErrorCode::Conflict,
                "runs the thread's agent started still run: pass jobs \"kill\" or \"keep\"",
            )
            .with_details(json!({ "runs": runs })))
        }
    }
}

pub(super) async fn threads(services: &Services, principal: &Principal, call: Call) -> Reply {
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
            if let Some(mode) = params.approval_mode {
                on_threads(services, move |all| {
                    all.set_approval_mode(&params.thread_id, mode.as_str())
                })
                .await?;
            } else {
                on_threads(services, move |all| all.get(&params.thread_id)).await?;
            }
            ok(&services.thread_summary(&id).await?)
        }
        Call::ThreadsArchive(params) => {
            close_jobs(services, &params.thread_id, params.jobs)?;
            supervisor
                .stop(&params.thread_id)
                .await
                .map_err(supervisor_error)?;
            on_threads(services, move |all| all.archive(&params.thread_id)).await?;
            services.refresh_index_quietly().await;
            empty()
        }
        Call::ThreadsUnarchive(params) => {
            on_threads(services, move |all| all.unarchive(&params.thread_id)).await?;
            services.refresh_index_quietly().await;
            empty()
        }
        Call::ThreadsDelete(params) => {
            let id = params.thread_id.clone();
            on_threads(services, move |all| all.get(&id)).await?;
            close_jobs(services, &params.thread_id, params.jobs)?;
            supervisor
                .delete(&params.thread_id)
                .await
                .map_err(supervisor_error)?;
            services.refresh_index_quietly().await;
            empty()
        }
        Call::ThreadGit(params) => {
            let thread = on_threads(services, move |all| all.get(&params.thread_id)).await?;
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
    params: MessageParams,
    message: fn(String, Option<Vec<ImageContent>>) -> QueuedMessage,
) -> Reply {
    let MessageParams {
        thread_id,
        text,
        images: attached,
    } = params;
    services
        .supervisor
        .queue(&thread_id, message(text.clone(), images(attached)))
        .await
        .map_err(supervisor_error)?;
    prompts::record(services, principal, &thread_id, None, &text).await;
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
            queue(services, principal, params, |message, images| {
                QueuedMessage::Steer(SteerCommand { message, images })
            })
            .await
        }
        Call::ThreadFollowUp(params) => {
            queue(services, principal, params, |message, images| {
                QueuedMessage::FollowUp(FollowUpCommand { message, images })
            })
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
            let command = SetModelCommand {
                provider: params.provider,
                model_id: params.model_id,
            };
            supervisor
                .set_model(&params.thread_id, command)
                .await
                .map_err(supervisor_error)?;
            empty()
        }
        Call::ThreadSetThinking(params) => {
            let name = thinking_name(params.level)?;
            let command = SetThinkingLevelCommand {
                level: params.level,
            };
            forward(params.thread_id.clone(), OmpCall::SetThinkingLevel(command)).await?;
            on_threads(services, move |all| {
                all.set_thinking_level(&params.thread_id, Some(&name))
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
        Call::CardAnswer(params) => {
            supervisor
                .answer_card(&params.thread_id, &params.tool_call_id, params.answer)
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use omfp_protocol::{CloseJobs, StartJobParams, ThreadCloseParams};
    use tempfile::TempDir;

    use super::*;
    use crate::agent::FakeLauncher;
    use crate::git::{FakeGit, Git};
    use crate::paths::DataDir;
    use crate::server::dispatch::dispatch;
    use crate::store::{Provenance, Store};
    use crate::supervisor::{Supervisor, SupervisorConfig};
    use crate::threads::Threads;
    use crate::widgets::Widgets;
    use crate::workspaces::{NewWorkspace, Workspaces};

    async fn services(dir: &TempDir) -> Services {
        let data = DataDir::new(dir.path().join("data"));
        std::fs::create_dir_all(data.root()).unwrap();
        let root = dir.path().join("workspaces");
        let store = Arc::new(Store::open(&data.db(), &root).unwrap());
        let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
        let workspaces = Arc::new(Workspaces::new(
            Arc::clone(&store),
            Arc::new(FakeGit::new()) as Arc<dyn Git>,
            root,
        ));
        let widgets = Arc::new(Widgets::new(data.widgets(), None));
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

    fn owner() -> Principal {
        Principal::Owner {
            client: "omfp/web".into(),
        }
    }

    /// A workspace, a thread in it and a run its agent started, still running.
    async fn thread_with_run(services: &Services) -> (String, String) {
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
        let agent = Principal::Agent {
            thread_id: thread.clone(),
            client: "omp".into(),
        };
        let job = dispatch(
            services,
            &agent,
            Call::JobsStart(StartJobParams {
                workspace_id: ws,
                command: "while true; do sleep 0.01; done".to_owned(),
                replaces: None,
            }),
        )
        .await
        .unwrap();
        (thread, job["id"].as_str().unwrap().to_owned())
    }

    fn close(delete: bool, thread_id: &str, jobs: Option<CloseJobs>) -> Call {
        let params = ThreadCloseParams {
            thread_id: thread_id.to_owned(),
            jobs,
        };
        if delete {
            Call::ThreadsDelete(params)
        } else {
            Call::ThreadsArchive(params)
        }
    }

    fn running(services: &Services, job_id: &str) -> bool {
        services.jobs.log(job_id).unwrap().with_state(|topic| {
            serde_json::to_value(topic).unwrap()["job"]["status"]["state"] == "running"
        })
    }

    async fn until_ended(services: &Services, job_id: &str) {
        let log = services.jobs.log(job_id).unwrap();
        let mut live = log.subscribe(None).live;
        tokio::time::timeout(Duration::from_secs(60), async {
            while running(services, job_id) {
                let _ = live.recv().await;
            }
        })
        .await
        .expect("the run did not end");
    }

    /// Whether the thread is still there and not archived.
    fn open(services: &Services, thread_id: &str) -> Option<bool> {
        services
            .threads
            .get(thread_id)
            .ok()
            .map(|thread| thread.archived_at.is_none())
    }

    // spec: PRT-208, DMN-333
    #[tokio::test(flavor = "multi_thread")]
    async fn closing_a_thread_whose_agents_runs_still_run_without_jobs_is_a_conflict_listing_them()
    {
        for delete in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let services = services(&dir).await;
            let (thread, job) = thread_with_run(&services).await;

            let error = dispatch(&services, &owner(), close(delete, &thread, None))
                .await
                .unwrap_err();

            assert_eq!(error.code, ErrorCode::Conflict);
            let details = error.details.unwrap();
            let runs = details["runs"].as_array().unwrap();
            assert_eq!(runs.len(), 1);
            let run = &runs[0];
            assert_eq!(run["id"], json!(job));
            assert_eq!(run["job"], Value::Null);
            assert_eq!(run["trigger"], json!("start"));
            assert_eq!(
                run["startedBy"],
                json!({ "kind": "thread", "threadId": thread })
            );
            assert_eq!(run["status"], json!("running"));
            assert_eq!(run["threadId"], Value::Null);
            assert_eq!(open(&services, &thread), Some(true));
            assert!(running(&services, &job));
            services.jobs.kill(&job).unwrap();
            until_ended(&services, &job).await;
        }
    }

    // spec: PRT-208, DMN-333
    #[tokio::test(flavor = "multi_thread")]
    async fn closing_a_thread_with_kill_kills_its_agents_runs_and_closes_it() {
        for delete in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let services = services(&dir).await;
            let (thread, job) = thread_with_run(&services).await;

            dispatch(
                &services,
                &owner(),
                close(delete, &thread, Some(CloseJobs::Kill)),
            )
            .await
            .unwrap();

            assert_eq!(open(&services, &thread), (!delete).then_some(false));
            until_ended(&services, &job).await;
        }
    }

    // spec: PRT-208, DMN-333
    #[tokio::test(flavor = "multi_thread")]
    async fn closing_a_thread_with_keep_leaves_its_agents_runs_running() {
        for delete in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let services = services(&dir).await;
            let (thread, job) = thread_with_run(&services).await;

            dispatch(
                &services,
                &owner(),
                close(delete, &thread, Some(CloseJobs::Keep)),
            )
            .await
            .unwrap();

            assert_eq!(open(&services, &thread), (!delete).then_some(false));
            assert!(running(&services, &job));
            services.jobs.kill(&job).unwrap();
            until_ended(&services, &job).await;
        }
    }

    // spec: DMN-329, PRT-208
    #[tokio::test(flavor = "multi_thread")]
    async fn a_declared_run_an_agent_asked_for_never_holds_its_thread_back() {
        for delete in [false, true] {
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
            let agent = Principal::Agent {
                thread_id: thread.clone(),
                client: "omp".into(),
            };
            dispatch(
                &services,
                &agent,
                Call::JobsDefine(omfp_protocol::DefineJobParams {
                    workspace_id: ws.clone(),
                    name: "loop".into(),
                    job: json!({ "command": "while true; do sleep 0.01; done" }),
                    replace: None,
                }),
            )
            .await
            .unwrap();
            let run = dispatch(
                &services,
                &agent,
                Call::JobsRun(omfp_protocol::JobNameParams {
                    workspace_id: ws.clone(),
                    job: "loop".into(),
                }),
            )
            .await
            .unwrap();
            assert_eq!(
                run["startedBy"],
                json!({ "kind": "thread", "threadId": thread })
            );
            let run_id = run["id"].as_str().unwrap();

            dispatch(&services, &owner(), close(delete, &thread, None))
                .await
                .unwrap();

            assert_eq!(open(&services, &thread), (!delete).then_some(false));
            assert!(running(&services, run_id));
            dispatch(
                &services,
                &owner(),
                Call::JobsKill(omfp_protocol::JobRef {
                    job_id: run_id.to_owned(),
                }),
            )
            .await
            .unwrap();
            until_ended(&services, run_id).await;
        }
    }

    // spec: PRT-209, DMN-334
    #[tokio::test(flavor = "multi_thread")]
    async fn a_job_started_thread_reads_back_its_provenance_and_belongs_to_no_spawned_tree() {
        let dir = tempfile::tempdir().unwrap();
        let services = services(&dir).await;
        let ws = services
            .workspaces
            .create(NewWorkspace::new())
            .unwrap()
            .workspace
            .id;
        let root = services
            .threads
            .create(NewThread::new("root", &ws))
            .unwrap()
            .id;
        let by_job = services
            .threads
            .create(NewThread {
                created_by: Provenance::Job("nightly".into()),
                ..NewThread::new("nightly", &ws)
            })
            .unwrap()
            .id;

        let thread = services.threads.get(&by_job).unwrap();
        assert_eq!(thread.created_by, Provenance::Job("nightly".into()));
        let summary = services.thread_summary(&by_job).await.unwrap();
        assert_eq!(
            serde_json::to_value(&summary.provenance).unwrap()["createdBy"],
            json!({ "kind": "job", "job": "nightly" })
        );
        assert_eq!(
            services.store.spawned_thread_ids(&root).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            services.store.spawned_thread_ids(&by_job).unwrap(),
            Vec::<String>::new()
        );
    }

    // spec: PRT-224, DMN-911
    #[tokio::test]
    async fn card_answer_ends_a_waiting_card_once_and_is_not_found_for_an_unknown_thread() {
        use omfp_protocol::{CardAnswer, CardAnswerParams, ErrorCode};

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
        let answer = |thread_id: &str, value: serde_json::Value| {
            Call::CardAnswer(CardAnswerParams {
                thread_id: thread_id.to_owned(),
                tool_call_id: "toolu_1".to_owned(),
                answer: CardAnswer::Submit { value },
            })
        };

        let unknown = dispatch(&services, &owner(), answer("01NOTHREAD", json!(1)))
            .await
            .unwrap_err();
        assert_eq!(unknown.code, ErrorCode::NotFound, "{unknown:?}");
        let never = dispatch(&services, &owner(), answer(&thread, json!(1)))
            .await
            .unwrap_err();
        assert_eq!(never.code, ErrorCode::AlreadyResolved, "{never:?}");

        let mut wait = services
            .supervisor
            .wait_card(&thread, "toolu_1")
            .await
            .unwrap();
        assert!(services.supervisor.status(&thread).needs_attention);
        let answered = dispatch(&services, &owner(), answer(&thread, json!({ "ok": true }))).await;
        assert_eq!(answered, Ok(json!({})));
        assert!(!services.supervisor.status(&thread).needs_attention);
        assert_eq!(
            wait.answer().await.unwrap(),
            CardAnswer::Submit {
                value: json!({ "ok": true })
            }
        );
        let second = dispatch(&services, &owner(), answer(&thread, json!(2)))
            .await
            .unwrap_err();
        assert_eq!(second.code, ErrorCode::AlreadyResolved, "{second:?}");
    }

    // spec: PRT-225
    #[tokio::test]
    async fn widgets_compile_answers_the_module_or_a_bad_request_at_card_tsx() {
        use omfp_protocol::{CompileWidgetParams, ErrorCode};

        let dir = tempfile::tempdir().unwrap();
        let services = services(&dir).await;
        let compile = |code: &str| {
            Call::WidgetsCompile(CompileWidgetParams {
                code: code.to_owned(),
            })
        };
        let compiled = dispatch(
            &services,
            &owner(),
            compile("import { Card } from \"omfp/kit\";\nexport default () => <Card />;\n"),
        )
        .await
        .unwrap();
        let module = compiled["module"].as_str().unwrap();
        assert!(module.contains("from \"react/jsx-runtime\""), "{module}");
        assert!(module.contains("//# sourceMappingURL=data:"), "{module}");

        let error = dispatch(&services, &owner(), compile("export {};\nconst = 1;\n"))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::BadRequest);
        assert!(error.message.starts_with("card.tsx:2:"), "{error:?}");
        let error = dispatch(
            &services,
            &owner(),
            compile("import a from \"../a\";\nexport default a;\n"),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::BadRequest);
        assert!(
            error
                .message
                .starts_with("card.tsx:1:15: cannot import \"../a\": a card is one file"),
            "{error:?}"
        );
    }
}
