//! The window tools (AGT-081, AGT-082): `omfp_windows` describes the open windows as the user sees them
//! (`windows.list`, `windows.state`, PRT-123; also `omfp://windows`) and `omfp_window_run` runs a command in one
//! (`windows.run`, PRT-124): the agent's way to check what its views, init files and widgets do in a real window.

use std::fmt::Write as _;

use omfp_protocol::{
    Call, CommandSource, ListWindowsParams, RanInWindow, RunInWindowParams, ViewSource,
    WidgetReportState, WindowCommand, WindowList, WindowRef, WindowState, WindowView,
};
use serde_json::{Map, Value, json};

use super::mounts::{required, run_call, text};
use super::{AgentTool, ToolContext, ToolFuture};
use crate::wire_name;

/// Name of the tool that describes the open windows (AGT-081).
pub const WINDOWS: &str = "omfp_windows";
/// Name of the tool that runs a command in a window (AGT-082).
pub const WINDOW_RUN: &str = "omfp_window_run";

/// The tools of this module, for the registry.
pub(crate) fn tools() -> Vec<AgentTool> {
    vec![windows(), window_run()]
}

/// A view's source in words (PRT-122).
pub(crate) fn view_source_name(source: ViewSource) -> &'static str {
    match source {
        ViewSource::Builtin => "the built-in default view",
        ViewSource::ViewsJson => "views.json",
        ViewSource::UserInit => "the user's init file",
        ViewSource::WorkspaceInit => "the workspace's init file",
    }
}

const WINDOW_ID: &str = "A window id from omfp_windows (default: the most recently focused window showing the workspace).";
const WORKSPACE_ID: &str =
    "The workspace (default: the calling thread's; without one, every window).";

fn windows() -> AgentTool {
    AgentTool {
        name: WINDOWS,
        label: "Windows",
        description: "Describes the open omfp windows showing the workspace, the most recently focused first, as the \
                      user sees them: each window's id, focus and thread; its views after views.json, the init files \
                      and their transforms (where each comes from, which widgets, the active one); the commands init \
                      files and widgets added, with their keys; and the errors it shows (init files, widgets). Read \
                      it after changing views, an init file or a widget (with omfp_widget_check), and to find \
                      the command or view to pass omfp_window_run.",
        parameters: json!({
            "type": "object",
            "properties": {
                "windowId": { "type": "integer", "minimum": 1, "description": "Describe only this window." },
                "workspaceId": { "type": "string", "minLength": 1, "description": WORKSPACE_ID }
            },
            "additionalProperties": false
        }),
        run: run_windows,
    }
}

fn window_run() -> AgentTool {
    AgentTool {
        name: WINDOW_RUN,
        label: "Run in window",
        description: "Runs a command in an open omfp window, as the user's `:<command> <args>` would: a registered \
                      command id or alias with string args, never JavaScript. Built-ins such as view.switch (args: \
                      the view id), view.next, window.theme (args: a colour scheme and/or a built-in theme), or a command an init file or widget registered \
                      (omfp_windows lists them, omfp://keymap the built-ins). E.g. { command: \"view.switch\", args: \
                      [\"ops\"] } shows the view ops so you can check it. Default window: the most recently focused \
                      one showing the workspace. Answers whether the window ran it (unknown command, disabled, \
                      failed); read omfp_windows afterwards to see the result.",
        parameters: json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "minLength": 1, "description": "A command id or alias, e.g. view.switch or view." },
                "args": { "type": "array", "items": { "type": "string" }, "description": "The command's arguments, e.g. [\"ops\"]." },
                "windowId": { "type": "integer", "minimum": 1, "description": WINDOW_ID },
                "workspaceId": { "type": "string", "minLength": 1, "description": WORKSPACE_ID }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
        run: run_window_run,
    }
}

/// The optional window id argument.
fn window_id(arguments: &Map<String, Value>) -> Result<Option<u64>, String> {
    match arguments.get("windowId") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| "`windowId` must be a window id from omfp_windows".to_owned()),
        Some(_) => Err("`windowId` must be a window id from omfp_windows".to_owned()),
    }
}

