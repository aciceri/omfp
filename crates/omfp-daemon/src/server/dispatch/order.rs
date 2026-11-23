//! `workspaces.reorder` and `threads.reorder`: the order of the left column, shared by every window (DMN-280..DMN-285).

use std::sync::Arc;

use omfp_protocol::{Call, ReorderThreadParams, ReorderWorkspaceParams};

use super::{Reply, empty, on_threads};
use crate::server::convert::{bad_request, blocking, internal};
use crate::server::services::Services;

pub(super) async fn reorder(services: &Services, call: Call) -> Reply {
    match call {
        Call::WorkspacesReorder(params) => workspace(services, params).await,
        Call::ThreadsReorder(params) => thread(services, params).await,
        _ => Err(internal("not a reorder method")),
    }
}

/// `workspaces.reorder` (DMN-281).
async fn workspace(services: &Services, params: ReorderWorkspaceParams) -> Reply {
    let ReorderWorkspaceParams {
        workspace_id,
        before_id,
    } = params;
    if before_id.as_ref() == Some(&workspace_id) {
        return Err(bad_request("a workspace cannot go before itself"));
    }
    let store = Arc::clone(&services.store);
    blocking(move || Ok(store.reorder_workspace(&workspace_id, before_id.as_deref())?)).await?;
    services.refresh_index_quietly().await;
    empty()
}

/// `threads.reorder` (DMN-282); into another workspace through the supervisor, which restarts its omp there
/// (DMN-283). Within its workspace the store reads that workspace in the same transaction, so a move by another
/// window meanwhile never goes unnoticed (DMN-285).
async fn thread(services: &Services, params: ReorderThreadParams) -> Reply {
    let ReorderThreadParams {
        thread_id,
        workspace_id,
        before_id,
    } = params;
    if before_id.as_ref() == Some(&thread_id) {
        return Err(bad_request("a thread cannot go before itself"));
    }
    let id = thread_id.clone();
    let current = on_threads(services, move |all| all.get(&id))
        .await?
        .workspace_id;
    let target = workspace_id.filter(|target| *target != current);
    if target.is_some() {
        let threads = Arc::clone(&services.threads);
        let id = thread_id.clone();
        let apply = move || threads.reorder(&id, target.as_deref(), before_id.as_deref());
        services.supervisor.relocate(&thread_id, apply).await?;
    } else {
        on_threads(services, move |all| {
            all.reorder(&thread_id, None, before_id.as_deref())
        })
        .await?;
    }
    services.refresh_index_quietly().await;
    empty()
}
