//! Params and results of the protocol methods that omfp defines itself. Results that omp defines are its wire types
//! (see `crate::method`).

use omfp_core::wire::{
    AskAnswer, GoalOp, ImageContent, LitTrue, QueuedMessageQueue, ThinkingLevel,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dto::{KeymapEntry, PathEntry, RepoStatus, SeenRepository, UiSettings, WidgetSource};
use crate::topic::Topic;

/// No params, or the `{}` result.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Empty {}

/// `system.hello`: first frame of every connection (PRT-001).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelloParams {
    /// The daemon's access token (PRT-002).
    pub token: String,
    /// Protocol version the client speaks (PRT-004).
    pub protocol: u32,
    /// Client name, recorded as provenance (PRT-033), e.g. `omfp/web`.
    pub client: String,
}

/// `system.hello` result (PRT-005).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelloResult {
    pub protocol: u32,
    /// Daemon version.
    pub daemon: String,
    /// omp version.
    pub omp: String,
    pub warnings: Vec<String>,
    /// Path under which this daemon serves widget modules to this client (`/widgets/<key>`, DMN-084, PRT-042).
    pub widget_base: String,
    /// Path under which this daemon serves the raw bytes of workspace files to this client (`/files/<key>`, DMN-250,
    /// PRT-150): `<fileBase>/<workspaceId>/<path>`.
    pub file_base: String,
    /// Number of this connection, unique while the daemon runs: the `owner.connection` of the live sessions it
    /// starts (PRT-051).
    pub connection: u64,
}

/// A position in a topic's event stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub epoch: String,
    pub seq: u64,
}

/// `subscribe` params (PRT-020, PRT-222).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeParams {
    pub topic: Topic,
    /// Last event the client has; the daemon replays from there when it still can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<Position>,
    /// Whether the subscription watches its thread (PRT-025): absent or `true` watches it; `false` follows a
    /// `thread/<id>` the window only keeps for a quick return (no widget shows it), which neither keeps its omp from
    /// the idle reaper (DMN-034) nor makes the window the one the user last used on it (DMN-060). Only `thread/<id>`
    /// topics take `false` (PRT-222).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<bool>,
}

/// `subscribe` result: catch-up from the ring buffer, or a full state (PRT-020, PRT-021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SubscribeResult {
    /// Events from `fromSeq` follow as `event` frames.
    Replay {
        epoch: String,
        #[serde(rename = "fromSeq")]
        from_seq: u64,
    },
    /// The topic's state at `seq`; live events after `seq` follow. `state` is a `omfp_core::ThreadState` for
    /// `thread/<id>`, an `IndexSnapshot` for `index`, and a list of `RepoStatus` for `workspace/<id>`.
    Snapshot {
        epoch: String,
        seq: u64,
        state: Value,
    },
}

/// Params naming one topic (`unsubscribe`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicParams {
    pub topic: Topic,
}

/// `watch` params: whether the live subscription of `topic` watches its thread (PRT-223).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WatchParams {
    pub topic: Topic,
    /// `true`: watched, as a shown thread; `false`: kept only for a quick return (PRT-222).
    pub watch: bool,
}

/// `repos.suggest` params (DMN-021).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestReposParams {
    /// Keeps repositories whose path or name contains it (case-insensitive); absent or empty: all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

/// `repos.suggest` result: most recently seen first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoSuggestions {
    pub repos: Vec<SeenRepository>,
}

/// `fs.complete` params (DMN-021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompletePathParams {
    /// An absolute path or one starting with `~`: ending with `/`, its children are listed; otherwise the children
    /// of its parent starting with its last segment.
    pub path: String,
}

/// `fs.complete` result: directories sorted by name, at most 200.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PathCompletions {
    pub entries: Vec<PathEntry>,
}

/// `workspaces.create` params (DMN-020).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceParams {
    /// Unique name; default: the folder name of `link` when given, else `workspace` (made unique).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A git repository to link into the new folder in the same call, as `workspaces.add_repo` with `link` does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

