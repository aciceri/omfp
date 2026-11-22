//! Polling of watched workspaces (DMN-023), with no filesystem watcher: every [`POLL`] the cheap check of each watched
//! workspace (its [`fingerprint`]), a full scan (the rescan, then the git status of `workspace/<id>`) only when the
//! fingerprint changed, and a full scan of each watched workspace every [`FULL_SCAN`] anyway (edits outside omfp that
//! touch no git file). The fingerprint does not see `.omfp/`, so every round also reads the views and widget folders
//! again (DMN-082; a full scan does it as part of the rescan). A workspace is watched while a client subscribes to
//! `workspace/<id>`, `views/<id>` or `jobs/<id>` or to the `thread/<id>` of one of its threads, or while one of its
//! threads runs omp. Every round also looks at the declared jobs file of every workspace, watched or not (DMN-322),
//! and compares the MCP configs of the running threads with the ones their omp read (DMN-201).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use omfp_protocol::ErrorBody;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::warn;

use super::{Services, blocking};
use crate::store::Store;
use crate::supervisor::Supervisor;
use crate::workspaces::{Fingerprint, fingerprint};

/// How often watched workspaces get the cheap check.
pub(crate) const POLL: Duration = Duration::from_secs(2);

/// How often a watched workspace gets a full scan whatever its fingerprint.
pub(crate) const FULL_SCAN: Duration = Duration::from_secs(30);

/// A watched workspace's last full scan.
struct Scanned {
    /// Taken after the scan, so files git rewrites while scanning (the index) do not trigger another.
    fingerprint: Fingerprint,
    at: Instant,
}

impl Services {
    /// Polls the watched workspaces until the returned task is aborted (DMN-023).
    pub fn spawn_polls(&self) -> JoinHandle<()> {
        let services = self.clone();
        tokio::spawn(async move {
            let mut scanned = HashMap::new();
            let mut ticks = tokio::time::interval(POLL);
            ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticks.tick().await;
                if let Err(error) = services.poll(&mut scanned).await {
                    warn!(error = %error.message, "workspace poll failed");
                }
            }
        })
    }

    /// One round: a full scan of each watched workspace that has none yet, changed, or is due; the views of the others;
    /// the jobs files of all.
    async fn poll(&self, scanned: &mut HashMap<String, Scanned>) -> Result<(), ErrorBody> {
        self.supervisor.check_mcp_configs().await;
        // Jobs run without windows: an edit of their file applies whether anyone watches the workspace or not.
        self.declared.poll_files().await;
        let (store, supervisor) = (Arc::clone(&self.store), self.supervisor.clone());
        let watched = blocking(move || watched(&store, &supervisor)).await?;
        // A workspace watched again starts with a full scan.
        scanned.retain(|id, _| watched.iter().any(|(watched, _)| watched == id));
        for (id, path) in watched {
            let due = match scanned.get(&id) {
                Some(last) if last.at.elapsed() < FULL_SCAN => {
                    let path = path.clone();
                    blocking(move || Ok(fingerprint(&path))).await? != last.fingerprint
                }
                _ => true,
            };
            if !due {
                self.refresh_views(&id).await;
                continue;
            }
            if let Err(error) = self.full_scan(&id).await {
                warn!(workspace = %id, error = %error.message, "workspace scan failed");
            }
            let fingerprint = blocking(move || Ok(fingerprint(&path))).await?;
            scanned.insert(
                id,
                Scanned {
                    fingerprint,
                    at: Instant::now(),
                },
            );
        }
        Ok(())
    }

    /// The rescan (a change reaches the index and the live threads' roots), then the git status published on
    /// `workspace/<id>`; also what a tool run's end triggers (`spawn_rescans`) and a written file
    /// (`workspaces.write_file`, DMN-090).
    pub(crate) async fn full_scan(&self, id: &str) -> Result<(), ErrorBody> {
        self.rescan(id).await?;
        self.workspace_status(id).await?;
        Ok(())
    }
}

