//! The activity log (AGT-072): the calls of agent principals beyond reading, recorded once they answered (DMN-160,
//! DMN-161), and `activity.list` (PRT-086).

use std::sync::Arc;

use omfp_protocol::{ActivityEntry, ActivityPage, Call, ErrorBody, ListActivityParams, Request};
use serde_json::{Value, json};
use tracing::warn;

use super::auth::{Permission, Principal, permission};
use super::convert::internal;
use super::services::Services;
use crate::store::{Activity, ActivityQuery, NewActivity, Provenance, now_ms};

/// The newest entries the log keeps (DMN-160).
pub(crate) const KEPT: u32 = 10_000;
/// Longer strings of logged params are cut to this many characters (DMN-161).
const MAX_STRING_CHARS: usize = 200;
/// `activity.list`'s default and largest `limit` (PRT-086).
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 500;

/// What to log about `call` by `principal` once it answered: `None` unless the principal is a thread's agent or the
/// MCP principal (AGT-053) and the method needs more than `Connect` or `Read` (DMN-161).
pub(crate) fn pending(principal: &Principal, call: &Call) -> Option<NewActivity> {
    let thread_id = match principal.created_by() {
        Provenance::Thread(thread_id) => Some(thread_id),
        Provenance::Mcp => None,
        Provenance::User | Provenance::Job(_) => return None,
    };
    if matches!(
        permission(call.method()),
        None | Some(Permission::Connect | Permission::Read)
    ) {
        return None;
    }
    let params = match call {
        Call::TerminalsInput(params) => json!({
            "terminalId": params.terminal_id,
            "data": "[redacted]",
        }),
        _ => serde_json::to_value(Request::new("", call.clone()))
            .ok()
            .and_then(|mut request| request.get_mut("params").map(Value::take))
            .unwrap_or(Value::Null),
    };
    Some(NewActivity {
        at: now_ms(),
        thread_id,
        client: principal.client().to_owned(),
        method: call.method().to_owned(),
        params: shorten(params),
        error: None,
    })
}

/// `value` with every string longer than [`MAX_STRING_CHARS`] cut to that many characters and `…` (DMN-161).
fn shorten(value: Value) -> Value {
    match value {
        Value::String(text) if text.chars().count() > MAX_STRING_CHARS => {
            let mut cut: String = text.chars().take(MAX_STRING_CHARS).collect();
            cut.push('…');
            Value::String(cut)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(shorten).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, shorten(value)))
                .collect(),
        ),
        other => other,
    }
}

/// Appends `entry` with the call's outcome; a failure is only a warning, never the call's (DMN-161).
pub(crate) async fn record(
    services: &Services,
    mut entry: NewActivity,
    reply: &Result<Value, ErrorBody>,
) {
    entry.error = reply.as_ref().err().map(|error| error.message.clone());
    let store = Arc::clone(&services.store);
    let method = entry.method.clone();
    match tokio::task::spawn_blocking(move || store.record_activity(&entry, KEPT)).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => warn!(%method, %error, "cannot log an agent's action"),
        Err(error) => warn!(%method, %error, "cannot log an agent's action"),
    }
}

/// `activity.list` (PRT-086).
pub(crate) async fn list(
    services: &Services,
    params: ListActivityParams,
) -> Result<ActivityPage, ErrorBody> {
    let query = ActivityQuery {
        thread_id: params.thread_id,
        before: params.before,
        after: params.after,
        limit: params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT),
    };
    let store = Arc::clone(&services.store);
    let (entries, more) = tokio::task::spawn_blocking(move || store.activity(&query))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    Ok(ActivityPage {
        entries: entries.into_iter().map(wire_entry).collect(),
        more,
    })
}