/// `workspaces.update` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkspaceParams {
    pub workspace_id: String,
    /// New unique name; the folder is not moved.
    pub name: String,
}

/// `workspaces.remove` params (DMN-028).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemoveWorkspaceParams {
    pub workspace_id: String,
    /// Remove worktrees and clones even with uncommitted changes (DMN-028).
    #[serde(default)]
    pub force: bool,
}

/// Params naming one workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRef {
    pub workspace_id: String,
}

/// `workspaces.add_repo` params (DMN-024).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddRepoParams {
    pub workspace_id: String,
    /// Entry name in the folder; default: derived from the source (made unique).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub source: RepoSource,
}

/// Where a repository added to a workspace comes from (DMN-024).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RepoSource {
    /// A symlink `<name> → <path>`.
    Link { path: String },
    /// `git worktree add` of a repository.
    #[serde(rename_all = "camelCase")]
    Worktree {
        /// A path inside a git repository (e.g. a `repos.suggest` path).
        repo: String,
        /// Existing branch to check out, or new branch name; default: a new `omfp/<slug>-<n>` branch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// Base of a new branch; default: the repository's checked-out branch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_ref: Option<String>,
    },
    /// `git clone <url> <name>`.
    Clone { url: String },
}

/// `workspaces.remove_repo` params (DMN-025).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemoveRepoParams {
    pub workspace_id: String,
    /// The repository's name in the workspace.
    pub name: String,
    /// Remove a worktree or clone even with uncommitted changes.
    #[serde(default)]
    pub force: bool,
    /// Also delete a worktree's branch.
    #[serde(default)]
    pub delete_branch: bool,
}

/// `workspaces.status` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoRef {
    pub workspace_id: String,
    /// The repository's name in the workspace.
    pub repo: String,
}

/// `workspaces.diff` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiffParams {
    pub workspace_id: String,
    /// The repository's name in the workspace.
    pub repo: String,
    /// Only this file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `nextCursor` of the previous page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `workspaces.read_file` params (DMN-089).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileParams {
    pub workspace_id: String,
    /// Relative to the workspace folder (else to one of its repositories), or absolute inside the workspace folder.
    pub path: String,
    /// The `hash` the client already shows: while the file still has it, the result is `unchanged` and carries
    /// neither its content nor its `HEAD` text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_hash: Option<String>,
}

/// `workspaces.find_file` params (DMN-092).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FindFileParams {
    pub workspace_id: String,
    /// A bare/partial path, or with `fuzzy`, a case-insensitive subsequence of a workspace-relative path.
    pub query: String,
    /// A thread of the workspace: files its tool cards name rank first among candidates of the same match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Search every eligible workspace file before applying the result limit; empty query lists candidates.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fuzzy: bool,
}

/// `workspaces.write_file` params (DMN-090).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileParams {
    pub workspace_id: String,
    /// As in `workspaces.read_file`, without the fallback to the repositories.
    pub path: String,
    /// The new text of the file (UTF-8).
    pub content: String,
    /// The `hash` of the file the text was based on: a file that no longer has it is `conflict`. Absent: the file
    /// must not exist yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_hash: Option<String>,
}

/// The workspace of a new thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ThreadWorkspace {
    /// An existing workspace.
    Id(String),
    /// A new workspace, as `workspaces.create` makes it; its name defaults to the thread's title, else to the first
    /// words of its first message.
    New(CreateWorkspaceParams),
}

/// omp's `--approval-mode` (`tools.approvalMode`): which tool calls omp asks the user to approve (DMN-074, DMN-210).
/// omp 18.22.0 exits at startup on any other value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalMode {
    /// Runs read tools; asks before write and exec tools.
    AlwaysAsk,
    /// Like `always-ask`, but a file read or write can be approved for a file or folder, for the session or for
    /// good.
    AlwaysAskCached,
    /// Runs read and write tools; asks before exec tools.
    Write,
    /// Runs every tool without asking (the default).
    Yolo,
}

