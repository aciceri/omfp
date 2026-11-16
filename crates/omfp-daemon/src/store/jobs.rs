//! The runs and state of declared jobs (DMN-330, migration 13).

use rusqlite::{OptionalExtension, Row, params};

use super::{Provenance, Result, Store};

/// Finished runs kept per job; the oldest go first.
pub const FINISHED_RUNS_KEPT: u32 = 50;

/// A run of a declared job as stored (DMN-330). `trigger` and `status` hold the protocol's names (`manual`, …,
/// `running`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    pub id: String,
    pub workspace_id: String,
    pub job: String,
    pub trigger: String,
    /// `None` for the scheduler and the supervisor.
    pub started_by: Option<Provenance>,
    /// A prompt run's thread.
    pub thread_id: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub status: String,
    pub reason: Option<String>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// A command run's command.
    pub command: Option<String>,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEnd<'a> {
    pub ended_at: i64,
    pub status: &'a str,
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

fn run_row(row: &Row<'_>) -> rusqlite::Result<JobRun> {
    Ok(JobRun {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        job: row.get(2)?,
        trigger: row.get(3)?,
        started_by: row
            .get::<_, Option<String>>(4)?
            .map(|value| Provenance::parse(&value)),
        thread_id: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
        status: row.get(8)?,
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
    /// Records a run: a running one, or one that ended as it began (`skipped`, or `failed` to start).
    pub fn insert_job_run(&self, run: &JobRun) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            &format!("INSERT INTO job_runs ({RUN_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"),
            params![
                run.id,
                run.workspace_id,
                run.job,
                run.trigger,
                run.started_by.as_ref().map(|by| by.as_str().into_owned()),
                run.thread_id,
                run.started_at,
                run.ended_at,
                run.status,
                run.reason,
                run.exit_code,
                run.signal,
                run.command,
            ],
        )?;
        prune(&tx, &run.workspace_id, &run.job)?;
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
                end.status,
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
    ) -> Result<(Vec<JobRun>, bool)> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM job_runs
             WHERE workspace_id = ?1 AND job = ?2
               AND (?3 IS NULL OR (started_at, id) <
                    (SELECT started_at, id FROM job_runs WHERE id = ?3 AND workspace_id = ?1 AND job = ?2))
             ORDER BY started_at DESC, id DESC LIMIT ?4"
        ))?;
        let mut runs = stmt
            .query_map(
                params![workspace_id, job, before, i64::from(limit) + 1],
                run_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = runs.len() > usize::try_from(limit).unwrap_or(usize::MAX);
        runs.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok((runs, more))
    }

    /// The newest finished run of the job that was not `skipped` (PRT-200 `lastRun`).
    pub fn last_job_run(&self, workspace_id: &str, job: &str) -> Result<Option<JobRun>> {
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
    pub fn job_run(&self, id: &str) -> Result<Option<(JobRun, RunOutput)>> {
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
        self.conn().execute(
            "INSERT INTO job_state (workspace_id, job, last_due_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (workspace_id, job) DO UPDATE SET last_due_at = ?3",
            params![workspace_id, job, at],
        )?;
        Ok(())
    }

    /// Sets the thread a `reuse` job sends to (DMN-328).
    pub fn set_job_thread(&self, workspace_id: &str, job: &str, thread_id: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO job_state (workspace_id, job, thread_id) VALUES (?1, ?2, ?3)
             ON CONFLICT (workspace_id, job) DO UPDATE SET thread_id = ?3",
            params![workspace_id, job, thread_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;
    use crate::store::{Workspace, new_id, now_ms};

    fn store() -> (TempDir, Store, String) {
        let dir = TempDir::new().unwrap();
        let store = Store::open(&dir.path().join("omfp.db"), &dir.path().join("ws")).unwrap();
        let workspace = Workspace {
            id: new_id(),
            name: "w".into(),
            path: PathBuf::from("/work/w"),
            created_by: Provenance::User,
            created_at: now_ms(),
        };
        store.insert_workspace(&workspace).unwrap();
        (dir, store, workspace.id)
    }

    fn run(workspace_id: &str, job: &str, started_at: i64, status: &str) -> JobRun {
        JobRun {
            id: new_id(),
            workspace_id: workspace_id.into(),
            job: job.into(),
            trigger: "manual".into(),
            started_by: Some(Provenance::User),
            thread_id: None,
            started_at,
            ended_at: (status != "running").then_some(started_at + 1),
            status: status.into(),
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
        let running = run(&ws, "test", 0, "running");
        store.insert_job_run(&running).unwrap();
        for at in 1..=55 {
            store
                .insert_job_run(&run(&ws, "test", at, "succeeded"))
                .unwrap();
        }
        store
            .insert_job_run(&run(&ws, "other", 1, "failed"))
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
        let runs: Vec<JobRun> = (1..=5).map(|at| run(&ws, "test", at, "skipped")).collect();
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
        let ended = run(&ws, "test", 1, "running");
        let died = run(&ws, "test", 2, "running");
        store.insert_job_run(&ended).unwrap();
        store.insert_job_run(&died).unwrap();
        store
            .finish_job_run(
                &ended.id,
                &RunEnd {
                    ended_at: 9,
                    status: "failed",
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
            (stored.status.as_str(), stored.ended_at, stored.exit_code),
            ("failed", Some(9), Some(3))
        );
        assert_eq!(output.line_count, 7);
        assert_eq!(
            output.output_tail.as_deref(),
            Some(r#"[{"stream":"stdout","text":"hi"}]"#)
        );
        assert_eq!(
            store.job_run(&died.id).unwrap().unwrap().0.status,
            "interrupted"
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
            .insert_job_run(&run(&ws, "mail", 1, "succeeded"))
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
