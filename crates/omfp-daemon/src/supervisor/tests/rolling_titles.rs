//! Rolling titles through admitted prompts and the fake title model; no real model or elapsed wall time.

use super::*;

const WORK: &str = "Implement the checkout payment service with idempotent requests, transaction recovery, and deterministic integration coverage.";
const SHIFT: &str = "Now build the deployment pipeline: publish containers, configure staging environments, and verify rollback procedures for this project.";

async fn batch(h: &Harness, id: &str, text: &str) {
    tokio::time::advance(Duration::from_secs(300)).await;
    for _ in 0..4 {
        h.supervisor.prompt(id, prompt(text)).await.unwrap();
    }
    h.supervisor.title_generated(id).await;
}

// spec: DMN-073
#[tokio::test(start_paused = true)]
async fn rolling_title_refresh_requires_conversation_and_confirmed_shift_and_names_the_session() {
    let h = harness(|_| {});
    let (id, _) = h.untitled_thread();
    for title in [
        "Checkout payments",
        "Explain a regex",
        "Deployment pipeline",
        "Deployment pipeline",
    ] {
        h.launcher.push_title(Ok(title));
    }
    h.supervisor.prompt(&id, prompt(WORK)).await.unwrap();
    h.supervisor.title_generated(&id).await;
    assert_eq!(
        h.title(&id),
        ("Checkout payments".into(), TitleSource::Generated)
    );
    let mut changes = h.supervisor.subscribe_changes();

    tokio::time::advance(Duration::from_hours(24)).await;
    assert_eq!(
        h.launcher.title_requests().len(),
        1,
        "idle time does not generate"
    );
    // One isolated digression is considered only with a batch, and cannot publish a new topic.
    for _ in 0..3 {
        h.supervisor.prompt(&id, prompt(WORK)).await.unwrap();
    }
    assert_eq!(h.launcher.title_requests().len(), 1);
    h.supervisor.prompt(&id, prompt("Explain a regex briefly before continuing the checkout implementation and its recovery tests.")).await.unwrap();
    h.supervisor.title_generated(&id).await;
    assert_eq!(h.title(&id).0, "Checkout payments");
    assert_eq!(h.session_names(), ["Checkout payments"]);

    batch(&h, &id, SHIFT).await;
    assert_eq!(
        h.title(&id).0,
        "Checkout payments",
        "a new candidate needs confirmation"
    );
    assert!(
        matches!(
            changes.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ),
        "idle time, digressions and pending candidates must not publish index changes"
    );
    batch(&h, &id, SHIFT).await;
    assert_eq!(
        h.title(&id),
        ("Deployment pipeline".into(), TitleSource::Generated)
    );
    assert_eq!(
        h.session_names(),
        ["Checkout payments", "Deployment pipeline"]
    );
    assert_eq!(
        changes.try_recv().unwrap(),
        id,
        "the index is notified after session naming"
    );
    let requests = h.launcher.title_requests();
    let context = &requests.last().unwrap().message;
    assert!(context.contains("Current title: Checkout payments"));
    assert!(context.contains(WORK), "earlier project continuity");
    assert!(
        context.contains(SHIFT),
        "recent work must not be truncated away"
    );
}

// spec: DMN-073
#[tokio::test(start_paused = true)]
async fn rolling_title_refresh_caps_attempts_deduplicates_and_keeps_the_title_on_failure() {
    let h = harness(|_| {});
    let (id, _) = h.untitled_thread();
    for title in [
        Ok("Checkout payments"),
        Ok("CHECKOUT payments!"),
        Ok("Checkout payment"),
        Err("model unavailable"),
    ] {
        h.launcher.push_title(title);
    }
    h.supervisor.prompt(&id, prompt(WORK)).await.unwrap();
    h.supervisor.title_generated(&id).await;
    for _ in 0..12 {
        h.supervisor.prompt(&id, prompt(WORK)).await.unwrap();
    }
    assert_eq!(
        h.launcher.title_requests().len(),
        1,
        "conversation cannot bypass cooldown"
    );
    for _ in 0..3 {
        batch(&h, &id, WORK).await;
    }
    assert_eq!(h.launcher.title_requests().len(), 4);
    assert_eq!(
        h.title(&id),
        ("Checkout payments".into(), TitleSource::Generated)
    );
    assert_eq!(
        h.session_names(),
        ["Checkout payments"],
        "unchanged titles never reach the session"
    );
    batch(&h, &id, "ok").await;
    assert_eq!(
        h.launcher.title_requests().len(),
        4,
        "short acknowledgements are not enough context"
    );
}