/// The watched workspaces with their folders, oldest first.
fn watched(store: &Store, supervisor: &Supervisor) -> Result<Vec<(String, PathBuf)>, ErrorBody> {
    let holding: HashSet<String> = store
        .threads()?
        .into_iter()
        .filter(|thread| supervisor.watchers(&thread.id) > 0 || supervisor.is_running(&thread.id))
        .map(|thread| thread.workspace_id)
        .collect();
    Ok(store
        .workspaces()?
        .into_iter()
        .filter(|ws| holding.contains(&ws.id) || supervisor.workspace_watchers(&ws.id) > 0)
        .map(|ws| (ws.id, ws.path))
        .collect())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use omfp_core::ThreadEvent;
    use omp_rpc::{RpcAgentEvent, RpcNotification};
    use tempfile::TempDir;

    use super::*;
    use crate::agent::FakeLauncher;
    use crate::git::{FakeGit, Git};
    use crate::paths::DataDir;
    use crate::server::services::AbortOnDrop;
    use crate::supervisor::SupervisorConfig;
    use crate::threads::{NewThread, Threads};
    use crate::workspaces::{NewWorkspace, Workspaces};

    struct Harness {
        dir: TempDir,
        services: Services,
        launcher: FakeLauncher,
        git: Arc<FakeGit>,
        /// The checkouts `git status` ran in, in order.
        statuses: Arc<Mutex<Vec<PathBuf>>>,
        /// A workspace holding one fake clone, `app`.
        workspace: String,
        app: PathBuf,
    }

    async fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().join("data"));
        std::fs::create_dir_all(data.root()).unwrap();
        let root = dir.path().join("workspaces");
        let store = Arc::new(Store::open(&data.db(), &root).unwrap());
        let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
        let git = Arc::new(FakeGit::new());
        let workspaces = Arc::new(Workspaces::new(
            Arc::clone(&store),
            Arc::clone(&git) as Arc<dyn Git>,
            root,
        ));
        let view = workspaces.create(NewWorkspace::new()).unwrap();
        let app = view.workspace.path.join("app");
        std::fs::create_dir_all(app.join(".git")).unwrap();
        std::fs::write(app.join(".git/index"), "1").unwrap();
        git.add_repo(&app, "main");
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&statuses);
        git.on_status(move |dir| seen.lock().unwrap().push(dir.to_owned()));
        let widgets = Arc::new(crate::widgets::Widgets::new(data.widgets(), None));
        let launcher = FakeLauncher::new();
        let supervisor = Supervisor::new(
            Arc::new(launcher.clone()),
            Arc::clone(&threads),
            Arc::clone(&workspaces),
            data,
            SupervisorConfig::new("omp"),
        );
        let services = Services::new(
            store,
            threads,
            workspaces,
            supervisor,
            widgets,
            Arc::new(crate::jobs::declared::SystemClock),
        )
        .await
        .unwrap();
        Harness {
            dir,
            services,
            launcher,
            git,
            statuses,
            workspace: view.workspace.id,
            app,
        }
    }

    impl Harness {
        /// Full scans of the workspace so far: `git status` runs once in `app` per full scan.
        fn full_scans(&self) -> usize {
            let statuses = self.statuses.lock().unwrap();
            statuses.iter().filter(|dir| **dir == self.app).count()
        }

        fn thread(&self) -> String {
            self.services
                .threads
                .create(NewThread::new("t", &self.workspace))
                .unwrap()
                .id
        }

        fn index_repos(&self) -> Vec<String> {
            self.services.index.with_state(|index| {
                index
                    .workspaces
                    .iter()
                    .find(|ws| ws.id == self.workspace)
                    .map(|ws| ws.repos.iter().map(|repo| repo.name.clone()).collect())
                    .unwrap_or_default()
            })
        }

        fn app_dirty(&self) -> Option<bool> {
            self.services
                .workspace_log_entry(&self.workspace)
                .with_state(|repos| {
                    repos
                        .iter()
                        .find(|repo| repo.name == "app")
                        .map(|repo| repo.status.dirty)
                })
        }
    }

    /// Lets paused time run to `at` seconds after the start of the test (`start`), between two polls.
    async fn until(start: Instant, at: u64) {
        tokio::time::sleep_until(start + Duration::from_secs(at)).await;
    }

    fn grow(file: &Path) {
        let mut text = std::fs::read_to_string(file).unwrap_or_default();
        text.push('x');
        std::fs::write(file, text).unwrap();
    }

    // spec: DMN-023
    #[tokio::test(start_paused = true)]
    async fn unwatched_workspaces_are_not_polled() {
        let h = harness().await;
        let start = Instant::now();
        let _polls = AbortOnDrop(h.services.spawn_polls());

        until(start, 61).await;

        assert_eq!(h.full_scans(), 0);
    }

    // spec: DMN-322
    #[tokio::test]
    async fn a_poll_round_applies_the_jobs_file_of_an_unwatched_workspace() {
        use omfp_protocol::{JobRunStatus, JobsEvent};

        let h = harness().await;
        let folder = h.app.parent().unwrap().to_owned();
        let log = h.services.declared.log(&h.workspace).await.unwrap();
        let mut events = log.listen();
        let mut next = async || {
            tokio::time::timeout(Duration::from_secs(30), events.recv())
                .await
                .expect("no jobs event came")
                .unwrap()
                .event
                .clone()
        };
        let service = |enabled: bool| {
            serde_json::json!({ "jobs": { "watch": {
                "trigger": "service", "command": "while true; do sleep 0.01; done", "enabled": enabled
            } } })
            .to_string()
        };
        std::fs::create_dir_all(folder.join(".omfp")).unwrap();
        std::fs::write(folder.join(".omfp/jobs.json"), service(true)).unwrap();

        h.services.poll(&mut HashMap::new()).await.unwrap();

        let run = loop {
            if let JobsEvent::RunStarted { run } = next().await {
                break run;
            }
        };
        std::fs::write(folder.join(".omfp/jobs.json"), service(false)).unwrap();

        h.services.poll(&mut HashMap::new()).await.unwrap();

        let ended = loop {
            if let JobsEvent::RunEnded { run } = next().await {
                break run;
            }
        };
        assert_eq!((ended.id, ended.status), (run.id, JobRunStatus::Killed));
        assert_eq!(h.full_scans(), 0, "the workspace was never rescanned");
    }

    // spec: DMN-023
    #[tokio::test(start_paused = true)]
    async fn a_watched_workspace_without_change_gets_a_full_scan_every_30_s_only() {
        let h = harness().await;
        let _watch = h.services.supervisor.watch_workspace(&h.workspace);
        let start = Instant::now();
        let _polls = AbortOnDrop(h.services.spawn_polls());

        until(start, 1).await;
        assert_eq!(h.full_scans(), 1, "the first poll scans");
        until(start, 29).await;
        assert_eq!(h.full_scans(), 1, "14 cheap checks, nothing changed");
        until(start, 31).await;
        assert_eq!(h.full_scans(), 2, "the 30 s full scan");
    }

    // spec: DMN-023
    #[tokio::test(start_paused = true)]
    async fn a_change_seen_by_the_cheap_check_runs_a_full_scan_at_the_next_poll() {
        let h = harness().await;
        let _watch = h.services.supervisor.watch_workspace(&h.workspace);
        let start = Instant::now();
        let _polls = AbortOnDrop(h.services.spawn_polls());
        until(start, 1).await;
        assert_eq!(
            (h.full_scans(), h.index_repos()),
            (1, vec!["app".to_owned()])
        );

        // A new link (a new top-level entry): the rescan updates the index.
        let lib = h.dir.path().join("src/lib");
        std::fs::create_dir_all(&lib).unwrap();
        h.git.add_repo(&lib, "main");
        let folder = h.app.parent().unwrap();
        std::os::unix::fs::symlink(&lib, folder.join("lib")).unwrap();
        until(start, 3).await;
        assert_eq!(h.full_scans(), 2);
        assert_eq!(h.index_repos(), ["app", "lib"]);

        // A touched index (git add, commit, checkout…): the git status reaches `workspace/<id>`.
        assert_eq!(h.app_dirty(), Some(false));
        h.git.set_dirty(&h.app, true);
        grow(&h.app.join(".git/index"));
        until(start, 5).await;
        assert_eq!(h.full_scans(), 3);
        assert_eq!(h.app_dirty(), Some(true));
    }

    // spec: DMN-023
    #[tokio::test(start_paused = true)]
    async fn the_fingerprint_is_taken_after_the_scan_so_files_git_rewrites_do_not_loop() {
        let h = harness().await;
        // `git status` refreshes the index, as real git does.
        let index = h.app.join(".git/index");
        let seen = Arc::clone(&h.statuses);
        h.git.on_status(move |dir| {
            seen.lock().unwrap().push(dir.to_owned());
            grow(&index);
        });
        let _watch = h.services.supervisor.watch_workspace(&h.workspace);
        let start = Instant::now();
        let _polls = AbortOnDrop(h.services.spawn_polls());

        until(start, 1).await;
        assert_eq!(h.full_scans(), 1);
        until(start, 29).await;
        assert_eq!(h.full_scans(), 1);
    }

    // spec: DMN-023, UI-025
    #[tokio::test(start_paused = true)]
    async fn a_tool_runs_end_publishes_a_dirty_state_the_cheap_check_cannot_see() {
        let h = harness().await;
        let thread = h.thread();
        h.services.supervisor.start(&thread).await.unwrap();
        h.services.workspace_log(&h.workspace).await.unwrap();
        assert_eq!(h.app_dirty(), Some(false));
        let _rescans = AbortOnDrop(h.services.spawn_rescans());

        // The agent's `write` to a tracked file: no top-level entry, no git file changes, no poll runs.
        h.git.set_dirty(&h.app, true);
        h.launcher.process(0).emit(vec![tool_end()]);

        tokio::time::timeout(Duration::from_secs(10), async {
            while h.app_dirty() != Some(true) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the tool run's end publishes the dirty state on `workspace/<id>`");
    }

    // spec: DMN-023
    #[tokio::test(start_paused = true)]
    async fn only_tools_that_can_change_files_trigger_a_git_scan() {
        let h = harness().await;
        let thread = h.thread();
        h.services.supervisor.start(&thread).await.unwrap();
        let _rescans = AbortOnDrop(h.services.spawn_rescans());
        let before = h.full_scans();

        for name in ["read", "grep", "glob", "web_search", "todo", "ask"] {
            h.launcher.process(0).emit(vec![tool_end_named(name)]);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(
            h.full_scans(),
            before,
            "read-only tools run no `git status`"
        );

        h.launcher.process(0).emit(vec![tool_end_named("edit")]);
        tokio::time::timeout(Duration::from_secs(10), async {
            while h.full_scans() == before {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("an `edit` end runs a full scan");
    }

    fn tool_end_named(name: &str) -> RpcNotification {
        let mut frame = tool_end();
        if let RpcNotification::RpcAgentEvent(RpcAgentEvent::ToolExecutionEnd(end)) = &mut frame {
            name.clone_into(&mut end.tool_name);
        }
        frame
    }

    /// The `tool_execution_end` of the recorded bash scenario.
    fn tool_end() -> RpcNotification {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/threads")
            .join(omfp_core::OMP_VERSION)
            .join("bash_tool_call.jsonl");
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<ThreadEvent>(line).unwrap())
            .find_map(|event| match event {
                ThreadEvent::Omp {
                    frame:
                        frame @ RpcNotification::RpcAgentEvent(RpcAgentEvent::ToolExecutionEnd(_)),
                } => Some(frame),
                _ => None,
            })
            .unwrap()
    }

    // spec: DMN-023
    #[tokio::test(start_paused = true)]
    async fn a_watched_or_running_thread_makes_its_workspace_watched() {
        let h = harness().await;
        let thread = h.thread();
        let start = Instant::now();
        let _polls = AbortOnDrop(h.services.spawn_polls());
        until(start, 1).await;
        assert_eq!(h.full_scans(), 0);

        // A client on `thread/<id>`.
        let watch = h.services.supervisor.watch(&thread, 1, true).await.unwrap();
        until(start, 3).await;
        assert_eq!(h.full_scans(), 1);
        drop(watch);
        until(start, 61).await;
        assert_eq!(h.full_scans(), 1, "unwatched again");

        // The thread's omp runs.
        h.services.supervisor.start(&thread).await.unwrap();
        let started = Instant::now();
        tokio::time::sleep_until(started + POLL).await;
        assert_eq!(h.full_scans(), 2);
    }

    // spec: DMN-082, AGT-013
    #[tokio::test(start_paused = true)]
    async fn every_poll_round_reads_the_views_of_a_watched_workspace_without_a_full_scan() {
        let h = harness().await;
        // The folder exists already: a new top-level entry is a change of the fingerprint.
        let folder = h.app.parent().unwrap().join(".omfp");
        std::fs::create_dir_all(folder.join("widgets")).unwrap();
        let log = h.services.views_log(&h.workspace).await.unwrap();
        let _watch = h.services.supervisor.watch_workspace(&h.workspace);
        let start = Instant::now();
        let _polls = AbortOnDrop(h.services.spawn_polls());
        until(start, 1).await;
        assert_eq!(h.full_scans(), 1);
        assert_eq!(log.with_state(|views| views.widgets.len()), 0);

        std::fs::create_dir(folder.join("widgets/tests")).unwrap();
        std::fs::write(folder.join("widgets/tests/index.tsx"), "export default 1;").unwrap();
        std::fs::write(
            folder.join("views.json"),
            r#"{ "views": { "a": { "layout": "./widgets/tests" } } }"#,
        )
        .unwrap();
        until(start, 3).await;

        assert_eq!(h.full_scans(), 1, "`.omfp/` is not in the fingerprint");
        let (manifest, widgets) =
            log.with_state(|views| (views.manifest.clone(), views.widgets.clone()));
        assert!(manifest.is_some());
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].entry.as_deref(), Some("index.tsx"));
        let version = widgets[0].version.clone();

        std::fs::write(folder.join("widgets/tests/index.tsx"), "export default 22;").unwrap();
        until(start, 5).await;
        assert_ne!(
            log.with_state(|views| views.widgets[0].version.clone()),
            version
        );
    }

    /// What a test's tracing subscriber writes.
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // spec: DMN-384
    #[tokio::test]
    async fn a_failing_repository_is_logged_once_per_change_of_its_error() {
        let h = harness().await;
        let logs = Arc::new(Mutex::new(Vec::new()));
        let writer = Arc::clone(&logs);
        // The scans' async part runs on this thread (a current-thread runtime), where the subscriber applies.
        let _logging = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(move || Captured(Arc::clone(&writer)))
                .finish(),
        );
        let warnings = || {
            String::from_utf8(logs.lock().unwrap().clone())
                .unwrap()
                .matches("cannot read the git status of a repository")
                .count()
        };
        let scan = async || h.services.full_scan(&h.workspace).await.unwrap();

        h.git.fail_status(&h.app, Some("fatal: one"));
        for _ in 0..3 {
            scan().await;
        }
        assert_eq!(
            warnings(),
            1,
            "every poll and rescan reads the status again"
        );

        h.git.fail_status(&h.app, Some("fatal: two"));
        scan().await;
        scan().await;
        assert_eq!(warnings(), 2, "another error is news");

        h.git.fail_status(&h.app, None);
        scan().await;
        assert_eq!(warnings(), 2);
        h.git.fail_status(&h.app, Some("fatal: two"));
        scan().await;
        assert_eq!(warnings(), 3, "failing again after a success is news");
    }
}
