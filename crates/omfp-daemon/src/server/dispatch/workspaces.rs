//! `workspaces.*`: workspace folders, their repositories and files.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use omfp_protocol::{Call, CreateWorkspaceParams, ErrorBody, ErrorCode};

use super::{Reply, empty, ok, on_workspaces};
use crate::server::auth::Principal;
use crate::server::convert::{diff_page, internal, supervisor_error, workspace_error};
use crate::server::services::{Services, blocking};
use crate::store::Provenance;
use crate::workspaces::{NewWorkspace, RepoSource, WorkspaceError, named_files};

/// A `workspaces.add_repo` source in the daemon's terms.
fn repo_source(source: omfp_protocol::RepoSource) -> RepoSource {
    match source {
        omfp_protocol::RepoSource::Link { path } => RepoSource::Link(PathBuf::from(path)),
        omfp_protocol::RepoSource::Worktree {
            repo,
            branch,
            base_ref,
        } => RepoSource::Worktree {
            repo: PathBuf::from(repo),
            branch,
            base_ref,
        },
        omfp_protocol::RepoSource::Clone { url } => RepoSource::Clone(url),
    }
}

/// Creates a workspace as `workspaces.create` does; `fallback_name` names it when the params do not.
pub(super) async fn create_workspace(
    services: &Services,
    params: CreateWorkspaceParams,
    fallback_name: Option<String>,
    created_by: Provenance,
    client: String,
) -> Result<String, ErrorBody> {
    let workspaces = Arc::clone(&services.workspaces);
    let store = Arc::clone(&services.store);
    let new = NewWorkspace {
        name: params.name,
        fallback_name,
        link: params.link.map(PathBuf::from),
        created_by,
    };
    let id = blocking(move || {
        let view = workspaces.create(new).map_err(workspace_error)?;
        store.set_created_client(&view.workspace.id, &client)?;
        Ok(view.workspace.id)
    })
    .await?;
    services.refresh_index_quietly().await;
    Ok(id)
}

#[allow(clippy::too_many_lines)] // One arm per method.
pub(super) async fn workspaces(services: &Services, principal: &Principal, call: Call) -> Reply {
    let id = match call {
        Call::WorkspacesCreate(params) => {
            let client = principal.client().to_owned();
            create_workspace(services, params, None, principal.created_by(), client).await?
        }
        Call::WorkspacesUpdate(params) => {
            let id = params.workspace_id.clone();
            on_workspaces(services, move |all| {
                all.rename(&params.workspace_id, &params.name)
            })
            .await?;
            id
        }
        Call::WorkspacesRemove(params) => {
            let (id, force) = (params.workspace_id, params.force);
            let checked = id.clone();
            on_workspaces(services, move |all| all.check_removal(&checked, force)).await?;
            // Declared jobs, jobs and PTYs die before the workspace folder is deleted (DMN-329, DMN-081, TRM-104);
            // the runs and job state go with the workspace's row.
            services.declared.remove_workspace(&id).await;
            services.jobs.kill_workspace(&id).await;
            services.terminals.kill_workspace(&id);
            let removed = id.clone();
            on_workspaces(services, move |all| all.remove(&removed, force)).await?;
            services.jobs.forget_workspace(&id);
            services.terminals.forget_workspace(&id);
            services.refresh_index_quietly().await;
            return empty();
        }
        Call::WorkspacesRescan(params) => params.workspace_id,
        Call::WorkspacesAddRepo(params) => {
            let id = params.workspace_id.clone();
            let source = repo_source(params.source);
            on_workspaces(services, move |all| {
                all.add_repo(&params.workspace_id, params.name.as_deref(), source)
            })
            .await?;
            id
        }
        Call::WorkspacesRemoveRepo(params) => {
            let id = params.workspace_id.clone();
            on_workspaces(services, move |all| {
                all.remove_repo(
                    &params.workspace_id,
                    &params.name,
                    params.force,
                    params.delete_branch,
                )
            })
            .await?;
            id
        }
        Call::WorkspacesStatus(params) => {
            services.rescan(&params.workspace_id).await?;
            let repos = services.workspace_status(&params.workspace_id).await?;
            return match repos.into_iter().find(|repo| repo.name == params.repo) {
                Some(repo) => match repo.error {
                    Some(error) => Err(ErrorBody::new(ErrorCode::GitError, error)),
                    None => ok(&repo.status),
                },
                None => Err(workspace_error(WorkspaceError::NoSuchRepo {
                    workspace: params.workspace_id,
                    name: params.repo,
                })),
            };
        }
        Call::WorkspacesDiff(params) => {
            services.rescan(&params.workspace_id).await?;
            let diff = on_workspaces(services, move |all| {
                all.diff(&all.repo(&params.workspace_id, &params.repo)?)
            })
            .await?;
            return ok(&diff_page(
                &diff,
                params.path.as_deref(),
                params.cursor.as_deref(),
            )?);
        }
        Call::WorkspacesReadFile(params) => {
            return ok(&on_workspaces(services, move |all| {
                all.read_file(
                    &params.workspace_id,
                    &params.path,
                    params.known_hash.as_deref(),
                )
            })
            .await?);
        }
        Call::WorkspacesFindFile(params) => {
            let mentioned = match &params.thread_id {
                Some(thread_id) => services
                    .supervisor
                    .thread_log(thread_id)
                    .await
                    .map_err(supervisor_error)?
                    .with_state(named_files),
                None => BTreeSet::new(),
            };
            return ok(&on_workspaces(services, move |all| {
                all.find_file(
                    &params.workspace_id,
                    &params.query,
                    &mentioned,
                    params.fuzzy,
                )
            })
            .await?);
        }
        Call::WorkspacesWriteFile(params) => {
            let id = params.workspace_id.clone();
            let written = on_workspaces(services, move |all| {
                all.write_file(
                    &params.workspace_id,
                    &params.path,
                    &params.content,
                    params.expected_hash.as_deref(),
                )
            })
            .await?;
            // Then, as after an agent's edit, the folder is rescanned and `workspace/<id>` gets the new git status.
            let services = services.clone();
            tokio::spawn(async move {
                if let Err(error) = services.full_scan(&id).await {
                    tracing::warn!(error = %error.message, "rescan after a file write failed");
                }
            });
            return ok(&written);
        }
        _ => return Err(internal("not a workspace method")),
    };
    // Every workspace method rescans (DMN-023); a change reaches the index and the live threads' roots.
    services.rescan(&id).await?;
    ok(&services.workspace_summary(&id).await?)
}
