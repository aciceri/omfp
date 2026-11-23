//! `threads.fork` (DMN-300..DMN-303): a new thread in the source thread's workspace whose omp starts on a copy of the
//! source's session, cut after an entry; the source thread is only read.

use std::path::PathBuf;
use std::sync::Arc;

use omfp_protocol::{CreatedBy, ErrorBody, ErrorCode, ForkParams, TitleSource};

use super::threads::discard_thread;
use super::{Reply, ok};
use crate::search;
use crate::server::auth::Principal;
use crate::server::convert::{bad_request, blocking, internal};
use crate::server::services::Services;
use crate::store::ThreadFork;
use crate::threads::{NewThread, Threads, session_file};

/// The title a fork starts with, a placeholder (DMN-300): the source's, marked as a fork.
fn fork_title(source: &str) -> String {
    let source = source.trim();
    if source.is_empty() {
        "Fork".to_owned()
    } else {
        format!("{source} (fork)")
    }
}

/// What forking `params` takes from the source thread: its session file and the new thread with its lineage. Fails
/// before anything is created when the source has no session file yet or the entry is no message of it (DMN-303).
fn plan(
    threads: &Threads,
    params: &ForkParams,
    created_by: CreatedBy,
) -> Result<(PathBuf, NewThread, ThreadFork), ErrorBody> {
    let source = threads.get(&params.thread_id)?;
    let file = source
        .omp_session_id
        .as_deref()
        .and_then(|session_id| session_file(&source.session_dir, session_id))
        .ok_or_else(|| {
            bad_request(format!(
                "thread {} has no conversation to fork yet",
                params.thread_id
            ))
        })?;
    let message = match &params.entry_id {
        None => None,
        Some(entry_id) => {
            let text = search::message_entry(&file, entry_id)
                .map_err(internal)?
                .ok_or_else(|| {
                    ErrorBody::new(
                        ErrorCode::NotFound,
                        format!(
                            "no message entry `{entry_id}` in the session of thread {}",
                            params.thread_id
                        ),
                    )
                })?;
            Some(text).filter(|text| !text.is_empty())
        }
    };
    let new = NewThread {
        title: fork_title(&source.title),
        title_source: TitleSource::Placeholder,
        workspace_id: source.workspace_id,
        model: source.model,
        thinking_level: source.thinking_level,
        approval_mode: source.approval_mode,
        created_by,
    };
    let fork = ThreadFork {
        source_thread_id: source.id,
        source_title: source.title,
        entry_id: params.entry_id.clone(),
        message,
    };
    Ok((file, new, fork))
}

/// `threads.fork`: the new thread's summary once its omp runs on the copy (DMN-300..DMN-303). When omp cannot start
/// or fork, the new thread is deleted again.
pub(super) async fn fork_thread(
    services: &Services,
    principal: &Principal,
    params: ForkParams,
) -> Reply {
    let client = principal.client().to_owned();
    let created_by = principal.created_by();
    let threads = Arc::clone(&services.threads);
    let store = Arc::clone(&services.store);
    let entry_id = params.entry_id.clone();
    let (source, thread) = blocking(move || {
        let (source, new, fork) = plan(&threads, &params, created_by)?;
        let thread = threads.create_fork(new, &fork)?;
        store.set_created_client(&thread.id, &client)?;
        Ok((source, thread))
    })
    .await?;
    if let Err(error) = services
        .supervisor
        .start_fork(&thread.id, source, entry_id)
        .await
    {
        discard_thread(services, &thread.id).await;
        services.refresh_index_quietly().await;
        return Err(error.into());
    }
    ok(&services.thread_summary(&thread.id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-300
    #[test]
    fn a_fork_is_titled_after_its_source() {
        assert_eq!(fork_title("Fix the cart"), "Fix the cart (fork)");
        assert_eq!(fork_title("  "), "Fork");
    }
}
