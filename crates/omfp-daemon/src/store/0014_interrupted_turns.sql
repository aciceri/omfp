-- Turns a daemon stop interrupted (DMN-370): the threads whose omp was mid-turn (or waited on async work) when the
-- daemon shut down, and when it did. `thread.resume`, or the thread's next message, deletes the row (DMN-372,
-- DMN-373); a deleted thread takes it along.

CREATE TABLE interrupted_turns (
    thread_id TEXT PRIMARY KEY REFERENCES threads (id) ON DELETE CASCADE,
    interrupted_at INTEGER NOT NULL
);
