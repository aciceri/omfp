-- Declared jobs (DMN-330): every run of a workspace's declared jobs, the 50 newest finished ones of each job kept, and
-- each job's schedule state and reused thread. Both go with their workspace. `command` is a command run's command (for
-- the `job/<id>` topic of a stored run), `output_tail` its last lines as a JSON array of `{ stream, text }`.

CREATE TABLE job_runs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    job TEXT NOT NULL,
    "trigger" TEXT NOT NULL,
    started_by TEXT,
    thread_id TEXT,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    status TEXT NOT NULL,
    reason TEXT,
    exit_code INTEGER,
    signal INTEGER,
    command TEXT,
    output_tail TEXT,
    line_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX job_runs_by_job ON job_runs (workspace_id, job, started_at);

CREATE TABLE job_state (
    workspace_id TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    job TEXT NOT NULL,
    last_due_at INTEGER,
    thread_id TEXT,
    PRIMARY KEY (workspace_id, job)
);
