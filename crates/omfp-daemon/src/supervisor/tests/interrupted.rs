//! Turns a daemon stop or a crash of omp interrupted (DMN-370..DMN-374, DMN-377), across a restart of the supervisor
//! over the same store.

use super::*;

/// The daemon started again over `h`'s data directory (TST-119): a new store connection, thread records and
/// supervisor, with a fake launcher of its own.
struct Restarted {
    launcher: FakeLauncher,
    supervisor: Supervisor,
    store: Arc<Store>,
}

fn restart(h: &Harness) -> Restarted {
    let root = h.dir.path().join("workspaces");
    let store = Arc::new(Store::open(&h.data.db()).unwrap());
    let threads = Arc::new(Threads::new(Arc::clone(&store), h.data.clone()));
    let workspaces = Arc::new(Workspaces::new(
        Arc::clone(&store),
        Arc::clone(&h.git) as Arc<dyn Git>,
        root,
    ));
    let launcher = FakeLauncher::new();
    let supervisor = Supervisor::new(
        Arc::new(launcher.clone()),
        threads,
        workspaces,
        h.data.clone(),
        SupervisorConfig::new("omp"),
    );
    Restarted {
        launcher,
        supervisor,
        store,
    }
}

impl Restarted {
    fn marked(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .store
            .interrupted_turns()
            .unwrap()
            .into_keys()
            .collect();
        ids.sort();
        ids
    }

    /// The prompts the `index`th process got.
    fn prompts(&self, index: usize) -> Vec<String> {
        self.launcher
            .process(index)
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                FakeCall::Prompt(prompt) => Some(prompt.message),
                _ => None,
            })
            .collect()
    }
}

/// `h`'s threads: one mid-turn, one idle, one whose omp never ran; the daemon then stops.
async fn shut_down_with_a_busy_thread(h: &Harness) -> (String, String, String) {
    let (busy, idle, stopped) = (h.thread(), h.thread(), h.thread());
    h.supervisor.start(&busy).await.unwrap();
    h.supervisor.start(&idle).await.unwrap();
    h.launcher.process(0).emit(vec![agent_start()]);
    h.wait(&busy, "the busy agent", |state| !state.settled)
        .await;
    h.supervisor.shutdown().await;
    (busy, idle, stopped)
}

// spec: DMN-370, DMN-371, TST-119
#[tokio::test]
async fn a_thread_busy_at_shutdown_is_marked_interrupted_after_a_restart_and_a_settled_one_is_not()
{
    let h = harness(|_| {});
    let before = crate::store::now_ms();

    let (busy, _idle, _stopped) = shut_down_with_a_busy_thread(&h).await;

    let restarted = restart(&h);
    assert_eq!(restarted.marked(), std::slice::from_ref(&busy));
    let at = restarted.store.interrupted_turns().unwrap()[&busy];
    assert!(at >= before, "marked at the shutdown: {at} < {before}");
    // The mark outlives another restart that nothing resumed in between.
    restarted.supervisor.shutdown().await;
    assert_eq!(restart(&h).marked(), [busy]);
    assert_eq!(
        restarted.launcher.launches().len(),
        0,
        "a restart starts no omp"
    );
}

// spec: DMN-372, PRT-230
#[tokio::test]
async fn resume_starts_omp_sends_the_notice_and_clears_the_mark() {
    let h = harness(|_| {});
    let (busy, idle, _stopped) = shut_down_with_a_busy_thread(&h).await;
    let restarted = restart(&h);
    let mut changes = restarted.supervisor.subscribe_changes();

    let admission = restarted.supervisor.resume(&busy).await.unwrap();

    assert!(admission.agent_invoked);
    assert_eq!(restarted.launcher.launches().len(), 1);
    assert_eq!(restarted.prompts(0), [RESUME_NOTICE]);
    assert_eq!(restarted.marked(), Vec::<String>::new());
    while changes.recv().await.unwrap() != busy {}
    // Resumed already, or never interrupted: refused, nothing sent, nothing started.
    for id in [&busy, &idle] {
        assert!(matches!(
            restarted.supervisor.resume(id).await,
            Err(SupervisorError::NotInterrupted)
        ));
    }
    assert_eq!(restarted.launcher.launches().len(), 1);
    assert_eq!(restarted.prompts(0), [RESUME_NOTICE]);
}

// spec: DMN-372
#[tokio::test]
async fn a_resume_whose_omp_cannot_start_keeps_the_mark() {
    let h = harness(|_| {});
    let (busy, _idle, _stopped) = shut_down_with_a_busy_thread(&h).await;
    let restarted = restart(&h);
    let at = restarted.store.interrupted_turns().unwrap()[&busy];
    restarted.launcher.fail_next_launch("omp: no model");

    let resumed = restarted.supervisor.resume(&busy).await;

    assert!(
        matches!(resumed, Err(SupervisorError::Start { .. })),
        "{resumed:?}"
    );
    assert_eq!(restarted.store.interrupted_turns().unwrap()[&busy], at);
}

