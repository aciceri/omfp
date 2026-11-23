//! Component tests (L1) of the supervisor: a fixture-driven fake launcher (TST-014), fake git, temp workspace
//! folders, paused time.

use std::path::Path;

use omfp_core::replay;
use omfp_protocol::InteractionResponse;
use omp_rpc::{AgentStartEvent, RpcAgentEvent, RpcNotification};
use serde_json::json;
use tempfile::TempDir;

use super::dialogs::dialog_id;
use super::*;
use crate::agent::{FakeCall, FakeLauncher, FakeReply, FakeRun};
use crate::event_log::Start;
use crate::git::{FakeGit, Git};
use crate::host_tools::{NOTIFY, OPEN_FILE, SET_THREAD_TITLE, run_set_thread_title};
use crate::store::{Store, StoreError};
use crate::threads::NewThread;
use crate::workspaces::{NewWorkspace, Workspaces};

#[path = "tests/interrupted.rs"]
mod interrupted;
#[path = "tests/predict.rs"]
mod predict;
#[path = "tests/rolling_titles.rs"]
mod rolling_titles;

struct Harness {
    dir: TempDir,
    launcher: FakeLauncher,
    supervisor: Supervisor,
    threads: Arc<Threads>,
    workspaces: Arc<Workspaces>,
    git: Arc<FakeGit>,
    data: DataDir,
}

fn harness(configure: impl FnOnce(&mut SupervisorConfig)) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path().join("data"));
    std::fs::create_dir_all(data.root()).unwrap();
    let root = dir.path().join("workspaces");
    let store = Arc::new(Store::open(&data.db(), &root).unwrap());
    let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
    let git = Arc::new(FakeGit::new());
    let workspaces = Arc::new(Workspaces::new(
        store,
        Arc::clone(&git) as Arc<dyn Git>,
        root,
    ));
    let launcher = FakeLauncher::new();
    let mut config = SupervisorConfig::new("omp");
    configure(&mut config);
    let supervisor = Supervisor::new(
        Arc::new(launcher.clone()),
        Arc::clone(&threads),
        Arc::clone(&workspaces),
        data.clone(),
        config,
    );
    Harness {
        dir,
        launcher,
        supervisor,
        threads,
        workspaces,
        git,
        data,
    }
}

impl Harness {
    /// A thread in a new, empty workspace.
    fn thread(&self) -> String {
        let (ws, _) = self.workspace();
        self.threads.create(NewThread::new("t", ws)).unwrap().id
    }

    /// A new workspace folder: its id and path.
    fn workspace(&self) -> (String, PathBuf) {
        let view = self.workspaces.create(NewWorkspace::new()).unwrap();
        (view.workspace.id, view.workspace.path)
    }

    /// A fake repository outside every workspace.
    fn external_repo(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join("src").join(name);
        std::fs::create_dir_all(&path).unwrap();
        self.git.add_repo(&path, "main");
        path
    }

    /// The local commands (`/add-dir`, `/remove-dir`) the thread's first process got.
    fn local_commands(&self) -> Vec<String> {
        self.local_commands_of(0)
    }

    /// The local commands the `index`th process got.
    fn local_commands_of(&self, index: usize) -> Vec<String> {
        self.launcher
            .process(index)
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                FakeCall::Prompt(prompt) if prompt.message.starts_with('/') => Some(prompt.message),
                _ => None,
            })
            .collect()
    }

    async fn log(&self, id: &str) -> Arc<ThreadLog> {
        self.supervisor.thread_log(id).await.unwrap()
    }

    async fn wait(&self, id: &str, what: &str, done: impl Fn(&ThreadState) -> bool) {
        let log = self.log(id).await;
        let mut live = log.subscribe(None).live;
        while !log.with_state(&done) {
            assert!(
                !matches!(live.recv().await, Err(broadcast::error::RecvError::Closed)),
                "log closed while waiting for {what}"
            );
        }
    }
}

fn fixture_text(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/threads")
        .join(omfp_core::OMP_VERSION)
        .join(format!("{name}.jsonl"));
    std::fs::read_to_string(path).unwrap()
}

fn fixture(name: &str) -> FakeRun {
    FakeRun::from_jsonl(&fixture_text(name)).unwrap()
}

fn prompt(text: &str) -> PromptCommand {
    PromptCommand {
        message: text.into(),
        images: None,
        streaming_behavior: None,
    }
}

fn settled_after_prompt(id: &'static str) -> impl Fn(&ThreadState) -> bool {
    move |state| state.settled && state.prompts.contains_key(id)
}

// spec: TST-014, ARC-012, DMN-070, DMN-040
#[tokio::test]
async fn a_prompt_on_a_stopped_thread_starts_omp_and_reduces_the_recorded_turn() {
    let h = harness(|_| {});
    h.launcher.push_run(fixture("text_reply"));
    let id = h.thread();

    let admission = h.supervisor.prompt(&id, prompt("say hi")).await.unwrap();
    h.wait(&id, "the turn", settled_after_prompt("req_7")).await;

    assert_eq!(admission.prompt_id, "req_7");
    assert_eq!(h.launcher.launches().len(), 1);
    let recorded: Vec<ThreadEvent> = fixture_text("text_reply")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let log = h.log(&id).await;
    assert_eq!(log.with_state(Clone::clone), replay(&recorded));
    let Start::Snapshot { seq, .. } = log.subscribe(None).start else {
        panic!("no position given");
    };
    // `Starting` belongs to the previous epoch; the new one starts with the history.
    assert_eq!(seq, u64::try_from(recorded.len()).unwrap());
}

// spec: DMN-030, DMN-029, DMN-060, ARC-008, AGT-024, DMN-107
#[tokio::test]
async fn omp_starts_in_its_workspace_folder_with_one_add_dir_per_repository() {
    let agent_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        agent_dir.path().join("APPEND_SYSTEM.md"),
        "Answer in haiku.\n",
    )
    .unwrap();
    let h = harness(|config| {
        config.omp_env = vec![("OMFP_TEST".into(), Some("1".into()))];
        config.agent_dir = Some(agent_dir.path().to_owned());
    });
    let (ws, folder) = h.workspace();
    let external = h.external_repo("shop-frontend");
    std::os::unix::fs::symlink(&external, folder.join("frontend")).unwrap();
    std::fs::create_dir(folder.join("backend")).unwrap();
    h.git.add_repo(&folder.join("backend"), "main");
    std::fs::create_dir(folder.join("notes")).unwrap();
    let mut new = NewThread::new("multi", &ws);
    new.model = Some("fake/scripted".into());
    new.approval_mode = Some("always-ask".into());
    let id = h.threads.create(new).unwrap().id;

    h.supervisor.start(&id).await.unwrap();

    let launch = &h.launcher.launches()[0];
    assert!(
        launch
            .spec
            .append_system_prompt
            .as_ref()
            .unwrap()
            .contains(&ws),
        "the agent receives its actual workspace id"
    );
    assert_eq!(launch.spec.cwd, folder);
    assert_eq!(
        launch.spec.add_dirs,
        [folder.join("backend"), folder.join("frontend")],
        "repositories by name, the symlink path and not its target; `notes` is no repository"
    );
    assert_eq!(
        launch.spec.append_system_prompt.as_deref(),
        Some(
            format!(
                "{}\n\nAnswer in haiku.",
                conventions::daemon_text("workspace", &ws, &folder, &h.data.widgets(), &id)
            )
            .as_str()
        ),
        "the conventions, the widgets note (user library), the control note (thread id), then the user's file"
    );
    assert_eq!(
        launch.spec.session_dir,
        h.threads.get(&id).unwrap().session_dir
    );
    assert_eq!(launch.spec.model.as_deref(), Some("fake/scripted"));
    assert_eq!(launch.spec.approval_mode.as_deref(), Some("always-ask"));
    assert_eq!(launch.spec.stderr_log, h.data.thread_log(&id));
    assert_eq!(
        launch.spec.extension.as_deref(),
        Some(h.data.omp_extension().as_path()),
        "the daemon's omp extension root ships the widget-builder agent"
    );
    assert_eq!(launch.spec.env, [("OMFP_TEST".into(), Some("1".into()))]);
    assert_eq!(launch.tools, [OPEN_FILE, NOTIFY, SET_THREAD_TITLE]);
    // The session is stored once omp opened it (DMN-032), and its roots are the flags (DMN-029).
    assert!(h.threads.get(&id).unwrap().omp_session_id.is_some());
    assert_eq!(h.threads.roots(&id).unwrap(), launch.spec.add_dirs);
    assert_eq!(h.local_commands(), Vec::<String>::new());
}

// spec: DMN-062, AGT-025, DMN-125
#[tokio::test]
async fn every_launch_registers_the_omfp_host_uri_scheme_next_to_the_tools() {
    let h = harness(|_| {});
    let id = h.thread();

    h.supervisor.start(&id).await.unwrap();
    h.supervisor.stop(&id).await.unwrap();
    h.supervisor.start(&id).await.unwrap();

    let launches = h.launcher.launches();
    assert_eq!(launches.len(), 2);
    for launch in launches {
        assert_eq!(launch.uris, ["omfp"]);
        assert_eq!(launch.tools, [OPEN_FILE, NOTIFY, SET_THREAD_TITLE]);
    }
}

// spec: DMN-060
#[tokio::test]
async fn open_file_asks_the_window_last_used_on_the_thread_and_fails_without_one() {
    let h = harness(|_| {});
    let id = h.thread();
    let log = h.log(&id).await;
    let opened = || log.with_state(|state| state.open_file.clone());

    assert_eq!(
        h.supervisor.open_file(&id, "notes.md", None),
        Err(NO_WINDOW.to_owned())
    );
    assert_eq!(opened(), None, "nothing to open without a window");

    let first = h.supervisor.watch(&id, 1, true).await.unwrap();
    let second = h.supervisor.watch(&id, 2, true).await.unwrap();
    h.supervisor.open_file(&id, "src/a.rs", Some(12)).unwrap();
    let request = opened().unwrap();
    assert_eq!(
        (request.connection, request.path.as_str(), request.line),
        (2, "src/a.rs", Some(12)),
        "the window that subscribed last"
    );

    h.supervisor.used(&id, 1);
    h.supervisor.used(&id, 7);
    h.supervisor.open_file(&id, "b.md", None).unwrap();
    let next = opened().unwrap();
    assert_eq!(
        (next.connection, next.path.as_str(), next.line),
        (1, "b.md", None),
        "the watching window that drove the thread last; an unwatching one does not count"
    );
    assert_ne!(next.id, request.id, "each request has its own id");

    drop(first);
    h.supervisor.open_file(&id, "c.md", None).unwrap();
    assert_eq!(opened().unwrap().connection, 2, "the last used window left");

    drop(second);
    assert_eq!(
        h.supervisor.open_file(&id, "c.md", None),
        Err(NO_WINDOW.to_owned())
    );
}

// spec: DMN-357, DMN-060
#[tokio::test]
async fn the_window_last_used_ignores_subscriptions_that_only_keep_the_thread() {
    let h = harness(|_| {});
    let id = h.thread();
    let log = h.log(&id).await;
    let asked = |path: &str| {
        h.supervisor.open_file(&id, path, None)?;
        Ok::<_, String>(log.with_state(|state| state.open_file.clone().unwrap().connection))
    };

    let shown = h.supervisor.watch(&id, 1, true).await.unwrap();
    let kept = h.supervisor.watch(&id, 2, false).await.unwrap();
    assert_eq!(
        asked("a.md"),
        Ok(1),
        "not the window that subscribed last, which keeps it"
    );
    h.supervisor.used(&id, 2);
    assert_eq!(
        asked("b.md"),
        Ok(1),
        "a keeping window that drives the thread does not count"
    );

    kept.set_watching(true);
    assert_eq!(
        asked("c.md"),
        Ok(2),
        "shown again: the window that watched it last"
    );
    kept.set_watching(false);
    assert_eq!(asked("d.md"), Ok(1));

    drop(shown);
    assert_eq!(
        asked("e.md"),
        Err(NO_WINDOW.to_owned()),
        "a window that only keeps the thread is no window for the agent"
    );
    assert_eq!(h.supervisor.watchers(&id), 0);
    drop(kept);
}

