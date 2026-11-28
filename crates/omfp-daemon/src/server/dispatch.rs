//! Request handlers: every protocol method mapped to the daemon's services (ARC-007), except the connection-level
//! ones (`system.hello`, `subscribe`, `unsubscribe`).

use std::path::PathBuf;
use std::sync::Arc;

use omfp_protocol::{
    Call, Empty, ErrorBody, PathCompletions, PathEntry, RepoSuggestions, SeenRepository,
    ThreadList, ThreadSearchResult, ThreadSearchResults, WorkspaceList,
};
use serde::Serialize;
use serde_json::Value;

use super::activity;
use super::auth::Principal;
use super::convert::{bad_request, blocking, internal};
use super::prompts;
use super::services::Services;
use crate::terminals::StartTerminal;
use crate::threads::Threads;
use crate::workspaces::Workspaces;
use crate::{fs_complete, search};

mod fork;
mod order;
mod session;
mod threads;
mod workspaces;

type Reply = Result<Value, ErrorBody>;

fn ok(value: &impl Serialize) -> Reply {
    serde_json::to_value(value).map_err(internal)
}

fn empty() -> Reply {
    ok(&Empty {})
}

/// Runs `work` on the workspaces on a blocking task.
async fn on_workspaces<T: Send + 'static>(
    services: &Services,
    work: impl FnOnce(&Workspaces) -> crate::workspaces::Result<T> + Send + 'static,
) -> Result<T, ErrorBody> {
    let workspaces = Arc::clone(&services.workspaces);
    blocking(move || work(&workspaces).map_err(ErrorBody::from)).await
}

/// Runs `work` on the thread records on a blocking task.
async fn on_threads<T: Send + 'static>(
    services: &Services,
    work: impl FnOnce(&Threads) -> crate::threads::Result<T> + Send + 'static,
) -> Result<T, ErrorBody> {
    let threads = Arc::clone(&services.threads);
    blocking(move || work(&threads).map_err(ErrorBody::from)).await
}

/// The thread whose agent calls, which a run or terminal it starts records (DMN-162, TRM-104).
fn agent_thread(principal: &Principal) -> Option<&str> {
    match principal {
        Principal::Agent { thread_id, .. } => Some(thread_id),
        Principal::Owner { .. } | Principal::Mcp { .. } => None,
    }
}

/// Runs a method for `principal`. Authorization already happened (PRT-034). An agent's call beyond reading goes into
/// the activity log once it answered (DMN-161).
pub(crate) async fn dispatch(services: &Services, principal: &Principal, call: Call) -> Reply {
    let logged = activity::pending(principal, &call);
    let reply = run(services, principal, call).await;
    if let Some(entry) = logged {
        activity::record(services, entry, &reply).await;
    }
    reply
}

