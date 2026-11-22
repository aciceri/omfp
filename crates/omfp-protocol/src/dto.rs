//! Entities the daemon exposes: workspaces, thread summaries, git status and repository suggestions.
//!
//! Timestamps are Unix epoch milliseconds.

use std::collections::BTreeMap;

use omfp_core::ProcessState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Who created an entity (ARC-009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub created_by: CreatedBy,
    /// `client` of the creating connection's `system.hello` (PRT-033).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

/// The principal behind a created entity: the user, the agent of a thread, or an MCP client outside omfp (PRT-093),
/// named by the provenance's `client`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreatedBy {
    User,
    Thread {
        #[serde(rename = "threadId")]
        thread_id: String,
    },
    Mcp,
    /// A run of the declared job `job` of the entity's workspace (PRT-209).
    Job {
        job: String,
    },
}

/// A repository the daemon has seen in a workspace, suggested when adding repositories (DMN-021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeenRepository {
    /// Canonical path: a link's target or a clone, the main repository of a worktree.
    pub path: String,
    /// The path's last component.
    pub name: String,
    pub last_seen_at: i64,
}

/// A child directory on the daemon host, offered while typing a path (DMN-021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PathEntry {
    pub name: String,
    /// Absolute path, `~` expanded.
    pub path: String,
    /// The directory is the root of a git repository.
    pub is_git_root: bool,
}

/// A folder threads work in; its repositories are its top-level entries (D-11, DMN-020).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    /// Unique among workspaces.
    pub name: String,
    /// Absolute path of the folder (under the daemon's workspaces root), unique among workspaces.
    pub path: String,
    /// Repositories discovered in the folder (DMN-022): the folder itself first when it is a git root, then the
    /// entries by name.
    pub repos: Vec<WorkspaceRepo>,
    /// Threads living in the workspace, archived ones included, oldest first (DMN-027).
    pub thread_ids: Vec<String>,
    pub provenance: Provenance,
    pub created_at: i64,
    /// Its place among the workspaces in the left column, from 0; the same in every window (PRT-172, DMN-284).
    pub position: u32,
    /// Its declared jobs at a glance; absent without jobs and without a refused file (PRT-210).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jobs: Option<JobCounts>,
}

/// A repository discovered in a workspace folder (DMN-022).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRepo {
    /// Entry name in the folder; the folder's own name for the root.
    pub name: String,
    /// Path inside the workspace (the symlink path for a link, never its target).
    pub path: String,
    #[serde(flatten)]
    pub kind: RepoKind,
    /// `None` on a detached HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Commit id of HEAD; `None` before the first commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// The workspace folder itself is this repository.
    pub is_root: bool,
    /// The checkout with symlinks resolved: entries with the same canonical path are the same files (DMN-027).
    pub canonical_path: String,
    /// Other workspaces holding the same checkout (same canonical path), so clients can warn (DMN-027).
    pub shared_with: Vec<String>,
}

/// How a repository sits in its workspace folder (DMN-022).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RepoKind {
    /// A symlink to a repository elsewhere.
    Link {
        /// The symlink's target, as written in the link.
        target: String,
    },
    /// A linked git worktree.
    Worktree {
        /// Root of the main working tree (`git rev-parse --git-common-dir`).
        main: String,
    },
    /// A repository of its own (a clone, or a folder initialized in place).
    Clone,
}

/// What the thread list shows about a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    /// `""` until the first message (DMN-073).
    pub title: String,
    pub title_source: TitleSource,
    /// The workspace the thread lives in: its omp cwd (ARC-008, DMN-029).
    pub workspace_id: String,
    /// `omfp_core::ProcessState`.
    #[schemars(with = "serde_json::Value")]
    pub process: ProcessState,
    /// Idle with nothing queued or pending (omp's `session_settled`).
    pub settled: bool,
    /// Interactive requests are waiting for an answer.
    pub needs_attention: bool,
    /// Why the thread's omp should restart while background work holds the restart back (PRT-121, PRT-134,
    /// DMN-204): each reason once, in the order they came; `thread.mcp_reconnect` restarts it now. Absent when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restart_pending: Vec<RestartReason>,
    pub provenance: Provenance,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<i64>,
    /// Its place among the threads of its workspace (archived ones included) in the left column, from 0; the same in
    /// every window (PRT-172, DMN-284).
    pub position: u32,
    /// The thread this one was forked from (`threads.fork`, DMN-300); absent for a thread that is no fork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<ForkOrigin>,
}

