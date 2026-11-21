//! The predictor omp answering the composer's word completions (DMN-360..DMN-363).

use super::*;

/// The `predict_word` calls a fake process received, as `(text, cursor)`.
fn predictions(process: &crate::agent::FakeProcess) -> Vec<(String, i64)> {
    process
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            FakeCall::Call(OmpCall::PredictWord(command)) => Some((command.text, command.cursor)),
            _ => None,
        })
        .collect()
}

// spec: DMN-360, DMN-361
#[tokio::test]
async fn one_omp_of_the_daemon_answers_every_word_completion() {
    let h = harness(|_| {});
    h.launcher
        .reply("predict_word", FakeReply::Ok(json!({ "suffix": "sts" })));

    let first = h.supervisor.predict_word("Run the te".into(), 10).await;
    let second = h.supervisor.predict_word("Fix the te".into(), 10).await;
    h.supervisor
        .predict_word_feedback("Run the te".into(), 10, "sts".into(), true)
        .await
        .unwrap();

    assert_eq!(
        (first.unwrap(), second.unwrap()),
        (Some("sts".to_owned()), Some("sts".to_owned()))
    );
    let launches = h.launcher.launches();
    assert_eq!(launches.len(), 1, "one omp started, on the first request");
    let spec = &launches[0].spec;
    assert_eq!(spec.cwd, h.data.predictor().join("work"));
    assert_eq!(spec.session_dir, h.data.predictor().join("sessions"));
    assert_eq!(
        spec.model.as_deref(),
        Some(crate::supervisor::login::LOGIN_MODEL)
    );
    let process = h.launcher.process(0);
    assert_eq!(
        predictions(&process),
        vec![("Run the te".to_owned(), 10), ("Fix the te".to_owned(), 10)]
    );
    assert!(process.calls().iter().any(|call| matches!(
        call,
        FakeCall::Call(OmpCall::PredictWordFeedback(feedback))
            if feedback.suggestion == "sts" && feedback.accepted
    )));

    h.supervisor.shutdown().await;
    assert!(process.is_closed(), "the daemon's shutdown stops it");
}

// spec: DMN-360
#[tokio::test]
async fn no_completion_is_null() {
    let h = harness(|_| {});
    assert_eq!(
        h.supervisor.predict_word("Run ".into(), 4).await.unwrap(),
        None
    );
}

// spec: DMN-361
#[tokio::test]
async fn a_predictor_omp_that_went_away_is_replaced_by_the_next_request() {
    let h = harness(|_| {});
    h.supervisor.predict_word("a".into(), 1).await.unwrap();
    h.launcher.process(0).exit();

    assert!(
        h.supervisor.predict_word("ab".into(), 2).await.is_err(),
        "the request it could not answer fails"
    );
    h.supervisor.predict_word("abc".into(), 3).await.unwrap();

    assert_eq!(h.launcher.launches().len(), 2);
    assert_eq!(
        predictions(&h.launcher.process(1)),
        vec![("abc".to_owned(), 3)]
    );
}

// spec: DMN-362
#[tokio::test(start_paused = true)]
async fn a_failed_start_is_answered_without_starting_omp_again_for_a_minute() {
    let h = harness(|_| {});
    // Both attempts of the start fail: with the login model, then without it.
    h.launcher.fail_next_launch("Error: no omp today");
    h.launcher.fail_next_launch("Error: no omp today");

    let error = h.supervisor.predict_word("a".into(), 1).await.unwrap_err();
    let SupervisorError::Start { stderr_tail, .. } = &error else {
        panic!("not a start failure: {error:?}");
    };
    assert!(stderr_tail.contains("no omp today"), "{stderr_tail}");
    assert!(matches!(
        h.supervisor.predict_word("ab".into(), 2).await,
        Err(SupervisorError::Start { .. })
    ));
    assert_eq!(h.launcher.launches().len(), 0, "no second start");

    tokio::time::advance(crate::supervisor::predict::PREDICTOR_RETRY_AFTER).await;
    h.supervisor.predict_word("abc".into(), 3).await.unwrap();
    assert_eq!(
        h.launcher.launches().len(),
        1,
        "started once the minute passed"
    );
}

// spec: DMN-363
#[tokio::test(start_paused = true)]
async fn the_reaper_stops_the_predictor_omp_once_unused_for_the_idle_timeout() {
    let timeout = Duration::from_mins(1);
    let h = harness(|config| config.idle_timeout = timeout);
    h.supervisor.predict_word("a".into(), 1).await.unwrap();
    let process = h.launcher.process(0);

    tokio::time::advance(timeout / 2).await;
    h.supervisor.predict_word("ab".into(), 2).await.unwrap();
    tokio::time::advance(timeout / 2).await;
    h.supervisor.reap_idle().await;
    assert!(!process.is_closed(), "used within the timeout");

    tokio::time::advance(timeout / 2).await;
    h.supervisor.reap_idle().await;
    assert!(process.is_closed(), "unused for the timeout");

    h.supervisor.predict_word("abc".into(), 3).await.unwrap();
    assert_eq!(
        h.launcher.launches().len(),
        2,
        "the next request starts another"
    );
}

// spec: DMN-363
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_does_not_wait_for_a_predictor_omp_still_starting() {
    let h = harness(|_| {});
    let launches = h.launcher.gate_launches();
    let supervisor = h.supervisor.clone();
    let request = tokio::spawn(async move { supervisor.predict_word("a".into(), 1).await });
    launches.arrived(1).await;

    tokio::time::timeout(Duration::from_secs(5), h.supervisor.shutdown())
        .await
        .expect("shutdown waits for no predictor start");
    launches.open();

    assert!(matches!(
        request.await.unwrap(),
        Err(SupervisorError::ShuttingDown)
    ));
    assert!(
        h.launcher.process(0).is_closed(),
        "the omp that started after shutdown is stopped"
    );
}