// spec: DMN-125, DMN-126
#[tokio::test]
async fn notify_reaches_the_window_last_used_on_the_thread_and_fails_without_one() {
    let h = harness(|_| {});
    let id = h.thread();
    let log = h.log(&id).await;
    let notified = || log.with_state(|state| state.notify.clone());

    assert_eq!(
        h.supervisor.notify(&id, "Tests", "All green"),
        Err(NOTIFY_NO_WINDOW.to_owned())
    );
    assert_eq!(notified(), None, "nobody to notify without a window");

    let first = h.supervisor.watch(&id, 1, true).await.unwrap();
    let second = h.supervisor.watch(&id, 2, true).await.unwrap();
    h.supervisor.notify(&id, "Tests", "All green").unwrap();
    let request = notified().unwrap();
    assert_eq!(
        (
            request.connection,
            request.title.as_str(),
            request.body.as_str()
        ),
        (2, "Tests", "All green"),
        "the window that subscribed last"
    );

    h.supervisor.used(&id, 1);
    h.supervisor.notify(&id, "Deploy", "Needs your OK").unwrap();
    let next = notified().unwrap();
    assert_eq!(
        (next.connection, next.title.as_str()),
        (1, "Deploy"),
        "the window that drove the thread last"
    );
    assert_ne!(next.id, request.id, "each request has its own id");
    assert_eq!(
        log.with_state(|state| state.open_file.clone()),
        None,
        "a notification opens no file"
    );

    drop(first);
    drop(second);
    assert_eq!(
        h.supervisor.notify(&id, "Late", "Nobody watches"),
        Err(NOTIFY_NO_WINDOW.to_owned())
    );
    assert_eq!(
        notified().unwrap().id,
        next.id,
        "nothing new without a window"
    );
}

// spec: DMN-375, DMN-376, DMN-126
#[tokio::test]
async fn notify_falls_back_to_the_keeping_window_then_to_the_focused_one() {
    let h = harness(|_| {});
    let id = h.thread();
    let log = h.log(&id).await;
    let notified =
        || log.with_state(|state| state.notify.clone().map(|request| request.connection));
    let focused = Arc::new(Mutex::new(None::<u64>));
    let sent = Arc::new(Mutex::new(Vec::<(u64, String, String, String)>::new()));
    h.supervisor.set_focused_notify({
        let (focused, sent) = (Arc::clone(&focused), Arc::clone(&sent));
        Arc::new(move |thread: &str, title: &str, body: &str| {
            let Some(window) = *focused.lock().unwrap() else {
                return Ok(false);
            };
            sent.lock().unwrap().push((
                window,
                thread.to_owned(),
                title.to_owned(),
                body.to_owned(),
            ));
            Ok(true)
        })
    });

    assert_eq!(
        h.supervisor.notify(&id, "Key", "Load it"),
        Err(NOTIFY_NO_WINDOW.to_owned()),
        "no window connected at all"
    );

    *focused.lock().unwrap() = Some(9);
    h.supervisor.notify(&id, "Key", "Load it").unwrap();
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        [(9, id.clone(), "Key".to_owned(), "Load it".to_owned())],
        "no window subscribes the thread: the most recently focused one, whatever it shows"
    );
    assert_eq!(
        notified(),
        None,
        "the thread itself carries no request then"
    );

    let shown = h.supervisor.watch(&id, 1, true).await.unwrap();
    let first_kept = h.supervisor.watch(&id, 2, false).await.unwrap();
    let kept = h.supervisor.watch(&id, 3, false).await.unwrap();
    h.supervisor.notify(&id, "Tests", "All green").unwrap();
    assert_eq!(notified(), Some(1), "the watching window first");

    drop(shown);
    h.supervisor.notify(&id, "Tests", "All green").unwrap();
    assert_eq!(
        notified(),
        Some(3),
        "else the window that subscribed it last and only keeps it"
    );
    assert_eq!(sent.lock().unwrap().len(), 1, "not the focused window");

    drop(kept);
    drop(first_kept);
    *focused.lock().unwrap() = None;
    assert_eq!(
        h.supervisor.notify(&id, "Late", "Nobody"),
        Err(NOTIFY_NO_WINDOW.to_owned())
    );
}

// spec: DMN-033
#[tokio::test]
async fn a_session_open_elsewhere_fails_the_start_with_a_clear_error() {
    let h = harness(|_| {});
    let id = h.thread();
    h.threads.set_omp_session_id(&id, "S-busy").unwrap();
    h.launcher.hold_lease("S-busy");

    let error = h.supervisor.start(&id).await.unwrap_err();

    assert!(
        matches!(error, SupervisorError::SessionLeased { .. }),
        "{error}"
    );
    assert!(
        error.to_string().contains("open in another omp process"),
        "{error}"
    );
    assert_eq!(h.launcher.launches().len(), 0);
    let state = h.log(&id).await.with_state(|state| state.process.clone());
    assert!(matches!(state, ProcessState::Failed { message, .. } if message.contains("S-busy")));
    assert!(!h.supervisor.is_running(&id));
}

/// Writes the stored session `session_id` of thread `id` as omp would: a header, a `model_change` to `model`, then one
/// question and its answer.
fn saved_session(h: &Harness, id: &str, session_id: &str, model: &str) {
    let dir = h.threads.get(id).unwrap().session_dir;
    std::fs::create_dir_all(&dir).unwrap();
    let (provider, model_id) = model.split_once('/').unwrap();
    let lines = [
        json!({ "type": "session", "version": 3, "id": session_id, "timestamp": "2026-11-11T23:25:36.429Z" }),
        json!({ "type": "model_change", "id": "m1", "parentId": null, "timestamp": "2026-11-11T23:25:36.489Z",
                "model": model }),
        json!({ "type": "message", "id": "u1", "parentId": "m1", "timestamp": "2026-11-11T23:25:36.751Z",
                "message": { "role": "user", "content": [{ "type": "text", "text": "old question" }], "timestamp": 1 } }),
        json!({ "type": "message", "id": "a1", "parentId": "u1", "timestamp": "2026-11-11T23:25:37.717Z",
                "message": { "role": "assistant", "content": [{ "type": "text", "text": "old answer" }],
                             "provider": provider, "model": model_id, "stopReason": "stop", "timestamp": 2 } }),
    ];
    let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
    std::fs::write(
        dir.join(format!("2026-11-11T23-25-36-429Z_{session_id}.jsonl")),
        text.join("\n"),
    )
    .unwrap();
    h.threads.set_omp_session_id(id, session_id).unwrap();
}

/// The roles and first texts of the thread's transcript.
async fn transcript(h: &Harness, id: &str) -> Vec<String> {
    h.log(id).await.with_state(|state| {
        state
            .transcript
            .iter()
            .map(|entry| {
                let message = serde_json::to_value(&entry.message).unwrap();
                format!(
                    "{}: {}",
                    message["role"].as_str().unwrap(),
                    message["content"][0]["text"].as_str().unwrap_or_default()
                )
            })
            .collect()
    })
}

// spec: DMN-337, DMN-338, DMN-339
#[tokio::test]
async fn a_thread_whose_saved_model_cannot_start_shows_its_history_and_resumes_with_a_chosen_model()
{
    let h = harness(|_| {});
    let id = h.thread();
    saved_session(&h, &id, "S-old", "anthropic/claude-opus-5-5");
    h.launcher.fail_next_launch(
        "SessionModelRestoreError: Could not restore model anthropic/claude-opus-5-5",
    );

    let error = h.supervisor.start(&id).await.unwrap_err();

    assert!(matches!(error, SupervisorError::Start { .. }), "{error}");
    assert_eq!(
        transcript(&h, &id).await,
        ["user: old question", "assistant: old answer"],
        "the conversation stays readable without omp"
    );
    let (process, session) = h
        .log(&id)
        .await
        .with_state(|state| (state.process.clone(), state.session.clone()));
    assert!(
        matches!(&process, ProcessState::Failed { stderr_tail, .. } if stderr_tail.contains("Could not restore model")),
        "{process:?}"
    );
    assert_eq!(
        session.map(|session| session.session_id).as_deref(),
        Some("S-old")
    );
    assert_eq!(
        h.supervisor.session_model(&id).as_deref(),
        Some("anthropic/claude-opus-5-5")
    );
    assert_eq!(
        h.threads.get(&id).unwrap().model,
        None,
        "nothing is chosen for the user"
    );

    // The user's explicit choice: stored, then omp starts with it on the same session.
    h.supervisor
        .set_model(
            &id,
            SetModelCommand {
                provider: "fake".into(),
                model_id: "scripted".into(),
            },
        )
        .await
        .unwrap();

    assert!(h.supervisor.is_running(&id));
    assert_eq!(
        h.threads.get(&id).unwrap().model.as_deref(),
        Some("fake/scripted")
    );
    let launches = h.launcher.launches();
    assert_eq!(launches.len(), 1, "the failed start never launched");
    assert_eq!(launches[0].spec.model.as_deref(), Some("fake/scripted"));
    assert_eq!(
        launches[0].spec.session_dir,
        h.threads.get(&id).unwrap().session_dir
    );
    assert!(
        !launches[0]
            .process
            .calls()
            .iter()
            .any(|call| matches!(call, FakeCall::Call(OmpCall::SetModel(_)))),
        "the launch flag carries the model"
    );
    assert_eq!(h.supervisor.session_model(&id), None);
}

// spec: DMN-339
#[tokio::test]
async fn a_running_thread_switches_its_model_through_omp_and_stores_it() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();

    h.supervisor
        .set_model(
            &id,
            SetModelCommand {
                provider: "fake".into(),
                model_id: "scripted".into(),
            },
        )
        .await
        .unwrap();

    assert_eq!(h.launcher.launches().len(), 1, "no restart");
    assert!(h.launcher.process(0).calls().iter().any(|call| matches!(
        call,
        FakeCall::Call(OmpCall::SetModel(command)) if command.provider == "fake" && command.model_id == "scripted"
    )));
    assert_eq!(
        h.threads.get(&id).unwrap().model.as_deref(),
        Some("fake/scripted")
    );
}

// spec: DMN-337
#[tokio::test]
async fn a_session_open_elsewhere_still_shows_its_history() {
    let h = harness(|_| {});
    let id = h.thread();
    saved_session(&h, &id, "S-busy", "fake/scripted");
    h.launcher.hold_lease("S-busy");

    h.supervisor.start(&id).await.unwrap_err();

    assert_eq!(
        transcript(&h, &id).await,
        ["user: old question", "assistant: old answer"]
    );
}

/// omp's `available_commands_update` announcing custom commands `names`.
fn commands_update(names: &[&str]) -> RpcNotification {
    let commands: Vec<Value> = names
        .iter()
        .map(|name| json!({ "name": name, "source": "custom" }))
        .collect();
    RpcNotification::AvailableCommandsUpdate(
        serde_json::from_value(json!({ "commands": commands })).unwrap(),
    )
}

