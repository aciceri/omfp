-- The schema of omfp v0.1.0 (`user_version` 12), the first release: a fresh database starts here, then takes the
-- migrations after 12. Databases from before it (versions 1 to 11) are refused (DMN-010).

-- Daemon-wide settings (the theme, DMN-075): JSON values by key.
CREATE TABLE settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

-- Workspace folders under the workspaces root (DMN-020), in the left column's order (DMN-280).
CREATE TABLE workspaces (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL UNIQUE,
    created_by TEXT NOT NULL,
    created_client TEXT,
    created_at INTEGER NOT NULL,
    position INTEGER NOT NULL DEFAULT 0
);

-- Threads (DMN-073 for `title_source`, DMN-280 for `position`).
CREATE TABLE threads (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces (id),
    session_dir TEXT NOT NULL,
    omp_session_id TEXT,
    approval_mode TEXT,
    model TEXT,
    thinking_level TEXT,
    created_by TEXT NOT NULL,
    created_client TEXT,
    created_at INTEGER NOT NULL,
    archived_at INTEGER,
    title_source TEXT NOT NULL DEFAULT 'set'
        CHECK (title_source IN ('placeholder', 'generated', 'set')),
    position INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX threads_workspace ON threads (workspace_id);

-- The roots each thread's session was started with (DMN-029).
CREATE TABLE thread_roots (
    thread_id TEXT NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    PRIMARY KEY (thread_id, path)
);

-- Repositories seen in workspaces.
CREATE TABLE seen_repositories (
    path TEXT PRIMARY KEY NOT NULL,
    last_seen_at INTEGER NOT NULL
);

-- Widget state (DMN-086).
CREATE TABLE widget_state (
    workspace_id TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    instance TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (workspace_id, instance, key)
);

-- Activity log (DMN-160): `thread_id` is null for the MCP principal's calls (AGT-053).
CREATE TABLE activity (
    seq INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    thread_id TEXT,
    client TEXT NOT NULL,
    method TEXT NOT NULL,
    params TEXT NOT NULL,
    error TEXT
);

CREATE INDEX activity_by_thread ON activity (thread_id, seq);

-- Prompt history (DMN-216).
CREATE TABLE prompt_history (
    text TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    at INTEGER NOT NULL
);

CREATE INDEX prompt_history_by_time ON prompt_history (at);

-- Fork lineage (DMN-300).
CREATE TABLE thread_forks (
    thread_id TEXT PRIMARY KEY REFERENCES threads (id) ON DELETE CASCADE,
    source_thread_id TEXT NOT NULL,
    source_title TEXT NOT NULL,
    entry_id TEXT,
    message TEXT
);