// spec: DMN-373
#[tokio::test]
async fn a_message_to_the_agent_clears_the_mark_and_a_local_command_does_not() {
    let h = harness(|_| {});
    let (first, second) = (h.thread(), h.thread());
    for (index, id) in [&first, &second].into_iter().enumerate() {
        h.supervisor.start(id).await.unwrap();
        h.launcher.process(index).emit(vec![agent_start()]);
        h.wait(id, "the busy agent", |state| !state.settled).await;
    }
    h.supervisor.shutdown().await;
    let restarted = restart(&h);
    assert_eq!(restarted.marked().len(), 2);

    restarted
        .supervisor
        .prompt(&first, prompt("/dirs"))
        .await
        .unwrap();
    assert_eq!(restarted.marked().len(), 2, "a local command is no turn");
    restarted
        .supervisor
        .prompt(&first, prompt("go on"))
        .await
        .unwrap();
    let follow_up = QueuedMessage::FollowUp(FollowUpCommand {
        message: "and then".into(),
        images: None,
    });
    restarted
        .supervisor
        .queue(&second, follow_up)
        .await
        .unwrap();

    assert_eq!(restarted.marked(), Vec::<String>::new());
}

// spec: DMN-374
#[tokio::test]
async fn only_the_daemon_stop_marks_a_turn_not_a_stop_of_the_thread() {
    let h = harness(|_| {});
    let (busy, prompted) = (h.thread(), h.thread());
    h.supervisor.start(&busy).await.unwrap();
    h.launcher.process(0).emit(vec![agent_start()]);
    h.wait(&busy, "the busy agent", |state| !state.settled)
        .await;
    // A prompt omp admitted and has not reported a turn for yet.
    h.supervisor
        .prompt(&prompted, prompt("go on"))
        .await
        .unwrap();

    h.supervisor.stop(&busy).await.unwrap();
    h.supervisor.stop(&prompted).await.unwrap();
    h.supervisor.shutdown().await;

    assert_eq!(restart(&h).marked(), Vec::<String>::new());
}

// spec: DMN-370, DMN-373
#[tokio::test]
async fn a_message_admitted_during_the_shutdown_leaves_the_mark_it_wrote() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![agent_start()]);
    h.wait(&id, "the busy agent", |state| !state.settled).await;
    let actor = h.supervisor.lookup(&id).unwrap();
    let admitted_by = actor.process().unwrap();

    h.supervisor.shutdown().await;
    // The admission of a message sent just before the stop clears the mark only after the shutdown wrote it.
    h.supervisor.clear_interrupted(&actor, &admitted_by).await;

    assert_eq!(restart(&h).marked(), [id]);
}

// spec: DMN-370
#[tokio::test]
async fn a_prompt_admitted_before_omp_reports_its_turn_is_marked_and_a_local_command_is_not() {
    let h = harness(|_| {});
    let (prompted, commanded) = (h.thread(), h.thread());
    // The fake admits the prompt and reports no turn: the session still reads settled at the shutdown.
    h.supervisor
        .prompt(&prompted, prompt("go on"))
        .await
        .unwrap();
    h.supervisor
        .prompt(&commanded, prompt("/dirs"))
        .await
        .unwrap();

    h.supervisor.shutdown().await;

    assert_eq!(restart(&h).marked(), [prompted]);
}

// spec: DMN-372
#[tokio::test]
async fn a_resume_whose_caller_goes_away_still_sends_the_notice() {
    let h = harness(|_| {});
    let (busy, _idle, _stopped) = shut_down_with_a_busy_thread(&h).await;
    let restarted = restart(&h);
    let launches = restarted.launcher.gate_launches();

    tokio::select! {
        _ = restarted.supervisor.resume(&busy) => panic!("the resume ended while its launch was held"),
        () = launches.arrived(1) => {}
    }
    launches.open();
    while restarted.launcher.launches().is_empty() || restarted.prompts(0).is_empty() {
        tokio::task::yield_now().await;
    }

    assert_eq!(restarted.prompts(0), [RESUME_NOTICE]);
    assert_eq!(restarted.marked(), Vec::<String>::new());
}

// spec: DMN-370, DMN-005
#[tokio::test(start_paused = true)]
async fn a_send_that_hangs_holds_the_marking_back_only_for_a_while() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![agent_start()]);
    h.wait(&id, "the busy agent", |state| !state.settled).await;
    // A message whose send to omp does not come back holds the marks (shared).
    let hung = h.supervisor.inner.interrupted_marks.read().await;
    let started = Instant::now();

    h.supervisor.shutdown().await;

    assert!(
        started.elapsed() <= SHUTDOWN_GRACE,
        "{:?}",
        started.elapsed()
    );
    drop(hung);
    assert_eq!(restart(&h).marked(), [id]);
}

