//! The check tools (DMN-063, DMN-196): `omfp_widget_check` checks a widget or an init file against the daemon's
//! compile and what the open windows report, and `omfp_views_errors` lists the errors windows report for init files;
//! registry tools, so threads (host tools) and MCP clients get the same.

use serde_json::{Map, Value, json};

use super::mounts::{required, workspace};
use super::{AgentTool, ToolContext, ToolFuture};
use crate::server::{CHECK_WAIT, authorize};

/// Name of the tool that checks a widget (DMN-063).
pub const WIDGET_CHECK: &str = "omfp_widget_check";
/// Name of the tool that lists the init file errors windows report (DMN-196).
pub const VIEWS_ERRORS: &str = "omfp_views_errors";

/// The tools of this module, for the registry.
pub(crate) fn tools() -> Vec<AgentTool> {
    vec![widget_check(), views_errors()]
}

const WORKSPACE_ID: &str = "The workspace (default: the calling thread's; required without one, e.g. over MCP with a user token).";

fn widget_check() -> AgentTool {
    AgentTool {
        name: WIDGET_CHECK,
        label: "Check widget",
        description: "Checks a workspace widget or an init file (`init`, `user:init`) without a browser: the \
                      daemon compiles it as it serves it, then reports what every open omfp window showing the \
                      workspace shows of it (for a widget: render, import and console errors, the rendered text, its \
                      jobs; for an init file: the errors the windows report running it and its views). A widget no \
                      window shows in its current view is rendered out of sight in the most recently focused one, \
                      and the answer says which views hold it. Use it after writing or changing one.",
        parameters: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "minLength": 1,
                    "description": "A widget: its folder name (`tests` for .omfp/widgets/tests/) or `./widgets/<name>`. An init file: `init` (or `.omfp/init.ts`, the workspace's) or `user:init` (the user's)."
                },
                "workspaceId": { "type": "string", "minLength": 1, "description": WORKSPACE_ID }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
        run: run_widget_check,
    }
}

fn views_errors() -> AgentTool {
    AgentTool {
        name: VIEWS_ERRORS,
        label: "View errors",
        description: "Lists the errors the open omfp windows showing the workspace report for its init files (the \
                      workspace's .omfp/init.ts and the user's init file): compile errors, throws, the layout errors \
                      of the views they add, window by window, after they caught up with the latest change. Use it \
                      after changing an init file; omfp_widget_check { name: \"init\" } checks one file.",
        parameters: json!({
            "type": "object",
            "properties": {
                "workspaceId": { "type": "string", "minLength": 1, "description": WORKSPACE_ID }
            },
            "additionalProperties": false
        }),
        run: run_views_errors,
    }
}

fn run_widget_check(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        // It renders in a window (`windows.check_widget`, DMN-207), so it needs that method's permission.
        authorize(&context.principal, "windows.check_widget")
            .map_err(|error| super::error_text(&error))?;
        let workspace_id = workspace(&context, &arguments)?;
        let name = required(&arguments, "name")?;
        context
            .services
            .check_widget(&workspace_id, &name, &|| false, CHECK_WAIT)
            .await
    })
}

fn run_views_errors(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        authorize(&context.principal, "views.errors").map_err(|error| super::error_text(&error))?;
        let workspace_id = workspace(&context, &arguments)?;
        context
            .services
            .describe_view_errors(&workspace_id, &|| false, CHECK_WAIT)
            .await
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Harness, args};
    use super::*;
    use crate::server::Principal;

    // spec: DMN-063, DMN-196, AGT-066
    #[tokio::test]
    async fn the_checks_answer_for_the_callers_workspace_and_need_one_over_mcp() {
        let harness = Harness::new().await;
        let context = harness.context();
        let unknown = run_widget_check(context.clone(), args(json!({ "name": "nothing" })))
            .await
            .unwrap_err();
        assert!(unknown.contains("no widget nothing"), "{unknown}");
        let errors = run_views_errors(context, args(json!({}))).await.unwrap();
        assert!(
            errors.starts_with("No omfp window shows this workspace now"),
            "{errors}"
        );

        let mcp = ToolContext {
            services: harness.services.clone(),
            principal: Principal::Mcp {
                client: "claude".to_owned(),
            },
            workspace_id: None,
            tool_call_id: None,
        };
        for run in [run_widget_check, run_views_errors] {
            let error = run(mcp.clone(), args(json!({ "name": "tests" })))
                .await
                .unwrap_err();
            assert!(error.contains("`workspaceId` is required"), "{error}");
        }
        let error = run_widget_check(mcp, args(json!({ "workspaceId": harness.workspace })))
            .await
            .unwrap_err();
        assert!(
            error.contains("`name` must be a non-empty string"),
            "{error}"
        );
    }
}