/// Where a forked thread comes from (PRT-181): the source thread and the message it was forked at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForkOrigin {
    /// The source thread; it may have been deleted since.
    pub thread_id: String,
    /// The source's title when it was forked.
    pub title: String,
    /// The omp session entry the fork ends at; absent when the whole session was copied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    /// The first line of that entry's message text (at most 120 characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Why the daemon restarts a thread's omp by itself (PRT-134, DMN-228): omp loads MCP servers and lists a provider's
/// models only at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestartReason {
    /// The MCP servers the thread's omp would load changed: a config file (DMN-201) or `/mcp add` (DMN-202).
    McpConfig,
    /// An MCP server was reauthorized (`/mcp reauth`, DMN-202): omp applies the credential on the next connection.
    McpReauth,
    /// A login stored a provider's credential (DMN-224): omp lists its models from the next start.
    Login,
}

/// Where a thread's title comes from (DMN-073).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TitleSource {
    /// The first words of the first message (or `""` before it), until a title is generated or set.
    #[default]
    Placeholder,
    /// Generated from the conversation by the daemon.
    Generated,
    /// Set by the user or an agent (`threads.create`, `threads.update`, host tool `set_thread_title`); never replaced.
    Set,
}

/// The user's theme: CSS applied after the built-in tokens (DMN-075).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    /// `null`: the built-in themes only.
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub css: Option<String>,
}

/// UI settings and key bindings the daemon keeps in its config file (DMN-009, UI-097).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub ui: UiSettings,
    /// Bindings over the defaults, in file order; `command: ""` removes a default binding.
    pub keymap: Vec<KeymapEntry>,
    /// Why the last reload of the config file failed (the previous settings stay); `null` when it is valid.
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub error: Option<String>,
}

/// The `[ui]` table of the config file; an absent value is the default (UI-097).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UiSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<Density>,
    /// Root font size in px (10..=28, default 17); every rem-based size token scales with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 10, max = 28))]
    pub font_size: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<NotificationMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fonts: Option<FontSettings>,
    /// The colour theme (UI-510); absent is Synth, omfp's own. The windows apply the user theme (`/theme.css`) only
    /// with `matugen`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<BuiltinTheme>,
    /// Show informational xdev notices in the transcript (UI-043); absent or false hides them, without dropping events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_xdev_notices: Option<bool>,
    /// The skin (UI-794): `crush` (absent, the default) or the opt-in `boring`; independent of `theme`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<Skin>,
    /// How many hidden threads each window keeps followed for a quick return (UI-996): a number (0..=100, default 20)
    /// or `"all"`. Their states stay in the window's memory, but their omp processes are not kept alive (PRT-222).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_threads: Option<RecentThreads>,
    /// The voice of live sessions (UI-999, PRT-228): one of omp's realtime voices; absent leaves it to omp (`sol`, or
    /// omp's own `live.voice` setting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_voice: Option<LiveVoice>,
}

/// A realtime voice of omp's live sessions (PRT-228, UI-999): the voices omp 18.22.0 accepts
/// (`packages/coding-agent/src/live/voices.ts`), named as omp names them; `sol` is omp's default. Any other name does
/// not decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveVoice {
    Arbor,
    Breeze,
    Cove,
    Ember,
    Juniper,
    Maple,
    Sol,
    Spruce,
    Vale,
}

impl LiveVoice {
    /// Every voice, in omp's order.
    pub const ALL: [Self; 9] = [
        Self::Arbor,
        Self::Breeze,
        Self::Cove,
        Self::Ember,
        Self::Juniper,
        Self::Maple,
        Self::Sol,
        Self::Spruce,
        Self::Vale,
    ];

    /// The name omp takes (`live_connect`/`live_start` `voice`, its `live.voice` setting).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Arbor => "arbor",
            Self::Breeze => "breeze",
            Self::Cove => "cove",
            Self::Ember => "ember",
            Self::Juniper => "juniper",
            Self::Maple => "maple",
            Self::Sol => "sol",
            Self::Spruce => "spruce",
            Self::Vale => "vale",
        }
    }
}

/// How many hidden threads a window keeps followed (UI-996): at most a number (0..=100; 0 keeps none), or `"all"`,
/// every thread shown in the window until it is closed or reloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RecentThreads {
    Count(u8),
    All(AllThreads),
}