// spec: DMN-373
#[tokio::test]
async fn a_message_whose_caller_goes_away_still_clears_the_mark() {
    let h = harness(|_| {});
    let (busy, _idle, _stopped) = shut_down_with_a_busy_thread(&h).await;
    let restarted = restart(&h);
    let launches = restarted.launcher.gate_launches();

    tokio::select! {
        _ = restarted.supervisor.prompt(&busy, prompt("go on")) => panic!("the prompt ended while its launch was held"),
        () = launches.arrived(1) => {}
    }
    launches.open();
    while !restarted.marked().is_empty() {
        tokio::task::yield_now().await;
    }

    assert_eq!(restarted.prompts(0), ["go on"]);
}

/// `h`'s thread `id` once its omp, the `index`th process, exited on its own and the thread failed.
async fn crashed(h: &Harness, id: &str, index: usize) {
    h.launcher.process(index).exit();
    h.wait(id, "the failure", |state| {
        matches!(state.process, ProcessState::Failed { .. })
    })
    .await;
}

// spec: DMN-370, DMN-377
#[tokio::test]
async fn a_turn_whose_omp_dies_with_the_daemon_is_marked_as_a_daemon_stop() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![agent_start()]);
    h.wait(&id, "the busy agent", |state| !state.settled).await;
    let actor = h.supervisor.lookup(&id).unwrap();
    // A message on its way to omp holds the marking back while omp dies with the daemon (one signal to them all).
    let held = h.supervisor.inner.interrupted_marks.read().await;
    let stopping = h.supervisor.shutdown();
    tokio::pin!(stopping);

    tokio::select! {
        () = &mut stopping => panic!("the shutdown ended before omp died"),
        () = async {
            while !h.supervisor.inner.shutting_down.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
            h.launcher.process(0).exit();
            while actor.process().is_some() {
                tokio::task::yield_now().await;
            }
        } => {}
    }
    drop(held);
    stopping.await;
    h.wait(&id, "the failure", |state| {
        matches!(state.process, ProcessState::Failed { .. })
    })
    .await;

    let restarted = restart(&h);
    assert_eq!(restarted.marked(), std::slice::from_ref(&id));
    restarted.supervisor.resume(&id).await.unwrap();
    assert_eq!(restarted.prompts(0), [RESUME_NOTICE]);
}

// spec: DMN-377, DMN-372
#[tokio::test]
async fn an_omp_crashing_mid_turn_marks_it_and_resume_names_the_crash() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![agent_start()]);
    h.wait(&id, "the busy agent", |state| !state.settled).await;
    let mut changes = h.supervisor.subscribe_changes();
    let before = crate::store::now_ms();

    crashed(&h, &id, 0).await;

    // The failure publishes the summary, the mark already in it.
    while changes.recv().await.unwrap() != id {}
    let restarted = restart(&h);
    assert_eq!(restarted.marked(), std::slice::from_ref(&id));
    let at = restarted.store.interrupted_turns().unwrap()[&id];
    assert!(at >= before, "marked at the crash: {at} < {before}");
    // A later daemon stop finds the thread failed and settled: the crash stays the cause.
    h.supervisor.shutdown().await;
    let restarted = restart(&h);
    restarted.supervisor.resume(&id).await.unwrap();
    assert_eq!(restarted.prompts(0), [CRASH_RESUME_NOTICE]);
}

// spec: DMN-377, DMN-374
#[tokio::test]
async fn an_omp_exiting_marks_a_turn_it_was_sent_and_not_a_settled_thread() {
    let h = harness(|_| {});
    let (settled, prompted) = (h.thread(), h.thread());
    h.supervisor.start(&settled).await.unwrap();
    // The fake admits the prompt and reports no turn: the session still reads settled when omp exits.
    h.supervisor
        .prompt(&prompted, prompt("go on"))
        .await
        .unwrap();

    crashed(&h, &settled, 0).await;
    crashed(&h, &prompted, 1).await;

    assert_eq!(restart(&h).marked(), [prompted]);
}

// spec: DMN-377, DMN-373
#[tokio::test]
async fn a_message_admitted_by_an_omp_that_crashed_since_leaves_the_crash_mark() {
    let h = harness(|_| {});
    let id = h.thread();
    h.supervisor.start(&id).await.unwrap();
    h.launcher.process(0).emit(vec![agent_start()]);
    h.wait(&id, "the busy agent", |state| !state.settled).await;
    let actor = h.supervisor.lookup(&id).unwrap();
    let admitted_by = actor.process().unwrap();

    crashed(&h, &id, 0).await;
    // The bookkeeping of a message omp admitted just before it died comes after the crash marked the turn.
    h.supervisor.clear_interrupted(&actor, &admitted_by).await;

    assert_eq!(restart(&h).marked(), [id]);
}