impl ApprovalMode {
    /// The value omp takes, as `--approval-mode <value>`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlwaysAsk => "always-ask",
            Self::AlwaysAskCached => "always-ask-cached",
            Self::Write => "write",
            Self::Yolo => "yolo",
        }
    }
}

/// `threads.create` params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateThreadParams {
    /// For scripts and agents; without it the title is generated from the conversation (DMN-073).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Required: `{ "id": … }` for an existing workspace or `{ "new": { "name"?, "link"? } }` for a new one.
    pub workspace: ThreadWorkspace,
    /// `provider/id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Value>")]
    pub thinking_level: Option<ThinkingLevel>,
    /// omp's `--approval-mode`; default `yolo` (DMN-074, DMN-210).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<ApprovalMode>,
    /// First message, sent once the thread is created; when omp cannot start for it, the thread is not created
    /// (DMN-211).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Images of the first message, sent with `prompt`; a first message may be images only (DMN-211).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<Value>")]
    pub images: Vec<ImageContent>,
}

/// `threads.update` params; absent fields are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadParams {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// omp's `--approval-mode`, applied from the next start of the thread's omp (DMN-074, DMN-210).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<ApprovalMode>,
}

/// `workspaces.reorder` params (PRT-170): where a workspace goes in the left column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReorderWorkspaceParams {
    pub workspace_id: String,
    /// The workspace it goes just before; absent: last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_id: Option<String>,
}

/// `threads.reorder` params (PRT-171): where a thread goes in the left column, in its workspace or another one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReorderThreadParams {
    pub thread_id: String,
    /// The workspace it goes to; absent: its own. Another one moves the thread there (DMN-283).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// The thread of that workspace it goes just before; absent: last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_id: Option<String>,
}

/// Params naming one thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRef {
    pub thread_id: String,
}

/// `threads.search` params (DMN-076).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchThreadsParams {
    /// Case-insensitive substring of the conversation text; shorter than 2 characters finds nothing.
    pub query: String,
}

/// `threads.search` result: most recently updated threads first, at most 200.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchResults {
    pub results: Vec<ThreadSearchResult>,
}

/// A thread whose conversation contains the query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchResult {
    pub thread_id: String,
    /// A short excerpt around the first match.
    pub snippet: String,
}

/// How a prompt sent while the agent is busy is queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PromptBehavior {
    Steer,
    FollowUp,
}

/// `thread.prompt` params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptParams {
    pub thread_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<Value>")]
    pub images: Vec<ImageContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<PromptBehavior>,
}

/// `thread.prompt` result, sent once omp admits the prompt (PRT-030).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptResult {
    /// The `id` of the `prompt_result` that ends this prompt.
    pub prompt_id: String,
    /// `false` when omp handled the prompt locally: no `prompt_result` follows.
    pub agent_invoked: bool,
}

/// `thread.steer` / `thread.follow_up` params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageParams {
    pub thread_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<Value>")]
    pub images: Vec<ImageContent>,
}

/// `thread.queue.remove` / `thread.queue.promote` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueMessageParams {
    pub thread_id: String,
    /// Text of the queued message.
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Value>")]
    pub queue: Option<QueuedMessageQueue>,
}

/// `thread.set_model` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetModelParams {
    pub thread_id: String,
    pub provider: String,
    pub model_id: String,
}

/// `thread.set_thinking` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetThinkingParams {
    pub thread_id: String,
    #[schemars(with = "Value")]
    pub level: ThinkingLevel,
}