#[allow(clippy::too_many_lines)] // One arm per method or group of methods.
async fn run(services: &Services, principal: &Principal, call: Call) -> Reply {
    match call {
        Call::SystemHello(_)
        | Call::Subscribe(_)
        | Call::Unsubscribe(_)
        | Call::Watch(_)
        | Call::WidgetsReport(_)
        | Call::WindowsAnswer(_)
        | Call::ThreadLiveConnect(_)
        | Call::ThreadLiveStart(_) => Err(bad_request("handled by the connection")),
        Call::ReposSuggest(params) => {
            let query = params.query.unwrap_or_default();
            let repos = on_workspaces(services, move |all| all.suggest(&query)).await?;
            ok(&RepoSuggestions {
                repos: repos
                    .into_iter()
                    .map(|repo| SeenRepository {
                        path: repo.path.to_string_lossy().into_owned(),
                        name: repo.name,
                        last_seen_at: repo.last_seen_at,
                    })
                    .collect(),
            })
        }
        Call::FsComplete(params) => {
            let entries = blocking(move || {
                let home = std::env::var_os("HOME").map(PathBuf::from);
                Ok(fs_complete::complete(&params.path, home.as_deref()))
            })
            .await?;
            ok(&PathCompletions {
                entries: entries
                    .into_iter()
                    .map(|entry| PathEntry {
                        name: entry.name,
                        path: entry.path.to_string_lossy().into_owned(),
                        is_git_root: entry.is_git_root,
                    })
                    .collect(),
            })
        }
        Call::ThreadsSearch(params) => {
            let store = Arc::clone(&services.store);
            let hits = blocking(move || {
                let threads = store.threads()?;
                Ok(search::search(
                    threads
                        .iter()
                        .map(|thread| (thread.id.as_str(), thread.session_dir.as_path())),
                    &params.query,
                ))
            })
            .await?;
            ok(&ThreadSearchResults {
                results: hits
                    .into_iter()
                    .map(|hit| ThreadSearchResult {
                        thread_id: hit.thread_id,
                        snippet: hit.snippet,
                    })
                    .collect(),
            })
        }
        call @ (Call::WorkspacesCreate(_)
        | Call::WorkspacesUpdate(_)
        | Call::WorkspacesRemove(_)
        | Call::WorkspacesRescan(_)
        | Call::WorkspacesAddRepo(_)
        | Call::WorkspacesRemoveRepo(_)
        | Call::WorkspacesStatus(_)
        | Call::WorkspacesDiff(_)
        | Call::WorkspacesReadFile(_)
        | Call::WorkspacesFindFile(_)
        | Call::WorkspacesWriteFile(_)) => workspaces::workspaces(services, principal, call).await,
        call @ (Call::ThreadsCreate(_)
        | Call::ThreadsUpdate(_)
        | Call::ThreadsArchive(_)
        | Call::ThreadsUnarchive(_)
        | Call::ThreadsDelete(_)
        | Call::ThreadGit(_)) => threads::threads(services, principal, call).await,
        call @ (Call::WorkspacesReorder(_) | Call::ThreadsReorder(_)) => {
            order::reorder(services, call).await
        }
        Call::ThreadsFork(params) => fork::fork_thread(services, principal, params).await,
        Call::ThreadsList(params) => {
            services.refresh_index().await?;
            let threads = services.index.with_state(|index| {
                index
                    .threads
                    .iter()
                    .filter(|thread| params.include_archived || thread.archived_at.is_none())
                    .filter(|thread| {
                        params
                            .workspace_id
                            .as_ref()
                            .is_none_or(|id| *id == thread.workspace_id)
                    })
                    .cloned()
                    .collect()
            });
            ok(&ThreadList { threads })
        }
        Call::WorkspacesList(_) => {
            services.refresh_index().await?;
            let workspaces = services.index.with_state(|index| index.workspaces.clone());
            ok(&WorkspaceList { workspaces })
        }
        Call::WorkspacesMcpServers(params) => ok(&services
            .supervisor
            .workspace_mcp_servers(&params.workspace_id)
            .await?),
        Call::McpTools(params) => {
            let id = params.workspace_id.clone();
            let workspace = on_workspaces(services, move |all| all.get(&id)).await?;
            let tools = services.mcp.tools(&workspace.path, &params.server).await?;
            ok(&omfp_protocol::McpTools {
                workspace_id: params.workspace_id,
                server: params.server,
                tools,
            })
        }
        Call::McpCall(params) => {
            let id = params.workspace_id.clone();
            let workspace = on_workspaces(services, move |all| all.get(&id)).await?;
            ok(&services
                .mcp
                .call(
                    &workspace.path,
                    &params.server,
                    &params.tool,
                    params.arguments.unwrap_or_default(),
                )
                .await?)
        }
        Call::ThemeGet(_) => ok(&services.theme()),
        Call::ThemeSet(params) => {
            services.set_theme(params.css).await?;
            empty()
        }
        Call::SettingsGet(_) => ok(&services.settings()),
        Call::SettingsSet(params) => ok(&services.set_settings(params).await?),
        Call::ViewsErrors(params) => ok(&services.view_errors(&params.workspace_id).await?),
        Call::WindowsList(params) => ok(&services.list_windows(params).await?),
        Call::WindowsState(params) => ok(&services.window_state(&params)?),
        Call::WindowsRun(params) => ok(&services.run_in_window(params).await?),
        Call::WindowsCheckWidget(params) => ok(&services.check_in_window(params).await?),
        Call::ViewsSet(params) => {
            services.set_views(params).await?;
            empty()
        }
        Call::WidgetsCopy(params) => ok(&services.copy_widget(params).await?),
        Call::ViewsMount(params) => ok(&services.mount(params).await?),
        Call::ViewsMountUpdate(params) => ok(&services.mount_update(params).await?),
        Call::ViewsUnmount(params) => ok(&services.unmount(params).await?),
        Call::WidgetsCompile(params) => {
            let widgets = Arc::clone(&services.widgets);
            let module = blocking(move || {
                widgets
                    .compile_card(&params.code)
                    .map_err(|error| bad_request(error.to_string()))
            })
            .await?;
            ok(&omfp_protocol::CompiledWidget { module })
        }
        Call::WidgetsStateSet(params) => {
            services.set_widget_state(params).await?;
            empty()
        }
        call @ (Call::JobsStart(_)
        | Call::JobsKill(_)
        | Call::JobsList(_)
        | Call::JobsRuns(_)
        | Call::JobsRun(_)
        | Call::JobsDefine(_)
        | Call::JobsRemove(_)
        | Call::JobsEnable(_)) => jobs(services, principal, call).await,
        call @ (Call::TerminalsStart(_)
        | Call::TerminalsScreen(_)
        | Call::TerminalsInput(_)
        | Call::TerminalsResize(_)
        | Call::TerminalsKill(_)) => terminals(services, principal, call).await,
        Call::ActivityList(params) => ok(&activity::list(services, params).await?),
        Call::PromptsHistory(params) => ok(&prompts::history(services, params).await?),
        Call::PromptsPredictWord(params) => ok(&omfp_protocol::PredictedWord {
            suffix: services
                .supervisor
                .predict_word(params.text, params.cursor)
                .await?,
        }),
        Call::PromptsPredictWordFeedback(params) => {
            services
                .supervisor
                .predict_word_feedback(
                    params.text,
                    params.cursor,
                    params.suggestion,
                    params.accepted,
                )
                .await?;
            empty()
        }
        Call::AuthStatus(params) => ok(&services.auth_status(params.recheck).await?),
        Call::AuthModels(_) => ok(&services.auth_models().await?),
        Call::AuthProviders(_) => ok(&services.auth_providers().await?),
        // A window's own `auth.login` is the connection's (DMN-221); an agent's goes to the focused window.
        Call::AuthLogin(params) => ok(&services.auth_login(None, params.provider_id).await?),
        Call::AuthLoginCancel(_) => ok(&omfp_protocol::LoginCancelled {
            provider_id: services.supervisor.cancel_login(None),
        }),
        Call::AuthLogout(params) => ok(&services.auth_logout(params.provider_id).await?),
        Call::AuthImportable(_) => ok(&services.auth_importable().await?),
        Call::AuthImport(_) => ok(&services.auth_import().await?),
        Call::OauthCallback(params) => ok(&services.oauth_callback(params).await?),
        call => threads::thread(services, principal, call).await,
    }
}

