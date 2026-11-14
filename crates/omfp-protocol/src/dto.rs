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
    /// The built-in colour theme (UI-510); absent is Synth, omfp's own. A user theme (`/theme.css`) still wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<BuiltinTheme>,
    /// Show informational xdev notices in the transcript (UI-043); absent or false hides them, without dropping events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_xdev_notices: Option<bool>,
}

/// Row height and spacing of the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    Normal,
}

/// A built-in colour theme of the UI (UI-510, PRT-143): an official palette mapped onto the Material 3 roles in light
/// and dark, or (`accent`) one derived from the system accent colour where the web view exposes it (UI-512).
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
    /// Follows the system accent colour (macOS); elsewhere Synth.
    Accent,
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