/// `thread.compact` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompactParams {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// `thread.navigate` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NavigateParams {
    pub thread_id: String,
    pub entry_id: String,
    /// Summarize the branch being left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarize: Option<bool>,
    /// Instructions for the summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// `thread.fork` and `threads.fork` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForkParams {
    pub thread_id: String,
    /// Fork history through this message entry (omp keeps the tool results answering it); the whole session when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
}

/// `thread.label` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelParams {
    pub thread_id: String,
    pub entry_id: String,
    /// The label is cleared when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// `thread.subagent_messages` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubagentMessagesParams {
    pub thread_id: String,
    pub subagent_id: String,
    /// `nextByte` of the previous read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_byte: Option<i64>,
}

/// `thread.subagent_steer` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSteerParams {
    pub thread_id: String,
    pub subagent_id: String,
    pub text: String,
}

/// Params naming one subagent of a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRef {
    pub thread_id: String,
    pub subagent_id: String,
}

/// `thread.goal` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GoalParams {
    pub thread_id: String,
    #[schemars(with = "Value")]
    pub op: GoalOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<i64>,
}

/// `thread.btw` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BtwParams {
    pub thread_id: String,
    pub question: String,
    /// Continue this side-question topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
}

/// `thread.bash` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BashParams {
    pub thread_id: String,
    pub command: String,
    /// Keep the command and its output out of the model's context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_from_context: Option<bool>,
}

/// `thread.git` result: the git status of every repository of the thread's workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGit {
    pub workspace_id: String,
    /// In the order of the workspace's `repos`.
    pub repos: Vec<RepoStatus>,
}

/// `thread.live_connect` params (PRT-050, DMN-100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LiveConnectParams {
    pub thread_id: String,
    /// SDP offer of the client's WebRTC peer: one sendrecv Opus audio track plus an `oai-events` data channel.
    pub sdp_offer: String,
}

/// `thread.live_connect` result: omp's answer once the call is signaled (PRT-050).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LiveConnected {
    /// SDP answer to apply to the peer right away.
    pub sdp_answer: String,
    /// The realtime voice omp uses.
    pub voice: String,
}

/// `thread.live_start` result: the voice omp uses once the host's session records (PRT-053, DMN-109).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LiveStarted {
    /// The realtime voice omp uses.
    pub voice: String,
}

/// `thread.live_mute` params (PRT-050, DMN-102).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LiveMuteParams {
    pub thread_id: String,
    pub muted: bool,
}

/// `thread.live_mute` result: the mute state omp applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LiveMuted {
    pub muted: bool,
}

/// `interaction.respond` params (DMN-051).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RespondParams {
    pub thread_id: String,
    pub interaction_id: String,
    pub response: InteractionResponse,
}

/// Answer to an interactive request; on the wire an object with exactly one of these keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "ResponseRepr", into = "ResponseRepr")]
pub enum InteractionResponse {
    /// Answers `select`, `input` or `editor`.
    Value(String),
    /// Answers `confirm`.
    Confirmed(bool),
    /// Dismisses the request.
    Cancelled,
    /// Answers `ask`: one entry per question, in order (DMN-054).
    Answers(Vec<AskAnswer>),
}

// Wire form of `InteractionResponse`.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "Answer to an interactive request: exactly one of `value`, `confirmed`, `cancelled` (always true) or `answers`."
)]
enum ResponseRepr {
    Value(String),
    Confirmed(bool),
    Cancelled(#[schemars(with = "ConstTrue")] LitTrue),
    Answers(#[schemars(with = "Vec<Value>")] Vec<AskAnswer>),
}

impl From<ResponseRepr> for InteractionResponse {
    fn from(repr: ResponseRepr) -> Self {
        match repr {
            ResponseRepr::Value(value) => Self::Value(value),
            ResponseRepr::Confirmed(confirmed) => Self::Confirmed(confirmed),
            ResponseRepr::Cancelled(LitTrue) => Self::Cancelled,
            ResponseRepr::Answers(answers) => Self::Answers(answers),
        }
    }
}

impl From<InteractionResponse> for ResponseRepr {
    fn from(response: InteractionResponse) -> Self {
        match response {
            InteractionResponse::Value(value) => Self::Value(value),
            InteractionResponse::Confirmed(confirmed) => Self::Confirmed(confirmed),
            InteractionResponse::Cancelled => Self::Cancelled(LitTrue),
            InteractionResponse::Answers(answers) => Self::Answers(answers),
        }
    }
}

impl JsonSchema for InteractionResponse {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "InteractionResponse".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        ResponseRepr::json_schema(generator)
    }
}

/// Schema of the constant `true`.
struct ConstTrue;

impl JsonSchema for ConstTrue {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ConstTrue".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "const": true })
    }
}

