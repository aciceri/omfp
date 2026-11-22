//! A turn a daemon restart interrupted (E2E-80), on the fake launcher: the thread's agent is mid-turn when the daemon
//! stops; once a new daemon runs over the same data directory, the page reconnects and the chat shows the interruption
//! with Resume, which starts the thread's omp and sends it the continuation notice (DMN-370..DMN-372, UI-932, UI-933).

use e2e::Journey;
use omfp_core::wire::RpcNotification;
use omfp_daemon::agent::{FakeCall, FakeLauncher};
use omfp_daemon::supervisor::RESUME_NOTICE;
use serde_json::json;

const NOTICE: &str = "[data-testid=interrupted-turn]";

// spec: E2E-80, TST-119, DMN-370, DMN-372, UI-932, UI-933
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_cut_by_a_daemon_restart_shows_in_the_chat_and_resume_continues_it()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let mut journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;
    let thread = journey.create_thread("interrupted").await?;
    let start: RpcNotification = serde_json::from_value(json!({ "type": "agent_start" }))?;
    journey
        .fake_process(&launcher, &thread)
        .await?
        .emit(vec![start]);
    journey.busy().await?;
    assert_eq!(journey.count(NOTICE).await?, 0);

    let restarted = FakeLauncher::new();
    journey.restart_fake(restarted.clone()).await?;
    journey.connected().await?;
    journey.element(NOTICE).await?;
    assert!(
        journey
            .text(NOTICE)
            .await?
            .contains("interrupted when the daemon restarted"),
        "{}",
        journey.text(NOTICE).await?
    );
    journey.screenshot("interrupted-turn").await?;

    journey.click("[data-testid=resume-turn]").await?;
    let process = journey.fake_process(&restarted, &thread).await?;
    journey
        .until("the continuation notice", async |_| {
            Ok(process
                .calls()
                .iter()
                .any(|call| matches!(call, FakeCall::Prompt(prompt) if prompt.message == RESUME_NOTICE))
                .then_some(()))
        })
        .await?;
    journey.gone(NOTICE).await?;
    journey.finish("interrupted").await
}
