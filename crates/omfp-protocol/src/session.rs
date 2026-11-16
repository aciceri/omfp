//! `thread.session`: the small facts of a thread's omp session, without omp's tool definitions and system prompt that
//! `thread.state` carries (PRT-125, PRT-126).

use omfp_core::ProcessState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `thread.session` result: what the daemon knows of a thread's omp session; it never starts omp (PRT-125). Absent
/// fields are unknown: `sessionId` before the thread's first start, `sessionFile` until omp wrote it, `messageCount`
/// until the daemon loaded the session in this run (omp's history, or the session file after a failed start),
/// `sessionModel` unless the thread's last start failed, `usage` while omp does not run (PRT-126, PRT-211).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSession {
    pub thread_id: String,
    pub workspace_id: String,
    /// omp's working directory: the workspace folder (ARC-008).
    pub cwd: String,
    /// The repository roots the thread's omp session has (`--add-dir` at its last start, `/add-dir` and
    /// `/remove-dir` since, DMN-029); `[]` before its first start.
    pub roots: Vec<String>,
    /// The thread's private omp session directory (`--session-dir`, ARC-020).
    pub session_dir: String,
    /// omp's session id; absent before the thread's first start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The session's `.jsonl` file: as omp reported it when it loaded the session, else the file of `sessionId` in
    /// `sessionDir` (DMN-213); absent while omp has not written it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    /// `provider/id` the thread was created or last set with (`thread.set_model`); absent: omp's default model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `provider/id` the stored session restores, read from its session file when the thread's omp could not start
    /// (DMN-338, PRT-211): its saved model, which may be one omp cannot use; absent unless the last start failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_model: Option<String>,
    /// omp's thinking level the thread was created or last set with (`thread.set_thinking`); absent: omp's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    /// omp's `--approval-mode` (DMN-074, DMN-210).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<crate::ApprovalMode>,
    /// `omfp_core::ProcessState`, as on the thread summary.
    #[schemars(with = "serde_json::Value")]
    pub process: ProcessState,
    /// Idle with nothing queued or pending (omp's `session_settled`); `false` while the agent works.
    pub settled: bool,
    /// Interactive requests are waiting for an answer.
    pub needs_attention: bool,
    /// Messages of the session's current branch (the transcript the thread topic shows); absent until omp loaded
    /// the session in this daemon run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_count: Option<u64>,
    /// What the session used so far, from the thread's running omp (DMN-214); absent while omp does not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsage>,
}

/// A session's cost and tokens (omp `get_session_stats`, DMN-214).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    /// Cost of the session's model calls so far, in US dollars.
    pub cost: f64,
    pub tokens: SessionTokens,
    /// How full the model's context window is; absent when omp cannot tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextUsage>,
}

/// Tokens of a session's model calls so far.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionTokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub total: i64,
}

/// The context window's use by the session's current branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub tokens: i64,
    pub context_window: i64,
    /// `tokens` as a percentage of `contextWindow`.
    pub percent: f64,
}
