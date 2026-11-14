//! The window's messages after a notice went (E2E-45), against a daemon on the fake launcher: another thread's omp
//! fails, its mode-line notice goes after 8 s (UI-192), and `:messages` still lists it with its kind and time
//! (UI-521, UI-522); Escape closes the dialog, and the mode line's messages button opens it again (UI-523).

use e2e::Journey;
use omfp_daemon::agent::FakeLauncher;
use thirtyfour::prelude::*;

const DIALOG: &str = "[data-testid=messages-dialog]";
const LINE: &str = "[data-testid=messages-dialog] [data-testid=message]";

// spec: E2E-45, UI-521, UI-522, UI-523
#[tokio::test(flavor = "multi_thread")]
async fn a_notice_gone_from_the_mode_line_is_listed_by_messages() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;
    let first = journey.create_thread("first").await?;
    journey.create_thread("second").await?;

    // The first thread's omp dies while the user is on the second: a notice in the mode line, gone after 8 s.
    journey.fake_process(&launcher, &first).await?.exit();
    journey
        .until("the failure notice", async |page| {
            let tones = page.attrs("[data-testid=mode-notice]", "data-tone").await?;
            Ok((tones == [Some("error".to_owned())]).then_some(()))
        })
        .await?;
    journey.gone("[data-testid=mode-notice]").await?;

    journey.command("messages").await?;
    journey.settled(DIALOG).await?;
    assert_eq!(
        journey.attrs(LINE, "data-tone").await?,
        [Some("error".to_owned())]
    );
    assert_eq!(
        journey
            .texts(&format!("{LINE} .omfp-messages__kind"))
            .await?,
        ["◆ notice"]
    );
    assert_eq!(
        journey
            .texts(&format!("{LINE} .omfp-messages__text"))
            .await?,
        ["failed"]
    );
    let time = journey.text(&format!("{LINE} time")).await?;
    assert_eq!(
        time.split(':').count(),
        3,
        "the line shows its time as hours, minutes and seconds: {time}"
    );
    journey.screenshot("messages").await?;

    journey.keys(Key::Escape).await?;
    journey.gone(DIALOG).await?;
    journey.click("[data-testid=mode-messages]").await?;
    journey.settled(DIALOG).await?;
    assert_eq!(journey.count(LINE).await?, 1);

    journey.finish("messages").await
}
