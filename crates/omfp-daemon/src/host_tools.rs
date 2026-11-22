//! Tools the daemon provides to omp (DMN-060..064), registered at launch through `omp-rpc`'s
//! `ClientOptions::tools`, and the `omfp://` host URI scheme next to them (DMN-062).

use std::path::PathBuf;
use std::sync::Arc;

use omp_rpc::{HostTool, HostToolError, HostToolOutput, HostUri, ToolLoadMode};
use serde_json::{Map, Value, json};

use crate::agent_tools::catalog;
use crate::widgets::{self, BUILTIN_URL, CHECK_URL, DOCS_URL, SCHEMA_URL, SCHEME, keymap, shell};

/// Name of the tool that opens a file in omfp for the user (DMN-060).
pub const OPEN_FILE: &str = "omfp_open_file";

/// Name of the tool that sends the user a notification about the thread (DMN-125).
pub const NOTIFY: &str = "omfp_notify";

/// Name of the tool that renames the agent's thread (DMN-073).
pub const SET_THREAD_TITLE: &str = "set_thread_title";

/// Longest title `set_thread_title` accepts, in characters.
pub const MAX_TITLE_CHARS: usize = 200;

/// Longest title `omfp_notify` accepts, in characters (DMN-127).
pub const MAX_NOTIFY_TITLE_CHARS: usize = 100;

/// Longest body `omfp_notify` accepts, in characters (DMN-127).
pub const MAX_NOTIFY_BODY_CHARS: usize = 500;

/// Opens a file (`path` as the agent wrote it, at `line`) in the window the user last used on the thread; the error,
/// e.g. no window shows the thread, goes back to the agent (DMN-060).
pub type Opener = dyn Fn(&str, Option<u64>) -> Result<(), String> + Send + Sync;

/// The [`Opener`] of a thread's `omfp_open_file` tool.
pub type OpenFile = Arc<Opener>;

/// Notifies the user (`title`, `body`) about the thread in the window they last used on it, else in their most
/// recently focused window; the error, e.g. no window is open, goes back to the agent (DMN-125, DMN-126).
pub type Notify = Arc<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>;

/// Notifies the user (`title`, `body`) about thread `threadId` in their most recently focused window, which need not
/// show the thread (DMN-375); `Ok(false)` when no window is connected. The server implements it.
pub type FocusedNotify = Arc<dyn Fn(&str, &str, &str) -> Result<bool, String> + Send + Sync>;

