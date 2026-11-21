//! Word completion in the composer (E2E-78), against a daemon on the fake launcher whose predictor omp completes every
//! word with `sts`: the ghost shows after the caret where the text area's next characters would be, in Crush and in
//! the boring skin (UI-179); Tab inserts it with a space, `→` without, and the acceptance reaches omp (UI-175..UI-177,
//! DMN-360, DMN-361).

use e2e::Journey;
use omfp_daemon::agent::{FakeCall, FakeLauncher, FakeReply, OmpCall};
use serde_json::json;
use thirtyfour::prelude::*;

const INPUT: &str = "[data-testid=composer-input]";
const GHOST: &str = "[data-testid=composer-ghost]";

/// Fails unless the ghost starts where the text area's next character would be: right after `typed` on the first line
/// (measured with the text area's own font on a canvas), and within that line's box.
async fn ghost_after_caret(journey: &Journey, typed: &str) -> anyhow::Result<()> {
    let value = journey
        .eval(
            "const area = document.querySelector(arguments[0]);
             const ghost = document.querySelector(arguments[1]).getBoundingClientRect();
             const style = getComputedStyle(area);
             const box = area.getBoundingClientRect();
             const context = document.createElement('canvas').getContext('2d');
             context.font = style.font;
             context.letterSpacing = style.letterSpacing;
             const left = box.left + area.clientLeft + parseFloat(style.paddingLeft)
               + context.measureText(arguments[2]).width - area.scrollLeft;
             const top = box.top + area.clientTop + parseFloat(style.paddingTop) - area.scrollTop;
             const line = parseFloat(style.lineHeight);
             return [ghost.left, left, (ghost.top + ghost.bottom) / 2, top, top + line];",
            vec![json!(INPUT), json!(GHOST), json!(typed)],
        )
        .await?;
    let (ghost_left, left, middle, line_top, line_bottom): (f64, f64, f64, f64, f64) =
        serde_json::from_value(value)?;
    anyhow::ensure!(
        (ghost_left - left).abs() <= 1.0,
        "the ghost starts at x {ghost_left}, the text area's next character at x {left}"
    );
    anyhow::ensure!(
        (line_top..=line_bottom).contains(&middle),
        "the ghost's middle y {middle} is outside the first line, {line_top}..{line_bottom}"
    );
    Ok(())
}

async fn value(journey: &Journey) -> anyhow::Result<String> {
    let value = journey
        .eval(
            "return document.querySelector(arguments[0]).value;",
            vec![json!(INPUT)],
        )
        .await?;
    Ok(value.as_str().unwrap_or_default().to_owned())
}

// spec: E2E-78, UI-175, UI-176, UI-177, UI-179, DMN-360, DMN-361
#[tokio::test(flavor = "multi_thread")]
async fn the_rest_of_the_word_shows_after_the_caret_and_tab_or_right_inserts_it()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    launcher.reply("predict_word", FakeReply::Ok(json!({ "suffix": "sts" })));
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;
    journey.create_thread("word completion").await?;

    let input = journey.element(INPUT).await?;
    input.send_keys("Run the te").await?;
    journey
        .until("the ghost", async |page| {
            Ok((page.count(GHOST).await? == 1 && page.text(GHOST).await? == "sts").then_some(()))
        })
        .await?;
    ghost_after_caret(&journey, "Run the te").await?;
    journey.screenshot("word-completion-crush").await?;

    input.send_keys(Key::Tab).await?;
    journey.gone(GHOST).await?;
    assert_eq!(value(&journey).await?, "Run the tests ");
    journey
        .until("the accepted completion's feedback", async |_| {
            Ok(launcher
                .launches()
                .iter()
                .any(|launch| {
                    launch.process.calls().iter().any(|call| {
                        matches!(
                            call,
                            FakeCall::Call(OmpCall::PredictWordFeedback(feedback))
                                if feedback.accepted && feedback.suggestion == "sts"
                        )
                    })
                })
                .then_some(()))
        })
        .await?;

    journey.command("set skin=boring").await?;
    journey.element("html[data-skin=boring]").await?;
    let input = journey.element(INPUT).await?;
    input.click().await?;
    input.send_keys(Key::End).await?;
    input.send_keys("te").await?;
    journey
        .until("the ghost in the boring skin", async |page| {
            Ok((page.count(GHOST).await? == 1).then_some(()))
        })
        .await?;
    ghost_after_caret(&journey, "Run the tests te").await?;
    journey.screenshot("word-completion-boring").await?;

    input.send_keys(Key::Right).await?;
    journey.gone(GHOST).await?;
    assert_eq!(value(&journey).await?, "Run the tests tests");

    // A word whose prefix fits at the end of the visual line but whose completion would not: the typed text keeps
    // its line, and the ghost starts right after the caret on that line, cut at the edge (UI-179).
    input.send_keys(Key::Enter).await?;
    journey
        .gone("[data-testid=composer-input]:not(:placeholder-shown)")
        .await?;
    let filler = journey
        .eval(
            "const area = document.querySelector(arguments[0]);
             const style = getComputedStyle(area);
             const width = area.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
             const context = document.createElement('canvas').getContext('2d');
             context.font = style.font;
             context.letterSpacing = style.letterSpacing;
             let n = 1;
             while (context.measureText('a'.repeat(n + 1) + ' te').width < width - 2) n += 1;
             return 'a'.repeat(n);",
            vec![json!(INPUT)],
        )
        .await?;
    let typed = format!("{} te", filler.as_str().unwrap_or_default());
    let input = journey.element(INPUT).await?;
    input.send_keys(&typed).await?;
    journey
        .until("the ghost at the line's end", async |page| {
            Ok((page.count(GHOST).await? == 1).then_some(()))
        })
        .await?;
    let value = journey
        .eval(
            "const ghost = document.querySelector(arguments[0]).getBoundingClientRect();
             const mirror = document.querySelector('[data-testid=composer-ghost-text]');
             const prefix = mirror.firstChild;
             const range = document.createRange();
             range.setStart(prefix, prefix.length - 1);
             range.setEnd(prefix, prefix.length);
             const last = range.getBoundingClientRect();
             const line = parseFloat(getComputedStyle(mirror).lineHeight);
             return [ghost.left, (ghost.top + ghost.bottom) / 2, last.right, (last.top + last.bottom) / 2, line];",
            vec![json!(GHOST)],
        )
        .await?;
    let (ghost_left, ghost_middle, caret_x, caret_middle, line): (f64, f64, f64, f64, f64) =
        serde_json::from_value(value)?;
    assert!(
        (ghost_middle - caret_middle).abs() < line / 2.0 && (ghost_left - caret_x).abs() <= 1.0,
        "the ghost at ({ghost_left}, {ghost_middle}) is not right after the caret at ({caret_x}, {caret_middle}) on its line"
    );
    journey.screenshot("word-completion-line-end").await?;

    journey.finish("word completion").await
}
