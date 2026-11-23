//! The init files as agents see them: the errors windows report for them (`widgets.report`'s `fileErrors`,
//! DMN-195), `views.errors` (PRT-116), `omfp_views_errors` (DMN-196) and the `omfp_widget_check` of an init file
//! (DMN-197).

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use omfp_protocol::{ErrorBody, FileErrorReport, ViewErrors, WindowFileErrors};

use super::widget_tools::{behind, entry, location, window_title};
use super::windows::Window;
use super::{Services, blocking};
use crate::widgets::compile::CompileError;
use crate::widgets::{Widgets, init};

/// The file an `omfp_widget_check` name stands for when it is no widget (DMN-197).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CheckedFile {
    /// `init` or `.omfp/init.ts`: the workspace's init file.
    WorkspaceInit,
    /// `user:init`: the user's init file.
    UserInit,
}

impl CheckedFile {
    /// The file `name` stands for; `None` when it names a widget.
    pub(super) fn parse(name: &str) -> Option<Self> {
        match name.trim() {
            "init" | ".omfp/init.ts" => Some(Self::WorkspaceInit),
            "user:init" => Some(Self::UserInit),
            _ => None,
        }
    }
}

/// What the daemon found of a checked file: the text so far, and how windows name it in their reports.
struct Examined {
    text: String,
    /// The report's `file` (`user` or `.omfp/init.ts`).
    file: String,
}

/// How a report's `file` reads in a sentence.
fn file_name(file: &str) -> String {
    if file == "user" {
        "the user's init file".to_owned()
    } else {
        file.to_owned()
    }
}

/// The daemon's compile check of a file, failing with `error`, whose compile errors read
/// `<prefix><file>:<line>:<column>`.
fn compiled(text: &mut String, prefix: &str, error: Option<&CompileError>) {
    match error {
        None => text.push_str("Daemon compile: it compiles.\n"),
        Some(error) => {
            let _ = writeln!(
                text,
                "Daemon compile: it fails; windows load a stand-in that shows this error instead:\n- {}: {}",
                location(prefix, error),
                error.message
            );
        }
    }
}

/// The daemon's part of the check of `file` of the workspace at `workspace`; `user_init` is the user's init file the
/// daemon follows (DMN-197).
fn examine(
    widgets: &Widgets,
    workspace: &Path,
    user_init: Option<&Path>,
    file: &CheckedFile,
) -> Examined {
    match file {
        CheckedFile::WorkspaceInit => {
            let path = init::workspace_init(workspace);
            Examined {
                text: examine_init(
                    widgets,
                    &path,
                    &format!("Init file .omfp/init.ts at {}", path.display()),
                    ".omfp/",
                    &format!(
                        "No file .omfp/init.ts ({}): windows run no init file of this workspace.",
                        path.display()
                    ),
                ),
                file: ".omfp/init.ts".to_owned(),
            }
        }
        CheckedFile::UserInit => Examined {
            text: user_init.map_or_else(
                || "This omfp daemon follows no user init file.\n".to_owned(),
                |path| {
                    let prefix = path
                        .parent()
                        .map(|folder| format!("{}/", folder.display()))
                        .unwrap_or_default();
                    examine_init(
                        widgets,
                        path,
                        &format!("The user's init file {}", path.display()),
                        &prefix,
                        &format!("No file {}: windows run no user init file.", path.display()),
                    )
                },
            ),
            file: "user".to_owned(),
        },
    }
}

/// The init file at `path`: `<title> (version <version>)` and its compile check (errors read `<prefix><file>:…`),
/// else `missing`.
fn examine_init(
    widgets: &Widgets,
    path: &Path,
    title: &str,
    prefix: &str,
    missing: &str,
) -> String {
    let mut text = String::new();
    match widgets.check_init(path) {
        None => {
            let _ = writeln!(text, "{missing}");
        }
        Some(result) => {
            let version = init::read_version(path).unwrap_or_else(|| "unreadable".to_owned());
            let _ = writeln!(text, "{title} (version {version})\n");
            compiled(&mut text, prefix, result.as_ref().err());
        }
    }
    text
}

