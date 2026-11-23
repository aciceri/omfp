//! The activity log of agent-originated actions (DMN-160) and the prompt history (DMN-216).

use rusqlite::params;

use super::{Result, Store, page, query_all};

/// One agent-originated action to log (AGT-072, DMN-160).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewActivity {
    /// Unix milliseconds.
    pub at: i64,
    /// The calling agent's thread; `None` for the MCP principal (AGT-053).
    pub thread_id: Option<String>,
    /// The calling principal's client (PRT-033).
    pub client: String,
    pub method: String,
    pub params: serde_json::Value,
    /// `None` when the call succeeded.
    pub error: Option<String>,
}

/// A logged action with its place in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub seq: i64,
    pub entry: NewActivity,
}

/// Which part of the activity log to read (PRT-086).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityQuery {
    pub thread_id: Option<String>,
    /// Only entries with a smaller `seq`.
    pub before: Option<i64>,
    /// Only entries with a greater `seq`.
    pub after: Option<i64>,
    pub limit: u32,
}

/// One prompt of the prompt history (DMN-216): its text, the thread and workspace it was last sent to, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentPrompt {
    pub text: String,
    pub thread_id: String,
    pub workspace_id: String,
    /// Unix milliseconds.
    pub at: i64,
}

/// How to read the prompt history (DMN-217): `thread_id`'s prompts first, then `workspace_id`'s, then the others,
/// newest first within each; only those containing `query` (case-insensitive) when given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptQuery {
    pub thread_id: Option<String>,
    pub workspace_id: Option<String>,
    pub query: Option<String>,
    pub limit: u32,
}

impl Store {
    /// Appends an entry to the activity log and deletes those older than the newest `keep` (DMN-160).
    pub fn record_activity(&self, entry: &NewActivity, keep: u32) -> Result<Activity> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO activity (at, thread_id, client, method, params, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                entry.at,
                entry.thread_id,
                entry.client,
                entry.method,
                entry.params.to_string(),
                entry.error
            ],
        )?;
        let seq = tx.last_insert_rowid();
        tx.execute(
            "DELETE FROM activity WHERE seq <= ?1",
            [seq - i64::from(keep)],
        )?;
        tx.commit()?;
        Ok(Activity {
            seq,
            entry: entry.clone(),
        })
    }

    /// The entries of the activity log `query` selects, newest first, and whether its limit left some out (PRT-086).
    pub fn activity(&self, query: &ActivityQuery) -> Result<(Vec<Activity>, bool)> {
        let rows: Vec<(Activity, String)> = query_all(
            &self.conn(),
            "SELECT seq, at, thread_id, client, method, params, error FROM activity
             WHERE (?1 IS NULL OR thread_id = ?1) AND (?2 IS NULL OR seq < ?2) AND (?3 IS NULL OR seq > ?3)
             ORDER BY seq DESC LIMIT ?4",
            params![
                query.thread_id,
                query.before,
                query.after,
                i64::from(query.limit) + 1
            ],
            |row| {
                let entry = NewActivity {
                    at: row.get(1)?,
                    thread_id: row.get(2)?,
                    client: row.get(3)?,
                    method: row.get(4)?,
                    params: serde_json::Value::Null,
                    error: row.get(6)?,
                };
                Ok((Activity { seq: row.get(0)?, entry }, row.get(5)?))
            },
        )?;
        let entries = rows
            .into_iter()
            .map(|(mut activity, params)| {
                activity.entry.params = serde_json::from_str(&params)?;
                Ok(activity)
            })
            .collect::<Result<_>>()?;
        Ok(page(entries, query.limit))
    }

    // Prompt history (DMN-216, DMN-217)

    /// Records a prompt the user sent: a text already kept takes this submission's thread, workspace and time; the
    /// history then keeps the newest `keep` prompts.
    pub fn record_prompt(&self, prompt: &SentPrompt, keep: u32) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO prompt_history (text, thread_id, workspace_id, at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (text) DO UPDATE SET thread_id = ?2, workspace_id = ?3, at = ?4",
            params![
                prompt.text,
                prompt.thread_id,
                prompt.workspace_id,
                prompt.at
            ],
        )?;
        tx.execute(
            "DELETE FROM prompt_history WHERE rowid NOT IN
                 (SELECT rowid FROM prompt_history ORDER BY at DESC, rowid DESC LIMIT ?1)",
            [keep],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The prompts `query` selects, in its order (DMN-217).
    pub fn prompts(&self, query: &PromptQuery) -> Result<Vec<SentPrompt>> {
        query_all(
            &self.conn(),
            "SELECT text, thread_id, workspace_id, at FROM prompt_history
             WHERE ?3 IS NULL OR ?3 = '' OR instr(lower(text), lower(?3)) > 0
             ORDER BY thread_id IS ?1 DESC, workspace_id IS ?2 DESC, at DESC, rowid DESC
             LIMIT ?4",
            params![
                query.thread_id,
                query.workspace_id,
                query.query,
                query.limit
            ],
            |row| {
                Ok(SentPrompt {
                    text: row.get(0)?,
                    thread_id: row.get(1)?,
                    workspace_id: row.get(2)?,
                    at: row.get(3)?,
                })
            },
        )
    }
}
