//! The agent tool registry (AGT-040): every M10 agent tool defined once — name, label, description, argument schema
//! and an async body over the daemon's services — for the adapters that hand them to agents: the RPC host tools of
//! every thread ([`rpc`], DMN-151) and the MCP server.

pub(crate) mod cards;
pub mod catalog;
pub(crate) mod checks;
#[cfg(test)]
pub(crate) mod mentions;
pub(crate) mod mounts;
mod omfp_call;
pub(crate) mod rpc;
mod wait;
pub(crate) mod windows;

use std::pin::Pin;

use omfp_protocol::{Call, ErrorBody};
use serde_json::{Map, Value};

use crate::server::dispatch::dispatch;
use crate::server::{Principal, Services, authorize};

/// Name of the tool that calls any protocol method (AGT-041).
pub const CALL: &str = "omfp_call";

/// Name of the tool that waits for a thread (AGT-043).
pub const THREAD_WAIT: &str = "omfp_thread_wait";

/// The methods that need a window's connection: `omfp_call` refuses them and the catalogue leaves them out (AGT-041,
/// AGT-042).
pub const CONNECTION_ONLY: &[&str] = &[
    "system.hello",
    "subscribe",
    "unsubscribe",
    "watch",
    "widgets.report",
    "windows.answer",
    "thread.live_connect",
    "thread.live_start",
];

/// Why agents cannot call `method`, naming the catalogue; `None` when they can (AGT-041, AGT-042).
pub(crate) fn not_for_agents(method: &str) -> Option<String> {
    if !CONNECTION_ONLY.contains(&method) {
        return None;
    }
    Some(format!(
        "`{method}` needs a window's connection, so agents cannot call it; {} lists the methods {CALL} accepts",
        catalog::METHODS_URL
    ))
}

/// What a tool runs with: the daemon's services, the principal it acts as (DMN-150), the calling thread's workspace
/// (`None` when no thread calls, e.g. MCP with a user token) and the id of the agent's tool call (`None` outside an
/// RPC host tool call).
#[derive(Clone)]
pub(crate) struct ToolContext {
    pub services: Services,
    pub principal: Principal,
    pub workspace_id: Option<String>,
    pub tool_call_id: Option<String>,
}

impl ToolContext {
    /// The calling thread, when the principal is a thread's agent.
    pub fn thread_id(&self) -> Option<&str> {
        match &self.principal {
            Principal::Agent { thread_id, .. } => Some(thread_id),
            Principal::Owner { .. } | Principal::Mcp { .. } => None,
        }
    }
}

/// A running tool body: its text answer, or the text of a tool error. Cancelling a call drops the future.
pub(crate) type ToolFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// One tool of the registry (AGT-040).
#[derive(Clone)]
pub(crate) struct AgentTool {
    pub name: &'static str,
    /// Short name shown on the tool's card.
    pub label: &'static str,
    /// What the model reads about the tool.
    pub description: &'static str,
    /// JSON Schema of the arguments object.
    pub parameters: Value,
    pub run: fn(ToolContext, Map<String, Value>) -> ToolFuture,
}

/// Every tool of the registry, in the order agents see them.
pub(crate) fn registry() -> Vec<AgentTool> {
    let mut tools = vec![omfp_call::tool(), wait::tool()];
    tools.extend(mounts::tools());
    tools.push(cards::tool());
    tools.extend(checks::tools());
    tools.extend(windows::tools());
    tools
}

/// Runs a protocol method as `context`'s principal: the single authorization check (PRT-034), then the handlers a
/// window's request reaches (AGT-040).
pub(crate) async fn call(context: &ToolContext, call: Call) -> Result<Value, ErrorBody> {
    authorize(&context.principal, call.method())?;
    dispatch(&context.services, &context.principal, call).await
}

/// A failed method as a tool error (AGT-041): `<code>: <message>`, omp's code and the details when there are some.
pub(crate) fn error_text(error: &ErrorBody) -> String {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|code| code.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{:?}", error.code));
    let omp_code = error
        .omp_code
        .as_ref()
        .map(|omp_code| format!(" (omp code {omp_code})"))
        .unwrap_or_default();
    let details = error
        .details
        .as_ref()
        .map(|details| format!("\ndetails: {details}"))
        .unwrap_or_default();
    format!("{code}: {}{omp_code}{details}", error.message)
}

