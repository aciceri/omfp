//! The method table: one marker type per protocol method tying its name to its params and result types.
//!
//! Results omp defines are its wire types; schemas are open except for model discovery's stable selection fields.

use schemars::schema_for;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde::ser::SerializeMap;
use serde_json::{Map, Value, json};
use std::fmt::Debug;

use crate::dto::{
    DiffPage, FileCandidates, FileContent, FileWritten, GitStatus, JobInfo, Settings, TerminalInfo,
    TerminalScreen, Theme, ThreadSummary, Workspace,
};
use crate::{activity, auth, lists, mcp, params, session, windows};
use omfp_core::wire;

/// A protocol method: its wire name and the types of its params and result.
pub trait Method {
    /// Wire name, e.g. `thread.prompt`.
    const NAME: &'static str;
    type Params: Serialize + DeserializeOwned + Debug + Clone + PartialEq;
    type Result: Serialize + DeserializeOwned + Debug + Clone + PartialEq;

    /// The request payload for these params.
    fn call(params: Self::Params) -> Call;
}

/// Schema of a result: its own, or the one given after `as` for omp-defined types.
macro_rules! result_schema {
    ($result:ty) => {
        schema_for!($result)
    };
    ($result:ty, $schema:ty) => {
        schema_for!($schema)
    };
}

macro_rules! methods {
    ($(
        $(#[doc = $doc:literal])*
        $variant:ident = $name:literal ($params:ty) -> $result:ty $(as $schema:ty)?;
    )*) => {
        $(
            $(#[doc = $doc])*
            #[derive(Debug, Clone, Copy)]
            pub struct $variant;

            impl Method for $variant {
                const NAME: &'static str = $name;
                type Params = $params;
                type Result = $result;

                fn call(params: Self::Params) -> Call {
                    Call::$variant(params)
                }
            }
        )*

        /// A method with its params: the payload of a request.
        #[derive(Debug, Clone, PartialEq)]
        pub enum Call {
            $(
                $(#[doc = $doc])*
                $variant($params),
            )*
        }

        /// Wire names of every method, in table order.
        pub const METHODS: &[&str] = &[$($name),*];

        impl Call {
            /// Wire name of the method.
            #[must_use]
            pub fn method(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => $name,)*
                }
            }

            /// Decodes the params of `method`; `None` for an unknown method.
            pub(crate) fn decode(method: &str, params: Value) -> Option<Result<Self, serde_json::Error>> {
                match method {
                    $($name => Some(serde_json::from_value(params).map(Self::$variant)),)*
                    _ => None,
                }
            }

            pub(crate) fn serialize_params<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
                match self {
                    $(Self::$variant(params) => map.serialize_entry("params", params),)*
                }
            }
        }

        /// JSON Schemas of every method's params and result, keyed by method name (AGT-007).
        #[must_use]
        pub fn method_schemas() -> Value {
            let mut schemas = Map::new();
            $(
                schemas.insert(
                    $name.to_owned(),
                    json!({
                        "params": schema_for!($params),
                        "result": result_schema!($result $(, $schema)?),
                    }),
                );
            )*
            Value::Object(schemas)
        }

        /// The description of `method`, its doc comment in the method table, on one line; `None` for an unknown
        /// method (PRT-092).
        #[must_use]
        pub fn method_description(method: &str) -> Option<String> {
            let lines: &[&str] = match method {
                $($name => &[$($doc),*],)*
                _ => return None,
            };
            Some(lines.iter().copied().map(str::trim).collect::<Vec<_>>().join(" "))
        }
    };
}

