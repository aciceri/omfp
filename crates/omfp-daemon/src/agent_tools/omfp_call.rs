//! `omfp_call { method, params? }` (AGT-041): any protocol method, by name, as the calling thread's agent.

use std::fmt::Write as _;

use omfp_protocol::decode_request;
use serde_json::{Map, Value, json};

use super::catalog::METHODS_URL;
use super::{AgentTool, CALL, ToolContext, ToolFuture, call, error_text, not_for_agents};

pub(super) fn tool() -> AgentTool {
    AgentTool {
        name: CALL,
        label: "omfp call",
        description: "Calls any omfp protocol method, as the user's omfp windows do: list, create and drive threads \
                      (threads.list, threads.create, thread.prompt, thread.session, interaction.respond), manage \
                      workspaces and their repositories, read git status and files, run jobs, change views. \
                      omfp://protocol/methods lists every method; omfp://protocol/methods/<name> gives its params \
                      and result JSON Schemas. Answers the method's JSON result.",
        parameters: json!({
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The method's name, e.g. `threads.list` (see omfp://protocol/methods)."
                },
                "params": {
                    "type": "object",
                    "description": "The method's params (see omfp://protocol/methods/<name>); default `{}`."
                }
            },
            "required": ["method"],
            "additionalProperties": false
        }),
        run,
    }
}

/// `thread.*` methods that only read: a missing `threadId` means the calling thread (AGT-041).
const READS_OWN_THREAD: &[&str] = &[
    "thread.session",
    "thread.state",
    "thread.mcp_servers",
    "thread.commands",
    "thread.models",
    "thread.thinking_levels",
    "thread.tree",
    "thread.subagents",
    "thread.subagent_messages",
    "thread.git",
];

/// Methods that only read one workspace: a missing `workspaceId` means the calling thread's workspace (AGT-041).
const READS_OWN_WORKSPACE: &[&str] = &[
    "workspaces.status",
    "workspaces.diff",
    "workspaces.read_file",
    "workspaces.find_file",
    "workspaces.mcp_servers",
    "mcp.tools",
    "jobs.list",
    "jobs.runs",
    "views.errors",
];

