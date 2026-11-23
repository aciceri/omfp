//! The windows as agents see and drive them (PRT-122..PRT-124): the state a window reports with its widget report
//! (`widgets.report`'s `window`), `windows.list` and `windows.state`, and the requests the daemon sends a window
//! (`window_request` frames, answered with `windows.answer`): `windows.run` runs a command, `windows.check_widget`
//! renders a widget out of sight, and `auth.login` shows its flow (PRT-131).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dto::KeymapMode;
use crate::params::{FileErrorReport, WidgetReport};

/// What a window shows besides its widgets (PRT-122): sent as `widgets.report`'s `window`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WindowReport {
    /// The thread the window shows; `null` for none.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Whether the window has the focus.
    #[serde(default)]
    pub focused: bool,
    /// The view the window shows; `null` without a workspace.
    #[serde(default)]
    pub active_view: Option<String>,
    /// The view shown when none is chosen.
    #[serde(default)]
    pub default_view: Option<String>,
    /// The views of the window's switcher, after `views.json`, the init files and their transforms, in its order.
    #[serde(default)]
    pub views: Vec<WindowView>,
    /// The commands registered beyond the built-ins: by an init file or a workspace or user widget.
    #[serde(default)]
    pub commands: Vec<WindowCommand>,
}

/// Where a view of a window comes from (PRT-122).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewSource {
    /// The built-in default view: `views.json` is missing, invalid or not loaded yet.
    Builtin,
    /// The workspace's `.omfp/views.json` (even when an init file changed its layout).
    ViewsJson,
    /// Added by the user's init file.
    UserInit,
    /// Added by the workspace's `.omfp/init.ts`.
    WorkspaceInit,
}

/// One view of a window (PRT-122).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WindowView {
    pub id: String,
    pub title: String,
    pub source: ViewSource,
    /// The widget references of its layout.
    pub widgets: Vec<String>,
}

/// Who registered a command of a window (PRT-122).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    UserInit,
    WorkspaceInit,
    /// A workspace or user widget (`widget` names it).
    Widget,
}

/// A command registered beyond the built-ins (PRT-122).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WindowCommand {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub source: CommandSource,
    /// The widget reference that registered it; `null` for an init file's.
    #[serde(default)]
    pub widget: Option<String>,
    /// Whether it runs now (a command may disable itself).
    pub enabled: bool,
    /// The keys bound to it, in every mode.
    #[serde(default)]
    pub bindings: Vec<WindowBinding>,
}

/// A key binding of a command (PRT-122).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WindowBinding {
    pub mode: KeymapMode,
    /// Tokens separated by spaces, e.g. `g h`.
    pub keys: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// `windows.list` params (PRT-123).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListWindowsParams {
    /// Only the windows showing this workspace; every window when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

/// `windows.list` result (PRT-123).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WindowList {
    /// The windows that reported, the most recently focused first.
    pub windows: Vec<WindowSummary>,
}

/// One window of `windows.list` (PRT-123).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WindowSummary {
    /// The window's connection number (`system.hello`'s `connection`).
    pub window_id: u64,
    /// The `client` of its `system.hello`.
    pub client: String,
    /// The workspace it shows; `null` for none.
    pub workspace_id: Option<String>,
    pub thread_id: Option<String>,
    pub focused: bool,
    pub active_view: Option<String>,
    /// How many errors it shows: init files (`fileErrors`) and widgets in an error state.
    pub errors: u32,
}

/// Params naming one window (PRT-123).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WindowRef {
    pub window_id: u64,
}

/// `windows.state` result (PRT-123): all a window reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub window_id: u64,
    pub client: String,
    pub workspace_id: Option<String>,
    pub thread_id: Option<String>,
    pub focused: bool,
    pub active_view: Option<String>,
    pub default_view: Option<String>,
    pub views: Vec<WindowView>,
    pub commands: Vec<WindowCommand>,
    /// The errors it shows of init files (PRT-115).
    pub file_errors: Vec<FileErrorReport>,
    /// The workspace and user widgets of its current view (PRT-043).
    pub widgets: Vec<WidgetReport>,
}