/// The names of a `thread.commands` answer.
fn command_names(answer: &Value) -> Vec<&str> {
    answer["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["name"].as_str().unwrap())
        .collect()
}

// spec: DMN-358, DMN-070
#[tokio::test]
async fn a_stopped_thread_answers_its_last_commands_without_starting_omp() {
    let h = harness(|_| {});
    let id = h.thread();

    let none = h.supervisor.commands(&id).await.unwrap();
    assert_eq!(none, json!({ "commands": [] }));
    assert_eq!(h.launcher.launches().len(), 0, "asking starts nothing");

    h.supervisor.prompt(&id, prompt("hi")).await.unwrap();
    assert_eq!(h.launcher.launches().len(), 1, "a prompt starts omp");
    h.launcher
        .process(0)
        .emit(vec![commands_update(&["zebra", "zest"])]);
    h.wait(&id, "the commands", |state| state.commands.len() == 2)
        .await;
    h.supervisor.stop(&id).await.unwrap();

    let known = h.supervisor.commands(&id).await.unwrap();
    assert_eq!(command_names(&known), ["zebra", "zest"]);
    assert_eq!(h.launcher.launches().len(), 1, "no start for the answer");

    // Another daemon on the same data: the list the thread's omp last announced was kept with the thread.
    let launcher = FakeLauncher::new();
    let restarted = Supervisor::new(
        Arc::new(launcher.clone()),
        Arc::clone(&h.threads),
        Arc::clone(&h.workspaces),
        h.data.clone(),
        SupervisorConfig::new("omp"),
    );
    let kept = restarted.commands(&id).await.unwrap();
    assert_eq!(command_names(&kept), ["zebra", "zest"]);
    assert_eq!(launcher.launches().len(), 0);
}

// spec: DMN-359
#[tokio::test]
async fn a_shown_stopped_thread_reads_its_history_from_the_session_file_without_omp() {
    let h = harness(|_| {});
    let id = h.thread();
    saved_session(&h, &id, "S-old", "anthropic/claude-opus-5-5");

    h.supervisor.read_history(&id).await.unwrap();

    assert_eq!(
        transcript(&h, &id).await,
        ["user: old question", "assistant: old answer"]
    );
    assert_eq!(h.launcher.launches().len(), 0, "no omp for it");
    let (process, session) = h
        .log(&id)
        .await
        .with_state(|state| (state.process.clone(), state.session.clone()));
    assert_eq!(process, ProcessState::Stopped);
    assert_eq!(
        session.map(|session| session.session_id).as_deref(),
        Some("S-old")
    );
    assert_eq!(
        h.supervisor.session_model(&id),
        None,
        "only a failed start reports the saved model"
    );

    // Once omp gave the thread its history, showing it again reads no file over it.
    h.supervisor.start(&id).await.unwrap();
    h.supervisor.stop(&id).await.unwrap();
    h.supervisor.read_history(&id).await.unwrap();
    assert_eq!(transcript(&h, &id).await, Vec::<String>::new(), "omp's");
    assert_eq!(h.launcher.launches().len(), 1);
}

// spec: DMN-038
#[tokio::test]
async fn starting_beyond_the_process_limit_fails_explicitly() {
    let h = harness(|config| config.max_processes = 1);
    let (a, b) = (h.thread(), h.thread());
    h.supervisor.start(&a).await.unwrap();

    let error = h.supervisor.start(&b).await.unwrap_err();
    assert!(matches!(error, SupervisorError::LimitReached(1)), "{error}");

    h.supervisor.stop(&a).await.unwrap();
    h.supervisor.start(&b).await.unwrap();
    assert!(h.supervisor.is_running(&b));
}

// spec: DMN-035, DMN-032
#[tokio::test]
async fn a_crash_fails_the_thread_and_the_next_prompt_restarts_it_with_history() {
    let h = harness(|_| {});
    h.launcher.push_run(fixture("approval_granted"));
    h.launcher.push_run(fixture("restart_with_history"));
    let id = h.thread();
    h.supervisor.prompt(&id, prompt("run it")).await.unwrap();
    h.wait(&id, "the approval request", |state| {
        !state.interactions.is_empty()
    })
    .await;
    std::fs::write(h.data.thread_log(&id), "booting\nfatal: out of memory\n").unwrap();

    h.launcher.process(0).exit();
    h.wait(&id, "the failure", |state| {
        matches!(state.process, ProcessState::Failed { .. })
    })
    .await;

    let state = h.log(&id).await.with_state(Clone::clone);
    let ProcessState::Failed { stderr_tail, .. } = &state.process else {
        unreachable!()
    };
    assert_eq!(stderr_tail, "booting\nfatal: out of memory");
    assert_eq!(state.interactions, Vec::new());
    assert!(!h.supervisor.is_running(&id));
    assert_eq!(h.launcher.launches().len(), 1, "no automatic restart");

    h.supervisor
        .prompt(&id, prompt("second question"))
        .await
        .unwrap();
    h.wait(&id, "the restarted turn", |state| {
        state.process == ProcessState::Running && state.session.as_ref().is_some_and(|s| s.resumed)
    })
    .await;
    assert_eq!(h.launcher.launches().len(), 2);
    assert!(
        h.log(&id)
            .await
            .with_state(|state| state.transcript.len() >= 2)
    );
}

// spec: DMN-037, ARC-005
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_commands_on_a_stopped_thread_start_a_single_process() {
    let h = harness(|_| {});
    let id = h.thread();

    let calls = (0..8).map(|_| h.supervisor.call(&id, OmpCall::State));
    let results = futures::future::join_all(calls).await;

    assert!(results.iter().all(Result::is_ok));
    assert_eq!(h.launcher.launches().len(), 1);
}

// spec: DMN-037
#[tokio::test(flavor = "multi_thread")]
async fn omp_commands_do_not_wait_for_each_other() {
    let h = harness(|_| {});
    let id = h.thread();
    let gate = h.launcher.gate("compact");
    let supervisor = h.supervisor.clone();
    let compacting = {
        let id = id.clone();
        tokio::spawn(async move {
            supervisor
                .call(
                    &id,
                    OmpCall::Compact(omp_rpc::CompactCommand {
                        custom_instructions: None,
                    }),
                )
                .await
        })
    };
    gate.arrived(1).await;

    // `abort` overtakes the running `compact` (omp orders them itself).
    h.supervisor
        .call(&id, OmpCall::AbortAndRestoreQueue)
        .await
        .unwrap();
    assert!(!compacting.is_finished());

    gate.open();
    compacting.await.unwrap().unwrap();
}

// spec: DMN-071
#[tokio::test]
async fn omp_errors_keep_their_code() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.reply(
        "navigate_tree",
        FakeReply::Fail {
            message: "entry not found".into(),
            code: Some("entry_not_found".into()),
        },
    );

    let error = h
        .supervisor
        .call(
            &id,
            OmpCall::NavigateTree(omp_rpc::NavigateTreeCommand {
                entry_id: "nope".into(),
                summarize: None,
                custom_instructions: None,
            }),
        )
        .await
        .unwrap_err();

    assert!(
        matches!(&error, SupervisorError::Omp { code: Some(code), .. } if code == "entry_not_found"),
        "{error:?}"
    );
}

fn call_names(h: &Harness) -> Vec<&'static str> {
    h.launcher
        .process(0)
        .calls()
        .iter()
        .filter_map(|call| match call {
            FakeCall::Call(call) => Some(call.name()),
            _ => None,
        })
        .collect()
}

fn navigate(entry_id: &str) -> OmpCall {
    OmpCall::NavigateTree(omp_rpc::NavigateTreeCommand {
        entry_id: entry_id.into(),
        summarize: None,
        custom_instructions: None,
    })
}

// spec: DMN-072, DMN-111
#[tokio::test]
async fn a_fork_stores_the_new_session_id_and_reloads_its_transcript() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.reply(
        "get_state",
        FakeReply::Ok(json!({ "sessionId": "S-forked", "sessionFile": "/s/forked.jsonl" })),
    );
    h.launcher.reply(
        "get_messages",
        FakeReply::Ok(
            json!({ "messages": [{ "role": "user", "content": "kept", "timestamp": 0 }] }),
        ),
    );

    h.supervisor
        .call(&id, OmpCall::Fork(omp_rpc::ForkCommand { entry_id: None }))
        .await
        .unwrap();

    assert_eq!(
        h.threads.get(&id).unwrap().omp_session_id.as_deref(),
        Some("S-forked")
    );
    assert_eq!(call_names(&h), ["fork", "get_state", "get_messages"]);
    let state = h.log(&id).await.with_state(Clone::clone);
    let session = state.session.unwrap();
    assert_eq!(
        (session.session_id.as_str(), session.session_file.as_deref()),
        ("S-forked", Some("/s/forked.jsonl"))
    );
    assert_eq!(state.transcript.len(), 1);
}

// spec: DMN-301
#[tokio::test]
async fn a_forked_thread_starts_once_on_a_copy_of_the_source_session_then_continues_its_own() {
    let h = harness(|_| {});
    let id = h.thread();
    let source = h.dir.path().join("source.jsonl");

    h.supervisor
        .start_fork(&id, source.clone(), None)
        .await
        .unwrap();
    assert_eq!(h.launcher.launches()[0].spec.fork, Some(source));
    // The whole session: no command moves the copy.
    assert_eq!(call_names(&h), Vec::<&str>::new());
    assert!(h.threads.get(&id).unwrap().omp_session_id.is_some());

    h.supervisor.stop(&id).await.unwrap();
    h.supervisor.start(&id).await.unwrap();
    assert_eq!(h.launcher.launches()[1].spec.fork, None);
}

// spec: DMN-302
#[tokio::test]
async fn a_fork_at_an_entry_moves_the_copy_onto_its_branch_and_deletes_the_full_copy() {
    let h = harness(|_| {});
    let id = h.thread();
    let run = FakeRun::fresh();
    let copied = run.startup().session.session_id.clone();
    h.launcher.push_run(run);
    h.launcher.reply(
        "get_state",
        FakeReply::Ok(json!({ "sessionId": "S-branch", "sessionFile": "/s/branch.jsonl" })),
    );
    let dir = h.threads.get(&id).unwrap().session_dir;
    let copy = dir.join(format!("2026-11-14T10-00-00-000Z_{copied}.jsonl"));
    let branch = dir.join("2026-11-14T10-00-01-000Z_S-branch.jsonl");
    std::fs::write(&copy, "{}\n").unwrap();
    std::fs::write(&branch, "{}\n").unwrap();

    h.supervisor
        .start_fork(&id, h.dir.path().join("source.jsonl"), Some("u1".into()))
        .await
        .unwrap();

    assert_eq!(call_names(&h), ["fork", "get_state", "get_messages"]);
    let forked_at = h
        .launcher
        .process(0)
        .calls()
        .into_iter()
        .find_map(|call| match call {
            FakeCall::Call(OmpCall::Fork(fork)) => fork.entry_id,
            _ => None,
        });
    assert_eq!(forked_at.as_deref(), Some("u1"));
    assert_eq!(
        h.threads.get(&id).unwrap().omp_session_id.as_deref(),
        Some("S-branch")
    );
    assert!(!copy.exists() && branch.exists());
}