/// `"all"`: no bound on the threads a window keeps (UI-996).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AllThreads {
    All,
}

/// Row height and spacing of the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    Normal,
}

/// A colour theme of the UI (UI-510, PRT-143): an official palette mapped onto the Material 3 roles in light and dark,
/// one of omfp's calm palettes for the boring skin (UI-794), one derived from the system accent colour where the web
/// view exposes it (`accent`, UI-512), or the user theme (`matugen`, DMN-075).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinTheme {
    /// omfp's own palette, cyan to indigo (UI-139; default).
    Synth,
    /// Catppuccin: Latte in light, Mocha in dark.
    Catppuccin,
    /// Gruvbox light and dark.
    Gruvbox,
    /// Tokyo Night: Day in light, Night in dark.
    TokyoNight,
    /// Nord: Snow Storm surfaces in light, Polar Night in dark.
    Nord,
    /// Solarized light and dark.
    Solarized,
    /// Rosé Pine: Dawn in light, the main variant in dark.
    RosePine,
    /// Cool neutral grays with a restrained indigo accent (inspired by Linear).
    Graphite,
    /// Warm off-white or warm charcoal with a muted blue accent (inspired by Notion).
    Paper,
    /// Ivory or warm dark with a terracotta accent (inspired by claude.ai).
    Clay,
    /// GitHub's Primer light and dark (MIT-licensed primitives).
    Primer,
    /// Follows the system accent colour (macOS); elsewhere Synth.
    Accent,
    /// The user theme (DMN-075: `theme.set`, the theme file matugen writes) over Synth; Synth without one.
    Matugen,
}

/// How much the UI animates (UI-145); `prefers-reduced-motion: reduce` caps it at `none`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    /// Every effect of the motion catalogue (default).
    Full,
    /// State indicators and short UI feedback only, with a steady caret.
    Reduced,
    /// No animation or transition.
    None,
}

/// The look of the whole UI (UI-794, PRT-213): Crush (default) or the opt-in boring skin. Any palette works with either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Skin {
    /// omfp's own look (UI-120): monospace chrome, box-drawing glyphs, gradients and motion (default).
    Crush,
    /// Calm and neutral (UI-795): system sans-serif, gray chrome, light borders, no decorative motion.
    Boring,
}

/// When the UI shows system notifications (UI-096).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationMode {
    /// Only while the window has no focus (default).
    Background,
    Always,
    Never,
}

/// CSS `font-family` values overriding the font tokens; absent ones keep the theme's.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FontSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// One key binding of the config file's `[keymap]` (UI-097).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeymapEntry {
    pub mode: KeymapMode,
    /// Key sequence, e.g. `g g`, `Ctrl-d`, `Space n`.
    pub keys: String,
    /// Command id; `""` removes the default binding of `keys`.
    pub command: String,
}

/// One binding of a workspace's `.omfp/keymap.json` (UI-210, AGT-030): a `[keymap]` entry (UI-097) with optional
/// arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBinding {
    /// `normal` when the file leaves it out.
    pub mode: KeymapMode,
    /// Key sequence, e.g. `g g`, `Ctrl-d`, `Space t`.
    pub keys: String,
    /// Command id, built-in or registered by a widget (`useCommand`); `""` removes the binding of `keys`.
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// The keymap mode a binding belongs to: `[keymap]`, `[keymap.insert]`, `[keymap.cmdline]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeymapMode {
    Normal,
    Insert,
    Cmdline,
}

/// State of the `index` topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexSnapshot {
    pub workspaces: Vec<Workspace>,
    pub threads: Vec<ThreadSummary>,
    pub theme: Theme,
    pub settings: Settings,
    /// The version of the user's init file (`~/.config/omfp/init.ts`, PRT-095): a hash of its bytes;
    /// `null` without one.
    #[serde(default)]
    pub init: Option<String>,
    /// How many models omp can use (PRT-141, `auth.status`); `null` until the daemon checked, which the first
    /// `auth.status` does (DMN-235).
    #[serde(default)]
    pub models: Option<u32>,
    /// The login running now (DMN-290), whichever connection started it; `null` when none runs.
    #[serde(default)]
    pub login: Option<crate::LoginActivity>,
}

/// Git status of one repository of a workspace (the `workspace/<id>` topic, DMN-026).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    /// The repository's name in the workspace.
    pub name: String,
    pub status: GitStatus,
}

