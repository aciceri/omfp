//! The workspace jobs journey (E2E-72, M18): a declared command job shows as a collapsed `Jobs (1)` row after its
//! workspace's `+ thread`, whose job row opens `omfp:jobs` on that job; the job runs from its row and its run's output
//! shows under it. Then the thread's agent starts a transient run with `omfp_call`, archiving the thread asks what to
//! do with it, and *Keep them running* archives the thread while the run goes on.

use anyhow::Context as _;
use e2e::Journey;
use fake_llm::{Reply, Step, ToolCall};
use omfp_protocol::method::{JobsDefine, Subscribe, ThreadsList};
use omfp_protocol::{DefineJobParams, ListThreadsParams, SubscribeParams, SubscribeResult, Topic};
use serde_json::json;

/// The gated reply the test releases with the agent's `jobs.start`, once the workspace's id is known.
const START_STEP: usize = 0;
const WATCH: &str = "echo watching; exec sleep 3600";
/// The main slot's tab of `omfp:jobs` a job's row in the left column reveals (UI-741).
const JOBS_TAB: &str = "[data-slot=main] [data-testid=widget-tab][data-tab=\"widget:omfp:jobs\"]";

fn job_row(name: &str) -> String {
    format!("[data-testid=jobs] [data-testid=job-row][data-job={name}]")
}

/// The section `css`'s rows and head: `+ thread` then `Jobs (N)`, the toggle's glyph where the thread row's mark sits
/// (no extra indent); with hover, the count flush right at rest, the menu and delete flush right in its place while
/// `hovered`.
async fn section_layout(journey: &Journey, css: &str, hovered: bool) -> anyhow::Result<()> {
    let problems = journey
        .eval(
            "const [css, hovered] = arguments;
             const section = document.querySelector(css);
             const box = (e) => e.getBoundingClientRect();
             const head = section.querySelector('.omfp-wsection__head');
             const count = head.querySelector('.omfp-wsection__count');
             const actions = head.querySelector('.omfp-wsection__actions');
             const add = section.querySelector('[data-testid=add-thread]');
             const toggle = section.querySelector('[data-testid=jobs-toggle]');
             const glyph = toggle.querySelector('.omfp-toggle-row__glyph');
             const mark = section.querySelector('[data-testid=thread-status]');
             const shown = (e) => getComputedStyle(e).display !== 'none';
             const flush = (e) => Math.abs(box(e).right - box(head).right) <= 1;
             const problems = [];
             if (box(add).bottom > box(toggle).top + 0.5) problems.push('Jobs (N) is not after + thread');
             if (Math.abs(box(glyph).left - box(mark).left) > 1) {
               problems.push(`the glyph starts at ${box(glyph).left}, the thread's mark at ${box(mark).left}`);
             }
             if (!matchMedia('(hover: hover)').matches) return problems;
             if (hovered) {
               if (shown(count) || !shown(actions)) problems.push('the hovered head keeps its count');
               else if (!flush(actions)) problems.push('the actions are not flush right');
             } else if (!shown(count) || shown(actions)) {
               problems.push('the resting head shows its actions');
             } else if (!flush(count)) {
               problems.push(`the count ends at ${box(count).right} of ${box(head).right}`);
             }
             return problems;",
            vec![json!(css), json!(hovered)],
        )
        .await?;
    anyhow::ensure!(problems == json!([]), "{problems}");
    Ok(())
}