/// `windows.run` params (PRT-124).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunInWindowParams {
    /// The window; default: the most recently focused window showing `workspaceId` (any window without it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// A command id or alias registered in the window (`view.switch`, `view`), run in the global scope.
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// `windows.run` result (PRT-124).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RanInWindow {
    /// The window that ran it.
    pub window_id: u64,
    /// The id of the command the name resolved to.
    pub command: String,
}

/// `windows.check_widget` params (PRT-124).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckWidgetParams {
    /// The window; default: the most recently focused window showing the workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<u64>,
    pub workspace_id: String,
    /// The widget reference: `./widgets/<name>`.
    pub widget: String,
}

/// `windows.check_widget` result (PRT-124).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetChecked {
    pub window_id: u64,
    /// The out-of-sight instance as the window saw it when it rendered, failed or ran out of time.
    pub report: WidgetReport,
    /// `false` when the window ran out of time before the widget rendered or failed (`report.state` is `loading`).
    pub settled: bool,
}

/// What the daemon asks a window (PRT-124): sent as `{ type: "window_request", id, request }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowRequest {
    /// Runs a registered command by id or alias, in the global scope.
    RunCommand { command: String, args: Vec<String> },
    /// Mounts the widget out of sight, waits until it rendered or failed (at most `timeoutMs`), then unmounts it.
    #[serde(rename_all = "camelCase")]
    CheckWidget {
        reference: String,
        /// The folder version to wait for; `null` takes whatever the window has.
        version: Option<String>,
        timeout_ms: u64,
    },
    /// omp's login of `providerId` asks to open `url` (PRT-131); `launchUrl`, a short loopback redirect to it on the
    /// daemon's host, is the safer copy. A notice: the window does not answer it.
    #[serde(rename_all = "camelCase")]
    LoginUrl {
        provider_id: String,
        name: String,
        url: String,
        launch_url: Option<String>,
        instructions: Option<String>,
    },
    /// Progress of the login of `providerId` (PRT-131). A notice: the window does not answer it.
    #[serde(rename_all = "camelCase")]
    LoginProgress {
        provider_id: String,
        name: String,
        message: String,
    },
    /// The login of `providerId` asks for text, e.g. the code or redirect URL the provider's page shows (PRT-131);
    /// answered with `login_input`.
    #[serde(rename_all = "camelCase")]
    LoginInput {
        provider_id: String,
        name: String,
        message: String,
        placeholder: Option<String>,
    },
    /// An agent's `omfp_notify` about thread `threadId` (DMN-125), sent to the user's most recently focused window when
    /// no window subscribes the thread (DMN-375): the window shows `title` and `body` as a notice about the thread
    /// (UI-936). A notice: the window does not answer it.
    #[serde(rename_all = "camelCase")]
    Notify {
        thread_id: String,
        title: String,
        body: String,
    },
}

/// Why a window did not run a command (PRT-124).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunFailure {
    /// No command answers to the name in the global scope.
    Unknown,
    /// The command is registered but disabled now.
    Disabled,
    /// It threw, rejected or reported an error.
    Failed,
}

/// A window's answer to a [`WindowRequest`] (PRT-124).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowAnswer {
    /// The command ran; `command` is the id the name resolved to.
    Ran { command: String },
    /// The command did not run, or failed.
    NotRun { reason: RunFailure, message: String },
    /// The out-of-sight render of a `check_widget`.
    Checked { report: WidgetReport, settled: bool },
    /// The text a `login_input` asked for; `null` cancels the login.
    LoginInput { value: Option<String> },
}

/// `windows.answer` params (PRT-124): a window's answer to the daemon's request `requestId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnswerWindowParams {
    pub request_id: String,
    pub answer: WindowAnswer,
}