/// Stores a title the agent set for its thread; the error goes back to the agent.
pub type SetTitle = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// What the `omfp://` URIs of a thread's workspace need from the daemon (DMN-108, DMN-199, AGT-081); the server
/// implements it. They run on an `omp-rpc` handler thread and may block it.
pub trait WidgetHost: Send + Sync {
    /// The `omfp_widget_check` answer for `name` of workspace `workspace_id`: a widget, an init file or a TSX view.
    fn check_widget(
        &self,
        workspace_id: &str,
        name: &str,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<String, String>;
    /// What the daemon's config file and the `.omfp/keymap.json` of workspace `workspace_id` add to the default keys.
    fn keymap_additions(&self, workspace_id: &str) -> shell::Additions;
    /// The `omfp://windows` answer for workspace `workspace_id`: the open windows showing it (AGT-081).
    fn windows(&self, workspace_id: &str) -> String;
}

/// Builds the host tools of a thread's agent from the agent tool registry (AGT-040, DMN-151); the server implements
/// it over its services.
pub trait AgentHost: Send + Sync {
    /// The registry's tools for the agent of thread `thread_id`, which lives in workspace `workspace_id`.
    fn tools(&self, thread_id: &str, workspace_id: &str) -> Vec<HostTool>;
}

/// The host tools of one thread: `opener` opens files and `notifier` notifies the user in their window, `set_title`
/// renames the thread, and `agent_tools` are the registry's tools for this thread (AGT-040), the widget checks among
/// them. Every tool is `essential`: a direct tool of the agent, not only an `xd://` device (DMN-106).
#[must_use]
pub fn thread_tools(
    opener: OpenFile,
    notifier: Notify,
    set_title: SetTitle,
    agent_tools: Vec<HostTool>,
) -> Vec<HostTool> {
    let mut tools = vec![
        open_file(opener),
        notify(notifier),
        set_thread_title(set_title),
    ];
    tools.extend(agent_tools);
    tools
        .into_iter()
        .map(|tool| tool.load_mode(ToolLoadMode::Essential))
        .collect()
}

/// The host URI schemes of every thread (DMN-062): `omfp://`, read-only — the method catalogue (AGT-042), the docs and
/// their index, the schemas, the built-in widgets' sources from `builtin` (`<static-dir>/builtin-widgets/` when the
/// daemon serves a bundle) and, with `widgets` (the thread's workspace id and the widget host),
/// `omfp://widgets/check/<name>` (DMN-108), `omfp://keymap` with the workspace's bindings (DMN-199) and
/// `omfp://windows` (AGT-081).
pub fn host_uris(
    builtin: Option<PathBuf>,
    widgets: Option<(String, Arc<dyn WidgetHost>)>,
) -> Result<Vec<HostUri>, omp_rpc::Error> {
    let omfp = HostUri::new(SCHEME, move |url, context| {
        let (workspace_id, host) = widgets
            .as_ref()
            .map_or(("", None), |(id, host)| (id.as_str(), Some(&**host)));
        let (base, selector) = widgets::selector::split(url);
        widgets::selector::read(base, selector, |url| {
            catalog::read_uri(url)
                .or_else(|| widgets::read_check(url, || context.is_cancelled(), workspace_id, host))
                .or_else(|| widgets::read_keymap(url, workspace_id, host))
                .or_else(|| widgets::read_windows(url, workspace_id, host))
                .unwrap_or_else(|| widgets::read_uri(url, builtin.as_deref()))
        })
        .map_err(Into::into)
    })?
    .description(format!(
        "omfp, the window the user works in: {} indexes these docs; {} lists the protocol methods omfp_call accepts \
         and {}/<name> gives one's params and result schemas; {DOCS_URL} documents widgets (SDK, kit, views, init \
         files, TSX views); {} describes the window and {} its keys and commands; {SCHEMA_URL} is the JSON Schema of \
         .omfp/views.json, {} of .omfp/keymap.json and {} of .omfp/jobs.json; {BUILTIN_URL} lists the built-in \
         widgets and \
         {BUILTIN_URL}/<name> shows one's source; {CHECK_URL}/<name> checks a widget, an init file (`init`, \
         `user:init`) or a TSX view (`views/<file>`): compile errors, what open windows report, a widget no window \
         shows rendered out of sight; {} describes the open windows showing this workspace (views and where each \
         comes from, added commands, errors)",
        shell::INDEX_URL,
        catalog::METHODS_URL,
        catalog::METHODS_URL,
        shell::SHELL_URL,
        shell::KEYMAP_URL,
        keymap::SCHEMA_URL,
        crate::jobs::file::SCHEMA_URL,
        shell::WINDOWS_URL
    ))
    .immutable(true);
    Ok(vec![omfp])
}

fn schema(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// `set_thread_title { title }`: renames the thread; a title set this way is never replaced by a generated one
/// (DMN-073).
#[must_use]
pub fn set_thread_title(set_title: SetTitle) -> HostTool {
    let parameters = json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TITLE_CHARS,
                "description": "The new title: a few words saying what the thread is about."
            }
        },
        "required": ["title"],
        "additionalProperties": false
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    HostTool::new(
        SET_THREAD_TITLE,
        "Renames this thread in the omfp thread list, e.g. when the user asks for another title.",
        parameters,
        move |arguments, context| {
            run_set_thread_title(&arguments, || context.is_cancelled(), &*set_title)
        },
    )
    .label("Set thread title")
}