fn wire_entry(Activity { seq, entry }: Activity) -> ActivityEntry {
    ActivityEntry {
        seq,
        at: entry.at,
        thread_id: entry.thread_id,
        client: entry.client,
        method: entry.method,
        params: entry.params,
        error: entry.error,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use omfp_protocol::{
        JobRef, PromptParams, SetWidgetStateParams, StartJobParams, StopThreadParams,
        TerminalInputParams, ThreadRef, WriteFileParams,
    };
    use serde_json::json;
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

    fn agent_of(thread_id: &str) -> Principal {
        Principal::Agent {
            thread_id: thread_id.to_owned(),
            client: "omp".into(),
        }
    }

    /// `job/<id>`'s job status as on the wire.
    async fn job_status(services: &Services, id: &str, done: bool) -> Value {
        let log = services.jobs.log(id).unwrap();
        let mut live = log.subscribe(None).live;
        let status = || {
            log.with_state(|topic| serde_json::to_value(topic).unwrap()["job"]["status"].clone())
        };
        tokio::time::timeout(Duration::from_secs(60), async {
            while done && status()["state"] == "running" {
                let _ = live.recv().await;
            }
        })
        .await
        .expect("the job did not finish");
        status()
    }

    // spec: TRM-104
    #[test]
    fn terminal_input_is_redacted_from_agent_activity() {
        let activity = pending(
            &agent_of("thread"),
            &Call::TerminalsInput(TerminalInputParams {
                terminal_id: "terminal".into(),
                data: "c2VjcmV0".into(),
            }),
        )
        .unwrap();
        assert_eq!(
            activity.params,
            json!({ "terminalId": "terminal", "data": "[redacted]" })
        );
    }

    // spec: AGT-072, DMN-161, PRT-086
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agents_actions_are_logged_with_their_outcome_and_listed_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let services = services(&dir).await;
        let ws = services
            .workspaces
            .create(NewWorkspace::new())
            .unwrap()
            .workspace
            .id;
        let agent = services
            .threads
            .create(NewThread::new("orchestrator", &ws))
            .unwrap()
            .id;
        let store_value = |instance: &str| {
            Call::WidgetsStateSet(SetWidgetStateParams {
                workspace_id: ws.clone(),
                instance: instance.to_owned(),
                key: "k".to_owned(),
                value: json!(1),
            })
        };
        let missing = Call::ThreadStop(StopThreadParams {
            thread_id: "01JUNKNOWN0000000000000000".to_owned(),
            cascade: false,
        });

        dispatch(&services, &agent_of(&agent), store_value("dashboard"))
            .await
            .unwrap();
        dispatch(&services, &agent_of(&agent), missing)
            .await
            .unwrap_err();
        dispatch(
            &services,
            &agent_of(&agent),
            Call::ThemeGet(omfp_protocol::Empty {}),
        )
        .await
        .unwrap();
        dispatch(&services, &owner(), store_value("mine"))
            .await
            .unwrap();

        let page = dispatch(
            &services,
            &owner(),
            Call::ActivityList(ListActivityParams::default()),
        )
        .await
        .unwrap();
        let page: ActivityPage = serde_json::from_value(page).unwrap();
        assert!(!page.more);
        let [failed, stored] = <[ActivityEntry; 2]>::try_from(page.entries).unwrap();
        assert!(failed.seq > stored.seq);
        assert_eq!(
            (
                stored.thread_id.as_deref(),
                stored.client.as_str(),
                stored.method.as_str()
            ),
            (Some(agent.as_str()), "omp", "widgets.state_set")
        );
        assert_eq!(
            stored.params,
            json!({ "workspaceId": ws, "instance": "dashboard", "key": "k", "value": 1 })
        );
        assert_eq!(stored.error, None);
        assert_eq!(failed.method, "thread.stop");
        assert!(
            failed
                .error
                .is_some_and(|error| error.contains("not found"))
        );

        let filtered = list(
            &services,
            ListActivityParams {
                thread_id: Some("01JOTHER".to_owned()),
                ..ListActivityParams::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(filtered.entries, Vec::new());
    }

    // spec: AGT-071, DMN-162, PRT-085
    #[tokio::test(flavor = "multi_thread")]
    async fn a_cascade_stop_answers_the_stopped_threads_and_kills_their_agents_jobs() {
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
        let child = services
            .threads
            .create(NewThread {
                created_by: Provenance::Thread(root.clone()),
                ..NewThread::new("child", &ws)
            })
            .unwrap()
            .id;
        let start = |principal: Principal| {
            let services = &services;
            let ws = ws.clone();
            async move {
                let job = dispatch(
                    services,
                    &principal,
                    Call::JobsStart(StartJobParams {
                        workspace_id: ws,
                        command: "while true; do sleep 0.01; done".to_owned(),
                        replaces: None,
                    }),
                )
                .await
                .unwrap();
                job["id"].as_str().unwrap().to_owned()
            }
        };
        let child_job = start(agent_of(&child)).await;
        let users_job = start(owner()).await;

        let stopped = dispatch(
            &services,
            &owner(),
            Call::ThreadStop(StopThreadParams {
                thread_id: root.clone(),
                cascade: true,
            }),
        )
        .await
        .unwrap();

        assert_eq!(stopped, json!({ "threadIds": [root, child] }));
        assert_eq!(
            job_status(&services, &child_job, true).await["signal"],
            json!(15)
        );
        assert_eq!(
            job_status(&services, &users_job, false).await["state"],
            json!("running")
        );
        services.jobs.kill(&users_job).unwrap();
        job_status(&services, &users_job, true).await;
    }

    fn agent() -> Principal {
        Principal::Agent {
            thread_id: "01JAGENT".into(),
            client: "omp".into(),
        }
    }

    // spec: AGT-072, DMN-161, AGT-053
    #[test]
    fn only_an_agents_and_an_mcp_clients_calls_beyond_reading_are_logged() {
        let owner = Principal::Owner {
            client: "omfp/web".into(),
        };
        let mcp = Principal::Mcp {
            client: "claude-code".into(),
        };
        let kill = Call::JobsKill(JobRef {
            job_id: "01JOB".into(),
        });
        let read = Call::ThreadState(ThreadRef {
            thread_id: "01JT".into(),
        });

        assert_eq!(pending(&owner, &kill), None);
        assert_eq!(pending(&agent(), &read), None);
        assert_eq!(pending(&mcp, &read), None);
        let entry = pending(&agent(), &kill).expect("logged");
        assert_eq!(entry.thread_id.as_deref(), Some("01JAGENT"));
        assert_eq!(entry.client, "omp");
        assert_eq!(entry.method, "jobs.kill");
        assert_eq!(entry.params, json!({ "jobId": "01JOB" }));
        assert_eq!(entry.error, None);
        let entry = pending(&mcp, &kill).expect("logged");
        assert_eq!(entry.thread_id, None);
        assert_eq!(entry.client, "claude-code");
        assert_eq!(entry.method, "jobs.kill");
    }

    // spec: DMN-161
    #[test]
    fn long_strings_of_logged_params_are_cut_to_200_characters() {
        let content = "é".repeat(300);
        let write = Call::WorkspacesWriteFile(WriteFileParams {
            workspace_id: "01JW".into(),
            path: "notes.md".into(),
            content: content.clone(),
            expected_hash: None,
        });
        let entry = pending(&agent(), &write).expect("logged");
        let cut = entry.params["content"].as_str().unwrap();
        assert_eq!(cut.chars().count(), 201);
        assert!(cut.ends_with('…'));
        assert!(content.starts_with(cut.trim_end_matches('…')));
        assert_eq!(entry.params["path"], json!("notes.md"));

        let prompt = Call::ThreadPrompt(PromptParams {
            thread_id: "01JT".into(),
            text: "x".repeat(200),
            images: Vec::new(),
            behavior: None,
        });
        assert_eq!(
            pending(&agent(), &prompt).expect("logged").params["text"],
            json!("x".repeat(200))
        );
    }
}
