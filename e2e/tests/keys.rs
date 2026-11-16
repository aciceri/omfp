//! The which-key journey (06-testing.md). E2E-26: `Space` opens the leader menu at
//! once, it follows the sequence and the key picked runs its command; a prefix shows its panel after the delay and
//! Escape cancels it; `Space Space` opens the command line. Run with `just test-e2e`.

use e2e::Journey;
use thirtyfour::prelude::*;

const PANEL: &str = "[data-testid=which-key]";

/// Leaves any text field, so the window is in normal mode.
async fn normal_mode(journey: &Journey) -> anyhow::Result<()> {
    journey
        .eval(
            "if (document.activeElement) { document.activeElement.blur(); }",
            Vec::new(),
        )
        .await?;
    Ok(())
}

// spec: E2E-26, UI-320, UI-321, UI-322, UI-324, UI-325, UI-326
#[tokio::test(flavor = "multi_thread")]
async fn space_opens_the_leader_menu_and_runs_the_binding_picked() -> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    journey.create_thread("which-key journey").await?;

    // `Space` shows the leader menu at once, in groups, its groups as prefixes.
    normal_mode(&journey).await?;
    journey.keys(" ").await?;
    journey
        .element("[data-testid=which-key][data-keys=Space]")
        .await?;
    let groups = journey
        .attrs("[data-testid=which-key-group]", "data-group")
        .await?;
    for group in ["threads", "views", "workspace", "files", "help", "commands"] {
        assert!(
            groups.iter().any(|found| found.as_deref() == Some(group)),
            "{group} missing: {groups:?}"
        );
    }
    let help = journey
        .text("[data-testid=which-key-entry][data-key=h]")
        .await?;
    assert!(help.contains("help…"), "{help}");
    journey.screenshot("which-key-leader").await?;

    // `h` follows the sequence; `k` picks `:describe-key`, which opens as the panel closes.
    journey.keys("h").await?;
    journey
        .element("[data-testid=which-key][data-keys='Space h']")
        .await?;
    journey
        .element("[data-testid=which-key-entry][data-command='help.describeKey']")
        .await?;
    journey.keys("k").await?;
    journey.gone(PANEL).await?;
    journey.element("[data-testid=describe-prompt]").await?;
    journey.keys(Key::Escape).await?;
    journey.gone("[data-testid=describe]").await?;

    // A prefix shows its panel after the delay; Escape drops it and runs nothing.
    normal_mode(&journey).await?;
    journey.keys("g").await?;
    journey
        .element("[data-testid=which-key][data-keys=g]")
        .await?;
    journey
        .element("[data-testid=which-key-entry][data-command='nav.top']")
        .await?;
    journey.keys(Key::Escape).await?;
    journey.gone(PANEL).await?;
    assert_eq!(journey.count("[data-testid=command-line]").await?, 0);

    // `Space Space` opens the command line.
    normal_mode(&journey).await?;
    journey.keys("  ").await?;
    journey.until_focused("command-line-input").await?;
    assert_eq!(journey.count(PANEL).await?, 0);
    journey.keys(Key::Escape).await?;
    journey.gone("[data-testid=command-line]").await?;
    Ok(())
}

// spec: UI-022, UI-095, UI-201, UI-092, UI-730, UI-736
#[tokio::test(flavor = "multi_thread")]
async fn thread_picker_previews_confirms_and_restores_with_keyboard_focus_at_both_widths()
-> anyhow::Result<()> {
    use omfp_daemon::agent::{FakeLauncher, FakeRun};
    use serde_json::json;
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    for workspace in ["original", "alpha-beta", "alpine-boat"] {
        launcher.push_run_for(
            journey.data_dir().join("workspaces").join(workspace),
            FakeRun::fresh().with_history(
                (0..100)
                    .map(|index| {
                        serde_json::from_value(json!({
                            "role": "user", "content": format!("Conversation message {index}"),
                            "timestamp": index
                        }))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        );
    }
    journey.connected().await?;
    let original = journey.create_thread("Original").await?;
    let alpha = journey.create_thread("Alpha beta").await?;
    let alpine = journey.create_thread("Alpine boat").await?;
    for width in [1400, 900] {
        journey.driver.set_window_rect(0, 0, width, 1000).await?;
        journey.command("threads").await?;
        journey.until_focused("thread-picker-input").await?;
        journey.keys("Original").await?;
        journey.keys(Key::Enter).await?;
        journey.opened(&original).await?;
        normal_mode(&journey).await?;
        journey.keys(" /").await?;
        journey.until_focused("thread-picker-input").await?;
        journey.keys("apbt").await?;
        journey.opened(&alpha).await?;
        journey
            .screenshot(&format!("thread-preview-{width}"))
            .await?;
        journey.keys(Key::Escape).await?;
        journey.opened(&original).await?;
        normal_mode(&journey).await?;
        journey.keys(" /").await?;
        journey.until_focused("thread-picker-input").await?;
        journey.keys("alb").await?;
        journey.opened(&alpha).await?;
        journey.keys(Key::Down).await?;
        journey.opened(&alpine).await?;
        journey.keys(Key::Enter).await?;
        journey.gone("[data-testid=thread-picker]").await?;
        journey.opened(&alpine).await?;
        normal_mode(&journey).await?;
        journey.keys(" t").await?;
        journey.until_focused("thread-row").await?;
        let section = "[data-testid=thread-row][aria-current=true]";
        assert_eq!(
            journey.attr(section, "data-thread-id").await?.as_deref(),
            Some(alpine.as_str())
        );
        journey.keys("za").await?;
        journey.until("workspace collapsed by keyboard", async |page| {
            let value = page.eval(
                "return document.activeElement.closest('[data-testid=workspace-section]').querySelector('[data-testid=workspace-collapse]').getAttribute('aria-expanded');",
                Vec::new(),
            ).await?;
            Ok((value == json!("false")).then_some(()))
        }).await?;
        journey.keys("za").await?;
        journey
            .screenshot(&format!("thread-column-focus-{width}"))
            .await?;
        search_current_conversation(&journey, width).await?;
    }
    journey
        .finish("thread picker and conversation search")
        .await
}

async fn search_current_conversation(journey: &Journey, width: u32) -> anyhow::Result<()> {
    use serde_json::json;
    normal_mode(journey).await?;
    journey.keys("/").await?;
    journey.until_focused("conversation-search-input").await?;
    journey
        .element("[data-testid=conversation-search] input")
        .await?
        .send_keys("message 12")
        .await?;
    journey
        .until("virtualized conversation match visible", async |page| {
            Ok((page
                .count("[data-testid=transcript-item][data-index='12'][aria-current=true]")
                .await?
                == 1)
                .then_some(()))
        })
        .await?;
    assert!(
        journey
            .text("[data-testid=conversation-search]")
            .await?
            .contains("1 / 1")
    );
    // The occurrence is painted as the current custom highlight (UI-736).
    journey
        .until("current occurrence highlighted", async |page| {
            let marked = page
                .eval(
                    "const current = CSS.highlights.get('omfp-search-current');
                     return current === undefined ? [] : [...current].map(String);",
                    Vec::new(),
                )
                .await?;
            Ok((marked == json!(["message 12"])).then_some(()))
        })
        .await?;
    journey
        .screenshot(&format!("conversation-search-{width}"))
        .await?;
    journey.keys(Key::Escape).await?;
    journey.gone("[data-testid=conversation-search]").await?;
    journey
        .until("search highlights cleared", async |page| {
            let left = page.eval("return CSS.highlights.size;", Vec::new()).await?;
            Ok((left == json!(0)).then_some(()))
        })
        .await?;
    Ok(())
}