/// Git state of a repository relative to its upstream or base ref (DMN-026).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    /// `None` on a detached HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// What `ahead`/`behind` count against: the branch's upstream; absent without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    /// Commits on HEAD not in the base ref.
    pub ahead: u32,
    /// Commits in the base ref not on HEAD.
    pub behind: u32,
    /// Uncommitted changes, untracked files included.
    pub dirty: bool,
    /// Files changed since the base ref, committed or not.
    pub files: Vec<FileChange>,
}

/// One changed file with its line counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// Previous path of a renamed file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
}

/// Kind of change to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
}

/// One page of a workspace diff (PRT-013).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiffPage {
    pub files: Vec<FileDiff>,
    /// Pass as `cursor` to fetch the next page; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Unified diff of one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub patch: String,
    /// The patch was cut to fit the frame limit.
    pub truncated: bool,
}

/// What a file of a workspace holds, as `workspaces.read_file` shows it (DMN-089).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// UTF-8 text without NUL bytes.
    Text,
    /// An image by its extension (`png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `bmp`, `ico`, `svg`).
    Image,
    /// Anything else: named, never shown.
    Binary,
}

/// A file of a workspace (DMN-089).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    /// Relative to the workspace folder, `..` resolved (symbolic links kept as they are); for a file `outside` the
    /// folder, the absolute path, `..` resolved.
    pub path: String,
    pub kind: FileKind,
    /// Size in bytes.
    pub size: u64,
    /// Hex SHA-1 of the file's bytes: the `expectedHash` of `workspaces.write_file`.
    pub hash: String,
    /// The `knownHash` of the request still matches: no `content`, no `head` and no `headHash`.
    #[serde(default)]
    pub unchanged: bool,
    /// `text`: the text (its first 2 MiB, cut at a line end, when `truncated`); `image`: the bytes in base64 (absent
    /// over 8 MiB); absent for `binary` and when `unchanged`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The text was cut (DMN-089's cap).
    #[serde(default)]
    pub truncated: bool,
    /// The MIME type of an image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// The name of the workspace repository holding the file; for a file `outside` the folder, the name of the folder
    /// of the git working tree holding it (DMN-311); absent outside every repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The file's text at `HEAD` of that repository; absent for a file `HEAD` does not have (new, untracked), for
    /// binary, truncated or `unchanged` files and outside repositories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// For an image: the hex SHA-1 of its bytes at `HEAD` of that repository (DMN-255), whatever its size; absent for
    /// an image `HEAD` does not have, for text and binary files, when `unchanged` and outside repositories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_hash: Option<String>,
    /// The file is outside the workspace folder, read by its absolute path (DMN-310, PRT-195): read-only,
    /// `workspaces.write_file` refuses it (DMN-313).
    #[serde(default)]
    pub outside: bool,
}

/// The result of `workspaces.write_file` (DMN-090).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileWritten {
    /// Relative to the workspace folder.
    pub path: String,
    /// Hex SHA-1 of the written bytes.
    pub hash: String,
    pub size: u64,
}

/// How a candidate of `workspaces.find_file` matches the query (DMN-092), best first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FileMatch {
    /// The query is the file's path relative to the workspace folder or to its repository.
    Exact,
    /// The file's path ends with the query, which has a `/`.
    Suffix,
    /// The file's name is the query's last component.
    Basename,
    /// The query's non-whitespace characters are a case-insensitive subsequence of the workspace-relative path.
    Fuzzy,
}

/// A file found by `workspaces.find_file` (DMN-092).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileCandidate {
    /// Relative to the workspace folder (what `workspaces.read_file` takes).
    pub path: String,
    /// The workspace repository holding the file; absent outside every repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(rename = "match")]
    pub matched: FileMatch,
    /// A tool card of the given thread names the file.
    #[serde(default)]
    pub mentioned: bool,
}

/// The result of `workspaces.find_file` (DMN-092): the candidates, best first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileCandidates {
    /// At most 50.
    pub candidates: Vec<FileCandidate>,
    /// More files matched than were returned.
    #[serde(default)]
    pub truncated: bool,
}

/// Where a widget folder lives (AGT-012, AGT-023).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WidgetSource {
    /// `<workspace>/.omfp/widgets/<name>/`, referenced as `./widgets/<name>`.
    Workspace,
    /// `<data-dir>/widgets/<name>/`, referenced as `user:<name>`.
    User,
}

