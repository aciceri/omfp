//! Finished background work (E2E-63), against a daemon on the fake launcher: a thread with many background jobs keeps
//! what runs and the most recently finished ones in sight in `omfp:background`, the older ones collapsed under
//! `Finished (N)` (UI-641); Clear finished hides them and the window remembers it across a reload (UI-642).

use e2e::Journey;
use omfp_core::wire::RpcNotification;
use omfp_daemon::agent::FakeLauncher;
use serde_json::Value;

const ROWS: &str = "[data-testid=background] [data-testid=background-row]";

/// The tool frames of omp's recorded `bash` call with `async` (the `async_bash_tool_call` fixture) as job `bg_<n>` of
/// its own call; with `finish` false, up to its last running update.
fn job(n: usize, finish: bool) -> anyhow::Result<Vec<RpcNotification>> {
    let path = format!(
        "{}/../fixtures/threads/{}/async_bash_tool_call.jsonl",
        env!("CARGO_MANIFEST_DIR"),
        omfp_core::OMP_VERSION
    );
    let text = std::fs::read_to_string(path)?
        .replace("call_bash_bg", &format!("call_bash_bg{n}"))
        .replace("\"bg_1\"", &format!("\"bg_{n}\""));
    let mut frames = Vec::new();
    for line in text.lines() {
        let event: Value = serde_json::from_str(line)?;
        let frame = &event["frame"];
        let kind = frame["type"].as_str().unwrap_or_default();
        if event["kind"] != "omp" || !kind.starts_with("tool_execution") {
            continue;
        }
        if !finish && frame["partialResult"]["details"]["async"]["state"] == "completed" {
            break;
        }
        frames.push(serde_json::from_value(frame.clone())?);
    }
    Ok(frames)
}

/// The job ids of the rows `omfp:background` shows in `list` (`background-recent`, …).
async fn ids(journey: &Journey, list: &str) -> anyhow::Result<Vec<Option<String>>> {
    journey
        .attrs(
            &format!("[data-testid={list}] [data-testid=background-row]"),
            "data-id",
        )
        .await
}

// spec: E2E-63, UI-641, UI-642
#[tokio::test(flavor = "multi_thread")]
async fn finished_background_jobs_collapse_and_clear_finished_hides_them_across_a_reload()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;
    let thread = journey.create_thread("background").await?;
    // The running job starts first, so the badge stays mounted while the finished ones come and go: it renders nothing
    // while nothing runs, and a badge found between two jobs would be gone when clicked (#236).
    let mut frames = job(8, false)?;
    for n in 1..=7 {
        frames.extend(job(n, true)?);
    }
    let process = journey.fake_process(&launcher, &thread).await?;
    process.emit(frames);

    // The badge shows the Background tab: the running job, the five most recent finished, two collapsed.
    journey
        .until("the badge", async |page| {
            let badge = page.text("[data-testid=background-badge]").await?;
            Ok((badge == "1 in background").then_some(()))
        })
        .await?;
    journey.click("[data-testid=background-badge]").await?;
    journey
        .until("eight rows", async |page| {
            Ok((page.count(ROWS).await? == 8).then_some(()))
        })
        .await?;
    let some = |ids: &[&str]| {
        ids.iter()
            .map(|id| Some((*id).to_owned()))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&journey, "background-running").await?, some(&["bg_8"]));
    assert_eq!(
        ids(&journey, "background-recent").await?,
        some(&["bg_7", "bg_6", "bg_5", "bg_4", "bg_3"])
    );
    let toggle = "[data-testid=background-older-toggle]";
    assert_eq!(journey.text(toggle).await?, "Finished (2)");
    journey.click(toggle).await?;
    journey
        .until("the older rows", async |page| {
            Ok((ids(page, "background-older").await? == some(&["bg_2", "bg_1"])).then_some(()))
        })
        .await?;
    journey.screenshot("background-finished").await?;

    // Clear finished leaves the running job, also after a reload.
    journey.click("[data-testid=background-clear]").await?;
    journey
        .until("one row", async |page| {
            Ok((page.count(ROWS).await? == 1).then_some(()))
        })
        .await?;
    journey.screenshot("background-cleared").await?;
    journey.driver.refresh().await?;
    journey.connected().await?;
    journey.click(&e2e::row(&thread)).await?;
    journey.click("[data-testid=background-badge]").await?;
    journey
        .until("the running job alone", async |page| {
            Ok((ids(page, "background-running").await? == some(&["bg_8"])
                && page.count(ROWS).await? == 1)
                .then_some(()))
        })
        .await?;
    assert_eq!(journey.count("[data-testid=background-clear]").await?, 0);
    journey.finish("finished background work").await
}