/// `settings.set` params: values merged into the config file (DMN-009); absent ones are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetSettingsParams {
    /// `[ui]` values to set; absent fields are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiSettings>,
    /// Bindings replacing the ones with the same mode and keys (`command: ""` unbinds the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keymap: Option<Vec<KeymapEntry>>,
}

/// `views.set` params (DMN-083).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetViewsParams {
    pub workspace_id: String,
    /// The whole manifest (AGT-010), written to `<workspace>/.omfp/views.json`.
    pub manifest: Value,
}

/// `widgets.copy` params (DMN-087).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CopyWidgetParams {
    pub workspace_id: String,
    /// `omfp:<name>` (a built-in) or `user:<name>` (the user library).
    pub widget: String,
    /// Folder name in `.omfp/widgets/`; default: the widget's own name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `widgets.copy` result: the new widget is `./widgets/<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetCopied {
    pub name: String,
}

/// `widgets.state_set` params (DMN-086).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetWidgetStateParams {
    pub workspace_id: String,
    /// The widget instance: its `instance` prop, else its reference (AGT-010).
    pub instance: String,
    pub key: String,
    /// Any JSON value; `null` deletes the key.
    pub value: Value,
}

/// `widgets.report` params (PRT-043, AGT-026): what this connection's window shows of a workspace's own widgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportWidgetsParams {
    /// The workspace whose view the window shows; `null` when it shows none.
    pub workspace_id: Option<String>,
    /// The position of `views/<workspaceId>` the window rendered; `null` before that topic arrived.
    pub views: Option<Position>,
    /// Every workspace and user widget instance of the view the window shows (built-ins are not reported).
    pub widgets: Vec<WidgetReport>,
    /// The runtime errors the window shows of init files and TSX views (PRT-115); absent means none.
    #[serde(default)]
    pub file_errors: Vec<FileErrorReport>,
    /// What else the window shows (PRT-122): its thread, views, commands; absent from clients that do not report it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<crate::windows::WindowReport>,
}

/// What kind of file a [`FileErrorReport`] is about (PRT-115).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileErrorKind {
    /// An init file: the user's or the workspace's `.omfp/init.ts`.
    Init,
    /// A TSX view file of the workspace's `.omfp/views/`.
    View,
}

/// One runtime error a window shows of an init file or a TSX view (PRT-115).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileErrorReport {
    pub kind: FileErrorKind,
    /// `user` for the user's init file, else the workspace-relative path (`.omfp/init.ts`, `.omfp/views/ops.tsx`).
    pub file: String,
    /// The error as the window shows it: a compile error with `file:line:column`, a throw, a render or layout error.
    pub message: String,
}

/// `views.errors` result (PRT-116): the latest init file and TSX view errors of every window whose
/// report is about the workspace, by connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewErrors {
    /// One entry per window showing the workspace, those without errors included.
    pub windows: Vec<WindowFileErrors>,
}

/// The init file and TSX view errors one window reports (PRT-116).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WindowFileErrors {
    /// The `client` of the window's `system.hello`.
    pub client: String,
    pub errors: Vec<FileErrorReport>,
}

