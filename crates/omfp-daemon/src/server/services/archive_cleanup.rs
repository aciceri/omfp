//! Deleting archived threads (DMN-395): at startup and every [`CHECK_EVERY_MS`] on the services' clock, every thread
//! archived for at least `delete_archived_after` (`--delete-archived-after`, else the config file's `[daemon]` value,
//! else [`DeleteArchivedAfter::DEFAULT`]) goes as `threads.delete` deletes it (DMN-039): its records, its omp and
//! `threads/<id>/`, then the index is published, one log line per thread. The age counts from `archived_at`, so an
//! unarchived thread is never deleted. The value is read again at each check, so a change of the config file or a
//! `settings.set` applies from the next one (DMN-396). A thread whose agent's transient runs still run is kept for a
//! later check (DMN-333): nothing is killed for it.

use std::sync::Arc;

use omfp_protocol::ErrorBody;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::{Services, blocking};
use crate::config_file::DeleteArchivedAfter;
use crate::lock;

/// How often the daemon looks for archived threads to delete, in milliseconds: hourly.
pub(crate) const CHECK_EVERY_MS: i64 = 3_600_000;

impl Services {
    /// Deletes the expired archived threads now, then every [`CHECK_EVERY_MS`], until the task is aborted (DMN-395).
    /// `flag` is `--delete-archived-after`, which wins over the config file.
    pub fn spawn_archive_cleanup(&self, flag: Option<DeleteArchivedAfter>) -> JoinHandle<()> {
        let services = self.clone();
        tokio::spawn(async move {
            loop {
                services.delete_expired_archived(flag).await;
                let next = services.clock.now_ms().saturating_add(CHECK_EVERY_MS);
                services.clock.sleep_until(next).await;
            }
        })
    }