// spec: DMN-303
#[tokio::test]
async fn a_fork_omp_refuses_fails_the_start_of_the_fork() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.reply(
        "fork",
        FakeReply::Fail {
            message: "Invalid entry ID for forking: x".into(),
            code: None,
        },
    );
    let error = h
        .supervisor
        .start_fork(&id, h.dir.path().join("source.jsonl"), Some("x".into()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Invalid entry ID"), "{error}");

    let id = h.thread();
    h.launcher
        .reply("fork", FakeReply::Ok(json!({ "cancelled": true })));
    let error = h
        .supervisor
        .start_fork(&id, h.dir.path().join("source.jsonl"), Some("u1".into()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error}");
}

// spec: DMN-111
#[tokio::test]
async fn a_navigation_that_moved_reloads_the_transcript_and_a_cancelled_one_does_not() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.reply(
        "navigate_tree",
        FakeReply::Ok(json!({ "cancelled": false, "editorText": "again" })),
    );
    h.launcher.reply(
        "get_messages",
        FakeReply::Ok(
            json!({ "messages": [{ "role": "user", "content": "before", "timestamp": 0 }] }),
        ),
    );

    let result = h.supervisor.call(&id, navigate("e2")).await.unwrap();

    assert_eq!(result["editorText"], "again");
    assert_eq!(
        call_names(&h),
        ["navigate_tree", "get_state", "get_messages"]
    );
    let transcript = h.log(&id).await.with_state(|state| state.transcript.len());
    assert_eq!(transcript, 1, "the branch the leaf moved to");

    h.launcher
        .reply("navigate_tree", FakeReply::Ok(json!({ "cancelled": true })));
    h.supervisor.call(&id, navigate("e1")).await.unwrap();
    assert_eq!(
        call_names(&h),
        [
            "navigate_tree",
            "get_state",
            "get_messages",
            "navigate_tree"
        ]
    );
}

// spec: DMN-051, DMN-050
#[tokio::test]
async fn the_first_valid_answer_wins_and_later_ones_are_already_resolved() {
    let h = harness(|_| {});
    h.launcher.push_run(fixture("approval_granted"));
    let id = h.thread();
    h.supervisor.prompt(&id, prompt("run it")).await.unwrap();
    h.wait(&id, "the approval request", |state| {
        !state.interactions.is_empty()
    })
    .await;
    let request_id = h
        .log(&id)
        .await
        .with_state(|state| dialog_id(&state.interactions[0]).unwrap().to_owned());

    let invalid = h
        .supervisor
        .respond(
            &id,
            &request_id,
            InteractionResponse::Value("Maybe".into()),
            "web",
        )
        .await
        .unwrap_err();
    assert!(
        matches!(invalid, SupervisorError::InvalidResponse(_)),
        "{invalid}"
    );
    let wrong_kind = h
        .supervisor
        .respond(
            &id,
            &request_id,
            InteractionResponse::Confirmed(true),
            "web",
        )
        .await
        .unwrap_err();
    assert!(matches!(wrong_kind, SupervisorError::InvalidResponse(_)));

    h.supervisor
        .respond(
            &id,
            &request_id,
            InteractionResponse::Value("Approve".into()),
            "web",
        )
        .await
        .unwrap();
    let late = h
        .supervisor
        .respond(&id, &request_id, InteractionResponse::Cancelled, "desktop")
        .await
        .unwrap_err();

    assert!(
        matches!(late, SupervisorError::AlreadyResolved(_)),
        "{late}"
    );
    h.wait(&id, "the end of the turn", settled_after_prompt("req_7"))
        .await;
    let responses: Vec<FakeCall> = h
        .launcher
        .process(0)
        .calls()
        .into_iter()
        .filter(|call| matches!(call, FakeCall::Respond(_)))
        .collect();
    assert_eq!(responses.len(), 1);
}

/// Workspace of `thread`.
fn workspace_of(h: &Harness, thread: &str) -> (String, PathBuf) {
    let ws = h.threads.get(thread).unwrap().workspace_id;
    let path = h.workspaces.get(&ws).unwrap().path;
    (ws, path)
}

// spec: DMN-029, ARC-008
#[tokio::test]
async fn root_sync_adds_new_repositories_and_removes_gone_ones() {
    let h = harness(|_| {});
    let id = h.thread();
    let (ws, folder) = workspace_of(&h, &id);
    h.supervisor.start(&id).await.unwrap();
    let link = folder.join("app");
    std::os::unix::fs::symlink(h.external_repo("app"), &link).unwrap();

    h.workspaces.rescan(&ws).unwrap();
    h.supervisor.sync_roots(&id).await.unwrap();
    assert_eq!(h.threads.roots(&id).unwrap(), std::slice::from_ref(&link));
    std::fs::remove_file(&link).unwrap();
    h.workspaces.rescan(&ws).unwrap();
    h.supervisor.sync_roots(&id).await.unwrap();
    h.supervisor.sync_roots(&id).await.unwrap();

    assert_eq!(
        h.local_commands(),
        [
            format!("/add-dir {}", link.display()),
            format!("/remove-dir {}", link.display())
        ],
        "nothing more once in sync"
    );
    assert_eq!(h.threads.roots(&id).unwrap(), Vec::<PathBuf>::new());
}

// spec: DMN-029, DMN-200
#[tokio::test]
async fn a_root_sync_is_neither_in_the_transcript_nor_activity_and_the_users_add_dir_shows() {
    let h = harness(|_| {});
    let first = h.thread();
    let (ws, folder) = workspace_of(&h, &first);
    let second = h
        .threads
        .create(NewThread::new("t", ws.clone()))
        .unwrap()
        .id;
    let ids = [first, second];
    for id in &ids {
        h.supervisor.start(id).await.unwrap();
    }
    let link = folder.join("app");
    std::os::unix::fs::symlink(h.external_repo("app"), &link).unwrap();
    h.workspaces.rescan(&ws).unwrap();
    let mut changes = h.supervisor.subscribe_changes();
    let mut before = Vec::new();
    for id in &ids {
        let actor = h.supervisor.lookup(id).unwrap();
        before.push((h.log(id).await.position().seq, *lock(&actor.last_activity)));
    }

    for id in &ids {
        h.supervisor.sync_roots(id).await.unwrap();
    }

    for (index, id) in ids.iter().enumerate() {
        assert_eq!(
            h.local_commands_of(index),
            [format!("/add-dir {}", link.display())]
        );
        let actor = h.supervisor.lookup(id).unwrap();
        let after = (h.log(id).await.position().seq, *lock(&actor.last_activity));
        assert_eq!(after, before[index], "no event and no activity");
    }
    assert!(
        matches!(
            changes.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ),
        "nothing for the index: the thread's updatedAt stays"
    );
    let elsewhere = h.external_repo("elsewhere");
    let typed = format!("/add-dir {}", elsewhere.display());
    h.supervisor.prompt(&ids[1], prompt(&typed)).await.unwrap();
    h.wait(&ids[1], "the output of /add-dir", |state| {
        !state.notices.is_empty()
    })
    .await;
    let log = h.log(&ids[1]).await;
    assert_eq!(
        log.with_state(command_outputs),
        [format!("Added {}.", elsewhere.display())],
        "the root sync's output never lands, the user's does"
    );
    assert_eq!(
        h.log(&ids[0]).await.with_state(command_outputs),
        Vec::<String>::new()
    );
}

// spec: DMN-029, DMN-202
#[tokio::test]
async fn another_commands_output_during_a_root_sync_reaches_the_log_and_its_reload() {
    let h = harness(|_| {});
    let id = h.thread();
    let (ws, folder) = workspace_of(&h, &id);
    h.supervisor.start(&id).await.unwrap();
    let link = folder.join("app");
    std::os::unix::fs::symlink(h.external_repo("app"), &link).unwrap();
    h.workspaces.rescan(&ws).unwrap();
    let reauthorized = "✓ Reauthorized \"gh\" (user config). Credentials apply on the next connection; restart the \
                        session if the server stays disconnected.";
    let added = format!(
        "Added {}.\nWorkspace directories:\n  {} (working directory)\n  {}",
        link.display(),
        folder.display(),
        link.display()
    );
    // A background `/mcp reauth` ends while the root command is in flight.
    h.launcher.local_output(
        &format!("/add-dir {}", link.display()),
        &[reauthorized, &added],
    );

    h.supervisor.sync_roots(&id).await.unwrap();
    h.launched(&id, 2).await;

    let log = h.log(&id).await;
    assert_eq!(log.with_state(command_outputs), [reauthorized]);
}

// spec: DMN-029
#[tokio::test]
async fn a_busy_thread_syncs_its_roots_once_its_session_settles() {
    let h = harness(|_| {});
    let id = h.thread();
    let (ws, folder) = workspace_of(&h, &id);
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![agent_start()]);
    h.wait(&id, "the busy agent", |state| !state.settled).await;
    let link = folder.join("app");
    std::os::unix::fs::symlink(h.external_repo("app"), &link).unwrap();
    h.workspaces.rescan(&ws).unwrap();

    h.supervisor.sync_roots(&id).await.unwrap();
    assert_eq!(
        h.local_commands(),
        Vec::<String>::new(),
        "omp refuses /add-dir while streaming"
    );
    let settled: RpcNotification =
        serde_json::from_value(json!({ "type": "session_settled" })).unwrap();
    h.launcher.process(0).emit(vec![settled]);
    while h.threads.roots(&id).unwrap().is_empty() {
        tokio::task::yield_now().await;
    }

    assert_eq!(h.local_commands(), [format!("/add-dir {}", link.display())]);
}

// spec: DMN-029
#[tokio::test]
async fn roots_the_session_kept_from_before_are_removed_at_launch() {
    let h = harness(|_| {});
    let id = h.thread();
    let old = h.external_repo("old-worktree");
    h.threads
        .set_roots(&id, std::slice::from_ref(&old))
        .unwrap();

    h.supervisor.start(&id).await.unwrap();

    assert_eq!(
        h.local_commands(),
        [format!("/remove-dir {}", old.display())]
    );
    assert_eq!(h.threads.roots(&id).unwrap(), Vec::<PathBuf>::new());
}

// spec: DMN-023
#[tokio::test]
async fn a_finished_tool_run_is_announced_for_a_rescan() {
    let h = harness(|_| {});
    let id = h.thread();
    let mut tool_ends = h.supervisor.subscribe_tool_ends();
    h.supervisor.start(&id).await.unwrap();
    let end = fixture_text("bash_tool_call")
        .lines()
        .map(|line| serde_json::from_str::<ThreadEvent>(line).unwrap())
        .find_map(|event| match event {
            ThreadEvent::Omp {
                frame: frame @ RpcNotification::RpcAgentEvent(RpcAgentEvent::ToolExecutionEnd(_)),
            } => Some(frame),
            _ => None,
        })
        .unwrap();

    h.launcher.process(0).emit(vec![end]);

    assert_eq!(tool_ends.recv().await.unwrap(), id);
}

fn agent_start() -> RpcNotification {
    let frame = json!({ "type": "agent_start" });
    let event: AgentStartEvent = serde_json::from_value(frame).unwrap();
    RpcNotification::RpcAgentEvent(RpcAgentEvent::AgentStart(event))
}

// spec: DMN-034, PRT-025
#[tokio::test(start_paused = true)]
async fn the_reaper_stops_only_idle_settled_unwatched_threads() {
    let timeout = Duration::from_mins(1);
    let h = harness(|config| config.idle_timeout = timeout);
    let (idle, watched, busy) = (h.thread(), h.thread(), h.thread());
    for id in [&idle, &watched, &busy] {
        h.supervisor.start(id).await.unwrap();
    }
    let guard = h.supervisor.watch(&watched, 1, true).await.unwrap();
    assert_eq!(h.supervisor.watchers(&watched), 1);
    h.launcher.process(2).emit(vec![agent_start()]);
    h.wait(&busy, "the turn start", |state| !state.settled)
        .await;
    // Paused time jumps to the next timer whenever the runtime waits (here, on the event pump's thread): the reaper
    // starts once the setup is done, so no virtual time passes before it.
    let reaper = h.supervisor.spawn_reaper();

    tokio::time::sleep(timeout / 2).await;
    assert!(h.supervisor.is_running(&idle), "stopped before the timeout");

    h.wait(&idle, "the idle stop", |state| {
        state.process == ProcessState::Stopped
    })
    .await;
    tokio::time::sleep(timeout * 10).await;
    assert!(
        h.supervisor.is_running(&watched),
        "a watched thread is never stopped"
    );
    assert!(
        h.supervisor.is_running(&busy),
        "a busy thread is never stopped"
    );
    assert!(h.launcher.process(0).is_closed());

    drop(guard);
    assert_eq!(h.supervisor.watchers(&watched), 0);
    h.wait(&watched, "the stop once unwatched", |state| {
        state.process == ProcessState::Stopped
    })
    .await;
    reaper.abort();
}

// spec: DMN-356, DMN-034, PRT-222, PRT-223
#[tokio::test(start_paused = true)]
async fn the_reaper_stops_an_idle_thread_a_window_only_keeps_and_not_one_watched_again() {
    let timeout = Duration::from_mins(1);
    let h = harness(|config| config.idle_timeout = timeout);
    let (kept, shown) = (h.thread(), h.thread());
    for id in [&kept, &shown] {
        h.supervisor.start(id).await.unwrap();
    }
    let _kept = h.supervisor.watch(&kept, 1, false).await.unwrap();
    let shown_again = h.supervisor.watch(&shown, 1, false).await.unwrap();
    shown_again.set_watching(true);
    assert_eq!(
        (h.supervisor.watchers(&kept), h.supervisor.watchers(&shown)),
        (0, 1)
    );

    // One reaper pass once both threads are idle for the timeout: the clock and the pass are driven by the test.
    tokio::time::advance(timeout).await;
    h.supervisor.reap_idle().await;
    h.wait(&kept, "the idle stop of the kept thread", |state| {
        state.process == ProcessState::Stopped
    })
    .await;
    assert!(
        h.supervisor.is_running(&shown),
        "a thread watched again is not stopped"
    );
    assert_eq!(
        h.supervisor.status(&kept).process,
        ProcessState::Stopped,
        "nothing restarts a kept thread until it is used"
    );

    shown_again.set_watching(false);
    h.supervisor.reap_idle().await;
    assert!(
        h.supervisor.is_running(&shown),
        "the idle timeout counts from the moment it stopped watching"
    );
    tokio::time::advance(timeout).await;
    h.supervisor.reap_idle().await;
    h.wait(&shown, "the stop once only kept", |state| {
        state.process == ProcessState::Stopped
    })
    .await;
}

/// The epoch of the thread's log: a new one once the daemon forgot the thread's conversation and loaded it again.
async fn epoch(h: &Harness, id: &str) -> String {
    h.log(id).await.position().epoch
}

// spec: DMN-365, DMN-359, DMN-034
#[tokio::test(start_paused = true)]
async fn the_reaper_releases_the_conversation_of_threads_without_omp_nobody_holds() {
    let timeout = Duration::from_mins(1);
    let h = harness(|config| config.idle_timeout = timeout);
    let (shown, kept, failed, ran) = (h.thread(), h.thread(), h.thread(), h.thread());
    for id in [&shown, &kept, &failed] {
        saved_session(&h, id, &format!("S-{id}"), "anthropic/claude-opus-5-5");
        h.supervisor.read_history(id).await.unwrap();
    }
    let _kept = h.supervisor.watch(&kept, 1, false).await.unwrap();
    h.launcher
        .fail_next_launch("SessionModelRestoreError: Could not restore model");
    h.supervisor.start(&failed).await.unwrap_err();
    h.supervisor.start(&ran).await.unwrap();
    let mut epochs = Vec::new();
    for id in [&shown, &kept, &failed, &ran] {
        epochs.push(epoch(&h, id).await);
    }

    h.supervisor.reap_idle().await;
    assert_eq!(
        epoch(&h, &shown).await,
        epochs[0],
        "kept until unused for the idle timeout"
    );

    tokio::time::advance(timeout).await;
    h.supervisor.reap_idle().await;
    h.wait(&ran, "the idle stop", |state| {
        state.process == ProcessState::Stopped
    })
    .await;
    tokio::time::advance(timeout).await;
    h.supervisor.reap_idle().await;

    assert_ne!(epoch(&h, &shown).await, epochs[0], "forgotten");
    assert_eq!(transcript(&h, &shown).await, Vec::<String>::new());
    assert_eq!(
        h.supervisor.status(&shown),
        ThreadStatus::default(),
        "the thread list shows it as before"
    );
    h.supervisor.read_history(&shown).await.unwrap();
    assert_eq!(
        transcript(&h, &shown).await,
        ["user: old question", "assistant: old answer"],
        "showing it again reads its session file"
    );
    assert_ne!(
        epoch(&h, &ran).await,
        epochs[3],
        "a stopped omp's thread is forgotten too"
    );
    assert_eq!(epoch(&h, &kept).await, epochs[1], "a window keeps it");
    assert_eq!(
        transcript(&h, &kept).await,
        ["user: old question", "assistant: old answer"]
    );
    assert_eq!(
        epoch(&h, &failed).await,
        epochs[2],
        "a failed start stays shown until it is resolved"
    );
    assert_eq!(
        h.launcher.launches().len(),
        1,
        "only `ran`'s: forgetting and reading again starts no omp"
    );
}

