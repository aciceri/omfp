//! The first omp of a profile starts alone (DMN-913).
//!
//! omp creates the schema of its `agent.db` with a check, then a create (`AgentStorage#initializeSchema`, omp
//! 19.1.0: `PRAGMA table_info(settings)`, then `CREATE TABLE settings` without `IF NOT EXISTS`), outside any
//! transaction. Two omps starting at once on a profile without that table can both find none, and the second exits
//! at startup with `SQLiteError: … table settings already exists`. Until an omp run of an agent directory has got
//! past its startup in this daemon, [`FirstStart`] runs that directory's omp invocations one at a time; afterwards
//! they run freely.

use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::{Arc, Mutex, PoisonError};

/// omp's agent directory variable: the profile whose `agent.db` an omp run opens (`omp_profile::environment`).
const AGENT_DIR: &str = "PI_CODING_AGENT_DIR";

/// The agent directory of an omp environment, as the daemon's launch specs set it; `None` for omp's default.
pub(crate) fn agent_dir(env: &[(OsString, Option<OsString>)]) -> Option<OsString> {
    env.iter()
        .rev()
        .find(|(key, _)| key == AGENT_DIR)
        .and_then(|(_, value)| value.clone())
}

/// Per agent directory: whether an omp run got past its startup (its `agent.db` has its schema).
#[derive(Debug, Default)]
pub(crate) struct FirstStart {
    profiles: Mutex<HashMap<Option<OsString>, Arc<Mutex<bool>>>>,
}

impl FirstStart {
    /// Runs `run`, an omp invocation on the agent directory `profile`: alone while no run of that directory has
    /// succeeded yet (a success marks it started), at once afterwards. `run` succeeds only once omp got past its
    /// startup (an RPC omp is ready, a one-shot command exited well).
    pub(crate) fn run<T, E>(
        &self,
        profile: Option<OsString>,
        run: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let gate = Arc::clone(
            self.profiles
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(profile)
                .or_default(),
        );
        let mut started = gate.lock().unwrap_or_else(PoisonError::into_inner);
        if *started {
            drop(started);
            return run();
        }
        let result = run();
        *started = result.is_ok();
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    const PROFILE: &str = "/profile/agent";

    /// Starts `runs` invocations of [`PROFILE`] at once; each stays inside `run` until `together` runs have entered
    /// (or a deadline passes), then answers `result`. Returns the most runs that were inside together.
    fn most_at_once(
        gate: &FirstStart,
        runs: usize,
        together: usize,
        result: Result<(), ()>,
    ) -> usize {
        let inside = AtomicUsize::new(0);
        let entered = AtomicUsize::new(0);
        let most = AtomicUsize::new(0);
        let start = Barrier::new(runs);
        std::thread::scope(|scope| {
            for _ in 0..runs {
                scope.spawn(|| {
                    start.wait();
                    let _ = gate.run(Some(PROFILE.into()), || {
                        most.fetch_max(inside.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                        entered.fetch_add(1, Ordering::SeqCst);
                        // Only a gate that lets fewer runs in together than expected waits until the deadline.
                        let deadline = Instant::now() + Duration::from_secs(10);
                        while entered.load(Ordering::SeqCst) < together && Instant::now() < deadline
                        {
                            std::thread::yield_now();
                        }
                        inside.fetch_sub(1, Ordering::SeqCst);
                        result
                    });
                });
            }
        });
        most.load(Ordering::SeqCst)
    }

    // spec: DMN-913
    #[test]
    fn omp_runs_of_a_fresh_profile_start_one_at_a_time_until_one_succeeds() {
        let gate = FirstStart::default();
        // Failed runs never open the profile: they keep going in one at a time, whoever comes first.
        assert_eq!(most_at_once(&gate, 4, 1, Err(())), 1);
        // A run that succeeds opens it: afterwards runs are inside together.
        assert_eq!(gate.run(Some(PROFILE.into()), || Ok::<_, ()>(())), Ok(()));
        assert_eq!(most_at_once(&gate, 2, 2, Ok(())), 2);
    }

    // spec: DMN-913
    #[test]
    fn the_agent_directory_is_the_last_value_the_environment_sets() {
        let env = vec![
            ("PI_CODING_AGENT_DIR".into(), Some("/old".into())),
            ("HOME".into(), Some("/home".into())),
            ("PI_CODING_AGENT_DIR".into(), Some("/new".into())),
        ];
        assert_eq!(agent_dir(&env), Some("/new".into()));
        assert_eq!(agent_dir(&env[1..2]), None);
    }
}