// spec: E2E-72, UI-020, UI-170, UI-442, UI-740, UI-741, UI-742, UI-743, UI-744, PRT-208, DMN-327, DMN-333
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // One journey, step by step.
async fn a_declared_job_runs_from_its_row_and_a_thread_closes_keeping_its_agents_run()
-> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::text("unused")).gated(),
        Step::reply(Reply::text("The watcher runs.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("jobs journey").await?;
    let mut connection = journey.connect().await?;
    let workspace = connection
        .call::<ThreadsList>(ListThreadsParams::default())
        .await?
        .threads
        .into_iter()
        .find(|summary| summary.id == thread)
        .context("the thread is listed")?
        .workspace_id;

    // `Jobs (1)` after `+ thread`, collapsed, without marks: nothing runs or failed (UI-740).
    connection
        .call::<JobsDefine>(DefineJobParams {
            workspace_id: workspace.clone(),
            name: "greet".to_owned(),
            job: json!({ "description": "Says hello", "command": "echo hello-from-job" }),
            replace: None,
        })
        .await?;
    let section = format!("[data-testid=workspace-section][data-workspace-id=\"{workspace}\"]");
    let toggle = format!("{section} [data-testid=jobs-toggle]");
    journey
        .until("the workspace's Jobs row", async |page| {
            Ok((page.count(&toggle).await? == 1).then_some(()))
        })
        .await?;
    assert_eq!(
        journey.text(&format!("{toggle} .omfp-row__title")).await?,
        "Jobs (1)"
    );
    assert_eq!(
        journey
            .attr(&format!("{toggle} .omfp-row__title [title]"), "title")
            .await?
            .as_deref(),
        Some("1 job: 1 manual")
    );
    assert_eq!(
        journey.attr(&toggle, "aria-expanded").await?.as_deref(),
        Some("false")
    );
    assert_eq!(
        journey
            .count(&format!("{toggle} [data-testid=job-counts]"))
            .await?,
        0
    );
    // The rows line up and the header's count is flush right, its menu and delete in its place once hovered
    // (UI-020, UI-170, UI-442).
    journey.hover(&toggle).await?;
    section_layout(&journey, &section, false).await?;
    journey
        .hover(&format!("{section} [data-testid=workspace-section-name]"))
        .await?;
    section_layout(&journey, &section, true).await?;

    // Expanded, its job's row reveals `omfp:jobs` in a closable tab of the main slot (UI-741) with that row open
    // (UI-742); the row runs the job and shows its output (UI-743).
    journey.click(&toggle).await?;
    let side =
        format!("{section} [data-testid=jobs-list] [data-testid=job-side-row][data-job=greet]");
    journey.settled(&side).await?;
    assert_eq!(
        journey.text(&format!("{side} .omfp-row__meta")).await?,
        "idle"
    );
    journey
        .click(&format!("{side} [data-testid=job-side-open]"))
        .await?;
    journey
        .element(&format!("{JOBS_TAB}[aria-selected=true]"))
        .await?;
    let row = job_row("greet");
    journey.settled(&row).await?;
    assert_eq!(
        journey
            .attr(&format!("{row} [data-testid=job-toggle]"), "aria-expanded")
            .await?
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        journey
            .text(&format!("{row} [data-testid=job-action]"))
            .await?,
        "echo hello-from-job"
    );
    journey
        .click(&format!("{row} [data-testid=job-run]"))
        .await?;
    let last = format!("{row} [data-testid=job-last-run][data-status=succeeded]");
    journey
        .until("the run to succeed", async |page| {
            Ok((page.count(&last).await? == 1).then_some(()))
        })
        .await?;
    let run = format!("{row} [data-testid=job-run-row][data-status=succeeded]");
    journey.settled(&run).await?;
    journey
        .click(&format!("{run} [data-testid=job-run-output-toggle]"))
        .await?;
    let output = format!("{run} [data-testid=job-run-output]");
    journey
        .until("the run's output", async |page| {
            Ok(page
                .text(&output)
                .await?
                .contains("hello-from-job")
                .then_some(()))
        })
        .await?;
    journey.screenshot("jobs-run-output").await?;
    // The tab closes by its own ×, back to the thread.
    journey
        .click(&format!("{JOBS_TAB} [data-testid=widget-tab-close]"))
        .await?;
    journey.gone(JOBS_TAB).await?;
    journey.opened(&thread).await?;

    // The agent starts a transient run (PRT-208's `started_by`).
    journey.send("start the watcher").await?;
    journey.fake.wait_for_gate(START_STEP).await;
    journey.fake.release_gate_with(
        START_STEP,
        Reply::tool_calls(vec![ToolCall::new(
            "call_watch",
            "omfp_call",
            json!({ "method": "jobs.start", "params": { "workspaceId": workspace, "command": WATCH } }),
        )]),
    );
    journey.reply("The watcher runs.").await?;
    journey.status(&thread, "idle").await?;

    // Archiving the thread asks once; *Keep them running* archives it and the run goes on (UI-744, DMN-333).
    journey.command("archive").await?;
    journey.settled("[data-testid=close-runs-dialog]").await?;
    let listed = journey
        .texts("[data-testid=close-runs-list] [data-testid=close-run]")
        .await?;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(listed[0].contains(WATCH), "{listed:?}");
    let run_id = journey
        .attr("[data-testid=close-run]", "data-run")
        .await?
        .context("the listed run's id")?;
    journey.screenshot("jobs-close-runs").await?;
    journey.click("[data-testid=close-runs-keep]").await?;
    journey.gone("[data-testid=close-runs-dialog]").await?;
    journey
        .until("the thread to be archived", async |page| {
            let mut connection = page.connect().await?;
            let archived = connection
                .call::<ThreadsList>(ListThreadsParams {
                    workspace_id: Some(workspace.clone()),
                    include_archived: true,
                })
                .await?
                .threads
                .into_iter()
                .any(|summary| summary.id == thread && summary.archived_at.is_some());
            connection.close().await?;
            Ok(archived.then_some(()))
        })
        .await?;
    let SubscribeResult::Snapshot { state, .. } = connection
        .call::<Subscribe>(SubscribeParams {
            topic: Topic::Job(run_id.clone()),
            since: None,
        })
        .await?
    else {
        anyhow::bail!("job/{run_id}: a snapshot was expected");
    };
    assert_eq!(
        state["job"]["status"],
        json!({ "state": "running" }),
        "{state}"
    );
    connection.close().await?;
    journey.finish("jobs").await
}