// spec: DMN-073
#[tokio::test(start_paused = true)]
async fn rolling_title_refresh_manual_rename_wins_in_flight_and_disables_future_calls() {
    let h = harness(|_| {});
    let (id, _) = h.untitled_thread();
    for title in [
        "Checkout payments",
        "Deployment pipeline",
        "Deployment pipeline",
    ] {
        h.launcher.push_title(Ok(title));
    }
    h.supervisor.prompt(&id, prompt(WORK)).await.unwrap();
    h.supervisor.title_generated(&id).await;
    batch(&h, &id, SHIFT).await;
    let gate = h.launcher.gate_titles();
    tokio::time::advance(Duration::from_secs(300)).await;
    for _ in 0..4 {
        h.supervisor.prompt(&id, prompt(SHIFT)).await.unwrap();
    }
    gate.arrived(1).await;
    h.supervisor
        .set_title(&id, "My project".into(), TitleSource::Set)
        .await
        .unwrap();
    gate.open();
    h.supervisor.title_generated(&id).await;
    batch(&h, &id, SHIFT).await;
    assert_eq!(h.title(&id), ("My project".into(), TitleSource::Set));
    assert_eq!(h.session_names(), ["Checkout payments", "My project"]);
    assert_eq!(h.launcher.title_requests().len(), 3);
}

// spec: DMN-073
#[tokio::test(start_paused = true)]
async fn rolling_title_refresh_never_overlaps_and_resumes_an_existing_generated_thread() {
    let h = harness(|_| {});
    let (id, _) = h.untitled_thread();
    h.threads
        .rename(&id, "Checkout payments", TitleSource::Generated)
        .unwrap();
    h.launcher.push_title(Ok("Deployment pipeline"));
    let gate = h.launcher.gate_titles();
    tokio::time::advance(Duration::from_secs(300)).await;
    for _ in 0..4 {
        h.supervisor.prompt(&id, prompt(SHIFT)).await.unwrap();
    }
    gate.arrived(1).await;
    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(300)).await;
        h.supervisor.prompt(&id, prompt(SHIFT)).await.unwrap();
    }
    assert_eq!(h.launcher.title_requests().len(), 1);
    gate.open();
    h.supervisor.title_generated(&id).await;
    assert_eq!(h.title(&id).0, "Checkout payments");
}

// spec: DMN-073
#[tokio::test]
async fn rolling_title_refresh_session_senders_read_the_latest_manual_title() {
    let h = harness(|_| {});
    let (id, _) = h.untitled_thread();
    h.launcher.push_title(Ok("Checkout payments"));
    h.supervisor.prompt(&id, prompt(WORK)).await.unwrap();
    h.supervisor.title_generated(&id).await;
    let actor = h.supervisor.lookup(&id).unwrap();
    let held = actor.title_sync.lock().await;
    let supervisor = h.supervisor.clone();
    let delayed_id = id.clone();
    let delayed = tokio::spawn(async move { supervisor.name_session(&delayed_id).await });
    // The agent's synchronous store write can precede its queued session-name task.
    let setter = h.supervisor.title_setter(&id);
    setter("Manual project name").unwrap();
    drop(held);
    delayed.await.unwrap();
    h.supervisor.name_session(&id).await;
    assert_eq!(
        h.title(&id),
        ("Manual project name".into(), TitleSource::Set)
    );
    assert!(
        h.session_names()[1..]
            .iter()
            .all(|name| name == "Manual project name")
    );
}

// spec: DMN-073, DMN-226
#[tokio::test(start_paused = true)]
async fn rolling_title_refresh_counts_steer_and_follow_up_but_not_local_commands() {
    let h = harness(|_| {});
    let (id, _) = h.untitled_thread();
    h.launcher.push_title(Ok("Checkout payments"));
    h.launcher.push_title(Ok("Deployment pipeline"));
    h.supervisor.prompt(&id, prompt(WORK)).await.unwrap();
    h.supervisor.title_generated(&id).await;
    tokio::time::advance(Duration::from_secs(300)).await;
    for _ in 0..8 {
        h.supervisor.prompt(&id, prompt("/dirs")).await.unwrap();
    }
    assert_eq!(h.launcher.title_requests().len(), 1);
    for _ in 0..2 {
        h.supervisor.queue(&id, steer(SHIFT)).await.unwrap();
        h.supervisor
            .queue(
                &id,
                QueuedMessage::FollowUp(FollowUpCommand {
                    message: SHIFT.into(),
                    images: None,
                }),
            )
            .await
            .unwrap();
    }
    h.supervisor.title_generated(&id).await;
    assert_eq!(h.launcher.title_requests().len(), 2);
    assert_eq!(
        h.title(&id).0,
        "Checkout payments",
        "first changed candidate stays pending"
    );
}