/// One widget instance as a window shows it (PRT-043).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetReport {
    /// The view node's reference (`./widgets/tests`, `user:clock`, or `omfp:<name>` overridden by the workspace).
    pub reference: String,
    /// The node's `instance` prop when it is a string, else its reference.
    pub instance: String,
    pub source: WidgetSource,
    /// The widget folder's name.
    pub name: String,
    /// The folder version the window imported; `null` while nothing is imported.
    pub version: Option<String>,
    pub state: WidgetReportState,
    /// The compile stand-in's, import's and error boundary's errors (`file:line:column: message` when known).
    pub errors: Vec<String>,
    /// Console errors and uncaught errors whose stack names the widget's module, oldest first.
    pub console: Vec<String>,
    /// The text the widget renders.
    pub text: String,
    /// Its `useJob` jobs (AGT-019).
    pub jobs: Vec<WidgetJobReport>,
}

/// Where a widget instance stands in a window (PRT-043).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WidgetReportState {
    Loading,
    Rendered,
    CompileError,
    LoadError,
    RenderError,
}

/// A `useJob` job of a widget instance (AGT-019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetJobReport {
    pub command: String,
    /// `pending`, `running`, `succeeded` or `failed`.
    pub status: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// How many jobs the instance started for the command.
    pub runs: u32,
    pub started_at: Option<i64>,
    /// The last output lines (stdout and stderr), oldest first.
    pub tail: Vec<String>,
}

/// `jobs.start` params (DMN-080, DMN-215).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartJobParams {
    pub workspace_id: String,
    /// Run as `sh -c <command>` in the workspace folder.
    pub command: String,
    /// A job of the same workspace the new one takes the place of: killed if it still runs, then forgotten (its
    /// topic is `not_found` for new subscribers); an unknown id is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
}

/// Params naming one job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobRef {
    pub job_id: String,
}

/// `terminals.start` params (TRM-100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartTerminalParams {
    pub workspace_id: String,
    /// Run as `sh -c <command>` attached to a PTY.
    pub command: String,
    /// Workspace-relative directory; omitted means the workspace folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

/// Params naming one terminal session (TRM-100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRef {
    pub terminal_id: String,
}

/// `terminals.input` params (TRM-101): raw bytes encoded as base64 for JSON transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInputParams {
    pub terminal_id: String,
    pub data: String,
}

/// `terminals.resize` params (TRM-101).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TerminalResizeParams {
    pub terminal_id: String,
    pub cols: u16,
    pub rows: u16,
}

/// `terminals.screen` params (TRM-106).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TerminalScreenParams {
    pub terminal_id: String,
    /// Return immediately when the screen has a different revision; otherwise wait for output, resize or exit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_revision: Option<u64>,
    /// Long-poll duration in milliseconds; at most 30 seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u32>,
}

/// `jobs.runs` params (PRT-203).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobRunsParams {
    pub workspace_id: String,
    /// The declared job's name.
    pub job: String,
    /// Only runs started before this run id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// Defaults to 20, capped at 50.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `jobs.run` params (PRT-204): one declared job of a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobNameParams {
    pub workspace_id: String,
    pub job: String,
}

/// `jobs.define` params (PRT-205).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DefineJobParams {
    pub workspace_id: String,
    pub name: String,
    /// The job as in `.omfp/jobs.json`; the daemon validates it (DMN-320).
    pub job: Value,
    /// Overwrite an existing job of that name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<bool>,
}

/// `jobs.remove` params (PRT-206).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobRemoveParams {
    pub workspace_id: String,
    pub name: String,
}

/// `jobs.enable` params (PRT-207).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnableJobParams {
    pub workspace_id: String,
    pub name: String,
    pub enabled: bool,
}

/// What closing a thread does with the transient runs its agent started (PRT-208).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloseJobs {
    /// Kill them as `jobs.kill` does, then close the thread.
    Kill,
    /// Leave them running.
    Keep,
}