fn run(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        let method = match arguments.get("method") {
            Some(Value::String(method)) if !method.trim().is_empty() => method.trim().to_owned(),
            _ => return Err("`method` must be a non-empty string".to_owned()),
        };
        let mut params = match arguments.get("params") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(params)) => params.clone(),
            Some(_) => return Err("`params` must be an object".to_owned()),
        };
        let own_thread = context.thread_id();
        if let Some(own) = own_thread
            && READS_OWN_THREAD.contains(&method.as_str())
        {
            params
                .entry("threadId")
                .or_insert_with(|| Value::String(own.to_owned()));
        }
        let own_workspace = context.workspace_id.as_deref();
        if let Some(own) = own_workspace
            && READS_OWN_WORKSPACE.contains(&method.as_str())
        {
            params
                .entry("workspaceId")
                .or_insert_with(|| Value::String(own.to_owned()));
        }
        if let Some(why) = not_for_agents(&method) {
            return Err(why);
        }
        let frame = json!({ "id": CALL, "type": "request", "method": method, "params": params });
        let request = decode_request(&frame.to_string()).map_err(|error| {
            let mut text = format!("{error}; {METHODS_URL} lists the methods and their params");
            if let Some(own) = own_thread
                && text.contains("missing field `threadId`")
            {
                let _ = write!(text, " (your own thread is {own})");
            }
            if let Some(own) = own_workspace
                && text.contains("missing field `workspaceId`")
            {
                let _ = write!(text, " (your own workspace is {own})");
            }
            text
        })?;
        let result = call(&context, request.call)
            .await
            .map_err(|error| error_text(&error))?;
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests::{Harness, args};
    use super::*;

    async fn call_tool(harness: &Harness, arguments: Value) -> Result<Value, String> {
        let text = run(harness.context(), args(arguments)).await?;
        Ok(serde_json::from_str(&text).expect("a JSON answer"))
    }

    // spec: AGT-041, DMN-150, ARC-009, PRT-090, PRT-091
    #[tokio::test]
    async fn omfp_call_runs_any_method_as_the_calling_threads_agent() {
        let harness = Harness::new().await;

        let created = call_tool(
            &harness,
            json!({
                "method": "threads.create",
                "params": { "title": "child", "workspace": { "id": harness.workspace } }
            }),
        )
        .await
        .unwrap();
        assert_eq!(created["title"], "child");
        assert_eq!(
            created["provenance"],
            json!({ "createdBy": { "kind": "thread", "threadId": harness.thread }, "client": "omp" })
        );

        let listed = call_tool(&harness, json!({ "method": "threads.list" }))
            .await
            .unwrap();
        let titles: Vec<&str> = listed["threads"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|thread| thread["title"].as_str())
            .collect();
        assert_eq!(titles.len(), 2, "{listed}");
        assert!(titles.contains(&"child"), "{listed}");

        let workspaces = call_tool(
            &harness,
            json!({ "method": "workspaces.list", "params": null }),
        )
        .await
        .unwrap();
        assert_eq!(
            workspaces["workspaces"][0]["id"],
            harness.workspace.as_str()
        );
    }

    // spec: AGT-041, PRT-135
    #[tokio::test]
    async fn bad_calls_are_tool_errors_that_say_why() {
        let harness = Harness::new().await;

        for (arguments, reason) in [
            (json!({}), "`method` must be a non-empty string"),
            (
                json!({ "method": " " }),
                "`method` must be a non-empty string",
            ),
            (
                json!({ "method": "threads.list", "params": [] }),
                "`params` must be an object",
            ),
        ] {
            let error = call_tool(&harness, arguments.clone()).await.unwrap_err();
            assert!(error.contains(reason), "{arguments}: {error}");
        }
        // Errors about the method name the catalogue.
        for (arguments, reason) in [
            (
                json!({ "method": "threads.nothing" }),
                "unknown method `threads.nothing`",
            ),
            (
                json!({ "method": "threads.update", "params": { "title": 3 } }),
                "invalid params for `threads.update`",
            ),
            (
                json!({ "method": "subscribe", "params": { "topic": "index" } }),
                "needs a window's connection",
            ),
            (
                json!({ "method": "thread.live_start", "params": { "threadId": harness.thread } }),
                "needs a window's connection",
            ),
            (
                json!({ "method": "terminal.list", "params": {} }),
                "unknown method `terminal.list`",
            ),
        ] {
            let error = call_tool(&harness, arguments.clone()).await.unwrap_err();
            assert!(error.contains(reason), "{arguments}: {error}");
            assert!(error.contains(METHODS_URL), "{arguments}: {error}");
        }

        let error = call_tool(
            &harness,
            json!({ "method": "threads.archive", "params": { "threadId": "01NOPE" } }),
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("not_found: "), "{error}");
    }

    // spec: AGT-041
    #[tokio::test]
    async fn reading_thread_methods_default_to_the_calling_thread() {
        let harness = Harness::new().await;

        let session = call_tool(
            &harness,
            json!({ "method": "thread.session", "params": {} }),
        )
        .await
        .unwrap();
        let explicit = call_tool(
            &harness,
            json!({ "method": "thread.session", "params": { "threadId": harness.thread } }),
        )
        .await
        .unwrap();
        assert_eq!(session, explicit);

        let error = call_tool(
            &harness,
            json!({ "method": "thread.prompt", "params": { "message": "hi" } }),
        )
        .await
        .unwrap_err();
        assert!(error.contains("missing field `threadId`"), "{error}");
        assert!(
            error.contains(&format!("(your own thread is {})", harness.thread)),
            "{error}"
        );
    }

    // spec: AGT-041
    #[tokio::test]
    async fn reading_workspace_methods_default_to_the_calling_threads_workspace() {
        let harness = Harness::new().await;

        for method in ["jobs.list", "workspaces.mcp_servers", "views.errors"] {
            let implicit = call_tool(&harness, json!({ "method": method, "params": {} }))
                .await
                .unwrap();
            let explicit = call_tool(
                &harness,
                json!({ "method": method, "params": { "workspaceId": harness.workspace } }),
            )
            .await
            .unwrap();
            assert_eq!(implicit, explicit, "{method}");
        }

        let error = call_tool(
            &harness,
            json!({ "method": "jobs.run", "params": { "job": "build" } }),
        )
        .await
        .unwrap_err();
        assert!(error.contains("missing field `workspaceId`"), "{error}");
        assert!(
            error.contains(&format!("(your own workspace is {})", harness.workspace)),
            "{error}"
        );
    }
}