/// A widget folder the daemon serves (DMN-082).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetInfo {
    pub source: WidgetSource,
    /// The folder's name.
    pub name: String,
    /// Hash of the relative path, size and modification time of every file in the folder: changes with every save.
    pub version: String,
    /// The entry file (`index.tsx`, `index.ts`, `index.jsx` or `index.js`, the first present); `null` without one.
    pub entry: Option<String>,
}

/// State of a `views/<workspaceId>` topic (PRT-040, DMN-082).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewsState {
    /// The workspace's `.omfp/views.json` when it is a valid manifest (AGT-010); `null` without a file or when invalid.
    pub manifest: Option<Value>,
    /// Why the file is refused (`<JSON path>: <reason>`, or why it cannot be read); `null` when valid or absent.
    pub error: Option<String>,
    /// Widget folders of the workspace, then of the user library, each by name.
    pub widgets: Vec<WidgetInfo>,
    /// What widgets stored with `useStored` (DMN-086): instance → key → value.
    pub stored: BTreeMap<String, BTreeMap<String, Value>>,
    /// The bindings of the workspace's `.omfp/keymap.json` (DMN-120, UI-210); an invalid file keeps the last valid
    /// ones (DMN-121); empty without a file.
    #[serde(default)]
    pub keymap: Vec<WorkspaceBinding>,
    /// Why `.omfp/keymap.json` is refused (`<JSON path>: <reason>`, or why it cannot be read); `null` when valid or
    /// absent.
    #[serde(default)]
    pub keymap_error: Option<String>,
    /// The version of the workspace's `.omfp/init.ts` (PRT-095): a hash of its bytes; `null` without
    /// one.
    #[serde(default)]
    pub init: Option<String>,
    /// The version of `.omfp/views/` (TSX views, PRT-096): a hash of its files' paths, sizes and
    /// modification times (`folders::version`); `null` without the folder.
    #[serde(default)]
    pub view_files: Option<String>,
}

/// A job's process state (DMN-080).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    /// The shell exited and both outputs closed.
    #[serde(rename_all = "camelCase")]
    Exited {
        /// Exit code; `null` when a signal ended it.
        code: Option<i32>,
        /// The signal that ended it, if any.
        signal: Option<i32>,
        ended_at: i64,
    },
}

/// A shell command the daemon runs for a workspace (DMN-080).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: String,
    pub workspace_id: String,
    /// Run as `sh -c <command>` in the workspace folder.
    pub command: String,
    pub started_at: i64,
    pub status: JobStatus,
}

/// Which output of a job a line comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobStream {
    Stdout,
    Stderr,
}

/// One output line of a job, without its line break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobLine {
    pub stream: JobStream,
    pub text: String,
}

/// State of a `job/<id>` topic (PRT-041).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobState {
    pub job: JobInfo,
    /// The newest lines, at most 1 MiB of text, oldest first.
    pub lines: Vec<JobLine>,
    /// How many older lines were left out.
    pub dropped: u64,
}

/// When a declared job runs (DMN-320).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobTrigger {
    /// Only when asked.
    #[default]
    Manual,
    /// At the times of its cron `schedule` (DMN-323).
    Schedule,
    /// Kept running (DMN-326).
    Service,
}

/// What a schedule does with times the daemon missed (DMN-325).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MissedPolicy {
    #[default]
    Skip,
    /// One catch-up run, however many times were missed.
    Run,
}

/// When a service starts again after its run ended (DMN-326).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RestartPolicy {
    #[default]
    #[serde(rename = "always")]
    Always,
    #[serde(rename = "on-failure")]
    OnFailure,
}

/// Which thread a prompt run sends to (DMN-328).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptThread {
    /// A new thread per run.
    #[default]
    New,
    /// The job's own thread, made on first use.
    Reuse,
}

/// The prompt a prompt job sends (DMN-328).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobPrompt {
    pub text: String,
    pub thread: PromptThread,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<String>,
}

/// A job of `.omfp/jobs.json` with its defaults filled in (DMN-320, PRT-200).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub trigger: JobTrigger,
    /// The cron expression of a `schedule` job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    pub missed: MissedPolicy,
    /// `sh -c` command; exactly one of `command` and `prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<JobPrompt>,
    /// Folder of a command run, relative to the workspace folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub restart: RestartPolicy,
    pub enabled: bool,
}

