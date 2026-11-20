//! The RPC host tool adapter (DMN-151): every registry tool (AGT-040) as an essential host tool of a thread, acting
//! as that thread's agent principal (DMN-150).

use std::time::Duration;

use omp_rpc::{HostTool, HostToolOutput};
use tokio::runtime::Handle;

use super::{AgentTool, ToolContext, ToolFuture, registry};
use crate::host_tools::AgentHost;
use crate::server::{Principal, Services};

/// How often a running tool looks at the cancellation flag (DMN-151).
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// The client agents' calls record as provenance (PRT-033).
pub const CLIENT: &str = "omp";

/// The server's [`AgentHost`]: runs the registry's tools on the daemon's runtime from the `omp-rpc` handler thread
/// a host tool runs on.
pub(crate) struct ServicesAgentHost {
    services: Services,
    runtime: Handle,
}

impl ServicesAgentHost {
    /// Must be called on the daemon's runtime.
    pub(crate) fn new(services: Services) -> Self {
        Self {
            services,
            runtime: Handle::current(),
        }
    }
}

impl AgentHost for ServicesAgentHost {
    fn tools(&self, thread_id: &str, workspace_id: &str) -> Vec<HostTool> {
        let context = ToolContext {
            services: self.services.clone(),
            principal: Principal::Agent {
                thread_id: thread_id.to_owned(),
                client: CLIENT.to_owned(),
            },
            workspace_id: Some(workspace_id.to_owned()),
            tool_call_id: None,
        };
        registry()
            .iter()
            .map(|tool| host_tool(tool, context.clone(), self.runtime.clone()))
            .collect()
    }
}

/// `tool` as a host tool running with `context`, in the agent's tool call, on `runtime`: the handler thread blocks on
/// the body, which is dropped once omp cancels the call.
fn host_tool(tool: &AgentTool, context: ToolContext, runtime: Handle) -> HostTool {
    let parameters = tool.parameters.as_object().cloned().unwrap_or_default();
    let run = tool.run;
    HostTool::new(
        tool.name,
        tool.description,
        parameters,
        move |arguments, call| {
            let context = ToolContext {
                tool_call_id: Some(call.tool_call_id().to_owned()),
                ..context.clone()
            };
            let body = run(context, arguments);
            runtime
                .block_on(until_cancelled(body, || call.is_cancelled()))
                .map(HostToolOutput::Text)
                .map_err(Into::into)
        },
    )
    .label(tool.label)
}

/// The body's answer, or `cancelled` once `is_cancelled` says omp cancelled the call (DMN-061, DMN-151).
async fn until_cancelled(
    body: ToolFuture,
    is_cancelled: impl Fn() -> bool,
) -> Result<String, String> {
    let cancelled = async {
        let mut tick = tokio::time::interval(CANCEL_POLL);
        loop {
            tick.tick().await;
            if is_cancelled() {
                return;
            }
        }
    };
    tokio::select! {
        answer = body => answer,
        () = cancelled => Err("cancelled".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use omfp_core::ProcessState;
    use omp_rpc::ToolLoadMode;

    use super::super::tests::Harness;
    use super::super::{CALL, THREAD_WAIT};
    use super::*;
    use crate::host_tools::thread_tools;

    // spec: DMN-151, DMN-106, AGT-040
    #[tokio::test]
    async fn every_thread_gets_the_registry_as_essential_host_tools() {
        let harness = Harness::new().await;
        let host = ServicesAgentHost::new(harness.services.clone());

        let tools = thread_tools(
            Arc::new(|_, _| Ok(())),
            Arc::new(|_, _| Ok(())),
            Arc::new(|_| Ok(())),
            host.tools(&harness.thread, &harness.workspace),
        );
        let registry = registry();
        for tool in &registry {
            let registered = tools
                .iter()
                .map(HostTool::definition)
                .find(|definition| definition.name == tool.name)
                .unwrap_or_else(|| panic!("{} is not registered", tool.name));
            assert_eq!(registered.load_mode, Some(ToolLoadMode::Essential));
            assert_eq!(registered.description, tool.description);
            assert_eq!(registered.label.as_deref(), Some(tool.label));
            assert_eq!(
                serde_json::Value::Object(registered.parameters.clone()),
                tool.parameters
            );
        }
        let names: Vec<&str> = registry.iter().map(|tool| tool.name).collect();
        assert!(names.contains(&CALL) && names.contains(&THREAD_WAIT));
    }

    // spec: DMN-151, DMN-061
    #[tokio::test(start_paused = true)]
    async fn a_cancelled_call_drops_its_body_within_the_poll() {
        let cancelled = AtomicBool::new(false);
        let forever: ToolFuture = Box::pin(std::future::pending());
        let waiting = until_cancelled(forever, || cancelled.load(Ordering::SeqCst));
        tokio::pin!(waiting);
        tokio::select! {
            biased;
            _ = &mut waiting => panic!("not cancelled yet"),
            () = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
        cancelled.store(true, Ordering::SeqCst);
        let started = tokio::time::Instant::now();
        assert_eq!(waiting.await, Err("cancelled".to_owned()));
        assert!(started.elapsed() <= CANCEL_POLL);

        let done: ToolFuture = Box::pin(async { Ok("done".to_owned()) });
        assert_eq!(until_cancelled(done, || false).await, Ok("done".to_owned()));
    }

    // spec: AGT-071, DMN-070, DMN-038, DMN-151
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_stopping_its_own_tree_with_omfp_call_stops_it_although_omp_cancels_the_call()
    {
        let harness = Harness::new().await;
        let supervisor = harness.services.supervisor.clone();
        let tool = registry()
            .into_iter()
            .find(|tool| tool.name == CALL)
            .unwrap();
        let call = |arguments: serde_json::Value| {
            let serde_json::Value::Object(arguments) = arguments else {
                unreachable!()
            };
            (tool.run)(harness.context(), arguments)
        };
        let child: serde_json::Value = serde_json::from_str(
            &call(serde_json::json!({
                "method": "threads.create",
                "params": { "title": "child", "workspace": { "id": harness.workspace } }
            }))
            .await
            .unwrap(),
        )
        .unwrap();
        let child = child["id"].as_str().unwrap().to_owned();
        supervisor.start(&harness.thread).await.unwrap();
        supervisor.start(&child).await.unwrap();
        let close = harness.launcher.gate_closes();

        // Killing the caller's own omp cancels its call (DMN-151): the body is dropped mid-way.
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        let stop = tokio::spawn(until_cancelled(
            call(serde_json::json!({
                "method": "thread.stop",
                "params": { "threadId": harness.thread, "cascade": true }
            })),
            move || flag.load(Ordering::SeqCst),
        ));
        close.arrived(1).await;
        cancelled.store(true, Ordering::SeqCst);
        assert_eq!(stop.await.unwrap(), Err("cancelled".to_owned()));
        close.open();

        // The lifecycle lock orders this stop after the cascade's stop of the caller; the cascade goes on to the child.
        supervisor.stop(&harness.thread).await.unwrap();
        assert_eq!(
            supervisor.status(&harness.thread).process,
            ProcessState::Stopped
        );
        let log = supervisor.thread_log(&child).await.unwrap();
        let mut live = log.subscribe(None).live;
        while log.with_state(|state| state.process != ProcessState::Stopped) {
            live.recv().await.unwrap();
        }
        assert!(harness.launcher.process(0).is_closed());
        assert!(harness.launcher.process(1).is_closed());
    }
}