/// The windows' part of the check of `examined` (DMN-197).
fn describe_windows(
    examined: &Examined,
    windows: &[(Window, Option<String>)],
    waited: Duration,
) -> String {
    let mut text = examined.text.clone();
    text.push('\n');
    if windows.is_empty() {
        text.push_str(
            "No omfp window shows this workspace now, so no window ran it: only the daemon's check above applies.\n",
        );
        return text;
    }
    let name = file_name(&examined.file);
    let _ = writeln!(text, "{} window(s) show this workspace:", windows.len());
    let mut reported = false;
    for (window, unsettled) in windows {
        let title = window_title(window, unsettled.as_ref(), waited);
        let errors: Vec<&FileErrorReport> = window
            .report
            .file_errors
            .iter()
            .filter(|error| error.file == examined.file)
            .collect();
        if errors.is_empty() {
            let _ = writeln!(text, "{title}: no error for {name}");
            continue;
        }
        reported = true;
        let _ = writeln!(text, "{title}:");
        for error in errors {
            entry(&mut text, "", &error.message);
        }
    }
    if !reported {
        text.push_str("No window reports an error for it.\n");
    }
    text
}

/// The answer of `omfp_views_errors` (DMN-196).
fn describe_errors(windows: &[(Window, Option<String>)], waited: Duration) -> String {
    if windows.is_empty() {
        return "No omfp window shows this workspace now, so no window runs its init files. Check one without a \
                window with omfp_widget_check { name } (`init` or `user:init`)."
            .to_owned();
    }
    let clean = windows
        .iter()
        .all(|(window, _)| window.report.file_errors.is_empty());
    let mut text = format!("{} window(s) show this workspace", windows.len());
    text.push_str(if clean {
        "; none reports an init file error.\n"
    } else {
        ".\n"
    });
    for (window, unsettled) in windows {
        let title = window_title(window, unsettled.as_ref(), waited);
        if window.report.file_errors.is_empty() {
            let _ = writeln!(text, "{title}: no init file errors.");
            continue;
        }
        let _ = writeln!(text, "{title}:");
        for error in &window.report.file_errors {
            entry(
                &mut text,
                "",
                &format!("{}: {}", file_name(&error.file), error.message),
            );
        }
    }
    text
}

impl Services {
    /// `views.errors` (PRT-116): the latest init file errors of every window whose report is about
    /// workspace `workspace_id`, those without errors included.
    pub async fn view_errors(&self, workspace_id: &str) -> Result<ViewErrors, ErrorBody> {
        let store = Arc::clone(&self.store);
        let id = workspace_id.to_owned();
        blocking(move || {
            store.workspace(&id)?;
            Ok(())
        })
        .await?;
        Ok(ViewErrors {
            windows: self
                .windows
                .showing(workspace_id)
                .into_iter()
                .map(|window| WindowFileErrors {
                    client: window.client,
                    errors: window.report.file_errors,
                })
                .collect(),
        })
    }

    /// `omfp_views_errors` (DMN-196): what `views.errors` gives, as text, once the windows caught up with the
    /// workspace's views (waiting at most `wait`).
    pub async fn describe_view_errors(
        &self,
        workspace_id: &str,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        wait: Duration,
    ) -> Result<String, String> {
        self.view_errors(workspace_id)
            .await
            .map_err(|error| error.message)?;
        // Windows get the current init file versions before they are asked.
        self.refresh_views(workspace_id).await;
        let windows = self
            .settled_windows(workspace_id, is_cancelled, wait, behind)
            .await?;
        Ok(describe_errors(&windows, wait))
    }

    /// `omfp_widget_check` of an init file (DMN-197): the daemon's compile of the file as it serves it,
    /// then the errors the windows showing the workspace report for it, waiting at most `wait` for them to catch up
    /// with the workspace's views.
    pub(super) async fn check_file(
        &self,
        workspace_id: &str,
        file: CheckedFile,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        wait: Duration,
    ) -> Result<String, String> {
        let (store, widgets) = (Arc::clone(&self.store), Arc::clone(&self.widgets));
        let (id, user_init) = (workspace_id.to_owned(), self.init_file());
        let examined = blocking(move || {
            let workspace = store.workspace(&id)?.path;
            Ok(examine(&widgets, &workspace, user_init.as_deref(), &file))
        })
        .await
        .map_err(|error| error.message)?;
        self.refresh_views(workspace_id).await;
        let windows = self
            .settled_windows(workspace_id, is_cancelled, wait, behind)
            .await?;
        Ok(describe_windows(&examined, &windows, wait))
    }
}