// spec: DMN-365
#[tokio::test(start_paused = true)]
async fn the_reaper_keeps_a_thread_whose_log_is_still_read() {
    let timeout = Duration::from_mins(1);
    let h = harness(|config| config.idle_timeout = timeout);
    let id = h.thread();
    let log = h.log(&id).await;
    let before = log.position().epoch;

    tokio::time::advance(timeout).await;
    h.supervisor.reap_idle().await;

    assert_eq!(
        epoch(&h, &id).await,
        before,
        "a reader of the log (a wait, a job) keeps the thread"
    );
    drop(log);
}

// spec: DMN-005
#[tokio::test]
async fn shutdown_stops_every_process_and_refuses_new_ones() {
    let h = harness(|_| {});
    let (a, b) = (h.thread(), h.thread());
    h.supervisor.start(&a).await.unwrap();
    h.supervisor.start(&b).await.unwrap();

    h.supervisor.shutdown().await;

    for (index, id) in [&a, &b].into_iter().enumerate() {
        assert!(h.launcher.process(index).is_closed());
        let process = h.log(id).await.with_state(|state| state.process.clone());
        assert_eq!(process, ProcessState::Stopped);
    }
    assert!(matches!(
        h.supervisor.start(&a).await,
        Err(SupervisorError::ShuttingDown)
    ));
}

/// Polls `operation` until the fake holds the process close, then drops it mid-way, as a host tool's call is dropped
/// when the agent stopping its own thread loses its omp process.
async fn drop_at_close(close: &crate::agent::Gate, operation: impl Future) {
    tokio::select! {
        _ = operation => panic!("the operation ended while the close was held"),
        () = close.arrived(1) => {}
    }
}

// spec: DMN-038, DMN-070, AGT-071
#[tokio::test]
async fn a_stop_dropped_mid_way_still_stops_the_thread_and_frees_its_process_slot() {
    let h = harness(|config| config.max_processes = 1);
    let (a, b) = (h.thread(), h.thread());
    h.supervisor.start(&a).await.unwrap();
    let close = h.launcher.gate_closes();

    drop_at_close(&close, h.supervisor.stop(&a)).await;
    close.open();

    h.wait(&a, "the stop", |state| {
        state.process == ProcessState::Stopped
    })
    .await;
    assert!(h.launcher.process(0).is_closed());
    assert!(!h.supervisor.is_running(&a));
    h.supervisor.start(&b).await.unwrap();
}

// spec: DMN-038, DMN-070
#[tokio::test]
async fn a_restart_dropped_mid_way_still_starts_the_new_process() {
    let h = harness(|config| config.max_processes = 1);
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    let close = h.launcher.gate_closes();

    drop_at_close(&close, h.supervisor.restart(&id, Restart::User)).await;
    close.open();

    // The lifecycle lock orders this start after the restart: a restart cut short would leave no process and its slot
    // taken, so this start would hit the limit.
    h.supervisor.start(&id).await.unwrap();
    assert_eq!(h.launcher.launches().len(), 2);
    assert!(h.launcher.process(0).is_closed());
    assert!(!h.launcher.process(1).is_closed());
}

// spec: DMN-039
#[tokio::test]
async fn a_start_asked_while_the_thread_is_deleted_leaves_no_process_and_no_directory() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    let close = h.launcher.gate_closes();
    let deletion = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.delete(&id).await }
    });
    close.arrived(1).await;
    // The records go before the stop: every start from now on finds no thread.
    assert!(matches!(
        h.threads.get(&id),
        Err(ThreadError::Store(StoreError::NotFound { .. }))
    ));

    // A window opening the thread asks omp for its commands while the deletion stops omp: the start waits for the
    // lifecycle lock the stop holds, and a launch after it would recreate the directory.
    let start = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.start(&id).await }
    });
    tokio::task::yield_now().await;
    close.open();

    deletion.await.unwrap().unwrap();
    assert!(matches!(
        start.await.unwrap(),
        Err(SupervisorError::Thread(ThreadError::Store(
            StoreError::NotFound { .. }
        )))
    ));
    assert_eq!(h.launcher.launches().len(), 1);
    assert!(h.launcher.process(0).is_closed());
    assert!(!h.supervisor.is_running(&id));
    assert!(!h.data.thread(&id).exists());
    assert!(matches!(
        h.supervisor.start(&id).await,
        Err(SupervisorError::Thread(_))
    ));
}

// spec: DMN-039
#[tokio::test]
async fn a_thread_deleted_while_its_omp_starts_has_that_omp_stopped_before_the_deletion_returns() {
    let h = harness(|_| {});
    let id = h.thread();
    let launches = h.launcher.gate_launches();
    let start = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.start(&id).await }
    });
    launches.arrived(1).await;

    let deletion = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.delete(&id).await }
    });
    tokio::task::yield_now().await;
    launches.open();

    deletion.await.unwrap().unwrap();
    let _ = start.await.unwrap();
    assert_eq!(h.launcher.launches().len(), 1);
    assert!(h.launcher.process(0).is_closed());
    assert!(!h.supervisor.is_running(&id));
    assert!(!h.data.thread(&id).exists());
}

// spec: DMN-028, DMN-039
#[tokio::test]
async fn a_workspace_is_not_removed_while_a_deleted_threads_omp_still_starts_in_it() {
    let h = harness(|_| {});
    let id = h.thread();
    let ws = h.threads.get(&id).unwrap().workspace_id;
    let launches = h.launcher.gate_launches();
    let start = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.start(&id).await }
    });
    launches.arrived(1).await;
    let deletion = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.delete(&id).await }
    });
    while h.threads.get(&id).is_ok() {
        tokio::task::yield_now().await;
    }

    // The thread's record is gone, but its omp still starts in the folder, which the launch creates again.
    let removal = h.workspaces.check_removal(&ws, false);
    launches.open();
    assert!(
        matches!(removal, Err(WorkspaceError::HasThreads(_))),
        "{removal:?}"
    );
    deletion.await.unwrap().unwrap();
    let _ = start.await.unwrap();
    assert!(h.launcher.process(0).is_closed());
    h.workspaces.remove(&ws, false).unwrap();
}

#[test]
fn the_stderr_tail_keeps_the_last_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("omp.log");
    let text = (1..=30)
        .map(|line| format!("line {line}\n"))
        .collect::<Vec<_>>()
        .concat();
    std::fs::write(&path, text).unwrap();

    let tail = stderr_tail(&path, 3);

    assert_eq!(tail, "line 28\nline 29\nline 30");
    assert_eq!(stderr_tail(&dir.path().join("missing"), 3), "");
}

impl Harness {
    /// A thread without a title in a new workspace: it gets one from its first message (DMN-073).
    fn untitled_thread(&self) -> (String, PathBuf) {
        let (ws, path) = self.workspace();
        let new = NewThread {
            title_source: TitleSource::Placeholder,
            ..NewThread::new("", ws)
        };
        (self.threads.create(new).unwrap().id, path)
    }

    fn title(&self, id: &str) -> (String, TitleSource) {
        let thread = self.threads.get(id).unwrap();
        (thread.title, thread.title_source)
    }

    /// The sessions names the thread's first process got.
    fn session_names(&self) -> Vec<String> {
        self.launcher
            .process(0)
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                FakeCall::Call(OmpCall::SetSessionName(command)) => Some(command.name),
                _ => None,
            })
            .collect()
    }
}

// spec: DMN-073
#[tokio::test]
async fn the_first_message_gives_a_placeholder_then_a_generated_title_and_session_name() {
    let h = harness(|_| {});
    h.launcher.push_run(fixture("text_reply"));
    h.launcher.push_title(Ok("Greeting the room"));
    // Held until the placeholder is checked: a fast title run would otherwise replace it first.
    let gate = h.launcher.gate_titles();
    let (id, _) = h.untitled_thread();

    h.supervisor
        .prompt(&id, prompt("say hi to everyone in the room please"))
        .await
        .unwrap();
    assert_eq!(
        h.title(&id),
        (
            "say hi to everyone in the…".into(),
            TitleSource::Placeholder
        )
    );
    gate.open();
    h.supervisor.title_generated(&id).await;

    assert_eq!(
        h.title(&id),
        ("Greeting the room".into(), TitleSource::Generated)
    );
    assert_eq!(h.session_names(), ["Greeting the room"]);
    // Later messages keep the title.
    h.supervisor.prompt(&id, prompt("again")).await.unwrap();
    assert_eq!(h.launcher.title_requests().len(), 1);
}

// spec: DMN-073
#[tokio::test]
async fn a_title_set_while_the_title_is_generated_is_kept() {
    let h = harness(|_| {});
    h.launcher.push_run(fixture("text_reply"));
    h.launcher.push_title(Ok("Generated"));
    let gate = h.launcher.gate_titles();
    let (id, _) = h.untitled_thread();
    h.supervisor.prompt(&id, prompt("say hi")).await.unwrap();
    gate.arrived(1).await;

    h.supervisor
        .set_title(&id, "Mine".into(), TitleSource::Set)
        .await
        .unwrap();
    gate.open();
    h.supervisor.title_generated(&id).await;

    assert_eq!(h.title(&id), ("Mine".into(), TitleSource::Set));
    assert_eq!(h.session_names(), ["Mine"]);
}

// spec: DMN-073
#[tokio::test]
async fn a_set_title_is_never_generated_and_a_failed_generation_keeps_the_placeholder() {
    let h = harness(|_| {});
    let titled = h.thread();
    h.supervisor
        .prompt(&titled, prompt("say hi"))
        .await
        .unwrap();
    assert_eq!(h.title(&titled), ("t".into(), TitleSource::Set));
    assert_eq!(h.launcher.title_requests(), Vec::new());

    h.launcher.push_title(Err("no model for the smol role"));
    let (untitled, _) = h.untitled_thread();
    h.supervisor
        .prompt(&untitled, prompt("fix the build"))
        .await
        .unwrap();
    h.supervisor.title_generated(&untitled).await;
    assert_eq!(
        h.title(&untitled),
        ("fix the build".into(), TitleSource::Placeholder)
    );
}

// spec: DMN-073
#[tokio::test]
async fn without_title_generation_the_placeholder_stays() {
    let h = harness(|config| config.title_generation = false);
    let (id, _) = h.untitled_thread();

    h.supervisor
        .prompt(&id, prompt("fix the build"))
        .await
        .unwrap();
    h.supervisor.title_generated(&id).await;

    assert_eq!(
        h.title(&id),
        ("fix the build".into(), TitleSource::Placeholder)
    );
    assert_eq!(h.launcher.title_requests(), Vec::new());
}

// spec: DMN-912
#[tokio::test]
async fn a_thread_started_by_voice_is_named_by_the_users_first_final_line() {
    let h = harness(|_| {});
    h.launcher.push_title(Ok("Checking the line"));
    h.launcher.script_live(crate::agent::FakeLive::connect(
        crate::agent::FakeLive::call_frames(),
    ));
    let (untitled, _) = h.untitled_thread();
    let titled = h.thread();
    let finals = |state: &ThreadState| {
        state.live.as_ref().is_some_and(|live| {
            live.transcript.len() == 2 && live.transcript.iter().all(|line| line.r#final)
        })
    };

    for id in [&untitled, &titled] {
        h.supervisor
            .live_connect(id, owner(1), OFFER.into(), None)
            .await
            .unwrap();
        h.wait(id, "the call's final transcript", finals).await;
        h.supervisor.title_generated(id).await;
    }

    assert_eq!(
        h.title(&untitled),
        ("Checking the line".into(), TitleSource::Generated)
    );
    assert_eq!(h.title(&titled), ("t".into(), TitleSource::Set));
    // Neither the partials nor the assistant's lines count; a named thread asks for nothing.
    let asked: Vec<String> = h
        .launcher
        .title_requests()
        .into_iter()
        .map(|request| request.message)
        .collect();
    assert_eq!(asked, vec!["Ciao, mi senti? Dimmi solo sì".to_owned()]);
}

fn steer(text: &str) -> QueuedMessage {
    QueuedMessage::Steer(SteerCommand {
        message: text.into(),
        images: None,
    })
}

// spec: DMN-226, DMN-073
#[tokio::test]
async fn steer_and_follow_up_reach_omp_and_name_an_untitled_thread_like_a_prompt() {
    let h = harness(|config| config.title_generation = false);
    let (id, _) = h.untitled_thread();

    h.supervisor
        .queue(
            &id,
            QueuedMessage::FollowUp(FollowUpCommand {
                message: "fix the build".into(),
                images: None,
            }),
        )
        .await
        .unwrap();
    h.supervisor
        .queue(&id, steer("and the tests"))
        .await
        .unwrap();

    assert_eq!(
        h.title(&id),
        ("fix the build".into(), TitleSource::Placeholder)
    );
    let sent: Vec<&str> = h
        .launcher
        .process(0)
        .calls()
        .iter()
        .filter_map(|call| match call {
            FakeCall::Call(call @ (OmpCall::Steer(_) | OmpCall::FollowUp(_))) => Some(call.name()),
            _ => None,
        })
        .collect();
    assert_eq!(sent, ["follow_up", "steer"]);
}

// spec: DMN-226, DMN-029
#[tokio::test]
async fn a_steer_waits_for_the_root_sync_in_flight() {
    let h = harness(|_| {});
    let id = h.thread();
    let (ws, folder) = workspace_of(&h, &id);
    h.supervisor.start(&id).await.unwrap();
    let link = folder.join("app");
    std::os::unix::fs::symlink(h.external_repo("app"), &link).unwrap();
    h.workspaces.rescan(&ws).unwrap();
    let gate = h.launcher.gate("prompt");

    let sync = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.sync_roots(&id).await }
    });
    gate.arrived(1).await;
    let steered = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.queue(&id, steer("look at app too")).await }
    });
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    let steers = || {
        h.launcher
            .process(0)
            .calls()
            .iter()
            .filter(|call| matches!(call, FakeCall::Call(OmpCall::Steer(_))))
            .count()
    };
    assert_eq!(steers(), 0, "the steer waits for /add-dir");
    gate.open();
    sync.await.unwrap().unwrap();
    steered.await.unwrap().unwrap();

    assert_eq!(steers(), 1);
    assert_eq!(h.local_commands(), [format!("/add-dir {}", link.display())]);
}

