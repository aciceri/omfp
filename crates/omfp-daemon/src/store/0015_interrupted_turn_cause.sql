-- What interrupted a marked turn (DMN-370, DMN-377): `daemon_stop` when the daemon stopped, `crash` when omp exited
-- on its own while the daemon ran. `thread.resume` tells the agent which (DMN-372); the marks of earlier daemons were
-- all daemon stops.

ALTER TABLE interrupted_turns
    ADD COLUMN cause TEXT NOT NULL DEFAULT 'daemon_stop' CHECK (cause IN ('daemon_stop', 'crash'));