/// The optional `args` argument: strings.
fn string_args(arguments: &Map<String, Value>) -> Result<Vec<String>, String> {
    let invalid = || "`args` must be an array of strings".to_owned();
    match arguments.get("args") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned).ok_or_else(invalid))
            .collect(),
        Some(_) => Err(invalid()),
    }
}

fn run_windows(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        if let Some(id) = window_id(&arguments)? {
            let state: WindowState =
                run_call(&context, Call::WindowsState(WindowRef { window_id: id })).await?;
            let mut answer = String::new();
            describe_window(&mut answer, &state);
            return Ok(answer);
        }
        let workspace = text(&arguments, "workspaceId")?.or_else(|| context.workspace_id.clone());
        let list: WindowList = run_call(
            &context,
            Call::WindowsList(ListWindowsParams { workspace_id: None }),
        )
        .await?;
        let mut states = Vec::new();
        for summary in &list.windows {
            if workspace.is_some() && summary.workspace_id != workspace {
                continue;
            }
            // A window closing meanwhile is left out.
            let window = WindowRef {
                window_id: summary.window_id,
            };
            if let Ok(state) = run_call(&context, Call::WindowsState(window)).await {
                states.push(state);
            }
        }
        let others = list.windows.len().saturating_sub(states.len());
        Ok(describe_windows(workspace.as_deref(), &states, others))
    })
}

fn run_window_run(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        let params = RunInWindowParams {
            window_id: window_id(&arguments)?,
            workspace_id: text(&arguments, "workspaceId")?.or_else(|| context.workspace_id.clone()),
            command: required(&arguments, "command")?,
            args: string_args(&arguments)?,
        };
        let shown = if params.args.is_empty() {
            params.command.clone()
        } else {
            format!("{} {:?}", params.command, params.args)
        };
        let ran: RanInWindow = run_call(&context, Call::WindowsRun(params)).await?;
        Ok(format!(
            "Window {} ran {shown} (command {}). omfp_windows {{ \"windowId\": {} }} shows what the window shows now.",
            ran.window_id, ran.command, ran.window_id
        ))
    })
}

/// The answer of `omfp_windows` and `omfp://windows` (AGT-081): the windows showing `workspace` (every window without
/// one), the most recently focused first; `others` windows show something else.
pub(crate) fn describe_windows(
    workspace: Option<&str>,
    states: &[WindowState],
    others: usize,
) -> String {
    let mut text = String::new();
    if states.is_empty() {
        if others == 0 {
            text.push_str(
                "No omfp window is open: no window runs this workspace's views, init files or widgets now. \
                 omfp_widget_check still compiles them; the user sees them once they open omfp.\n",
            );
        } else {
            let _ = writeln!(
                text,
                "No omfp window shows this workspace; {others} window(s) show another workspace or none (omfp_windows \
                 {{ \"windowId\": <id> }} describes one)."
            );
        }
        return text;
    }
    let _ = write!(
        text,
        "{} window(s) {}, the most recently focused first",
        states.len(),
        if workspace.is_some() {
            "show this workspace"
        } else {
            "are open"
        }
    );
    if others > 0 {
        let _ = write!(
            text,
            "; {others} other window(s) show another workspace or none"
        );
    }
    text.push_str(".\n");
    for state in states {
        text.push('\n');
        describe_window(&mut text, state);
    }
    text
}

fn describe_view(text: &mut String, view: &WindowView, active: bool, default: bool) {
    let mut from = view_source_name(view.source).to_owned();
    if default {
        from.push_str(", the default view");
    }
    let widgets = if view.widgets.is_empty() {
        "no widgets".to_owned()
    } else {
        view.widgets.join(", ")
    };
    let _ = writeln!(
        text,
        "  {} {} {:?} ({from}): {widgets}",
        if active { "*" } else { "-" },
        view.id,
        view.title
    );
}