// spec: DMN-060, DMN-073
#[tokio::test]
async fn set_thread_title_stores_a_set_title_and_names_the_session() {
    let h = harness(|_| {});
    let (id, _) = h.untitled_thread();
    h.supervisor.start(&id).await.unwrap();
    let mut changes = h.supervisor.subscribe_changes();
    let gate = h.launcher.gate("set_session_name");
    let set_title = h.supervisor.title_setter(&id);

    // omp-rpc runs host tool handlers on their own threads.
    let result = tokio::task::spawn_blocking(move || {
        let arguments = json!({ "title": "Agent's choice" });
        run_set_thread_title(arguments.as_object().unwrap(), || false, &*set_title)
            .map(|_| ())
            .map_err(|error| error.to_string())
    })
    .await
    .unwrap();

    assert_eq!(result, Ok(()));
    assert_eq!(h.title(&id), ("Agent's choice".into(), TitleSource::Set));
    assert_eq!(changes.recv().await.unwrap(), id);
    gate.arrived(1).await;
    gate.open();
    assert_eq!(h.session_names(), ["Agent's choice"]);
}

fn owner(connection: u64) -> LiveOwner {
    LiveOwner {
        client: "omfp/web".into(),
        connection,
    }
}

const OFFER: &str = "v=0\r\na=setup:actpass\r\na=mid:0\r\n";

/// The `live_connect` offers a fake process received.
fn live_offers(process: &crate::agent::FakeProcess) -> Vec<String> {
    process
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            FakeCall::Call(OmpCall::LiveConnect(connect)) => Some(connect.sdp_offer),
            _ => None,
        })
        .collect()
}

fn live_phase(state: &ThreadState) -> Option<&'static str> {
    state
        .live
        .as_ref()
        .and_then(|live| live.phase)
        .map(omp_rpc::LivePhase::as_str)
}

// spec: DMN-100, TST-015
#[tokio::test]
async fn live_connect_starts_omp_records_the_owner_and_relays_the_answer() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.script_live(crate::agent::FakeLive::connect(
        crate::agent::FakeLive::call_frames(),
    ));

    let answer = h
        .supervisor
        .live_connect(&id, owner(4), OFFER.into(), None)
        .await
        .unwrap();

    assert_eq!(answer.voice, "sol");
    assert_eq!(answer.sdp_answer, crate::agent::answer_for(OFFER));
    assert_eq!(live_offers(&h.launcher.process(0)), vec![OFFER.to_owned()]);
    // Partials arrive first: wait for both lines' finals, not just for two lines.
    h.wait(&id, "the call's final transcript", |state| {
        state.live.as_ref().is_some_and(|live| {
            live.transcript.len() == 2 && live.transcript.iter().all(|line| line.r#final)
        })
    })
    .await;
    let state = h.log(&id).await.with_state(Clone::clone);
    let live = state.live.clone().unwrap();
    assert_eq!(live.owner, owner(4));
    assert_eq!(live_phase(&state), Some("listening"));
    assert_eq!(live.transcript.len(), 2);
}

// spec: DMN-105
#[tokio::test]
async fn live_connect_sends_the_live_prompt_with_the_threads_conversation() {
    let h = harness(|_| {});
    let (ws, _) = h.workspace();
    let id = h
        .threads
        .create(NewThread::new("Login bug", ws))
        .unwrap()
        .id;
    let history = [
        json!({ "role": "user", "content": "why does {{login}} fail?" }),
        json!({ "role": "assistant", "content": [{ "type": "text", "text": "The token was stale." }] }),
    ]
    .map(|message| omp_rpc::AgentMessage::from_value(message).unwrap());
    h.launcher
        .push_run(FakeRun::fresh().with_history(history.into()));

    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();

    let instructions: Vec<Option<String>> = h
        .launcher
        .process(0)
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            FakeCall::Call(OmpCall::LiveConnect(connect)) => Some(connect.instructions),
            _ => None,
        })
        .collect();
    let state = h.log(&id).await.with_state(Clone::clone);
    assert_eq!(
        instructions,
        vec![Some(crate::live_context::live_instructions(
            "Login bug",
            &state,
            &[]
        ))]
    );
    let sent = instructions[0].as_deref().unwrap();
    assert!(sent.starts_with("You: omp Live, realtime voice surface"));
    assert!(sent.contains("Thread title: Login bug"));
    assert!(sent.contains(
        "User: why does {{default \"\" \"{{\"}}login}} fail?\n\nAssistant: The token was stale."
    ));
}

// spec: DMN-100
#[tokio::test]
async fn a_blank_offer_is_refused_without_omp() {
    let h = harness(|_| {});
    let id = h.thread();

    let error = h
        .supervisor
        .live_connect(&id, owner(1), " \n".into(), None)
        .await
        .unwrap_err();

    assert!(matches!(error, SupervisorError::EmptyOffer), "{error:?}");
    assert_eq!(h.launcher.launches().len(), 0);
}

// spec: DMN-101, DMN-102
#[tokio::test]
async fn one_live_session_per_thread_until_omp_ends_it() {
    let h = harness(|_| {});
    let (id, other) = (h.thread(), h.thread());
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();

    let error = h
        .supervisor
        .live_connect(&id, owner(2), OFFER.into(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, SupervisorError::LiveActive { client } if client == "omfp/web"),
        "{error:?}"
    );
    assert_eq!(
        live_offers(&h.launcher.process(0)).len(),
        1,
        "omp never saw the second offer"
    );
    h.supervisor
        .live_connect(&other, owner(2), OFFER.into(), None)
        .await
        .expect("another thread has its own session");

    h.supervisor.live_stop(&id).await.unwrap();
    h.wait(&id, "the end of the session", |state| state.live.is_none())
        .await;
    h.supervisor
        .live_connect(&id, owner(3), OFFER.into(), None)
        .await
        .expect("a new session once omp ended the first");
    let state = h.log(&id).await.with_state(Clone::clone);
    assert_eq!(state.live.map(|live| live.owner), Some(owner(3)));
}

/// The `custom` voice conversations in the thread's transcript (DMN-113).
fn voice_logs(state: &ThreadState) -> Vec<String> {
    state
        .transcript
        .iter()
        .filter_map(|entry| match &entry.message {
            omp_rpc::AgentMessage::Custom(custom)
                if custom.custom_type.as_deref() == Some(crate::voice_log::CUSTOM_TYPE) =>
            {
                serde_json::to_value(&custom.content)
                    .ok()?
                    .as_str()
                    .map(str::to_owned)
            }
            _ => None,
        })
        .collect()
}

/// omp announces omfp's extension command, as it does once it loaded the extension (DMN-113).
async fn announce_voice_log(h: &Harness, id: &str) {
    let actor = h.supervisor.actor(id).await.unwrap();
    h.supervisor.ensure_running(&actor).await.unwrap();
    let commands = frame(json!({
        "type": "available_commands_update",
        "commands": [{ "name": crate::voice_log::COMMAND, "source": "extension" }],
    }));
    h.supervisor
        .push(&actor, ThreadEvent::Omp { frame: commands });
}

// spec: DMN-113
#[tokio::test]
async fn an_ended_live_session_leaves_its_conversation_in_the_thread() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.script_live(crate::agent::FakeLive::connect(
        crate::agent::FakeLive::call_frames(),
    ));
    announce_voice_log(&h, &id).await;
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();
    assert_eq!(
        h.local_commands_of(0),
        Vec::<String>::new(),
        "nothing is recorded while the session runs"
    );

    h.supervisor.live_stop(&id).await.unwrap();
    h.wait(&id, "the voice conversation", |state| {
        !voice_logs(state).is_empty()
    })
    .await;

    let state = h.log(&id).await.with_state(Clone::clone);
    assert_eq!(state.live, None);
    let [log] = voice_logs(&state).try_into().unwrap();
    assert!(log.starts_with("Voice conversation · 2 turns\n"), "{log}");
    assert!(
        log.ends_with("\nUser: Ciao, mi senti? Dimmi solo sì\nAssistant: Sì."),
        "{log}"
    );
    assert_eq!(h.local_commands_of(0).len(), 1, "one session, one record");
}

// spec: DMN-113
#[tokio::test]
async fn a_conversation_waiting_for_a_busy_thread_reaches_the_next_session() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.script_live(crate::agent::FakeLive::connect(
        crate::agent::FakeLive::call_frames(),
    ));
    announce_voice_log(&h, &id).await;
    let actor = h.supervisor.lookup(&id).unwrap();
    h.supervisor.push(
        &actor,
        ThreadEvent::Omp {
            frame: agent_start(),
        },
    );
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();
    h.supervisor.live_stop(&id).await.unwrap();
    h.wait(&id, "the end of the session", |state| state.live.is_none())
        .await;
    h.supervisor.record_voice(&id).await;
    assert_eq!(
        h.local_commands_of(0),
        Vec::<String>::new(),
        "a busy thread records nothing"
    );

    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();
    let instructions: Vec<String> = h
        .launcher
        .process(0)
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            FakeCall::Call(OmpCall::LiveConnect(connect)) => connect.instructions,
            _ => None,
        })
        .collect();
    assert!(
        instructions[1]
            .contains("Voice conversation:\nUser: Ciao, mi senti? Dimmi solo sì\nAssistant: Sì."),
        "{}",
        instructions[1]
    );

    h.supervisor.live_stop(&id).await.unwrap();
    h.wait(&id, "the end of the second session", |state| {
        state.live.is_none()
    })
    .await;
    h.supervisor.push(
        &actor,
        ThreadEvent::Omp {
            frame: frame(json!({ "type": "session_settled" })),
        },
    );
    h.wait(&id, "both conversations", |state| {
        voice_logs(state).len() == 2
    })
    .await;
}

// spec: DMN-113
#[tokio::test]
async fn without_the_extensions_command_nothing_is_sent() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.script_live(crate::agent::FakeLive::connect(
        crate::agent::FakeLive::call_frames(),
    ));
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();
    h.supervisor.live_stop(&id).await.unwrap();
    h.wait(&id, "the end of the session", |state| state.live.is_none())
        .await;

    h.supervisor.record_voice(&id).await;

    assert_eq!(
        h.local_commands_of(0),
        Vec::<String>::new(),
        "omp would hand the text to the model"
    );

    // A process that announces the command (as a new one does at its start) takes the waiting conversation.
    announce_voice_log(&h, &id).await;
    h.wait(&id, "the voice conversation", |state| {
        voice_logs(state).len() == 1
    })
    .await;
}

// spec: DMN-102
#[tokio::test]
async fn live_stop_and_mute_never_start_omp() {
    let h = harness(|_| {});
    let id = h.thread();

    h.supervisor.live_stop(&id).await.unwrap();
    let error = h.supervisor.live_mute(&id, true).await.unwrap_err();

    assert!(matches!(error, SupervisorError::NoLiveSession), "{error:?}");
    assert_eq!(h.launcher.launches().len(), 0);
}

// spec: DMN-102, TST-015
#[tokio::test]
async fn live_mute_forwards_the_mute_and_omp_reports_the_phase() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();

    assert!(h.supervisor.live_mute(&id, true).await.unwrap());
    h.wait(&id, "the muted phase", |state| {
        live_phase(state) == Some("muted")
    })
    .await;
    assert!(!h.supervisor.live_mute(&id, false).await.unwrap());
    h.wait(&id, "the listening phase", |state| {
        live_phase(state) == Some("listening")
    })
    .await;
    let mutes: Vec<Option<bool>> = h
        .launcher
        .process(0)
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            FakeCall::Call(OmpCall::LiveMute(mute)) => Some(mute.muted),
            _ => None,
        })
        .collect();
    assert_eq!(mutes, vec![Some(true), Some(false)]);
}

