//! Reopening a thread whose saved model omp cannot use (E2E-73, issue #95), against a daemon on the fake launcher:
//! every start without an explicit model fails as omp's `SessionModelRestoreError` does. Opening the thread still shows
//! its conversation, read from the session file (DMN-337); the header marks the saved model unavailable and offers the
//! usable ones and Log in (UI-751); picking one stores it and starts omp with it on the same conversation (DMN-339).

use e2e::{Journey, row};
use omfp_core::wire::AgentMessage;
use omfp_daemon::agent::{FakeLauncher, FakeReply, FakeRun};
use omfp_protocol::method::ThreadsCreate;
use omfp_protocol::{CreateThreadParams, CreateWorkspaceParams, ThreadWorkspace};
use serde_json::{Value, json};

const SAVED: &str = "anthropic/claude-opus-5-5";
const NOTE: &str = "[data-testid=model-unavailable]";

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }], "timestamp": 1 })
}

fn assistant(text: &str) -> Value {
    json!({
        "role": "assistant", "content": [{ "type": "text", "text": text }], "api": "anthropic-messages",
        "provider": "anthropic", "model": "claude-opus-5-5", "stopReason": "stop", "timestamp": 2,
        "usage": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
                   "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 } }
    })
}

/// A fake omp that cannot restore the saved model (every start without `--model` fails), lists `fake/scripted` as
/// usable and, once started, names it in `get_state`.
fn launcher() -> FakeLauncher {
    let launcher = FakeLauncher::new();
    launcher.fail_launches_without_model(&format!(
        "SessionModelRestoreError: Could not restore model {SAVED}"
    ));
    launcher.reply(
        "get_available_models",
        FakeReply::Ok(json!({ "models": [{
            "id": "scripted", "name": "Scripted", "api": "openai-completions", "provider": "fake",
            "baseUrl": "http://127.0.0.1:4000/v1", "reasoning": false, "input": ["text"],
            "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 },
            "contextWindow": 128_000, "maxTokens": 8192
        }]})),
    );
    launcher.reply(
        "get_state",
        FakeReply::Ok(json!({
            "sessionId": "S-old", "thinkingLevel": "off",
            "model": { "id": "scripted", "name": "Scripted", "provider": "fake" }
        })),
    );
    launcher
}

/// A thread whose session file was saved with [`SAVED`], one question and its answer; the process a picked model
/// starts resumes that conversation. Returns the thread's id.
async fn saved_thread(journey: &Journey, launcher: &FakeLauncher) -> anyhow::Result<String> {
    let mut connection = journey.connect().await?;
    let thread = connection
        .call::<ThreadsCreate>(CreateThreadParams {
            title: Some("old conversation".to_owned()),
            workspace: ThreadWorkspace::New(CreateWorkspaceParams {
                name: Some("unavailable".to_owned()),
                link: None,
            }),
            model: None,
            thinking_level: None,
            approval_mode: None,
            prompt: None,
            images: Vec::new(),
        })
        .await?
        .id;
    connection.close().await?;
    let sessions = journey
        .data_dir()
        .join("threads")
        .join(&thread)
        .join("sessions");
    std::fs::create_dir_all(&sessions)?;
    let entries = [
        json!({ "type": "session", "version": 3, "id": "S-old", "timestamp": "2026-11-11T23:25:36.429Z" }),
        json!({ "type": "model_change", "id": "m1", "parentId": null, "timestamp": "2026-11-11T23:25:36.489Z",
                "model": SAVED }),
        json!({ "type": "message", "id": "u1", "parentId": "m1", "timestamp": "2026-11-11T23:25:36.751Z",
                "message": user("What did we decide?") }),
        json!({ "type": "message", "id": "a1", "parentId": "u1", "timestamp": "2026-11-11T23:25:37.717Z",
                "message": assistant("We decided to ship on Friday.") }),
    ];
    let lines: Vec<String> = entries.iter().map(ToString::to_string).collect();
    std::fs::write(
        sessions.join("2026-11-11T23-25-36-429Z_S-old.jsonl"),
        lines.join("\n"),
    )?;
    let history = vec![
        AgentMessage::from_value(user("What did we decide?"))?,
        AgentMessage::from_value(assistant("We decided to ship on Friday."))?,
    ];
    launcher.push_run_for(
        journey.workspace_folder(&thread).await?,
        FakeRun::fresh().with_history(history),
    );
    Ok(thread)
}

// spec: E2E-73, UI-751, DMN-337, DMN-338, DMN-339
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_whose_saved_model_is_unavailable_opens_readable_and_resumes_with_a_picked_model()
-> anyhow::Result<()> {
    let launcher = launcher();
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;
    let thread = saved_thread(&journey, &launcher).await?;
    // Opening it fails omp's start, yet the conversation reads from the session file.
    journey.click(&row(&thread)).await?;
    journey.opened(&thread).await?;
    journey.reply("We decided to ship on Friday.").await?;
    journey.element(NOTE).await?;
    journey.status(&thread, "failed").await?;
    assert_eq!(
        journey.text(NOTE).await?,
        format!(
            "The saved model {SAVED} is unavailable. Choose another model or log in to resume this conversation."
        )
    );
    assert_eq!(
        journey
            .attr("[data-testid=model-select]", "data-unavailable")
            .await?,
        Some("true".to_owned())
    );
    assert_eq!(journey.count("[data-testid=header-login]").await?, 1);
    assert!(
        launcher
            .launches()
            .iter()
            .all(|launch| launch.spec.model.as_deref() != Some("fake/scripted")),
        "nothing switches the model by itself"
    );
    journey.screenshot("unavailable-model").await?;

    // The user's pick: stored, then omp starts with it on the same conversation.
    journey.click("[data-testid=model-select] button").await?;
    journey
        .click("[data-testid=select-option][data-key=\"fake/scripted\"]")
        .await?;
    journey.gone(NOTE).await?;
    journey
        .until("omp started with the picked model", async |_| {
            Ok(launcher
                .launches()
                .iter()
                .any(|launch| {
                    launch.spec.model.as_deref() == Some("fake/scripted")
                        && launch
                            .spec
                            .session_dir
                            .parent()
                            .is_some_and(|dir| dir.ends_with(&thread))
                })
                .then_some(()))
        })
        .await?;
    journey.status(&thread, "idle").await?;
    journey.reply("We decided to ship on Friday.").await?;
    journey
        .until("the picked model in the header", async |page| {
            let text = page
                .text("[data-testid=model-select] .omfp-select__value")
                .await?;
            Ok(text.contains("Scripted").then_some(()))
        })
        .await?;
    journey.screenshot("unavailable-model-resumed").await?;
    journey.finish("unavailable model").await
}
