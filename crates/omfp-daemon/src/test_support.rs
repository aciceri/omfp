//! The daemon's stack for unit tests, over a temporary directory: the store, threads and workspaces on a fake git, a
//! supervisor on a fake launcher, and the services over them.

use std::path::Path;
use std::sync::Arc;

use crate::agent::FakeLauncher;
use crate::git::{FakeGit, Git};
use crate::jobs::declared::SystemClock;
use crate::paths::DataDir;
use crate::server::Services;
use crate::store::Store;
use crate::supervisor::{Supervisor, SupervisorConfig};
use crate::threads::Threads;
use crate::widgets::Widgets;
use crate::workspaces::Workspaces;

pub(crate) struct Stack {
    pub store: Arc<Store>,
    pub threads: Arc<Threads>,
    pub workspaces: Arc<Workspaces>,
    pub supervisor: Supervisor,
}

impl Stack {
    /// The stack over `dir`: the data directory `<dir>/data` (opened again when it exists), workspace folders under
    /// `<dir>/workspaces`.
    pub(crate) fn new(
        dir: &Path,
        git: Arc<FakeGit>,
        launcher: &FakeLauncher,
        config: SupervisorConfig,
    ) -> Self {
        let data = DataDir::new(dir.join("data"));
        std::fs::create_dir_all(data.root()).unwrap();
        let store = Arc::new(Store::open(&data.db()).unwrap());
        let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
        let workspaces = Arc::new(Workspaces::new(
            Arc::clone(&store),
            git as Arc<dyn Git>,
            dir.join("workspaces"),
        ));
        let supervisor = Supervisor::new(
            Arc::new(launcher.clone()),
            Arc::clone(&threads),
            Arc::clone(&workspaces),
            data,
            config,
        );
        Self {
            store,
            threads,
            workspaces,
            supervisor,
        }
    }

    /// [`Stack::new`] on a fake git of its own and the default supervisor config.
    pub(crate) fn with_launcher(dir: &Path, launcher: &FakeLauncher) -> Self {
        let git = Arc::new(FakeGit::new());
        Self::new(dir, git, launcher, SupervisorConfig::new("omp"))
    }

    /// The services over the stack, its widgets served from the web bundle `static_dir`.
    pub(crate) async fn services(self, static_dir: Option<&Path>) -> Services {
        Services::new(
            self.store,
            self.threads,
            self.workspaces,
            self.supervisor,
            Arc::new(Widgets::new(static_dir)),
            Arc::new(SystemClock),
        )
        .await
        .unwrap()
    }
}