// spec: DMN-104, TST-015
#[tokio::test]
async fn a_missing_codex_login_reads_as_a_login_hint() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher
        .script_live(crate::agent::FakeLive::no_credential());

    let error = h
        .supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap_err();

    let SupervisorError::Live { message, omp } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(omp, crate::agent::NO_CREDENTIAL);
    assert!(message.contains("`openai-codex` login"), "{message}");
    assert!(message.contains(":login openai-codex"), "{message}");
    h.wait(&id, "the end of the session", |state| {
        state.live.is_none() && state.live_error.is_some()
    })
    .await;
    let state = h.log(&id).await.with_state(Clone::clone);
    assert_eq!(state.live_error.as_deref(), Some(message.as_str()));
    // omp's `live_end` freed the thread for another try.
    h.launcher.script_live(crate::agent::FakeLive::default());
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();
}

// spec: DMN-101, DMN-104
#[tokio::test]
async fn a_refusal_before_the_session_closes_it_at_once() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.reply(
        "live_connect",
        FakeReply::Fail {
            message: "End dictation before starting live voice".into(),
            code: None,
        },
    );

    let error = h
        .supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "end dictation before starting live voice"
    );
    let state = h.log(&id).await.with_state(Clone::clone);
    assert_eq!(state.live, None, "a `live_session` without owner closed it");
    let again = h
        .supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap_err();
    assert!(matches!(again, SupervisorError::Live { .. }), "{again:?}");
}

// spec: DMN-103
#[tokio::test]
async fn a_closed_connection_stops_only_its_own_sessions() {
    let h = harness(|_| {});
    let (mine, theirs) = (h.thread(), h.thread());
    h.supervisor
        .live_connect(&mine, owner(1), OFFER.into(), None)
        .await
        .unwrap();
    h.supervisor
        .live_connect(&theirs, owner(2), OFFER.into(), None)
        .await
        .unwrap();

    h.supervisor.release_live(1).await;

    h.wait(&mine, "the stop", |state| state.live.is_none())
        .await;
    assert!(!h.launcher.process(0).live_active());
    assert!(h.launcher.process(1).live_active());
    let state = h.log(&theirs).await.with_state(Clone::clone);
    assert_eq!(state.live.map(|live| live.owner), Some(owner(2)));
}

// spec: DMN-103
#[tokio::test]
async fn a_live_session_ends_with_its_process() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();

    h.launcher.process(0).exit();
    h.wait(&id, "the crash", |state| {
        matches!(state.process, ProcessState::Failed { .. })
    })
    .await;

    assert_eq!(
        h.log(&id).await.with_state(|state| state.live.clone()),
        None
    );
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .expect("the slot died with the process");
    assert_eq!(h.launcher.launches().len(), 2);
}

// spec: DMN-103, DMN-034
#[tokio::test(start_paused = true)]
async fn the_reaper_keeps_a_process_with_a_live_session() {
    let timeout = Duration::from_mins(1);
    let h = harness(|config| config.idle_timeout = timeout);
    let id = h.thread();
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();
    let reaper = h.supervisor.spawn_reaper();

    tokio::time::sleep(timeout * 10).await;
    assert!(h.supervisor.is_running(&id), "a live session keeps omp");

    h.supervisor.live_stop(&id).await.unwrap();
    h.wait(&id, "the idle stop after the session", |state| {
        state.process == ProcessState::Stopped
    })
    .await;
    reaper.abort();
}

/// How many `live_stop` commands the thread's first process got.
fn live_stops(h: &Harness) -> usize {
    h.launcher
        .process(0)
        .calls()
        .into_iter()
        .filter(|call| matches!(call, FakeCall::Call(OmpCall::LiveStop)))
        .count()
}

// spec: DMN-114
#[tokio::test]
async fn a_live_session_nobody_talks_to_ends() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher
        .script_live(crate::agent::FakeLive::connect(Vec::new()));
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();
    h.wait(&id, "the listening phase", |state| {
        live_phase(state) == Some("listening")
    })
    .await;
    // The clock moves only by `advance` from here, and the spoken line goes straight into the thread: no frame is in
    // transit while time jumps.
    tokio::time::pause();
    let actor = h.supervisor.lookup(&id).unwrap();
    let almost = live::LIVE_SILENCE.saturating_sub(Duration::from_mins(1));

    tokio::time::advance(almost).await;
    h.supervisor.push(
        &actor,
        ThreadEvent::Omp {
            frame: crate::agent::live_transcript(omp_rpc::LiveRole::User, 1, "still here", true),
        },
    );
    tokio::time::advance(almost).await;
    assert_eq!(live_stops(&h), 0, "a word restarts the silence");

    tokio::time::advance(Duration::from_mins(2)).await;
    h.wait(&id, "the end of the silent session", |state| {
        state.live.is_none()
    })
    .await;
    assert_eq!(live_stops(&h), 1);
}

// spec: DMN-114
#[tokio::test]
async fn delegated_work_keeps_the_live_session_and_its_end_restarts_the_silence() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.script_live(crate::agent::FakeLive::connect(vec![
        crate::agent::live_phase(omp_rpc::LivePhase::Working),
    ]));
    h.supervisor
        .live_connect(&id, owner(1), OFFER.into(), None)
        .await
        .unwrap();
    h.wait(&id, "the working phase", |state| {
        live_phase(state) == Some("working")
    })
    .await;
    tokio::time::pause();
    let actor = h.supervisor.lookup(&id).unwrap();

    tokio::time::advance(live::LIVE_SILENCE * 3 + Duration::from_mins(5)).await;
    assert_eq!(live_stops(&h), 0, "the voice waits for its delegated work");

    // The work ends between two looks of the watch: the silence counts from then.
    h.supervisor.push(
        &actor,
        ThreadEvent::Omp {
            frame: crate::agent::live_phase(omp_rpc::LivePhase::Listening),
        },
    );
    tokio::time::advance(live::LIVE_SILENCE.saturating_sub(Duration::from_mins(1))).await;
    assert_eq!(live_stops(&h), 0);
    tokio::time::advance(Duration::from_mins(2)).await;
    h.wait(&id, "the end of the silent session", |state| {
        state.live.is_none()
    })
    .await;
    assert_eq!(live_stops(&h), 1);
}

/// The `live_start` calls a fake process received, with their instructions.
fn live_starts(process: &crate::agent::FakeProcess) -> Vec<Option<String>> {
    process
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            FakeCall::Call(OmpCall::LiveStart(start)) => Some(start.instructions),
            _ => None,
        })
        .collect()
}

// spec: DMN-109, DMN-105, TST-051
#[tokio::test]
async fn live_start_starts_omp_records_the_owner_and_relays_the_voice() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.script_live(crate::agent::FakeLive::connect(vec![
        crate::agent::live_phase(omp_rpc::LivePhase::Speaking),
    ]));

    let voice = h.supervisor.live_start(&id, owner(4), None).await.unwrap();

    assert_eq!(voice, "sol");
    h.wait(&id, "omp's speaking phase", |state| {
        live_phase(state) == Some("speaking")
    })
    .await;
    let state = h.log(&id).await.with_state(Clone::clone);
    assert_eq!(state.live.map(|live| live.owner), Some(owner(4)));
    let state = h.log(&id).await.with_state(Clone::clone);
    assert_eq!(
        live_starts(&h.launcher.process(0)),
        vec![Some(crate::live_context::live_instructions(
            "t",
            &state,
            &[]
        ))]
    );
    assert_eq!(live_offers(&h.launcher.process(0)), Vec::<String>::new());
}

/// The `voice` of every `live_connect` and `live_start` a fake process received, in order.
fn live_voices(process: &crate::agent::FakeProcess) -> Vec<Option<String>> {
    process
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            FakeCall::Call(OmpCall::LiveConnect(connect)) => Some(connect.voice),
            FakeCall::Call(OmpCall::LiveStart(start)) => Some(start.voice),
            _ => None,
        })
        .collect()
}

// spec: DMN-112
#[tokio::test]
async fn a_chosen_voice_reaches_omp_and_none_leaves_omps_own() {
    use omfp_protocol::LiveVoice;
    let h = harness(|_| {});
    let id = h.thread();

    let answer = h
        .supervisor
        .live_connect(&id, owner(1), OFFER.into(), Some(LiveVoice::Maple))
        .await
        .unwrap();
    assert_eq!(answer.voice, "maple");
    h.supervisor.live_stop(&id).await.unwrap();
    h.wait(&id, "the end of the session", |state| state.live.is_none())
        .await;
    let voice = h
        .supervisor
        .live_start(&id, owner(1), Some(LiveVoice::Vale))
        .await
        .unwrap();
    assert_eq!(voice, "vale");
    h.supervisor.live_stop(&id).await.unwrap();
    h.wait(&id, "the end of the session", |state| state.live.is_none())
        .await;
    let voice = h.supervisor.live_start(&id, owner(1), None).await.unwrap();
    assert_eq!(voice, "sol");

    assert_eq!(
        live_voices(&h.launcher.process(0)),
        vec![Some("maple".to_owned()), Some("vale".to_owned()), None]
    );
}

// spec: DMN-109, DMN-101
#[tokio::test]
async fn live_start_and_live_connect_share_the_threads_one_session() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.live_start(&id, owner(1), None).await.unwrap();

    let error = h
        .supervisor
        .live_connect(&id, owner(2), OFFER.into(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, SupervisorError::LiveActive { .. }),
        "{error:?}"
    );
    let error = h
        .supervisor
        .live_start(&id, owner(2), None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, SupervisorError::LiveActive { .. }),
        "{error:?}"
    );
    assert_eq!(live_starts(&h.launcher.process(0)).len(), 1);
    assert_eq!(live_offers(&h.launcher.process(0)).len(), 0);

    h.supervisor.live_stop(&id).await.unwrap();
    h.wait(&id, "the end of the session", |state| state.live.is_none())
        .await;
    h.supervisor
        .live_connect(&id, owner(3), OFFER.into(), None)
        .await
        .expect("a new session once omp ended the first");
}

// spec: DMN-109, DMN-103
#[tokio::test]
async fn a_closed_connection_stops_its_live_start_session() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.live_start(&id, owner(1), None).await.unwrap();

    h.supervisor.release_live(1).await;

    h.wait(&id, "the stop", |state| state.live.is_none()).await;
    assert!(!h.launcher.process(0).live_active());
}

// spec: DMN-109, DMN-104, TST-051
#[tokio::test]
async fn a_failed_live_start_reads_like_live_connect() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher
        .script_live(crate::agent::FakeLive::no_credential());

    let error = h
        .supervisor
        .live_start(&id, owner(1), None)
        .await
        .unwrap_err();

    let SupervisorError::Live { message, omp } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(omp, crate::agent::NO_CREDENTIAL);
    assert!(message.contains(":login openai-codex"), "{message}");
    h.wait(&id, "the end of the session", |state| {
        state.live.is_none() && state.live_error.as_deref() == Some(message.as_str())
    })
    .await;
    h.launcher.script_live(crate::agent::FakeLive::default());
    h.supervisor
        .live_start(&id, owner(1), None)
        .await
        .expect("omp's `live_end` freed the thread");
}

// spec: DMN-110, TST-051
#[tokio::test]
async fn live_levels_reach_the_levels_stream_and_never_the_log() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher.script_live(crate::agent::FakeLive::connect(vec![
        crate::agent::live_levels(0.25, 0.5),
        crate::agent::live_levels(0.125, 0.0),
        crate::agent::live_phase(omp_rpc::LivePhase::Speaking),
    ]));
    let mut levels = h.supervisor.live_levels(&id).await.unwrap();

    h.supervisor.live_start(&id, owner(1), None).await.unwrap();

    h.wait(&id, "omp's speaking phase", |state| {
        live_phase(state) == Some("speaking")
    })
    .await;
    let level = |input, output| omp_rpc::LiveLevelsEvent { input, output };
    assert_eq!(levels.recv().await.unwrap(), level(0.25, 0.5));
    assert_eq!(levels.recv().await.unwrap(), level(0.125, 0.0));
    let log = h.log(&id).await;
    let Start::Replay(events) = log
        .subscribe(Some(&crate::event_log::Position {
            epoch: log.position().epoch,
            seq: 0,
        }))
        .start
    else {
        panic!("the epoch's events are all kept");
    };
    let frames: Vec<String> = events
        .iter()
        .filter_map(|sequenced| match &sequenced.event {
            ThreadEvent::Omp { .. } => serde_json::to_value(&sequenced.event)
                .ok()
                .and_then(|value| value["frame"]["type"].as_str().map(str::to_owned)),
            _ => None,
        })
        .filter(|kind| kind.starts_with("live_"))
        .collect();
    assert_eq!(frames, ["live_phase", "live_phase", "live_phase"]);
}