fn describe_command(text: &mut String, command: &WindowCommand) {
    let from = match command.source {
        CommandSource::UserInit => "the user's init file".to_owned(),
        CommandSource::WorkspaceInit => "the workspace's init file".to_owned(),
        CommandSource::Widget => format!(
            "widget {}",
            command.widget.as_deref().unwrap_or("(unknown)")
        ),
    };
    let _ = write!(text, "  - {} {:?} ({from}", command.id, command.title);
    if !command.aliases.is_empty() {
        let _ = write!(text, "; aliases {}", command.aliases.join(", "));
    }
    if command.bindings.is_empty() {
        text.push_str("; no keys");
    } else {
        let keys: Vec<String> = command
            .bindings
            .iter()
            .map(|binding| {
                let mut key = format!("{} ({})", binding.keys, wire_name(binding.mode));
                if !binding.args.is_empty() {
                    let _ = write!(key, " with {:?}", binding.args);
                }
                key
            })
            .collect();
        let _ = write!(text, "; keys {}", keys.join(", "));
    }
    if !command.enabled {
        text.push_str("; disabled now");
    }
    text.push_str(")\n");
}

/// One window, as `omfp_windows` describes it (AGT-081).
fn describe_window(text: &mut String, state: &WindowState) {
    let _ = write!(text, "Window {} ({})", state.window_id, state.client);
    if state.focused {
        text.push_str(", focused");
    }
    let Some(workspace) = &state.workspace_id else {
        text.push_str(": shows no workspace, so no views, init file commands or errors.\n");
        return;
    };
    let _ = writeln!(
        text,
        ": workspace {workspace}, {}",
        state.thread_id.as_deref().map_or_else(
            || "no thread selected".to_owned(),
            |thread| format!("thread {thread}")
        )
    );
    if state.views.is_empty() {
        text.push_str("Views: none reported yet.\n");
    } else {
        text.push_str("Views (switcher order; * the one it shows):\n");
        for view in &state.views {
            describe_view(
                text,
                view,
                state.active_view.as_deref() == Some(view.id.as_str()),
                state.default_view.as_deref() == Some(view.id.as_str()),
            );
        }
    }
    if state.commands.is_empty() {
        text.push_str("Commands added by init files and widgets: none.\n");
    } else {
        text.push_str("Commands added by init files and widgets:\n");
        for command in &state.commands {
            describe_command(text, command);
        }
    }
    if !state.widgets.is_empty() {
        let widgets: Vec<String> = state
            .widgets
            .iter()
            .map(|widget| format!("{} ({})", widget.instance, state_name(widget.state)))
            .collect();
        let _ = writeln!(
            text,
            "Workspace and user widgets in the view it shows: {}",
            widgets.join(", ")
        );
    }
    let mut errors: Vec<String> = state
        .file_errors
        .iter()
        .map(|error| {
            let file = if error.file == "user" {
                "the user's init file"
            } else {
                error.file.as_str()
            };
            format!("{file}: {}", error.message)
        })
        .collect();
    for widget in &state.widgets {
        for error in &widget.errors {
            errors.push(format!("{}: {}", widget.instance, error));
        }
        for error in &widget.console {
            errors.push(format!("{} (console): {}", widget.instance, error));
        }
    }
    if errors.is_empty() {
        text.push_str("Errors: none.\n");
    } else {
        text.push_str("Errors:\n");
        for error in errors {
            let mut lines = error.lines();
            let _ = writeln!(text, "  - {}", lines.next().unwrap_or_default());
            for line in lines {
                let _ = writeln!(text, "    {line}");
            }
        }
    }
}

/// A widget's state as agents read it (`compile error`, …).
pub(crate) fn state_name(state: WidgetReportState) -> &'static str {
    match state {
        WidgetReportState::Loading => "loading",
        WidgetReportState::Rendered => "rendered",
        WidgetReportState::CompileError => "compile error",
        WidgetReportState::LoadError => "load error",
        WidgetReportState::RenderError => "render error",
    }
}

