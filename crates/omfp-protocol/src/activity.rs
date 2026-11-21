//! Params and results of the cascade stop and the activity log (M10, AGT-071, AGT-072).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `thread.stop` params (PRT-085).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StopThreadParams {
    pub thread_id: String,
    /// Also stop every thread of its spawned tree: the threads it spawned, transitively (AGT-070, AGT-071).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cascade: bool,
}

/// `thread.stop` result (PRT-085).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsStopped {
    /// The threads whose omp process is now stopped: the thread first, then the rest of its spawned tree in the
    /// order they were stopped.
    pub thread_ids: Vec<String>,
}

/// `activity.list` params (PRT-086).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListActivityParams {
    /// Only the actions of this calling thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Only entries with a smaller `seq` (older ones).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    /// Only entries with a greater `seq` (newer ones).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<i64>,
    /// At most this many entries; default 50, capped at 500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `activity.list` result (PRT-086).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    /// Newest first.
    pub entries: Vec<ActivityEntry>,
    /// Entries matching the query were left out by the limit.
    pub more: bool,
}

/// One agent-originated action (AGT-072, DMN-160).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    /// Increasing with every entry.
    pub seq: i64,
    /// Unix milliseconds.
    pub at: i64,
    /// The thread whose agent made the call; `null` for a call of an MCP client outside omfp (PRT-094).
    pub thread_id: Option<String>,
    /// The `client` of the calling principal (PRT-033): the MCP client's name when `thread_id` is `null`.
    pub client: String,
    /// The protocol method called.
    pub method: String,
    /// Its params, long strings cut to 200 characters (DMN-161).
    pub params: Value,
    /// `null` when the call succeeded, else the error's message.
    pub error: Option<String>,
}

/// `prompts.history` params (PRT-127): the prompts the user sent from omfp, ranked for one composer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptHistoryParams {
    /// Ranks this thread's prompts first (it filters nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Ranks this workspace's prompts next (it filters nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Only prompts containing this text, case-insensitive (ASCII); absent or empty: all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// At most this many prompts; default 100, capped at 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `prompts.history` result (PRT-127).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptHistory {
    /// The given thread's prompts, then the given workspace's, then the others; newest first within each.
    pub prompts: Vec<PromptHistoryEntry>,
}

/// One prompt of the history (PRT-128, DMN-216): each text once, with its latest submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptHistoryEntry {
    pub text: String,
    /// The thread it was last sent to.
    pub thread_id: String,
    /// That thread's workspace when it was sent.
    pub workspace_id: String,
    /// When it was last sent (Unix milliseconds).
    pub at: i64,
}

/// `prompts.predict_word` params (PRT-226): the composer's text and its caret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PredictWordParams {
    pub text: String,
    /// The caret, in UTF-16 code units of `text` (a JavaScript string index).
    pub cursor: u32,
}

/// `prompts.predict_word` result (PRT-226).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PredictedWord {
    /// The characters completing the word that ends at the caret; `null` when no completion applies.
    pub suffix: Option<String>,
}

/// `prompts.predict_word_feedback` params (PRT-227): a shown completion the user accepted or typed past.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PredictWordFeedbackParams {
    /// The text and caret the completion was predicted for.
    pub text: String,
    pub cursor: u32,
    /// The completion shown (a [`PredictedWord::suffix`]).
    pub suggestion: String,
    pub accepted: bool,
}