/// The body of [`set_thread_title`], without the `omp-rpc` context. Bad arguments and cancellation become tool errors
/// (`isError: true`), never panics (DMN-061).
pub fn run_set_thread_title(
    arguments: &Map<String, Value>,
    is_cancelled: impl Fn() -> bool,
    set_title: &dyn Fn(&str) -> Result<(), String>,
) -> Result<HostToolOutput, HostToolError> {
    if is_cancelled() {
        return Err("cancelled".into());
    }
    let title = match arguments.get("title") {
        Some(Value::String(title)) if !title.trim().is_empty() => title.trim(),
        _ => return Err("`title` must be a non-empty string".into()),
    };
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("`title` must be at most {MAX_TITLE_CHARS} characters").into());
    }
    if is_cancelled() {
        return Err("cancelled".into());
    }
    set_title(title)?;
    Ok(HostToolOutput::Text(format!(
        "The thread is now titled \"{title}\"."
    )))
}

/// `omfp_open_file { path, line? }`: opens a file in omfp, in the window the user last used on the thread, as a click
/// on its path does (DMN-060).
#[must_use]
pub fn open_file(open: OpenFile) -> HostTool {
    let parameters = schema(&json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "File to open: relative to the workspace folder (or one of its repositories), absolute (a file outside the workspace folder opens read-only), or a bare or partial name the user's window looks up."
            },
            "line": { "type": "integer", "minimum": 1, "description": "Line to scroll to." }
        },
        "required": ["path"],
        "additionalProperties": false
    }));
    HostTool::new(
        OPEN_FILE,
        "Opens a file for the user in omfp, as a file tab of the window they last used on this thread, as if they \
         clicked its path. Fails when no omfp window shows this thread.",
        parameters,
        move |arguments, context| run_open_file(&arguments, || context.is_cancelled(), &*open),
    )
    .label("Open file")
}

/// The body of [`open_file`], without the `omp-rpc` context. Bad arguments, cancellation and a thread no window
/// shows become tool errors (`isError: true`), never panics (DMN-061).
pub fn run_open_file(
    arguments: &Map<String, Value>,
    is_cancelled: impl Fn() -> bool,
    open: impl Fn(&str, Option<u64>) -> Result<(), String>,
) -> Result<HostToolOutput, HostToolError> {
    if is_cancelled() {
        return Err("cancelled".into());
    }
    let path = match arguments.get("path") {
        Some(Value::String(path)) if !path.trim().is_empty() => path.trim(),
        _ => return Err("`path` must be a non-empty string".into()),
    };
    let line = match arguments.get("line") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64().filter(|line| *line >= 1) {
            Some(line) => Some(line),
            None => return Err("`line` must be a positive integer".into()),
        },
    };
    if is_cancelled() {
        return Err("cancelled".into());
    }
    open(path, line)?;
    let shown = line.map_or_else(|| path.to_owned(), |line| format!("{path}:{line}"));
    Ok(HostToolOutput::Text(format!(
        "Opened {shown} in the user's omfp window."
    )))
}

/// `omfp_notify { title, body }`: a notification about the thread in the window the user last used on it, else in the
/// one keeping it, else in their most recently focused window (DMN-125, DMN-375, DMN-376).
#[must_use]
pub fn notify(notifier: Notify) -> HostTool {
    let parameters = schema(&json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_NOTIFY_TITLE_CHARS,
                "description": "A few words: what it is about."
            },
            "body": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_NOTIFY_BODY_CHARS,
                "description": "One or two sentences: what happened or what you need from the user."
            }
        },
        "required": ["title", "body"],
        "additionalProperties": false
    }));
    HostTool::new(
        NOTIFY,
        "Notifies the user about this thread in omfp: a notice in the window they last used on it, else in the omfp \
         window they focused last (even if it shows another thread), or a system notification while that window is in \
         the background; opening it shows this thread. Use it when a long task finished or you need the user (a \
         decision, an approval), at most once per occasion, never for progress. Fails only when no omfp window is open.",
        parameters,
        move |arguments, context| run_notify(&arguments, || context.is_cancelled(), &*notifier),
    )
    .label("Notify")
}