#[cfg(test)]
pub(super) mod tests {
    use serde_json::{Map, Value};
    use tempfile::TempDir;

    use super::*;
    use crate::agent::FakeLauncher;
    use crate::test_support::Stack;
    use crate::threads::NewThread;
    use crate::workspaces::NewWorkspace;

    /// The tool call id of [`Harness::context`].
    pub const TOOL_CALL_ID: &str = "toolu_01";

    /// Services on the fake launcher with one workspace and one thread in it, the caller.
    pub struct Harness {
        _dir: TempDir,
        pub services: Services,
        pub launcher: FakeLauncher,
        pub workspace: String,
        pub thread: String,
    }

    impl Harness {
        pub async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let launcher = FakeLauncher::new();
            let services = Stack::with_launcher(dir.path(), &launcher)
                .services(None)
                .await;
            let workspace = services
                .workspaces
                .create(NewWorkspace::new())
                .unwrap()
                .workspace
                .id;
            let thread = services
                .threads
                .create(NewThread::new("caller", workspace.clone()))
                .unwrap()
                .id;
            Self {
                _dir: dir,
                services,
                launcher,
                workspace,
                thread,
            }
        }

        /// The context of the caller thread's host tools, in its tool call [`TOOL_CALL_ID`].
        pub fn context(&self) -> ToolContext {
            ToolContext {
                services: self.services.clone(),
                principal: Principal::Agent {
                    thread_id: self.thread.clone(),
                    client: "omp".to_owned(),
                },
                workspace_id: Some(self.workspace.clone()),
                tool_call_id: Some(TOOL_CALL_ID.to_owned()),
            }
        }
    }

    pub fn args(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    // spec: AGT-040
    #[test]
    fn the_registry_starts_with_omfp_call_and_omfp_thread_wait_and_every_tool_has_an_object_schema()
    {
        let tools = registry();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name).collect();
        assert_eq!(names[..2], [CALL, THREAD_WAIT]);
        for tool in &tools {
            assert_eq!(tool.parameters["type"], "object", "{}", tool.name);
            assert!(!tool.description.is_empty(), "{}", tool.name);
            assert!(!tool.label.is_empty(), "{}", tool.name);
        }
    }

    // spec: AGT-024, AGT-029, AGT-045, DMN-062, DMN-198
    #[test]
    fn every_tool_and_uri_a_thread_reads_about_exists_for_that_thread() {
        use crate::{conventions, host_tools};

        let host = host_tools::thread_tools(
            std::sync::Arc::new(|_, _| Ok(())),
            std::sync::Arc::new(|_, _| Ok(())),
            std::sync::Arc::new(|_| Ok(())),
            Vec::new(),
        );
        let registry = registry();
        let tools: Vec<&str> = host
            .iter()
            .map(|tool| tool.definition().name.as_str())
            .chain(registry.iter().map(|tool| tool.name))
            .collect();
        let uris: Vec<&str> = mentions::STATIC_URIS
            .into_iter()
            .chain(mentions::THREAD_URIS)
            .collect();
        let mut texts = mentions::static_texts();
        texts.push((
            "the appended prompt".to_owned(),
            conventions::daemon_text("w", "01WORKSPACE", std::path::Path::new("/ws"), "01THREAD"),
        ));
        for tool in &host {
            let definition = tool.definition();
            texts.push((
                definition.name.clone(),
                format!("{} {:?}", definition.description, definition.parameters),
            ));
        }
        for tool in &registry {
            texts.push((
                tool.name.to_owned(),
                format!("{} {}", tool.description, tool.parameters),
            ));
        }
        let scheme = host_tools::host_uris(None, None).unwrap();
        texts.push((
            "the omfp:// scheme".to_owned(),
            scheme[0]
                .definition()
                .description
                .clone()
                .unwrap_or_default(),
        ));
        let missing: Vec<String> = texts
            .iter()
            .flat_map(|(served, text)| mentions::missing(served, text, &tools, &uris))
            .collect();
        assert_eq!(missing, Vec::<String>::new());

        // The widget builder, a subagent, gets no omfp tool, only the URIs (S-14).
        let builder = conventions::widget_builder_definition();
        let uris: Vec<&str> = mentions::STATIC_URIS
            .into_iter()
            .chain([crate::widgets::CHECK_URL])
            .collect();
        assert_eq!(
            mentions::missing("widget-builder", &builder, &[], &uris),
            Vec::<String>::new()
        );
    }
}