#[cfg(test)]
mod tests {
    use omfp_protocol::{
        AnswerWindowParams, FileErrorReport, ReportWidgetsParams, ServerFrame, ViewSource,
        WidgetReport, WidgetSource, WindowAnswer, WindowBinding, WindowReport, WindowRequest,
    };

    use super::super::tests::{Harness, args};
    use super::*;
    use crate::server::Outbound;

    fn report(workspace: Option<&str>, window: WindowReport) -> ReportWidgetsParams {
        ReportWidgetsParams {
            workspace_id: workspace.map(str::to_owned),
            views: None,
            widgets: Vec::new(),
            file_errors: Vec::new(),
            window: Some(window),
        }
    }

    fn shown_window(harness: &Harness) -> ReportWidgetsParams {
        let mut shown = report(
            Some(&harness.workspace),
            WindowReport {
                thread_id: Some(harness.thread.clone()),
                focused: true,
                active_view: Some("ops".to_owned()),
                default_view: Some("default".to_owned()),
                views: vec![
                    WindowView {
                        id: "default".to_owned(),
                        title: "Default".to_owned(),
                        source: ViewSource::ViewsJson,
                        widgets: vec!["omfp:chat".to_owned()],
                    },
                    WindowView {
                        id: "ops".to_owned(),
                        title: "Ops".to_owned(),
                        source: ViewSource::WorkspaceInit,
                        widgets: vec!["omfp:chat".to_owned(), "./widgets/tests".to_owned()],
                    },
                    WindowView {
                        id: "review".to_owned(),
                        title: "Review".to_owned(),
                        source: ViewSource::UserInit,
                        widgets: Vec::new(),
                    },
                ],
                commands: vec![WindowCommand {
                    id: "demo.solo".to_owned(),
                    title: "Show the init view".to_owned(),
                    aliases: vec!["solo".to_owned()],
                    source: CommandSource::WorkspaceInit,
                    widget: None,
                    enabled: true,
                    bindings: vec![WindowBinding {
                        mode: omfp_protocol::KeymapMode::Normal,
                        keys: "g h".to_owned(),
                        args: Vec::new(),
                    }],
                }],
            },
        );
        shown.widgets = vec![WidgetReport {
            reference: "./widgets/tests".to_owned(),
            instance: "./widgets/tests".to_owned(),
            source: WidgetSource::Workspace,
            name: "tests".to_owned(),
            version: Some("1".to_owned()),
            state: WidgetReportState::RenderError,
            errors: vec!["x is not defined".to_owned()],
            console: Vec::new(),
            text: String::new(),
            jobs: Vec::new(),
        }];
        shown.file_errors = vec![FileErrorReport {
            file: ".omfp/init.ts".to_owned(),
            message: "TypeError: boom".to_owned(),
        }];
        shown
    }

    // spec: AGT-081, PRT-123
    #[tokio::test]
    async fn omfp_windows_describes_each_window_showing_the_workspace() {
        let harness = Harness::new().await;
        let context = harness.context();
        let answer = run_windows(context.clone(), args(json!({}))).await.unwrap();
        assert!(answer.starts_with("No omfp window is open"), "{answer}");

        harness
            .services
            .report_widgets(2, "web", report(None, WindowReport::default()));
        let answer = run_windows(context.clone(), args(json!({}))).await.unwrap();
        assert!(
            answer.starts_with(
                "No omfp window shows this workspace; 1 window(s) show another workspace or none"
            ),
            "{answer}"
        );
        assert_eq!(
            run_windows(context.clone(), args(json!({ "windowId": 2 })))
                .await
                .unwrap(),
            "Window 2 (web): shows no workspace, so no views, init file commands or errors.\n"
        );

        harness
            .services
            .report_widgets(3, "desktop", shown_window(&harness));
        let answer = run_windows(context.clone(), args(json!({}))).await.unwrap();
        let expected = format!(
            "1 window(s) show this workspace, the most recently focused first; 1 other window(s) show another \
             workspace or none.\n\n\
             Window 3 (desktop), focused: workspace {}, thread {}\n\
             Views (switcher order; * the one it shows):\n\
             \x20 - default \"Default\" (views.json, the default view): omfp:chat\n\
             \x20 * ops \"Ops\" (the workspace's init file): omfp:chat, ./widgets/tests\n\
             \x20 - review \"Review\" (the user's init file): no widgets\n\
             Commands added by init files and widgets:\n\
             \x20 - demo.solo \"Show the init view\" (the workspace's init file; aliases solo; keys g h (normal))\n\
             Workspace and user widgets in the view it shows: ./widgets/tests (render error)\n\
             Errors:\n\
             \x20 - .omfp/init.ts: TypeError: boom\n\
             \x20 - ./widgets/tests: x is not defined\n",
            harness.workspace, harness.thread
        );
        assert_eq!(answer, expected);
        let error = run_windows(context, args(json!({ "windowId": 9 })))
            .await
            .unwrap_err();
        assert!(error.starts_with("not_found: no omfp window 9"), "{error}");
    }