    /// One check: deletes every thread archived for at least `flag`, else the config file's `delete_archived_after`,
    /// else the default, one by one; returns the ids of the deleted threads.
    pub(crate) async fn delete_expired_archived(
        &self,
        flag: Option<DeleteArchivedAfter>,
    ) -> Vec<String> {
        let after = flag.unwrap_or_else(|| {
            lock(&self.settings)
                .daemon
                .delete_archived_after
                .unwrap_or(DeleteArchivedAfter::DEFAULT)
        });
        let Some(after_ms) = after.millis() else {
            return Vec::new();
        };
        // Archived at or before this time: archived for at least `after`.
        let cutoff = self.clock.now_ms().saturating_sub(after_ms);
        let threads = Arc::clone(&self.threads);
        let listed = match blocking(move || threads.list().map_err(ErrorBody::from)).await {
            Ok(listed) => listed,
            Err(error) => {
                warn!(error = %error.message, "cannot list the threads to delete the archived ones");
                return Vec::new();
            }
        };
        let mut deleted = Vec::new();
        for thread in listed {
            if thread.archived_at.is_none_or(|at| at > cutoff) {
                continue;
            }
            if !self.jobs.running_started_by(&thread.id).is_empty() {
                debug!(thread = %thread.id, "archived thread kept: runs its agent started still run");
                continue;
            }
            // Checked again as the records go: a thread unarchived since the list was read stays.
            match self.supervisor.delete_archived(&thread.id, cutoff).await {
                Ok(true) => {
                    info!(
                        thread = %thread.id,
                        title = %thread.title,
                        archived_at = thread.archived_at,
                        "deleted an archived thread: archived for longer than {after}"
                    );
                    deleted.push(thread.id);
                }
                Ok(false) => {}
                Err(error) => {
                    warn!(thread = %thread.id, %error, "cannot delete an archived thread");
                }
            }
        }
        if !deleted.is_empty() {
            self.refresh_index_quietly().await;
        }
        deleted
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use omfp_protocol::{DaemonSettings, ErrorCode, SetSettingsParams};
    use tempfile::TempDir;

    use super::*;
    use crate::agent::FakeLauncher;
    use crate::config_file::DaemonOptions;
    use crate::git::{FakeGit, Git};
    use crate::jobs::declared::{Clock, SystemClock, TokioClock};
    use crate::paths::DataDir;
    use crate::server::services::AbortOnDrop;
    use crate::store::Store;
    use crate::supervisor::{Supervisor, SupervisorConfig};
    use crate::threads::{NewThread, Threads};
    use crate::widgets::Widgets;
    use crate::workspaces::{NewWorkspace, Workspaces};

    const DAY_MS: i64 = 86_400_000;

    /// The test clock's start, in unix milliseconds.
    const ORIGIN_MS: i64 = 1_795_000_000_000;

    struct Harness {
        dir: TempDir,
        services: Services,
        data: DataDir,
        workspace: String,
        folder: PathBuf,
    }

    async fn harness(clock: Arc<dyn Clock>) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().join("data"));
        std::fs::create_dir_all(data.root()).unwrap();
        let store = Arc::new(Store::open(&data.db()).unwrap());
        let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
        let workspaces = Arc::new(Workspaces::new(
            Arc::clone(&store),
            Arc::new(FakeGit::new()) as Arc<dyn Git>,
            dir.path().join("workspaces"),
        ));
        let view = workspaces.create(NewWorkspace::new()).unwrap();
        let supervisor = Supervisor::new(
            Arc::new(FakeLauncher::new()),
            Arc::clone(&threads),
            Arc::clone(&workspaces),
            data.clone(),
            SupervisorConfig::new("omp"),
        );
        let services = Services::new(
            store,
            threads,
            workspaces,
            supervisor,
            Arc::new(Widgets::new(None)),
            clock,
        )
        .await
        .unwrap();
        Harness {
            dir,
            services,
            data,
            workspace: view.workspace.id,
            folder: view.workspace.path,
        }
    }

    fn set_after(value: &str) -> SetSettingsParams {
        SetSettingsParams {
            daemon: Some(DaemonSettings {
                delete_archived_after: Some(value.to_owned()),
            }),
            ..SetSettingsParams::default()
        }
    }

    impl Harness {
        /// A new thread archived at `archived_at` (unix ms); `None`: not archived.
        fn thread(&self, archived_at: Option<i64>) -> String {
            let id = self
                .services
                .threads
                .create(NewThread::new("t", &self.workspace))
                .unwrap()
                .id;
            self.services
                .store
                .set_thread_archived_at(&id, archived_at)
                .unwrap();
            id
        }

        fn exists(&self, id: &str) -> bool {
            self.services.threads.get(id).is_ok()
        }

        fn indexed(&self, id: &str) -> bool {
            self.services
                .index
                .with_state(|index| index.threads.iter().any(|thread| thread.id == id))
        }

        /// Lets the runtime's tasks run until `done`, Tokio's paused time advancing when they all wait.
        async fn until(&self, done: impl Fn(&Self) -> bool) {
            for _ in 0..3000 {
                if done(self) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("the condition never held");
        }
    }

    // spec: DMN-395, DMN-039
    #[tokio::test(start_paused = true)]
    async fn by_default_threads_archived_over_a_week_go_at_startup_then_at_hourly_checks() {
        let h = harness(Arc::new(TokioClock::new(ORIGIN_MS))).await;
        let old = h.thread(Some(ORIGIN_MS - 8 * DAY_MS));
        let recent = h.thread(Some(ORIGIN_MS - 6 * DAY_MS));
        let open = h.thread(None);
        h.services.refresh_index().await.unwrap();
        assert!(h.indexed(&old));

        let _cleanup = AbortOnDrop(h.services.spawn_archive_cleanup(None));

        h.until(|h| !h.indexed(&old)).await;
        assert!(!h.exists(&old));
        assert!(!h.data.thread(&old).exists(), "its directory goes too");
        assert!(h.exists(&recent) && h.indexed(&recent));
        assert!(h.exists(&open));

        // A day and an hour later the other one has been archived for over a week too.
        let later = u64::try_from(DAY_MS + CHECK_EVERY_MS).unwrap();
        tokio::time::advance(Duration::from_millis(later)).await;
        h.until(|h| !h.indexed(&recent)).await;
        assert!(!h.exists(&recent));
        assert!(h.exists(&open), "an unarchived thread is never deleted");
        assert!(h.indexed(&open));
    }

    // spec: DMN-395, DMN-396, PRT-231
    #[tokio::test(start_paused = true)]
    async fn never_keeps_archived_threads_and_settings_set_applies_from_the_next_check() {
        let h = harness(Arc::new(TokioClock::new(ORIGIN_MS))).await;
        h.services.set_settings(set_after("never")).await.unwrap();
        let old = h.thread(Some(ORIGIN_MS - 30 * DAY_MS));
        let newer = h.thread(Some(ORIGIN_MS - 10 * DAY_MS));

        assert_eq!(
            h.services.delete_expired_archived(None).await,
            Vec::<String>::new()
        );
        assert!(h.exists(&old));

        let error = h.services.set_settings(set_after("7")).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::BadRequest);
        assert_eq!(
            h.services.delete_expired_archived(None).await,
            Vec::<String>::new(),
            "a refused value changes nothing"
        );

        let settings = h.services.set_settings(set_after("2w")).await.unwrap();
        assert_eq!(
            settings.daemon,
            Some(DaemonSettings {
                delete_archived_after: Some("2w".to_owned())
            })
        );
        assert_eq!(h.services.settings().daemon, settings.daemon);
        assert_eq!(h.services.delete_expired_archived(None).await, [old]);
        assert!(h.exists(&newer));

        // Only a thread still archived by the cutoff goes, whatever the list said.
        let open = h.thread(None);
        for id in [&newer, &open] {
            let cutoff = ORIGIN_MS - 20 * DAY_MS;
            assert!(
                !h.services
                    .supervisor
                    .delete_archived(id, cutoff)
                    .await
                    .unwrap()
            );
            assert!(h.exists(id));
        }
    }

    // spec: DMN-396, DMN-091
    #[tokio::test(start_paused = true)]
    async fn an_inconclusive_first_config_read_keeps_the_startup_value() {
        let h = harness(Arc::new(TokioClock::new(ORIGIN_MS))).await;
        let old = h.thread(Some(ORIGIN_MS - 30 * DAY_MS));
        // A directory cannot be read: the first read settles nothing.
        let path = h.dir.path().join("config.toml");
        std::fs::create_dir(&path).unwrap();
        let startup = DaemonOptions {
            delete_archived_after: Some(DeleteArchivedAfter::Never),
            ..DaemonOptions::default()
        };

        let _follow = AbortOnDrop(h.services.follow_config_file(path, startup).await.unwrap());

        assert_eq!(
            h.services.delete_expired_archived(None).await,
            Vec::<String>::new()
        );
        assert!(h.exists(&old));
    }

    // spec: DMN-396
    #[tokio::test(start_paused = true)]
    async fn the_flag_wins_over_the_config_file() {
        let h = harness(Arc::new(TokioClock::new(ORIGIN_MS))).await;
        h.services.set_settings(set_after("1w")).await.unwrap();
        let day_old = h.thread(Some(ORIGIN_MS - 2 * DAY_MS));
        let week_old = h.thread(Some(ORIGIN_MS - 8 * DAY_MS));

        assert_eq!(
            h.services
                .delete_expired_archived(Some(DeleteArchivedAfter::Never))
                .await,
            Vec::<String>::new()
        );
        assert_eq!(
            h.services
                .delete_expired_archived(Some(DeleteArchivedAfter::After {
                    count: 1,
                    unit: 'd'
                }))
                .await
                .len(),
            2
        );
        assert!(!h.exists(&day_old) && !h.exists(&week_old));
    }

    // spec: DMN-395, DMN-333
    #[tokio::test(flavor = "multi_thread")]
    async fn an_archived_thread_whose_agents_runs_still_run_is_kept_until_they_end() {
        let h = harness(Arc::new(SystemClock)).await;
        let thread = h.thread(Some(crate::store::now_ms() - 8 * DAY_MS));
        let job = h
            .services
            .jobs
            .start(
                &h.workspace,
                &h.folder,
                "while true; do sleep 0.01; done",
                Some(thread.as_str()),
                None,
            )
            .unwrap();

        assert_eq!(
            h.services.delete_expired_archived(None).await,
            Vec::<String>::new()
        );
        assert!(h.exists(&thread));
        assert_eq!(
            h.services.jobs.running_started_by(&thread).len(),
            1,
            "nothing is killed"
        );

        let log = h.services.jobs.log(&job.id).unwrap();
        let mut live = log.subscribe(None).live;
        h.services.jobs.kill(&job.id).unwrap();
        tokio::time::timeout(Duration::from_secs(60), async {
            while !h.services.jobs.running_started_by(&thread).is_empty() {
                let _ = live.recv().await;
            }
        })
        .await
        .expect("the run did not end");
        assert_eq!(h.services.delete_expired_archived(None).await, [thread]);
    }
}