/// `threads.archive` and `threads.delete` params (PRT-208).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCloseParams {
    pub thread_id: String,
    /// Required while runs the thread's agent started still run; otherwise `conflict`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jobs: Option<CloseJobs>,
}

/// Where `views.mount` puts a widget in a view (PRT-080); `omfp_ui_mount` takes the same `place` (AGT-061).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MountPlace {
    /// A tab of the view's `tabs` container whose `id` prop is `side`.
    #[default]
    SideTab,
    /// A tab of the view's `tabs` container whose `id` prop is `main`, after the conversation.
    MainTab,
    /// The end of the view's root container.
    End,
}

/// `views.mount` params (PRT-080, AGT-061): exactly one of `widget` and `code`; `panel` excludes `view` and `place`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MountParams {
    pub workspace_id: String,
    /// The mount id, a widget name (letters, digits, `_`, `-`, `.` not first): the node's `instance` prop, the panel
    /// id, and the folder `code` is written to.
    pub id: String,
    /// A widget reference (`./widgets/<name>`, `user:<name>`, `omfp:<name>`); a bare name means `./widgets/<name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<String>,
    /// The source of a component (TSX), written to `.omfp/widgets/<id>/index.tsx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The widget's props; `instance` is always the mount id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub props: Option<serde_json::Map<String, Value>>,
    /// The view to mount into (default: the default view); a new id creates a view showing only the widget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// Where in the view (default: `side_tab`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<MountPlace>,
    /// Mount as a floating panel over every view (`panels.<id>`) instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub panel: bool,
}

/// `views.mount_update` params (PRT-081, AGT-062).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MountUpdateParams {
    pub workspace_id: String,
    pub id: String,
    /// Merged into the node's props; a `null` value deletes the prop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub props: Option<serde_json::Map<String, Value>>,
    /// New source of `.omfp/widgets/<id>/index.tsx`, for a mount of `./widgets/<id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// `views.mount` and `views.mount_update` result (PRT-080, PRT-081).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Mounted {
    pub id: String,
    /// The widget reference the node renders.
    pub reference: String,
    /// The view showing it; `null` for a floating panel.
    pub view: Option<String>,
    /// Whether it is the floating panel `id`.
    pub panel: bool,
    /// Whether a node of this mount id existed before, or the mount's code folder did (its code was replaced).
    pub replaced: bool,
}

/// `views.unmount` params (PRT-082, AGT-063).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnmountParams {
    pub workspace_id: String,
    pub id: String,
}

/// `views.unmount` result (PRT-082).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Unmounted {
    /// The views it removed nodes from (and kept), in id order.
    pub views: Vec<String>,
    /// The views it removed because they were left empty, in id order.
    pub removed_views: Vec<String>,
    /// Whether it removed the floating panel `id`.
    pub panel: bool,
}

/// `widgets.save` params (PRT-083, AGT-064): exactly one of `widget` (with `workspaceId`) and `code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveWidgetParams {
    /// The component's name in the user library (a widget name).
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// A widget of that workspace: `./widgets/<name>` or a bare name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<String>,
    /// The component's source (TSX), saved as its `index.tsx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Replace an existing component of that name.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace: bool,
}

/// `widgets.save` result (PRT-083).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetSaved {
    pub name: String,
    /// `user:<name>`.
    pub reference: String,
}

/// `widgets.library` result (PRT-084): the user library by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetLibrary {
    pub components: Vec<LibraryComponent>,
}

/// A component of the user library (PRT-084, AGT-065).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LibraryComponent {
    pub name: String,
    /// `user:<name>`.
    pub reference: String,
    /// The entry file; `null` when the folder has none.
    pub entry: Option<String>,
    /// The folder's files, relative, in path order.
    pub files: Vec<String>,
    /// The folder's version (DMN-084).
    pub version: String,
    /// The entry's leading comment, whitespace collapsed; empty without one.
    pub description: String,
}