    // spec: AGT-082, DMN-206
    #[tokio::test]
    async fn omfp_window_run_runs_a_command_in_the_focused_window_and_says_why_it_did_not() {
        let harness = Harness::new().await;
        let context = harness.context();
        let error = run_window_run(context.clone(), args(json!({ "command": "view" })))
            .await
            .unwrap_err();
        assert_eq!(error, "not_found: no omfp window is open");

        harness
            .services
            .report_widgets(3, "web", shown_window(&harness));
        let (out, mut frames) = tokio::sync::mpsc::channel(8);
        harness.services.link_window(3, out);
        let services = harness.services.clone();
        let window = tokio::spawn(async move {
            let mut asked = Vec::new();
            while let Some(Outbound::Text(text)) = frames.recv().await {
                let Ok(ServerFrame::WindowRequest { id, request }) = serde_json::from_str(&text)
                else {
                    continue;
                };
                let answer = match &request {
                    WindowRequest::RunCommand { command, .. } if command == "view" => {
                        WindowAnswer::Ran {
                            command: "view.switch".to_owned(),
                        }
                    }
                    WindowRequest::RunCommand { command, .. } if command == "view.next" => {
                        WindowAnswer::NotRun {
                            reason: omfp_protocol::RunFailure::Disabled,
                            message: "view.next is disabled now".to_owned(),
                        }
                    }
                    _ => WindowAnswer::NotRun {
                        reason: omfp_protocol::RunFailure::Unknown,
                        message: "no command `nope`".to_owned(),
                    },
                };
                asked.push(request);
                let params = AnswerWindowParams {
                    request_id: id,
                    answer,
                };
                services.answer_window(3, params).unwrap();
            }
            asked
        });

        let answer = run_window_run(
            context.clone(),
            args(json!({ "command": "view", "args": ["ops"] })),
        )
        .await
        .unwrap();
        assert_eq!(
            answer,
            "Window 3 ran view [\"ops\"] (command view.switch). omfp_windows { \"windowId\": 3 } shows what the \
             window shows now."
        );
        let error = run_window_run(context.clone(), args(json!({ "command": "view.next" })))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            "conflict: omfp window 3 (web) refused it: view.next is disabled now"
        );
        let error = run_window_run(context.clone(), args(json!({ "command": "nope" })))
            .await
            .unwrap_err();
        assert_eq!(error, "not_found: omfp window 3 (web): no command `nope`");
        let error = run_window_run(context, args(json!({ "command": "view", "args": [1] })))
            .await
            .unwrap_err();
        assert_eq!(error, "`args` must be an array of strings");

        harness.services.forget_widget_reports(3);
        assert_eq!(
            window.await.unwrap(),
            [
                WindowRequest::RunCommand {
                    command: "view".to_owned(),
                    args: vec!["ops".to_owned()]
                },
                WindowRequest::RunCommand {
                    command: "view.next".to_owned(),
                    args: Vec::new()
                },
                WindowRequest::RunCommand {
                    command: "nope".to_owned(),
                    args: Vec::new()
                },
            ]
        );
    }
}
