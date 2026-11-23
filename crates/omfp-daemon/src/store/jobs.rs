//! The runs and state of declared jobs (DMN-330, migration 13).

use omfp_protocol::{JobRunStatus, RunInfo, RunTrigger};
use rusqlite::{OptionalExtension, Row, params};

use super::{Result, Store, page, parse_provenance, provenance_text, query_all};
use crate::{from_wire_name, wire_name};

/// Finished runs kept per job; the oldest go first.
pub const FINISHED_RUNS_KEPT: u32 = 50;

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEnd<'a> {
    pub ended_at: i64,
    pub status: JobRunStatus,
    pub reason: Option<&'a str>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// A command run's last lines (JSON array of `{ stream, text }`).
    pub output_tail: Option<&'a str>,
    pub line_count: u64,
}

/// What the store kept of a run's output: its last lines (JSON) and the count of every line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub output_tail: Option<String>,
    pub line_count: u64,
}

/// A job's state (DMN-325, DMN-328).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobState {
    pub last_due_at: Option<i64>,
    /// The thread `reuse` sends to.
    pub thread_id: Option<String>,
}

const RUN_COLUMNS: &str = "id, workspace_id, job, \"trigger\", started_by, thread_id, started_at, ended_at, status, \
     reason, exit_code, signal, command";

/// A stored run (DMN-330); `trigger` and `status` are stored under their protocol names.
fn run_row(row: &Row<'_>) -> rusqlite::Result<RunInfo> {
    Ok(RunInfo {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        job: Some(row.get(2)?),
        trigger: from_wire_name(&row.get::<_, String>(3)?).unwrap_or(RunTrigger::Manual),
        started_by: row
            .get::<_, Option<String>>(4)?
            .map(|value| parse_provenance(&value)),
        thread_id: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
        status: from_wire_name(&row.get::<_, String>(8)?).unwrap_or(JobRunStatus::Failed),
        reason: row.get(9)?,
        exit_code: row.get(10)?,
        signal: row.get(11)?,
        command: row.get(12)?,
    })
}

/// Keeps the [`FINISHED_RUNS_KEPT`] newest finished runs of the job.
fn prune(conn: &rusqlite::Connection, workspace_id: &str, job: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM job_runs WHERE workspace_id = ?1 AND job = ?2 AND status <> 'running' AND id NOT IN
             (SELECT id FROM job_runs WHERE workspace_id = ?1 AND job = ?2 AND status <> 'running'
              ORDER BY started_at DESC, id DESC LIMIT ?3)",
        params![workspace_id, job, FINISHED_RUNS_KEPT],
    )?;
    Ok(())
}

