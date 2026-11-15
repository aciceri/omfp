//! The first prompt uses the model selected before there is any conversation (issue #47).

use e2e::Journey;
use fake_llm::{Reply, Step};
use omfp_protocol::ThreadRef;
use omfp_protocol::method::ThreadSession;
use thirtyfour::prelude::*;

async fn select_scripted(journey: &Journey) -> anyhow::Result<()> {
    journey
        .until("the model picker to be ready", async |page| {
            Ok((page
                .count("[data-testid=model-select] button:not([disabled])")
                .await?
                == 1)
                .then_some(()))
        })
        .await?;
    journey.click("[data-testid=model-select] button").await?;
    journey
        .until("the scripted option", async |page| {
            Ok((page
                .count("[data-testid=select-option][data-key=\"fake/scripted\"]")
                .await?
                == 1)
                .then_some(()))
        })
        .await?;
    journey
        .click("[data-testid=select-option][data-key=\"fake/scripted\"]")
        .await?;
    journey
        .until("the selected model in the header", async |page| {
            Ok(page
                .texts("[data-testid=model-select] button")
                .await?
                .iter()
                .any(|text| text.contains("Scripted fake model"))
                .then_some(()))
        })
        .await
}

// spec: UI-023, UI-030
#[tokio::test(flavor = "multi_thread")]
async fn selecting_the_model_before_the_first_prompt_works_for_drafts_and_created_threads()
-> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::text("Draft used the selected model.")),
        Step::reply(Reply::text("Created thread used the selected model.")),
    ])
    .await?;
    journey.connected().await?;
    // A different default makes a visually-only selection fail: the fake LLM accepts only `scripted`.
    let models = journey.env.agent_dir().join("models.yml");
    let yaml = std::fs::read_to_string(&models)?;
    std::fs::write(
        models,
        format!(
            "{yaml}      - id: default\n        name: Other default\n        reasoning: false\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 8192\n"
        ),
    )?;
    std::fs::write(
        journey.env.agent_dir().join("config.yml"),
        "modelRoles:\n  default: fake/default\n",
    )?;

    journey
        .type_into("[data-testid=new-workspace-input]", "initial model")
        .await?;
    journey.keys(Key::Enter).await?;
    journey.element("[data-testid=draft-thread-view]").await?;
    select_scripted(&journey).await?;
    assert_eq!(journey.thread_ids().await?, Vec::<String>::new());
    assert_eq!(journey.fake.requests(), Vec::<serde_json::Value>::new());
    journey.screenshot("initial-model-draft").await?;
    let draft = journey.first_message("use my selected model").await?;
    journey.reply("Draft used the selected model.").await?;
    journey.status(&draft, "idle").await?;

    let created = journey
        .create_asking_thread("created before sending")
        .await?;
    select_scripted(&journey).await?;
    journey.send("first prompt in the created thread").await?;
    journey
        .reply("Created thread used the selected model.")
        .await?;
    journey.status(&created, "idle").await?;
    let mut connection = journey.connect().await?;
    for id in [draft, created] {
        let session = connection
            .call::<ThreadSession>(ThreadRef { thread_id: id })
            .await?;
        assert_eq!(session.model.as_deref(), Some("fake/scripted"));
    }
    connection.close().await?;
    let models: Vec<_> = journey
        .fake
        .requests()
        .iter()
        .map(|request| request["model"].clone())
        .collect();
    assert_eq!(
        models,
        vec![serde_json::json!("scripted"), serde_json::json!("scripted")]
    );
    journey.screenshot("initial-model-created").await?;
    journey.finish("initial-model").await
}

// spec: UI-023, UI-064
#[tokio::test(flavor = "multi_thread")]
async fn a_partial_first_prompt_failure_retries_the_surviving_conversation() -> anyhow::Result<()> {
    use omfp_daemon::agent::FakeLauncher;
    use omfp_protocol::{ListThreadsParams, method::ThreadsList};

    let launcher = FakeLauncher::new();
    let gate = launcher.gate("prompt");
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;
    journey
        .type_into("[data-testid=new-workspace-input]", "partial-first-prompt")
        .await?;
    journey.keys(Key::Enter).await?;
    journey.element("[data-testid=draft-thread-view]").await?;
    journey.send("keep this unsent message").await?;
    tokio::time::timeout(std::time::Duration::from_secs(10), gate.arrived(1)).await?;
    let mut connection = journey.connect().await?;
    let created = connection
        .call::<ThreadsList>(ListThreadsParams::default())
        .await?
        .threads;
    assert_eq!(created.len(), 1);
    let id = &created[0].id;
    // Crash after creation/start but before prompt admission: the real daemon returns details.threadId.
    journey.fake_process(&launcher, id).await?.exit();
    gate.open();
    journey.opened(id).await?;
    journey
        .until("the surviving thread's unsent composer", async |page| {
            Ok((page
                .eval(
                    "return document.querySelector('[data-testid=composer-input]').value;",
                    Vec::new(),
                )
                .await?
                == serde_json::json!("keep this unsent message"))
            .then_some(()))
        })
        .await?;
    assert_eq!(journey.thread_ids().await?, std::slice::from_ref(id));
    assert_eq!(journey.count("[data-testid=error]").await?, 1);
    journey
        .click("[data-testid=error] [data-testid=toast-dismiss]")
        .await?;
    journey.gone("[data-testid=error]").await?;
    journey.click("[data-testid=send]").await?;
    journey
        .until(
            "the retry to clear the surviving composer's draft",
            async |page| {
                Ok((page
                    .eval(
                        "return document.querySelector('[data-testid=composer-input]').value;",
                        Vec::new(),
                    )
                    .await?
                    == serde_json::json!(""))
                .then_some(()))
            },
        )
        .await?;
    journey.until("the retry to reach the surviving thread's running omp", async |_| {
        Ok(e2e::fake_thread(&launcher, id).filter(|process| !process.is_closed())
            .is_some_and(|process| process.calls().iter().any(|call| {
                matches!(call, omfp_daemon::agent::FakeCall::Prompt(prompt) if prompt.message == "keep this unsent message")
            })).then_some(()))
    }).await?;
    let after = connection
        .call::<ThreadsList>(ListThreadsParams::default())
        .await?
        .threads;
    assert_eq!(
        after.iter().map(|thread| &thread.id).collect::<Vec<_>>(),
        [id]
    );
    connection.close().await?;
    journey.screenshot("partial-first-prompt-recovered").await?;
    journey.finish("partial-first-prompt-recovery").await
}