// spec: AGT-070, AGT-071, DMN-162
#[tokio::test]
async fn a_cascade_stop_stops_the_whole_spawned_tree_and_nothing_else() {
    let h = harness(|_| {});
    let (ws, _) = h.workspace();
    let spawn = |title: &str, created_by: crate::store::Provenance| {
        h.threads
            .create(NewThread {
                created_by,
                ..NewThread::new(title, ws.clone())
            })
            .unwrap()
            .id
    };
    let root = spawn("root", crate::store::Provenance::User);
    let child = spawn("child", crate::store::Provenance::Thread(root.clone()));
    let grandchild = spawn(
        "grandchild",
        crate::store::Provenance::Thread(child.clone()),
    );
    let idle = spawn("idle", crate::store::Provenance::Thread(root.clone()));
    let unrelated = spawn("unrelated", crate::store::Provenance::User);
    for id in [&root, &child, &grandchild, &unrelated] {
        h.supervisor.start(id).await.unwrap();
    }

    let stopped = h.supervisor.stop_tree(&root).await.unwrap();

    // The thread first, then its tree in creation order; one that ran no process counts as stopped.
    assert_eq!(
        stopped,
        [
            root.clone(),
            child.clone(),
            grandchild.clone(),
            idle.clone()
        ]
    );
    for id in &stopped {
        assert!(!h.supervisor.is_running(id), "{id} still runs");
    }
    assert!(h.supervisor.is_running(&unrelated));
    assert_eq!(
        h.supervisor.stop_tree(&grandchild).await.unwrap(),
        [grandchild]
    );
    assert!(
        h.supervisor
            .stop_tree("01JUNKNOWN0000000000000000")
            .await
            .is_err()
    );
}

fn frame(value: serde_json::Value) -> RpcNotification {
    serde_json::from_value(value).unwrap()
}

fn subagent(status: &str) -> RpcNotification {
    frame(json!({ "type": "subagent_lifecycle", "payload": {
        "id": "Counter", "agent": "task", "agentSource": "bundled", "status": status, "index": 0,
    } }))
}

fn write_mcp_config(folder: &Path, servers: &serde_json::Value) {
    std::fs::create_dir_all(folder.join(".omp")).unwrap();
    std::fs::write(
        folder.join(".omp/mcp.json"),
        json!({ "mcpServers": servers }).to_string(),
    )
    .unwrap();
}

impl Harness {
    /// Waits until the launcher started `count` processes and the last one runs.
    async fn launched(&self, id: &str, count: usize) {
        while self.launcher.launches().len() < count {
            tokio::task::yield_now().await;
        }
        self.wait(id, "the new process", |state| {
            state.process == ProcessState::Running
        })
        .await;
    }
}

fn command_outputs(state: &ThreadState) -> Vec<String> {
    state
        .notices
        .iter()
        .filter_map(|notice| match &notice.frame {
            omfp_core::NoticeFrame::CommandOutput(output) => Some(output.text.clone()),
            _ => None,
        })
        .collect()
}

// spec: DMN-200
#[tokio::test]
async fn the_daemons_own_checks_stay_out_of_the_transcript_and_the_users_commands_show() {
    let h = harness(|_| {});
    let id = h.thread();
    let (_, folder) = workspace_of(&h, &id);
    write_mcp_config(&folder, &json!({ "fs": { "command": "fs" } }));
    h.launcher
        .local_output("/mcp test fs", &["Server \"fs\" connected (2 tools)."]);
    h.launcher
        .local_output("/dirs", &["Workspace directories:\n  /w"]);
    h.supervisor.start(&id).await.unwrap();

    let listing = h.supervisor.thread_mcp_servers(&id).await.unwrap();
    h.supervisor.prompt(&id, prompt("/dirs")).await.unwrap();
    h.wait(&id, "the output of /dirs", |state| {
        !state.notices.is_empty()
    })
    .await;

    assert_eq!(listing.servers[0].tools, Some(2));
    let log = h.log(&id).await;
    assert_eq!(
        log.with_state(command_outputs),
        ["Workspace directories:\n  /w"]
    );
}

// spec: DMN-201, DMN-203
#[tokio::test]
async fn an_idle_thread_restarts_at_once_when_its_mcp_configs_change() {
    let h = harness(|_| {});
    let id = h.thread();
    let (_, folder) = workspace_of(&h, &id);
    h.supervisor.start(&id).await.unwrap();
    h.supervisor.check_mcp_configs().await;
    assert_eq!(h.launcher.launches().len(), 1, "nothing changed");

    write_mcp_config(&folder, &json!({ "fs": { "command": "fs" } }));
    h.supervisor.check_mcp_configs().await;
    h.launched(&id, 2).await;
    h.supervisor.check_mcp_configs().await;

    assert_eq!(h.launcher.launches().len(), 2, "the new process read them");
    assert_eq!(h.supervisor.status(&id).restart_pending, Vec::new());
}

// spec: DMN-203
#[tokio::test]
async fn a_stopped_thread_needs_no_reload() {
    let h = harness(|_| {});
    let id = h.thread();
    let (_, folder) = workspace_of(&h, &id);
    h.supervisor.start(&id).await.unwrap();
    h.supervisor.stop(&id).await.unwrap();

    write_mcp_config(&folder, &json!({ "fs": { "command": "fs" } }));
    h.supervisor.check_mcp_configs().await;

    assert_eq!(h.launcher.launches().len(), 1);
    assert!(!h.supervisor.is_running(&id));
}

// spec: DMN-202, DMN-203
#[tokio::test]
async fn a_reauthorized_server_restarts_a_busy_thread_once_its_turn_settles() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![agent_start()]);
    h.wait(&id, "the busy agent", |state| !state.settled).await;
    let reauthorized = "✓ Reauthorized \"gh\" (user config). Credentials apply on the next connection; restart the \
                        session if the server stays disconnected.";

    h.launcher.process(0).emit(vec![frame(
        json!({ "type": "command_output", "text": reauthorized }),
    )]);
    h.wait(&id, "the output", |state| !state.notices.is_empty())
        .await;
    assert_eq!(h.launcher.launches().len(), 1, "the turn goes on");
    assert_eq!(h.supervisor.status(&id).restart_pending, Vec::new());

    h.launcher
        .process(0)
        .emit(vec![frame(json!({ "type": "session_settled" }))]);
    h.launched(&id, 2).await;

    // Same session (`--continue`), and the card stays.
    let log = h.log(&id).await;
    assert_eq!(log.with_state(command_outputs), [reauthorized]);
}

// spec: DMN-202, DMN-203
#[tokio::test]
async fn mcp_add_restarts_the_idle_thread_after_answering_the_prompt() {
    let h = harness(|_| {});
    let id = h.thread();
    h.launcher
        .local_output("/mcp add fs -- fs", &["Added MCP server \"fs\" (project)."]);
    h.supervisor.start(&id).await.unwrap();

    let admission = h.supervisor.prompt(&id, prompt("/mcp add fs -- fs")).await;
    h.launched(&id, 2).await;

    assert!(!admission.unwrap().agent_invoked);
}

// spec: DMN-204, PRT-121
#[tokio::test]
async fn background_work_holds_the_reload_back_and_flags_the_thread_until_it_ends() {
    let h = harness(|_| {});
    let id = h.thread();
    let (_, folder) = workspace_of(&h, &id);
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![subagent("started")]);
    h.wait(&id, "the subagent", |state| !state.subagents.is_empty())
        .await;
    let mut changes = h.supervisor.subscribe_changes();

    write_mcp_config(&folder, &json!({ "fs": { "command": "fs" } }));
    h.supervisor.check_mcp_configs().await;

    assert_eq!(changes.recv().await.unwrap(), id);
    assert_eq!(
        h.supervisor.status(&id).restart_pending,
        [RestartReason::McpConfig]
    );
    assert_eq!(h.launcher.launches().len(), 1);

    h.launcher.process(0).emit(vec![subagent("completed")]);
    h.launched(&id, 2).await;
    assert_eq!(h.supervisor.status(&id).restart_pending, Vec::new());
}

// spec: DMN-204, PRT-121
#[tokio::test]
async fn a_reconnect_loads_the_held_back_tools_and_clears_the_flag() {
    let h = harness(|_| {});
    let id = h.thread();
    let (_, folder) = workspace_of(&h, &id);
    h.launcher
        .local_output("/mcp test fs", &["Server \"fs\" connected (1 tool)."]);
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![subagent("started")]);
    h.wait(&id, "the subagent", |state| !state.subagents.is_empty())
        .await;
    write_mcp_config(&folder, &json!({ "fs": { "command": "fs" } }));
    h.supervisor.check_mcp_configs().await;
    assert_eq!(
        h.supervisor.status(&id).restart_pending,
        [RestartReason::McpConfig]
    );

    h.supervisor.mcp_reconnect(&id, None).await.unwrap();

    assert_eq!(h.launcher.launches().len(), 2);
    assert_eq!(h.supervisor.status(&id).restart_pending, Vec::new());
}

// spec: DMN-224, DMN-228, PRT-121, PRT-134
#[tokio::test]
async fn a_held_back_restart_names_each_reason_once_in_the_order_they_came() {
    let h = harness(|_| {});
    let id = h.thread();
    let (_, folder) = workspace_of(&h, &id);
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![subagent("started")]);
    h.wait(&id, "the subagent", |state| !state.subagents.is_empty())
        .await;

    h.supervisor.restart_after_login();
    assert_eq!(
        h.supervisor.status(&id).restart_pending,
        [RestartReason::Login]
    );
    write_mcp_config(&folder, &json!({ "fs": { "command": "fs" } }));
    h.supervisor.check_mcp_configs().await;
    h.supervisor.restart_after_login();
    assert_eq!(
        h.supervisor.status(&id).restart_pending,
        [RestartReason::Login, RestartReason::McpConfig]
    );
    assert_eq!(h.launcher.launches().len(), 1);

    h.launcher.process(0).emit(vec![subagent("completed")]);
    h.launched(&id, 2).await;
    assert_eq!(h.supervisor.status(&id).restart_pending, Vec::new());
}

// spec: DMN-227, DMN-226
#[tokio::test]
async fn a_restart_waits_for_the_message_on_its_way_to_omp() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    let gate = h.launcher.gate("prompt");

    let sending = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.prompt(&id, prompt("/dirs")).await }
    });
    gate.arrived(1).await;
    let restarting = tokio::spawn({
        let (supervisor, id) = (h.supervisor.clone(), id.clone());
        async move { supervisor.restart(&id, Restart::User).await }
    });
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        h.launcher.launches().len(),
        1,
        "the restart waits for omp's answer"
    );
    gate.open();
    sending.await.unwrap().unwrap();
    restarting.await.unwrap().unwrap();

    assert_eq!(h.launcher.launches().len(), 2);
}

// spec: DMN-340
#[tokio::test]
async fn an_omp_that_exits_unexpectedly_leaves_no_sign_in_waiting() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    let authorize =
        "https://auth.example.com/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fcallback";
    h.launcher.process(0).emit(vec![frame(json!({
        "type": "command_output",
        "text": format!("Opening your browser to authorize. If it did not open:\n{authorize}"),
    }))]);
    h.wait(&id, "the output", |state| !state.notices.is_empty())
        .await;
    let pasted = "http://localhost:1455/callback?code=c0de";
    assert!(matches!(
        h.supervisor.oauth_target(Some(&id), pasted).await,
        Ok(Ok(_))
    ));

    // Its listener is gone: another process may take the port.
    h.launcher.process(0).exit();
    h.wait(&id, "the failure", |state| {
        matches!(state.process, ProcessState::Failed { .. })
    })
    .await;
    assert!(matches!(
        h.supervisor.oauth_target(Some(&id), pasted).await,
        Ok(Err(crate::oauth_relay::Refusal::NotPending))
    ));
}

// spec: DMN-230
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_whose_caller_went_away_finishes_and_holds_the_credential_writes_until_then() {
    let home = tempfile::tempdir().unwrap();
    let agent = home.path().join(".omp").join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    rusqlite::Connection::open(agent.join("agent.db"))
        .unwrap()
        .execute_batch("CREATE TABLE standalone (n INTEGER);")
        .unwrap();
    let env = omfp_testkit::standalone_omp_env(home.path());
    let h = harness(|config| config.user_omp_env = env);
    h.launcher.set_export(Ok(
        r#"{"version":1,"credentials":[{"provider":"openai","credential":{"type":"api_key","key":"k"}}]}"#,
    ));
    let gate = h.launcher.gate_imports();
    let supervisor = h.supervisor.clone();
    let caller = tokio::spawn(async move { supervisor.import_omp_logins().await });
    gate.arrived(1).await;

    // The caller goes away (an MCP stream closed) while omp imports.
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(
        h.supervisor.inner.credential_writes.try_lock().is_err(),
        "the copy still holds the credential writes: a login or a logout waits"
    );

    gate.open();
    drop(h.supervisor.credential_writes().await);
    assert_eq!(h.launcher.imports().len(), 1, "the copy finished");
}
