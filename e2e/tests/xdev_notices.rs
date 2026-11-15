//! UI-043: presentation-only xdev filtering, changed live from the settings command line.

use e2e::Journey;
use omfp_core::wire::RpcNotification;
use omfp_daemon::agent::{FakeLauncher, FakeRun};
use omfp_daemon::config_file::ConfigFile;
use serde_json::json;

const SUMMARIES: &str = "[data-testid=system-summary]";

fn notice_frames() -> serde_json::Result<Vec<RpcNotification>> {
    [
        json!({ "type": "notice", "level": "info", "source": "xdev", "message": "xd://: mounted mcp__test" }),
        json!({ "type": "notice", "level": "info", "source": "xdev", "message": "xd://: unmounted mcp__test\nNo devices." }),
        json!({ "type": "notice", "level": "warning", "source": "xdev", "message": "mount warning" }),
    ].into_iter().map(serde_json::from_value).collect()
}

// spec: UI-043, UI-097
#[tokio::test(flavor = "multi_thread")]
async fn settings_reveal_retained_xdev_notices_without_restarting_omp() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    let cwd = journey.data_dir().join("workspaces/xdev-notices");
    launcher.push_run_for(
        cwd.clone(),
        FakeRun::fresh().with_history(vec![serde_json::from_value(json!({
            "role": "custom", "customType": "xdev-mount-notice", "display": false,
            "content": "<system-notice>\nDevice inventory\n</system-notice>"
        }))?]),
    );
    journey.connected().await?;
    let thread_id = journey.create_thread("xdev-notices").await?;
    let log = journey.daemon().supervisor().thread_log(&thread_id).await?;
    let position = log.position();
    journey
        .fake_process(&launcher, &thread_id)
        .await?
        .emit(notice_frames()?);
    journey
        .until("the warning after the hidden notices", async |page| {
            Ok(page
                .text("[data-testid=transcript]")
                .await?
                .contains("mount warning")
                .then_some(()))
        })
        .await?;
    assert_eq!(journey.texts(SUMMARIES).await?, ["Device inventory"]);
    let retained_state = log.with_state(Clone::clone);
    let retained_events = log.subscribe(Some(&position)).start;
    journey.screenshot("xdev-notices-default").await?;

    journey.command("set show_xdev_notices=true").await?;
    journey
        .until("retained notices revealed as system rows", async |page| {
            Ok((page.texts(SUMMARIES).await?
                == [
                    "Device inventory",
                    "xd://: mounted mcp__test",
                    "xd://: unmounted mcp__test",
                ])
            .then_some(()))
        })
        .await?;
    journey
        .click("[data-testid=system-record]:has([data-testid=system-text]) [data-testid=toggle]")
        .await?;
    assert!(
        journey
            .text("[data-testid=transcript]")
            .await?
            .contains("No devices.")
    );
    assert_eq!(
        ConfigFile::read(&journey.config_file())
            .map_err(anyhow::Error::msg)?
            .map(|file| file.ui.show_xdev_notices),
        Some(Some(true))
    );
    journey.screenshot("xdev-notices-shown").await?;

    journey.command("set show_xdev_notices=false").await?;
    journey
        .until("xdev notices hidden again", async |page| {
            Ok((page.texts(SUMMARIES).await? == ["Device inventory"]).then_some(()))
        })
        .await?;
    assert!(
        journey
            .text("[data-testid=transcript]")
            .await?
            .contains("mount warning")
    );
    assert_eq!(
        ConfigFile::read(&journey.config_file())
            .map_err(anyhow::Error::msg)?
            .map(|file| file.ui.show_xdev_notices),
        Some(Some(false))
    );
    assert_eq!(
        launcher
            .launches()
            .iter()
            .filter(|launch| launch.spec.cwd == cwd)
            .count(),
        1,
        "settings never restart the thread's omp"
    );
    assert_eq!(
        log.with_state(Clone::clone),
        retained_state,
        "presentation leaves reducer state intact"
    );
    assert_eq!(
        log.subscribe(Some(&position)).start,
        retained_events,
        "presentation leaves event replay intact"
    );
    journey.screenshot("xdev-notices-hidden").await?;
    journey.finish("xdev notice visibility").await
}