/// `jobs.*`: transient runs (DMN-080, DMN-081; an agent's run records its thread, DMN-162) and the jobs a workspace
/// declares (PRT-202..PRT-207); `jobs.kill` of a declared run goes through its job (DMN-326).
async fn jobs(services: &Services, principal: &Principal, call: Call) -> Reply {
    let declared = &services.declared;
    match call {
        Call::JobsStart(params) => {
            let workspace =
                on_workspaces(services, move |all| all.get(&params.workspace_id)).await?;
            ok(&services.jobs.start(
                &workspace.id,
                &workspace.path,
                &params.command,
                agent_thread(principal),
                params.replaces.as_deref(),
            )?)
        }
        Call::JobsKill(params) => {
            match declared.kill(&params.job_id).await {
                Some(killed) => killed?,
                None => services.jobs.kill(&params.job_id)?,
            }
            empty()
        }
        Call::JobsList(params) => ok(&declared.list(&params.workspace_id).await?),
        Call::JobsRuns(params) => ok(&declared
            .runs(params.workspace_id, params.job, params.before, params.limit)
            .await?),
        Call::JobsRun(params) => ok(&declared
            .run(&params.workspace_id, &params.job, principal.created_by())
            .await?),
        Call::JobsDefine(params) => ok(&declared
            .define(
                &params.workspace_id,
                params.name,
                params.job,
                params.replace.unwrap_or(false),
            )
            .await?),
        Call::JobsRemove(params) => {
            declared.remove(&params.workspace_id, params.name).await?;
            empty()
        }
        Call::JobsEnable(params) => ok(&declared
            .enable(&params.workspace_id, params.name, params.enabled)
            .await?),
        _ => Err(internal("not a jobs method")),
    }
}

/// `terminals.*` (TRM-100..102); terminal input bytes are redacted from agent activity.
async fn terminals(services: &Services, principal: &Principal, call: Call) -> Reply {
    match call {
        Call::TerminalsStart(params) => {
            let workspace =
                on_workspaces(services, move |all| all.get(&params.workspace_id)).await?;
            ok(&services.terminals.start(StartTerminal {
                workspace_id: &workspace.id,
                workspace: &workspace.path,
                command: &params.command,
                cwd: params.cwd.as_deref(),
                cols: params.cols,
                rows: params.rows,
                started_by: agent_thread(principal),
            })?)
        }
        Call::TerminalsScreen(params) => ok(&services
            .terminals
            .screen(&params.terminal_id, params.after_revision, params.wait_ms)
            .await?),
        Call::TerminalsInput(params) => {
            services
                .terminals
                .input(&params.terminal_id, &params.data)?;
            empty()
        }
        Call::TerminalsResize(params) => {
            services
                .terminals
                .resize(&params.terminal_id, params.cols, params.rows)?;
            empty()
        }
        Call::TerminalsKill(params) => {
            services.terminals.kill(&params.terminal_id)?;
            empty()
        }
        _ => Err(internal("not a terminals method")),
    }
}
