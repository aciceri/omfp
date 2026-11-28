//! Types of the UI <-> daemon protocol (docs/specs/03-protocol.md).
//!
//! Shared by the daemon and the Rust clients (the `omfp` CLI, the tests); `just gen-ts` generates the UI's TypeScript
//! types from its JSON Schemas.
//!
//! A client sends [`Request`]s (decode them with [`decode_request`]); the daemon answers with [`ServerFrame`]s. Each
//! method has a marker type in [`method`] implementing [`Method`], which names its params and result types.

mod activity;
mod auth;
mod dto;
mod envelope;
mod lists;
mod mcp;
pub mod method;
mod params;
mod schema;
mod session;
mod topic;
mod windows;

pub use activity::{
    ActivityEntry, ActivityPage, ListActivityParams, PredictWordFeedbackParams, PredictWordParams,
    PredictedWord, PromptHistory, PromptHistoryEntry, PromptHistoryParams, StopThreadParams,
    ThreadsStopped,
};
pub use auth::{
    AuthProvider, AuthProviders, AuthStatus, AuthStatusParams, AvailableModel, AvailableModels,
    LoggedIn, LoggedOut, LoginActivity, LoginCancelled, OAuthCallbackParams, OAuthCallbackRelayed,
    OmpLogin, OmpLogins, ProviderRef,
};
pub use dto::{
    AllThreads, BuiltinTheme, CreatedBy, DaemonSettings, Density, DiffPage, FileCandidate,
    FileCandidates, FileChange, FileContent, FileDiff, FileKind, FileMatch, FileStatus,
    FileWritten, FontSettings, ForkOrigin, GitStatus, IndexSnapshot, JobCounts, JobDefinition,
    JobInfo, JobLine, JobPrompt, JobRunStatus, JobRuns, JobState, JobStatus, JobStream, JobTrigger,
    JobView, JobViewStatus, JobsState, KeymapEntry, KeymapMode, LiveVoice, MissedPolicy, Motion,
    NotificationMode, PathEntry, PromptThread, Provenance, RecentThreads, RepoKind, RepoStatus,
    RestartPolicy, RestartReason, RunInfo, RunTrigger, SeenRepository, Settings, Skin,
    TerminalCursor, TerminalInfo, TerminalScreen, TerminalState, TerminalStatus, Theme,
    ThreadSummary, TitleSource, UiSettings, ViewsState, WidgetInfo, WidgetSource, Workspace,
    WorkspaceBinding, WorkspaceRepo,
};
pub use envelope::{
    ErrorBody, ErrorCode, EventFrame, IndexEvent, JobEvent, JobsEvent, ProtocolError, Request,
    Response, ServerFrame, TerminalEvent, TopicEvent, TransientEvent, ViewsEvent, WorkspaceEvent,
    decode_request,
};
pub use lists::{ListThreadsParams, ThreadList, WorkspaceList};
pub use mcp::{
    McpCallParams, McpCallResult, McpReconnectParams, McpServer, McpServers, McpSource, McpState,
    McpTool, McpTools, McpToolsParams, McpTransport,
};
pub use method::{Call, METHODS, Method, method_description, method_schemas};
pub use params::{
    AddRepoParams, ApprovalMode, BashParams, BtwParams, CloseJobs, CompactParams,
    CompletePathParams, CopyWidgetParams, CreateThreadParams, CreateWorkspaceParams,
    DefineJobParams, Empty, EnableJobParams, FindFileParams, ForkParams, GoalParams, HelloParams,
    HelloResult, InteractionResponse, JobNameParams, JobRef, JobRemoveParams, JobRunsParams,
    LabelParams, LiveConnectParams, LiveConnected, LiveMuteParams, LiveMuted, LiveStartParams,
    LiveStarted, MessageParams, NavigateParams, PathCompletions, Position, PromptBehavior,
    PromptParams, PromptResult, QueueMessageParams, ReadFileParams, RemoveRepoParams,
    RemoveWorkspaceParams, ReorderThreadParams, ReorderWorkspaceParams, RepoRef, RepoSource,
    RepoSuggestions, ReportWidgetsParams, RespondParams, SearchThreadsParams, SetModelParams,
    SetSettingsParams, SetThinkingParams, SetViewsParams, SetWidgetStateParams, StartJobParams,
    StartTerminalParams, SubagentMessagesParams, SubagentRef, SubagentSteerParams, SubscribeParams,
    SubscribeResult, SuggestReposParams, TerminalInputParams, TerminalRef, TerminalResizeParams,
    TerminalScreenParams, ThreadCloseParams, ThreadGit, ThreadRef, ThreadSearchResult,
    ThreadSearchResults, ThreadWorkspace, TopicParams, UpdateThreadParams, UpdateWorkspaceParams,
    WidgetCopied, WidgetJobReport, WidgetReport, WidgetReportState, WorkspaceDiffParams,
    WorkspaceRef, WriteFileParams,
};
pub use params::{CardAnswer, CardAnswerParams, CompileWidgetParams, CompiledWidget, ViewsFile};
pub use params::{FileErrorReport, ViewErrors, WatchParams, WindowFileErrors};
pub use params::{MountParams, MountPlace, MountUpdateParams, Mounted, UnmountParams, Unmounted};
pub use schema::type_schemas;
pub use session::{ContextUsage, SessionTokens, SessionUsage, ThreadSession};
pub use topic::{Topic, TopicParseError};
pub use windows::{
    AnswerWindowParams, CheckWidgetParams, CommandSource, ListWindowsParams, RanInWindow,
    RunFailure, RunInWindowParams, ViewSource, WidgetChecked, WindowAnswer, WindowBinding,
    WindowCommand, WindowList, WindowRef, WindowReport, WindowRequest, WindowState, WindowSummary,
    WindowView,
};

/// Protocol version announced in `system.hello` (PRT-004).
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest text frame either side may send (PRT-013); bigger payloads are paged or truncated.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