methods! {
    /// Authenticates the connection and negotiates the protocol version.
    SystemHello = "system.hello" (params::HelloParams) -> params::HelloResult;
    /// Starts delivery of a topic.
    Subscribe = "subscribe" (params::SubscribeParams) -> params::SubscribeResult;
    /// Stops delivery of a topic.
    Unsubscribe = "unsubscribe" (params::TopicParams) -> params::Empty;
    /// Repositories seen in any workspace, most recent first (DMN-021).
    ReposSuggest = "repos.suggest" (params::SuggestReposParams) -> params::RepoSuggestions;
    /// Child directories of a path on the daemon host (DMN-021).
    FsComplete = "fs.complete" (params::CompletePathParams) -> params::PathCompletions;
    /// Creates a workspace: a new folder under the workspaces root, optionally holding a link to a repository.
    WorkspacesCreate = "workspaces.create" (params::CreateWorkspaceParams) -> Workspace;
    /// Renames a workspace.
    WorkspacesUpdate = "workspaces.update" (params::UpdateWorkspaceParams) -> Workspace;
    /// Removes a workspace without threads: its repositories, then its folder.
    WorkspacesRemove = "workspaces.remove" (params::RemoveWorkspaceParams) -> params::Empty;
    /// Scans the workspace folder for repositories again.
    WorkspacesRescan = "workspaces.rescan" (params::WorkspaceRef) -> Workspace;
    /// Adds a repository to a workspace: link, worktree or clone.
    WorkspacesAddRepo = "workspaces.add_repo" (params::AddRepoParams) -> Workspace;
    /// Removes a repository from a workspace.
    WorkspacesRemoveRepo = "workspaces.remove_repo" (params::RemoveRepoParams) -> Workspace;
    /// Git status of one repository of a workspace.
    WorkspacesStatus = "workspaces.status" (params::RepoRef) -> GitStatus;
    /// One page of the diff of one repository of a workspace.
    WorkspacesDiff = "workspaces.diff" (params::WorkspaceDiffParams) -> DiffPage;
    /// A file of a workspace with its hash and `HEAD` text (DMN-089).
    WorkspacesReadFile = "workspaces.read_file" (params::ReadFileParams) -> FileContent;
    /// Files across every repository and the workspace folder: strict name/path lookup or fuzzy path search (DMN-092).
    WorkspacesFindFile = "workspaces.find_file" (params::FindFileParams) -> FileCandidates;
    /// Writes a file of a workspace unless it changed since it was read (DMN-090).
    WorkspacesWriteFile = "workspaces.write_file" (params::WriteFileParams) -> FileWritten;
    /// Puts a workspace just before another in the left column, or last (PRT-170, DMN-281).
    WorkspacesReorder = "workspaces.reorder" (params::ReorderWorkspaceParams) -> params::Empty;
    /// Creates a thread in an existing or a new workspace.
    ThreadsCreate = "threads.create" (params::CreateThreadParams) -> ThreadSummary;
    /// Renames a thread or changes its approval mode.
    ThreadsUpdate = "threads.update" (params::UpdateThreadParams) -> ThreadSummary;
    /// Puts a thread just before another of a workspace in the left column, or last; another workspace moves it
    /// there (PRT-171, DMN-282, DMN-283).
    ThreadsReorder = "threads.reorder" (params::ReorderThreadParams) -> params::Empty;
    /// Archives a thread.
    ThreadsArchive = "threads.archive" (params::ThreadRef) -> params::Empty;
    /// Brings an archived thread back.
    ThreadsUnarchive = "threads.unarchive" (params::ThreadRef) -> params::Empty;
    /// Deletes a thread; its workspace is kept.
    ThreadsDelete = "threads.delete" (params::ThreadRef) -> params::Empty;
    /// Forks a thread into a new thread of the same workspace, with its conversation up to an entry (DMN-300); the
    /// source is untouched.
    ThreadsFork = "threads.fork" (params::ForkParams) -> ThreadSummary;
    /// Threads whose conversation text contains a query (DMN-076).
    ThreadsSearch = "threads.search" (params::SearchThreadsParams) -> params::ThreadSearchResults;
    /// Sends a prompt (omp `prompt`).
    ThreadPrompt = "thread.prompt" (params::PromptParams) -> params::PromptResult;
    /// Steers the running turn (omp `steer`).
    ThreadSteer = "thread.steer" (params::MessageParams) -> params::Empty;
    /// Queues a follow-up (omp `follow_up`).
    ThreadFollowUp = "thread.follow_up" (params::MessageParams) -> params::Empty;
    /// Aborts and returns the queued messages (omp `abort_and_restore_queue`).
    ThreadAbort = "thread.abort" (params::ThreadRef) -> wire::AbortAndRestoreQueueResult as Value;
    /// Removes a queued message (omp `remove_queued_message`).
    ThreadQueueRemove = "thread.queue.remove" (params::QueueMessageParams) -> wire::RemoveQueuedMessageResult as Value;
    /// Promotes a queued message (omp `promote_queued_message`).
    ThreadQueuePromote = "thread.queue.promote" (params::QueueMessageParams) -> wire::PromoteQueuedMessageResult as Value;
    /// Changes the model (omp `set_model`).
    ThreadSetModel = "thread.set_model" (params::SetModelParams) -> params::Empty;
    /// Changes the thinking level (omp `set_thinking_level`).
    ThreadSetThinking = "thread.set_thinking" (params::SetThinkingParams) -> params::Empty;
    /// Available models (omp `get_available_models`). Each entry supplies `provider` and `id`: use
    /// `threads.create { model: "<provider>/<id>", workspace: ... }`, or
    /// `thread.set_model { threadId: ..., provider: "<provider>", modelId: "<id>" }`.
    ThreadModels = "thread.models" (params::ThreadRef) -> wire::GetAvailableModelsResult as auth::AvailableModels;
    /// Available thinking levels (omp `get_available_thinking_levels`).
    ThreadThinkingLevels = "thread.thinking_levels" (params::ThreadRef) -> wire::GetAvailableThinkingLevelsResult as Value;
    /// Available slash commands (omp `get_available_commands`).
    ThreadCommands = "thread.commands" (params::ThreadRef) -> wire::GetAvailableCommandsResult as Value;
    /// Compacts the context (omp `compact`).
    ThreadCompact = "thread.compact" (params::CompactParams) -> wire::CompactionResult as Value;
    /// Session tree (omp `get_tree`).
    ThreadTree = "thread.tree" (params::ThreadRef) -> wire::SessionTree as Value;
    /// Moves the session leaf (omp `navigate_tree`).
    ThreadNavigate = "thread.navigate" (params::NavigateParams) -> wire::NavigateTreeResult as Value;
    /// Forks into a new session file of the same thread (omp `fork`); `threads.fork` makes a new thread instead.
    ThreadFork = "thread.fork" (params::ForkParams) -> wire::CancellationResult as Value;
    /// Labels a session entry (omp `set_label`).
    ThreadLabel = "thread.label" (params::LabelParams) -> params::Empty;
    /// Subagents of the thread (omp `get_subagents`).
    ThreadSubagents = "thread.subagents" (params::ThreadRef) -> wire::GetSubagentsResult as Value;
    /// A subagent's transcript (omp `get_subagent_messages`).
    ThreadSubagentMessages = "thread.subagent_messages" (params::SubagentMessagesParams) -> wire::SubagentMessages as Value;
    /// Steers a subagent (omp `steer_subagent`).
    ThreadSubagentSteer = "thread.subagent_steer" (params::SubagentSteerParams) -> params::Empty;
    /// Cancels a subagent (omp `cancel_subagent`).
    ThreadSubagentCancel = "thread.subagent_cancel" (params::SubagentRef) -> params::Empty;
    /// Reads or changes the session goal (omp `goal`).
    ThreadGoal = "thread.goal" (params::GoalParams) -> wire::GoalResult as Value;
    /// Asks a side question (omp `btw`).
    ThreadBtw = "thread.btw" (params::BtwParams) -> wire::BtwResult as Value;
    /// Runs a shell command in the session (omp `bash`).
    ThreadBash = "thread.bash" (params::BashParams) -> wire::BashResult as Value;
    /// Git status of every repository of the thread's workspace.
    ThreadGit = "thread.git" (params::ThreadRef) -> params::ThreadGit;
    /// omp's full session state (`get_state`, with tool definitions and system prompt: tens of KB); starts omp.
    ThreadState = "thread.state" (params::ThreadRef) -> wire::SessionState as Value;
    /// The small facts of the thread's omp session (id, file, model, state, usage) without starting omp (PRT-125).
    ThreadSession = "thread.session" (params::ThreadRef) -> session::ThreadSession;
    /// Restarts the omp process.
    ThreadRestart = "thread.restart" (params::ThreadRef) -> params::Empty;
    /// Stops the omp process; with `cascade`, those of every thread it spawned too (PRT-085, AGT-071).
    ThreadStop = "thread.stop" (activity::StopThreadParams) -> activity::ThreadsStopped;
    /// Starts a live voice session on the client's WebRTC peer (omp `live_connect`, DMN-100).
    ThreadLiveConnect = "thread.live_connect" (params::LiveConnectParams) -> params::LiveConnected;
    /// Starts a live voice session on the daemon machine's microphone and speakers (omp `live_start`, DMN-109): the
    /// desktop webview has no WebRTC (S-09).
    ThreadLiveStart = "thread.live_start" (params::ThreadRef) -> params::LiveStarted;
    /// Stops the thread's live voice session, if any (omp `live_stop`, DMN-102).
    ThreadLiveStop = "thread.live_stop" (params::ThreadRef) -> params::Empty;
    /// Mutes or unmutes the live voice session (omp `live_mute`, DMN-102).
    ThreadLiveMute = "thread.live_mute" (params::LiveMuteParams) -> params::LiveMuted;
    /// Answers an interactive request; `already_resolved` if another client answered first.
    InteractionRespond = "interaction.respond" (params::RespondParams) -> params::Empty;
    /// The user's theme CSS (DMN-075).
    ThemeGet = "theme.get" (params::Empty) -> Theme;
    /// Replaces the user's theme CSS; `null` restores the built-in themes (DMN-075).
    ThemeSet = "theme.set" (Theme) -> params::Empty;
    /// The UI settings of the config file (DMN-009, UI-097).
    SettingsGet = "settings.get" (params::Empty) -> Settings;
    /// Merges UI settings into the config file, written back with its comments (DMN-009).
    SettingsSet = "settings.set" (params::SetSettingsParams) -> Settings;
    /// Writes the workspace's views manifest, `.omfp/views.json` (DMN-083).
    ViewsSet = "views.set" (params::SetViewsParams) -> params::Empty;
    /// Copies a built-in or user widget into the workspace's `.omfp/widgets/` (DMN-087).
    WidgetsCopy = "widgets.copy" (params::CopyWidgetParams) -> params::WidgetCopied;
    /// Stores one key of a widget instance's state (DMN-086).
    WidgetsStateSet = "widgets.state_set" (params::SetWidgetStateParams) -> params::Empty;
    /// Reports what this window shows of a workspace's widgets; replaces its previous report (PRT-043).
    WidgetsReport = "widgets.report" (params::ReportWidgetsParams) -> params::Empty;
    /// Runs a shell command in the workspace folder as a job (DMN-080).
    JobsStart = "jobs.start" (params::StartJobParams) -> JobInfo;
    /// Kills a job's process group (DMN-081).
    JobsKill = "jobs.kill" (params::JobRef) -> params::Empty;
    /// Starts an interactive PTY session in a workspace (TRM-100).
    TerminalsStart = "terminals.start" (params::StartTerminalParams) -> TerminalInfo;
    /// The current plain-text terminal screen, optionally waiting for a revision change (TRM-106).
    TerminalsScreen = "terminals.screen" (params::TerminalScreenParams) -> TerminalScreen;
    /// Writes base64-encoded bytes to a PTY session (TRM-101).
    TerminalsInput = "terminals.input" (params::TerminalInputParams) -> params::Empty;
    /// Resizes a PTY session (TRM-101).
    TerminalsResize = "terminals.resize" (params::TerminalResizeParams) -> params::Empty;
    /// Kills a PTY session (TRM-101).
    TerminalsKill = "terminals.kill" (params::TerminalRef) -> params::Empty;
    /// The init file and TSX view errors every window showing the workspace reports (PRT-116).
    ViewsErrors = "views.errors" (params::WorkspaceRef) -> params::ViewErrors;
    /// The windows that reported to the daemon, the most recently focused first: workspace, thread, view, errors
    /// (PRT-123).
    WindowsList = "windows.list" (windows::ListWindowsParams) -> windows::WindowList;
    /// All a window reports: its views and where each comes from, its added commands and bindings, its errors and
    /// widgets (PRT-123).
    WindowsState = "windows.state" (windows::WindowRef) -> windows::WindowState;
    /// Runs a registered command (id or alias) in a window, by default the most recently focused one showing the
    /// workspace; never JavaScript (PRT-124).
    WindowsRun = "windows.run" (windows::RunInWindowParams) -> windows::RanInWindow;
    /// Renders a widget out of sight in a window showing the workspace and returns what it rendered (PRT-124).
    WindowsCheckWidget = "windows.check_widget" (windows::CheckWidgetParams) -> windows::WidgetChecked;
    /// A window's answer to the daemon's `window_request` frame (PRT-124).
    WindowsAnswer = "windows.answer" (windows::AnswerWindowParams) -> params::Empty;
    /// Mounts a widget (a reference or inline code) into a view or as a floating panel; replaces an earlier mount of
    /// the same id (PRT-080, AGT-061).
    ViewsMount = "views.mount" (params::MountParams) -> params::Mounted;
    /// Merges props into a mounted widget, or rewrites its inline code (PRT-081, AGT-062).
    ViewsMountUpdate = "views.mount_update" (params::MountUpdateParams) -> params::Mounted;
    /// Removes a mounted widget or floating panel from the views (PRT-082, AGT-063).
    ViewsUnmount = "views.unmount" (params::UnmountParams) -> params::Unmounted;
    /// Saves a workspace widget or inline code as a component of the user library (PRT-083, AGT-064).
    WidgetsSave = "widgets.save" (params::SaveWidgetParams) -> params::WidgetSaved;
    /// Lists the user library's components (PRT-084, AGT-065).
    WidgetsLibrary = "widgets.library" (params::Empty) -> params::WidgetLibrary;
    /// The activity log of agent-originated actions, newest first (PRT-086, AGT-072).
    ActivityList = "activity.list" (activity::ListActivityParams) -> activity::ActivityPage;
    /// The prompts the user sent from omfp: the thread's first, then its workspace's, then the rest, newest first
    /// within each (PRT-127, DMN-216).
    PromptsHistory = "prompts.history" (activity::PromptHistoryParams) -> activity::PromptHistory;
    /// Thread summaries as on the `index` topic, without archived threads unless asked (PRT-090).
    ThreadsList = "threads.list" (lists::ListThreadsParams) -> lists::ThreadList;
    /// Every workspace as on the `index` topic (PRT-091).
    WorkspacesList = "workspaces.list" (params::Empty) -> lists::WorkspaceList;
    /// The MCP servers configured for a workspace, where each comes from, unchecked (PRT-110).
    WorkspacesMcpServers = "workspaces.mcp_servers" (params::WorkspaceRef) -> mcp::McpServers;
    /// The MCP servers of a thread's workspace, checked by its running omp (`/mcp test`, PRT-111).
    ThreadMcpServers = "thread.mcp_servers" (params::ThreadRef) -> mcp::McpServers;
    /// Reconnects a thread's MCP servers by restarting its idle omp process, then checks them again (PRT-112).
    ThreadMcpReconnect = "thread.mcp_reconnect" (mcp::McpReconnectParams) -> mcp::McpServers;
    /// How many models omp can use; checked by a short-lived omp the first time, then kept current (PRT-140).
    AuthStatus = "auth.status" (params::Empty) -> auth::AuthStatus;
    /// Available usable models before creating a thread, from a short-lived omp (UI-023, UI-030). Each entry supplies
    /// `provider` and `id`: use `threads.create { model: "<provider>/<id>", workspace: ... }`, or
    /// `thread.set_model { threadId: ..., provider: "<provider>", modelId: "<id>" }`.
    AuthModels = "auth.models" (params::Empty) -> wire::GetAvailableModelsResult as auth::AvailableModels;
    /// The providers omp can log in to with OAuth and whether it holds a credential for each, from a short-lived omp
    /// (PRT-129).
    AuthProviders = "auth.providers" (params::Empty) -> auth::AuthProviders;
    /// Logs omp in to a provider; the flow (the URL to open, progress, a code to paste) goes to the calling window
    /// as `login_*` window requests (PRT-130, PRT-131).
    AuthLogin = "auth.login" (auth::ProviderRef) -> auth::LoggedIn;
    /// Cancels the running login, if any; its `auth.login` fails (PRT-132).
    AuthLoginCancel = "auth.login_cancel" (params::Empty) -> auth::LoginCancelled;
    /// Removes every credential omp stored for a provider (PRT-133).
    AuthLogout = "auth.logout" (auth::ProviderRef) -> auth::LoggedOut;
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: PRT-092
    #[test]
    fn every_method_has_a_one_line_description_from_its_doc_comment() {
        for name in METHODS {
            let description = method_description(name).unwrap_or_default();
            assert!(!description.is_empty(), "{name} has no description");
            assert!(!description.contains('\n'), "{name}: {description}");
            assert_eq!(description.trim(), description, "{name}");
        }
        assert_eq!(
            method_description("thread.live_start").as_deref(),
            Some(
                "Starts a live voice session on the daemon machine's microphone and speakers (omp `live_start`, \
                 DMN-109): the desktop webview has no WebRTC (S-09)."
            )
        );
        assert_eq!(method_description("threads.nothing"), None);
    }
}