/// The body of [`notify`], without the `omp-rpc` context. Bad arguments, cancellation and no omfp window open become
/// tool errors (`isError: true`), never panics (DMN-061, DMN-126, DMN-127).
pub fn run_notify(
    arguments: &Map<String, Value>,
    is_cancelled: impl Fn() -> bool,
    notify: impl Fn(&str, &str) -> Result<(), String>,
) -> Result<HostToolOutput, HostToolError> {
    if is_cancelled() {
        return Err("cancelled".into());
    }
    let title = bounded_text(arguments, "title", MAX_NOTIFY_TITLE_CHARS)?;
    let body = bounded_text(arguments, "body", MAX_NOTIFY_BODY_CHARS)?;
    if is_cancelled() {
        return Err("cancelled".into());
    }
    notify(title, body)?;
    Ok(HostToolOutput::Text(
        "Notified the user in their omfp window.".to_owned(),
    ))
}

/// The trimmed string argument `key`: not blank, at most `max` characters.
fn bounded_text<'a>(
    arguments: &'a Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<&'a str, HostToolError> {
    match arguments.get(key) {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            let text = text.trim();
            if text.chars().count() > max {
                return Err(format!("`{key}` must be at most {max} characters").into());
            }
            Ok(text)
        }
        _ => Err(format!("`{key}` must be a non-empty string").into()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn args(value: Value) -> Map<String, Value> {
        let Value::Object(map) = value else {
            panic!("not an object")
        };
        map
    }

    /// What an opener was asked to open: paths and lines.
    type Opened = Vec<(String, Option<u64>)>;

    /// Runs `omfp_open_file` with an opener that records its calls and fails with `refusal` when given.
    fn run(
        arguments: Value,
        cancelled: bool,
        refusal: Option<&str>,
    ) -> (Result<String, String>, Opened) {
        let opened = Mutex::new(Vec::new());
        let result = run_open_file(
            &args(arguments),
            || cancelled,
            |path: &str, line| {
                opened.lock().unwrap().push((path.to_owned(), line));
                refusal.map_or(Ok(()), |refusal| Err(refusal.to_owned()))
            },
        );
        let result = match result {
            Ok(HostToolOutput::Text(text)) => Ok(text),
            Ok(other) => panic!("unexpected output {other:?}"),
            Err(error) => Err(error.to_string()),
        };
        (result, opened.into_inner().unwrap())
    }

    // spec: DMN-060
    #[test]
    fn opens_the_path_as_written_at_a_line() {
        let (result, opened) = run(json!({ "path": " src/main.rs ", "line": 12 }), false, None);

        assert_eq!(
            result.unwrap(),
            "Opened src/main.rs:12 in the user's omfp window."
        );
        assert_eq!(opened, [("src/main.rs".to_owned(), Some(12))]);

        let (result, opened) = run(json!({ "path": "theme.ts", "line": null }), false, None);
        assert_eq!(
            result.unwrap(),
            "Opened theme.ts in the user's omfp window."
        );
        assert_eq!(opened, [("theme.ts".to_owned(), None)]);
    }

    // spec: DMN-060, DMN-061
    #[test]
    fn a_thread_no_window_shows_is_a_tool_error_with_the_openers_reason() {
        let (result, _) = run(
            json!({ "path": "a.rs" }),
            false,
            Some("no omfp window shows this thread"),
        );

        assert_eq!(result, Err("no omfp window shows this thread".to_owned()));
    }

    // spec: DMN-061
    #[test]
    fn bad_arguments_are_tool_errors_not_panics() {
        for arguments in [
            json!({}),
            json!({ "path": "" }),
            json!({ "path": " " }),
            json!({ "path": 3 }),
            json!({ "path": "a.rs", "line": 0 }),
            json!({ "path": "a.rs", "line": "x" }),
        ] {
            let (result, opened) = run(arguments.clone(), false, None);
            assert!(result.is_err(), "{arguments}");
            assert_eq!(opened, Vec::new());
        }
    }

    // spec: DMN-061
    #[test]
    fn a_cancelled_call_does_nothing_and_reports_an_error() {
        let (result, opened) = run(json!({ "path": "/repo/a.rs" }), true, None);

        assert_eq!(result, Err("cancelled".to_owned()));
        assert_eq!(opened, Vec::new());
    }

    // spec: DMN-060, DMN-125
    #[test]
    fn the_tools_are_registered_with_their_schemas() {
        let tools = thread_tools(
            Arc::new(|_, _| Ok(())),
            Arc::new(|_, _| Ok(())),
            Arc::new(|_| Ok(())),
            Vec::new(),
        );
        let definitions: Vec<_> = tools.iter().map(HostTool::definition).collect();

        assert_eq!(definitions[0].name, OPEN_FILE);
        assert_eq!(definitions[0].label.as_deref(), Some("Open file"));
        assert_eq!(definitions[0].parameters["required"], json!(["path"]));
        assert_eq!(definitions[1].name, NOTIFY);
        assert_eq!(definitions[1].label.as_deref(), Some("Notify"));
        assert_eq!(
            definitions[1].parameters["required"],
            json!(["title", "body"])
        );
        assert_eq!(
            (
                &definitions[1].parameters["properties"]["title"]["maxLength"],
                &definitions[1].parameters["properties"]["body"]["maxLength"]
            ),
            (
                &json!(MAX_NOTIFY_TITLE_CHARS),
                &json!(MAX_NOTIFY_BODY_CHARS)
            )
        );
        for when in ["long task finished", "never for progress"] {
            assert!(
                definitions[1].description.contains(when),
                "{when:?}: {}",
                definitions[1].description
            );
        }
        assert_eq!(definitions[2].name, SET_THREAD_TITLE);
        assert_eq!(definitions[2].parameters["required"], json!(["title"]));
        assert_eq!(definitions.len(), 3, "no registry tools given");
        for definition in &definitions {
            assert_eq!(
                definition.load_mode,
                Some(ToolLoadMode::Essential),
                "{} is a direct tool of the agent",
                definition.name
            );
        }
    }

    /// Runs `omfp_notify` with a notifier that records its calls and fails with `refusal` when given.
    fn notify_with(
        arguments: Value,
        cancelled: bool,
        refusal: Option<&str>,
    ) -> (Result<String, String>, Vec<(String, String)>) {
        let sent = Mutex::new(Vec::new());
        let result = run_notify(
            &args(arguments),
            || cancelled,
            |title: &str, body: &str| {
                sent.lock()
                    .unwrap()
                    .push((title.to_owned(), body.to_owned()));
                refusal.map_or(Ok(()), |refusal| Err(refusal.to_owned()))
            },
        );
        (text(result), sent.into_inner().unwrap())
    }

    // spec: DMN-125
    #[test]
    fn notify_sends_the_trimmed_title_and_body() {
        let (result, sent) = notify_with(
            json!({ "title": " Tests ", "body": " All 412 pass.\n" }),
            false,
            None,
        );

        assert_eq!(
            result,
            Ok("Notified the user in their omfp window.".to_owned())
        );
        assert_eq!(sent, [("Tests".to_owned(), "All 412 pass.".to_owned())]);
    }

    // spec: DMN-126, DMN-061
    #[test]
    fn notify_without_a_window_is_a_tool_error_with_the_notifiers_reason() {
        let (result, _) = notify_with(
            json!({ "title": "Tests", "body": "Done" }),
            false,
            Some("no omfp window shows this thread"),
        );

        assert_eq!(result, Err("no omfp window shows this thread".to_owned()));
    }

    // spec: DMN-127, DMN-061
    #[test]
    fn notify_refuses_bad_arguments_and_honours_cancellation() {
        for arguments in [
            json!({}),
            json!({ "title": "Tests" }),
            json!({ "body": "Done" }),
            json!({ "title": " ", "body": "Done" }),
            json!({ "title": "Tests", "body": "" }),
            json!({ "title": 3, "body": "Done" }),
            json!({ "title": "x".repeat(MAX_NOTIFY_TITLE_CHARS + 1), "body": "Done" }),
            json!({ "title": "Tests", "body": "x".repeat(MAX_NOTIFY_BODY_CHARS + 1) }),
        ] {
            let (result, sent) = notify_with(arguments.clone(), false, None);
            assert!(result.is_err(), "{arguments}");
            assert_eq!(sent, Vec::new());
        }
        let (result, sent) = notify_with(
            json!({ "title": "x".repeat(MAX_NOTIFY_TITLE_CHARS), "body": "x".repeat(MAX_NOTIFY_BODY_CHARS) }),
            false,
            None,
        );
        assert!(result.is_ok(), "the longest title and body pass");
        assert_eq!(sent.len(), 1);

        let (result, sent) = notify_with(json!({ "title": "Tests", "body": "Done" }), true, None);
        assert_eq!(result, Err("cancelled".to_owned()));
        assert_eq!(sent, Vec::new());
    }

    fn text(output: Result<HostToolOutput, HostToolError>) -> Result<String, String> {
        match output {
            Ok(HostToolOutput::Text(text)) => Ok(text),
            Ok(other) => panic!("unexpected output {other:?}"),
            Err(error) => Err(error.to_string()),
        }
    }

    fn rename(arguments: Value, cancelled: bool) -> (Result<String, String>, Vec<String>) {
        let titles = Mutex::new(Vec::new());
        let result = run_set_thread_title(&args(arguments), || cancelled, &|title| {
            titles.lock().unwrap().push(title.to_owned());
            Ok(())
        });
        let result = match result {
            Ok(HostToolOutput::Text(text)) => Ok(text),
            Ok(other) => panic!("unexpected output {other:?}"),
            Err(error) => Err(error.to_string()),
        };
        (result, titles.into_inner().unwrap())
    }

    // spec: DMN-060, DMN-073
    #[test]
    fn set_thread_title_renames_the_thread() {
        let (result, titles) = rename(json!({ "title": "  Checkout v2 " }), false);

        assert_eq!(result.unwrap(), "The thread is now titled \"Checkout v2\".");
        assert_eq!(titles, ["Checkout v2"]);
    }

    // spec: DMN-061
    #[test]
    fn set_thread_title_rejects_bad_titles_and_honours_cancellation() {
        for arguments in [
            json!({}),
            json!({ "title": " " }),
            json!({ "title": 3 }),
            json!({ "title": "x".repeat(MAX_TITLE_CHARS + 1) }),
        ] {
            let (result, titles) = rename(arguments.clone(), false);
            assert!(result.is_err(), "{arguments}");
            assert_eq!(titles, Vec::<String>::new());
        }
        let (result, titles) = rename(json!({ "title": "Late" }), true);
        assert_eq!(result, Err("cancelled".to_owned()));
        assert_eq!(titles, Vec::<String>::new());

        let failed = run_set_thread_title(&args(json!({ "title": "x" })), || false, &|_| {
            Err("thread 01X not found".into())
        });
        assert_eq!(failed.unwrap_err().to_string(), "thread 01X not found");
    }

    // spec: DMN-062, AGT-042, DMN-198
    #[test]
    fn the_omfp_scheme_is_read_only_and_describes_the_method_catalogue() {
        let uris = host_uris(None, None).unwrap();
        assert_eq!(uris.len(), 1);
        assert_eq!(uris[0].scheme(), "omfp");
        assert!(!uris[0].writable());
        let definition = uris[0].definition();
        assert_eq!(definition.immutable, Some(true));
        let description = definition.description.clone().unwrap_or_default();
        assert!(
            description
                .contains("omfp://protocol/methods lists the protocol methods omfp_call accepts"),
            "{description}"
        );
        for part in [
            "omfp://docs indexes these docs",
            "omfp://docs/shell describes the window and omfp://keymap its keys and commands",
            "an init file (`init`, `user:init`) or a TSX view (`views/<file>`)",
        ] {
            assert!(description.contains(part), "{part:?}: {description}");
        }
    }
}
