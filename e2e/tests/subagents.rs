//! A subagent in Background (E2E-42), with the pinned omp and the fake LLM: the agent delegates with `task`; the
//! subagent's row of `omfp:background` opens its transcript and steer field under it while it runs (UI-490, UI-491),
//! and once it ends the transcript reads its answer and the steer field goes. No other widget or tab opens.

use e2e::Journey;
use fake_llm::{Matcher, Reply, Step, ToolCall};
use serde_json::json;

const ASSIGNMENT: &str = "Name the colour of a clear sky";
const ROW: &str = "[data-testid=background-row][data-kind=subagent]";
const DETAIL: &str = "[data-testid=background-subagent]";

/// Waits until a transcript line of the open subagent contains `needle`.
async fn line_with(journey: &Journey, needle: &str) -> anyhow::Result<()> {
    let lines = format!("{DETAIL} [data-testid=subagent-line]");
    journey
        .until(&format!("a subagent line with {needle:?}"), async |page| {
            let texts = page.texts(&lines).await?;
            Ok(texts.iter().any(|text| text.contains(needle)).then_some(()))
        })
        .await
}

// spec: E2E-42, UI-490, UI-491
#[tokio::test(flavor = "multi_thread")]
async fn a_subagent_opens_under_its_background_row_with_its_transcript_and_steer_field()
-> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_task_1",
            "task",
            json!({
                "context": "Colours.",
                "tasks": [{ "name": "Sky", "task": ASSIGNMENT, "solutionSpace": "one word" }],
                "i": "Delegating the colour"
            }),
        )])),
        // The subagent's answer, held so it is seen running.
        Step::when(
            Matcher::LastUserContains(ASSIGNMENT.into()),
            Reply::text("SKY-IS-BLUE"),
        )
        .gated(),
        Step::reply(Reply::text("The sky is blue.")),
    ])
    .await?;
    // A blocking task: the parent's turn ends after the subagent's.
    journey.env.configure(&[], "async:\n  enabled: false\n")?;
    journey.connected().await?;
    let thread = journey.create_thread("subagents journey").await?;
    journey.send("what colour is the sky?").await?;
    journey.fake.wait_for_gate(1).await;

    // The badge shows the default view's Background tab with the running subagent; its row opens it in place.
    journey
        .until("the badge", async |page| {
            let badge = page.text("[data-testid=background-badge]").await?;
            Ok((badge == "1 in background").then_some(()))
        })
        .await?;
    journey.click("[data-testid=background-badge]").await?;
    journey
        .element(&format!("{ROW}[data-status=running]"))
        .await?;
    assert_eq!(
        journey.attr(ROW, "aria-expanded").await?.as_deref(),
        Some("false")
    );
    journey.click(ROW).await?;
    assert_eq!(
        journey.attr(ROW, "aria-expanded").await?.as_deref(),
        Some("true")
    );
    line_with(&journey, ASSIGNMENT).await?;
    journey
        .element(&format!("{DETAIL} [data-testid=subagent-steer-input]"))
        .await?;
    assert_eq!(journey.count("[data-testid=background-cancel]").await?, 1);
    journey.screenshot("subagent-running").await?;

    // Its answer: the row completes, the transcript reads what is new, nothing to steer any more.
    journey.fake.release_gate(1);
    journey
        .element(&format!("{ROW}[data-status=completed]"))
        .await?;
    line_with(&journey, "SKY-IS-BLUE").await?;
    journey
        .gone(&format!("{DETAIL} [data-testid=subagent-steer-input]"))
        .await?;
    journey.reply("The sky is blue.").await?;
    journey.status(&thread, "idle").await?;
    // All of it in Background: no widget tab, no floating panel.
    assert_eq!(journey.count("[data-testid=widget-tab]").await?, 0);
    assert_eq!(journey.count("[data-testid=floating-widget]").await?, 0);
    journey.screenshot("subagent-done").await?;
    journey.finish("E2E-42").await
}
