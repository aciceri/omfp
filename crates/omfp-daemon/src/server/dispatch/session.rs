//! `thread.session`: the small facts of a thread's omp session, from the daemon's records and the thread's state, its
//! usage from omp only while omp runs; it never starts omp (PRT-125, PRT-126, DMN-213, DMN-214).

use std::path::Path;
use std::sync::Arc;

use omfp_protocol::{ContextUsage, ErrorBody, SessionTokens, SessionUsage, ThreadSession};
use omp_rpc::SessionStats;

use crate::agent::OmpCall;
use crate::server::convert::{internal, supervisor_error, thread_error, workspace_error};
use crate::server::services::{Services, blocking};
use crate::threads;

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(super) async fn thread_session(
    services: &Services,
    id: &str,
) -> Result<ThreadSession, ErrorBody> {
    let supervisor = &services.supervisor;
    // The session omp loaded in this daemon run (its `history` event), with the transcript of its branch.
    let (loaded, message_count) = supervisor
        .thread_log(id)
        .await
        .map_err(supervisor_error)?
        .with_state(|state| {
            let count = state
                .session
                .as_ref()
                .map(|_| state.transcript.len() as u64);
            (state.session.clone(), count)
        });
    let status = supervisor.status(id);
    let session_model = supervisor.session_model(id);
    let usage = supervisor
        .call_if_running(id, OmpCall::SessionStats)
        .await
        .map_err(supervisor_error)?
        .map(serde_json::from_value::<SessionStats>)
        .transpose()
        .map_err(internal)?
        .map(session_usage);

    let records = Arc::clone(&services.threads);
    let workspaces = Arc::clone(&services.workspaces);
    let thread_id = id.to_owned();
    blocking(move || {
        let thread = records.get(&thread_id).map_err(thread_error)?;
        let cwd = workspaces
            .get(&thread.workspace_id)
            .map_err(workspace_error)?
            .path;
        let roots = records.roots(&thread_id).map_err(thread_error)?;
        // omp's own word when it loaded this session; a fork may have moved the thread on since (DMN-072).
        let session_file = match (&thread.omp_session_id, loaded) {
            (Some(session_id), Some(loaded))
                if loaded.session_id == *session_id && loaded.session_file.is_some() =>
            {
                loaded.session_file
            }
            (Some(session_id), _) => {
                threads::session_file(&thread.session_dir, session_id).map(|file| path_text(&file))
            }
            (None, _) => None,
        };
        Ok(ThreadSession {
            thread_id: thread.id,
            workspace_id: thread.workspace_id,
            cwd: path_text(&cwd),
            roots: roots.iter().map(|root| path_text(root)).collect(),
            session_dir: path_text(&thread.session_dir),
            session_id: thread.omp_session_id,
            session_file,
            model: thread.model,
            session_model,
            thinking_level: thread.thinking_level,
            // A value saved before DMN-210 that omp does not take is left out.
            approval_mode: thread
                .approval_mode
                .as_deref()
                .and_then(|mode| serde_json::from_value(serde_json::Value::from(mode)).ok()),
            process: status.process,
            settled: status.settled,
            needs_attention: status.needs_attention,
            message_count,
            usage,
        })
    })
    .await
}

fn session_usage(stats: SessionStats) -> SessionUsage {
    SessionUsage {
        cost: stats.cost,
        tokens: SessionTokens {
            input: stats.tokens.input,
            output: stats.tokens.output,
            cache_read: stats.tokens.cache_read,
            cache_write: stats.tokens.cache_write,
            total: stats.tokens.total,
        },
        context: stats.context_usage.map(|context| ContextUsage {
            tokens: context.tokens,
            context_window: context.context_window,
            percent: context.percent,
        }),
    }
}