impl Store {
    /// Records a run of a declared job: a running one, or one that ended as it began (`skipped`, or `failed` to
    /// start).
    pub fn insert_job_run(&self, run: &RunInfo) -> Result<()> {
        let job = run.job.as_deref().unwrap_or_default();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            &format!("INSERT INTO job_runs ({RUN_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"),
            params![
                run.id,
                run.workspace_id,
                job,
                wire_name(run.trigger),
                run.started_by.as_ref().map(provenance_text),
                run.thread_id,
                run.started_at,
                run.ended_at,
                wire_name(run.status),
                run.reason,
                run.exit_code,
                run.signal,
                run.command,
            ],
        )?;
        prune(&tx, &run.workspace_id, job)?;
        tx.commit()?;
        Ok(())
    }

    /// Records how the run `id` ended, then keeps the job's newest finished runs.
    pub fn finish_job_run(&self, id: &str, end: &RunEnd<'_>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let owner: Option<(String, String)> = tx
            .query_row(
                "SELECT workspace_id, job FROM job_runs WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((workspace_id, job)) = owner else {
            return Ok(());
        };
        tx.execute(
            "UPDATE job_runs SET ended_at = ?2, status = ?3, reason = ?4, exit_code = ?5, signal = ?6,
                 output_tail = ?7, line_count = ?8
             WHERE id = ?1",
            params![
                id,
                end.ended_at,
                wire_name(end.status),
                end.reason,
                end.exit_code,
                end.signal,
                end.output_tail,
                i64::try_from(end.line_count).unwrap_or(i64::MAX),
            ],
        )?;
        prune(&tx, &workspace_id, &job)?;
        tx.commit()?;
        Ok(())
    }

    /// Runs still `running` when the daemon starts died with the previous daemon: they become `interrupted`.
    pub fn interrupt_job_runs(&self) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE job_runs SET status = 'interrupted' WHERE status = 'running'",
            [],
        )?)
    }

    /// The runs of a job, newest first, only those started before the run `before` when given (PRT-203): at most
    /// `limit`, and whether more follow.
    pub fn job_runs(
        &self,
        workspace_id: &str,
        job: &str,
        before: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<RunInfo>, bool)> {
        let runs = query_all(
            &self.conn(),
            &format!(
                "SELECT {RUN_COLUMNS} FROM job_runs
                 WHERE workspace_id = ?1 AND job = ?2
                   AND (?3 IS NULL OR (started_at, id) <
                        (SELECT started_at, id FROM job_runs WHERE id = ?3 AND workspace_id = ?1 AND job = ?2))
                 ORDER BY started_at DESC, id DESC LIMIT ?4"
            ),
            params![workspace_id, job, before, i64::from(limit) + 1],
            run_row,
        )?;
        Ok(page(runs, limit))
    }

    /// The newest finished run of the job that was not `skipped` (PRT-200 `lastRun`).
    pub fn last_job_run(&self, workspace_id: &str, job: &str) -> Result<Option<RunInfo>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {RUN_COLUMNS} FROM job_runs
                     WHERE workspace_id = ?1 AND job = ?2 AND status NOT IN ('running', 'skipped')
                     ORDER BY started_at DESC, id DESC LIMIT 1"
                ),
                [workspace_id, job],
                run_row,
            )
            .optional()?)
    }

    /// One run with what was kept of its output.
    pub fn job_run(&self, id: &str) -> Result<Option<(RunInfo, RunOutput)>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {RUN_COLUMNS}, output_tail, line_count FROM job_runs WHERE id = ?1"
                ),
                [id],
                |row| {
                    Ok((
                        run_row(row)?,
                        RunOutput {
                            output_tail: row.get(13)?,
                            line_count: u64::try_from(row.get::<_, i64>(14)?).unwrap_or_default(),
                        },
                    ))
                },
            )
            .optional()?)
    }

    /// A job's state; the default when none is kept.
    pub fn job_state(&self, workspace_id: &str, job: &str) -> Result<JobState> {
        Ok(self
            .conn()
            .query_row(
                "SELECT last_due_at, thread_id FROM job_state WHERE workspace_id = ?1 AND job = ?2",
                [workspace_id, job],
                |row| {
                    Ok(JobState {
                        last_due_at: row.get(0)?,
                        thread_id: row.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Sets a job's `last_due_at` (DMN-325).
    pub fn set_job_last_due(&self, workspace_id: &str, job: &str, at: i64) -> Result<()> {
        self.set_job_state(workspace_id, job, "last_due_at", at)
    }

    /// Sets the thread a `reuse` job sends to (DMN-328).
    pub fn set_job_thread(&self, workspace_id: &str, job: &str, thread_id: &str) -> Result<()> {
        self.set_job_state(workspace_id, job, "thread_id", thread_id)
    }

    fn set_job_state(
        &self,
        workspace_id: &str,
        job: &str,
        column: &str,
        value: impl rusqlite::ToSql,
    ) -> Result<()> {
        self.conn().execute(
            &format!(
                "INSERT INTO job_state (workspace_id, job, {column}) VALUES (?1, ?2, ?3)
                 ON CONFLICT (workspace_id, job) DO UPDATE SET {column} = ?3"
            ),
            params![workspace_id, job, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use omfp_protocol::CreatedBy;
    use tempfile::TempDir;

    use super::*;
    use crate::store::{Workspace, new_id, now_ms};

    fn store() -> (TempDir, Store, String) {
        let dir = TempDir::new().unwrap();
        let store = Store::open(&dir.path().join("omfp.db")).unwrap();
        let workspace = Workspace {
            id: new_id(),
            name: "w".into(),
            path: PathBuf::from("/work/w"),
            created_by: CreatedBy::User,
            created_at: now_ms(),
        };
        store.insert_workspace(&workspace).unwrap();
        (dir, store, workspace.id)
    }

    fn run(workspace_id: &str, job: &str, started_at: i64, status: JobRunStatus) -> RunInfo {
        RunInfo {
            id: new_id(),
            workspace_id: workspace_id.into(),
            job: Some(job.into()),
            trigger: RunTrigger::Manual,
            started_by: Some(CreatedBy::User),
            thread_id: None,
            started_at,
            ended_at: (status != JobRunStatus::Running).then_some(started_at + 1),
            status,
            reason: None,
            exit_code: None,
            signal: None,
            command: Some("true".into()),
        }
    }

    // spec: DMN-330
    #[test]
    fn a_job_keeps_its_50_newest_finished_runs_and_every_running_one() {
        let (_dir, store, ws) = store();
        let running = run(&ws, "test", 0, JobRunStatus::Running);
        store.insert_job_run(&running).unwrap();
        for at in 1..=55 {
            store
                .insert_job_run(&run(&ws, "test", at, JobRunStatus::Succeeded))
                .unwrap();
        }
        store
            .insert_job_run(&run(&ws, "other", 1, JobRunStatus::Failed))
            .unwrap();

        let (runs, more) = store.job_runs(&ws, "test", None, 100).unwrap();

        assert!(!more);
        assert_eq!(runs.len(), 51);
        assert_eq!(runs[0].started_at, 55);
        assert_eq!(runs[49].started_at, 6);
        assert_eq!(runs[50].id, running.id);
        assert_eq!(store.job_runs(&ws, "other", None, 100).unwrap().0.len(), 1);
    }

    // spec: DMN-330, PRT-203
    #[test]
    fn runs_page_newest_first_before_a_run() {
        let (_dir, store, ws) = store();
        let runs: Vec<RunInfo> = (1..=5)
            .map(|at| run(&ws, "test", at, JobRunStatus::Skipped))
            .collect();
        for run in &runs {
            store.insert_job_run(run).unwrap();
        }

        let (page, more) = store.job_runs(&ws, "test", None, 2).unwrap();
        assert!(more);
        assert_eq!(
            page.iter().map(|run| run.started_at).collect::<Vec<_>>(),
            [5, 4]
        );
        let (page, more) = store.job_runs(&ws, "test", Some(&page[1].id), 2).unwrap();
        assert!(more);
        assert_eq!(
            page.iter().map(|run| run.started_at).collect::<Vec<_>>(),
            [3, 2]
        );
        let (page, more) = store.job_runs(&ws, "test", Some(&page[1].id), 2).unwrap();
        assert!(!more);
        assert_eq!(
            page.iter().map(|run| run.started_at).collect::<Vec<_>>(),
            [1]
        );
    }

    // spec: DMN-330
    #[test]
    fn a_finished_run_keeps_its_end_and_output_and_running_runs_become_interrupted() {
        let (_dir, store, ws) = store();
        let ended = run(&ws, "test", 1, JobRunStatus::Running);
        let died = run(&ws, "test", 2, JobRunStatus::Running);
        store.insert_job_run(&ended).unwrap();
        store.insert_job_run(&died).unwrap();
        store
            .finish_job_run(
                &ended.id,
                &RunEnd {
                    ended_at: 9,
                    status: JobRunStatus::Failed,
                    reason: None,
                    exit_code: Some(3),
                    signal: None,
                    output_tail: Some(r#"[{"stream":"stdout","text":"hi"}]"#),
                    line_count: 7,
                },
            )
            .unwrap();

        assert_eq!(store.interrupt_job_runs().unwrap(), 1);

        let (stored, output) = store.job_run(&ended.id).unwrap().unwrap();
        assert_eq!(
            (stored.status, stored.ended_at, stored.exit_code),
            (JobRunStatus::Failed, Some(9), Some(3))
        );
        assert_eq!(output.line_count, 7);
        assert_eq!(
            output.output_tail.as_deref(),
            Some(r#"[{"stream":"stdout","text":"hi"}]"#)
        );
        assert_eq!(
            store.job_run(&died.id).unwrap().unwrap().0.status,
            JobRunStatus::Interrupted
        );
        assert_eq!(
            store.last_job_run(&ws, "test").unwrap().unwrap().id,
            died.id
        );
    }

    // spec: DMN-330, DMN-329
    #[test]
    fn job_state_is_kept_per_job_and_goes_with_its_workspace() {
        let (_dir, store, ws) = store();
        store.set_job_last_due(&ws, "mail", 42).unwrap();
        store.set_job_thread(&ws, "mail", "T1").unwrap();
        store
            .insert_job_run(&run(&ws, "mail", 1, JobRunStatus::Succeeded))
            .unwrap();

        assert_eq!(
            store.job_state(&ws, "mail").unwrap(),
            JobState {
                last_due_at: Some(42),
                thread_id: Some("T1".into())
            }
        );
        assert_eq!(store.job_state(&ws, "other").unwrap(), JobState::default());

        store.delete_workspace(&ws).unwrap();
        assert_eq!(store.job_state(&ws, "mail").unwrap(), JobState::default());
        assert_eq!(store.job_runs(&ws, "mail", None, 50).unwrap().0, []);
    }
}