/// How a run ended, or that it still runs (DMN-330).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobRunStatus {
    Running,
    Succeeded,
    Failed,
    Killed,
    /// Not started: the job still ran, or its reused thread was busy (`reason`).
    Skipped,
    /// Still running when the daemon stopped.
    Interrupted,
}

/// What started a run (PRT-201).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    Manual,
    Schedule,
    /// A catch-up run for missed times (DMN-325).
    Missed,
    Service,
    /// A transient run of `jobs.start`.
    Start,
}

/// One run of a job (PRT-201); its id is a job id (`job/<id>`, `jobs.kill`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub id: String,
    pub workspace_id: String,
    /// The declared job; `null` for a transient run.
    pub job: Option<String>,
    pub trigger: RunTrigger,
    /// Who asked; `null` for the scheduler and the supervisor.
    pub started_by: Option<CreatedBy>,
    /// A prompt run's thread.
    pub thread_id: Option<String>,
    /// The command a command run ran (its output is `job/<id>`); `null` for a prompt run.
    pub command: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub status: JobRunStatus,
    /// Why it was skipped or could not start.
    pub reason: Option<String>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

/// Where a declared job stands (PRT-200).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobViewStatus {
    Disabled,
    Idle,
    Running,
    /// A service between restarts (`nextAt`).
    Waiting,
    /// A service killed through the protocol, or one that ended under `on-failure` without failing.
    Stopped,
}

/// A declared job with its state (PRT-200).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub name: String,
    pub job: JobDefinition,
    pub status: JobViewStatus,
    /// The running run.
    pub run: Option<RunInfo>,
    /// The newest finished run that was not skipped.
    pub last_run: Option<RunInfo>,
    /// The next time of a schedule or a waiting service's restart (Unix ms).
    pub next_at: Option<i64>,
}

/// State of a `jobs/<workspaceId>` topic (PRT-200).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobsState {
    /// The jobs of the last valid `jobs.json`, by name.
    pub jobs: Vec<JobView>,
    /// Why the current file is refused; `null` when valid or absent.
    pub error: Option<String>,
}

/// `jobs.runs` result (PRT-203): newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobRuns {
    pub runs: Vec<RunInfo>,
    /// Older runs exist.
    pub more: bool,
}

/// A workspace's declared jobs at a glance, on the `index` topic (PRT-210).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobCounts {
    pub total: u32,
    pub manual: u32,
    pub schedule: u32,
    pub service: u32,
    pub running: u32,
    /// Enabled jobs whose last run failed.
    pub failing: u32,
    /// The current `jobs.json` is refused.
    pub error: bool,
}

/// The process state of a daemon-owned PTY session (TRM-100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TerminalStatus {
    Running,
    /// The shell exited; PTY output is still available in the topic snapshot.
    #[serde(rename_all = "camelCase")]
    Exited {
        code: Option<i32>,
        ended_at: i64,
    },
}

/// An interactive PTY session (TRM-100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    pub workspace_id: String,
    /// Run as `sh -c <command>` attached to a PTY.
    pub command: String,
    /// Absolute working directory selected under the workspace.
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
    pub started_at: i64,
    pub status: TerminalStatus,
}

/// State of a `terminal/<id>` topic (TRM-100, TRM-103).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TerminalState {
    pub terminal: TerminalInfo,
    /// Base64-encoded PTY bytes, oldest first; the client feeds these to its emulator.
    pub scrollback: Vec<String>,
    /// Raw bytes currently retained; internal bookkeeping, not serialized.
    #[serde(skip)]
    #[schemars(skip)]
    pub retained_bytes: usize,
    /// Raw bytes dropped from the bounded scrollback.
    pub dropped: u64,
}

/// Cursor position in an agent-readable terminal screen (TRM-106); row and column are zero-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCursor {
    pub row: u16,
    pub column: u16,
    pub visible: bool,
}

/// Current visible plain-text screen of a daemon-owned PTY session (TRM-106).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TerminalScreen {
    /// Increases after output, resize or exit; pass it as `afterRevision` to wait for a change.
    pub revision: u64,
    pub cols: u16,
    pub rows: u16,
    /// Visible cells as plain text. Trailing blank cells and rows are omitted.
    pub contents: String,
    pub cursor: TerminalCursor,
    pub alternate_screen: bool,
    pub status: TerminalStatus,
}
