//! URLs omp opens and the output of local commands (E2E-27), against a daemon on the
//! fake launcher: an `open_url` request of the thread's omp toasts in the window with Open and Copy (UI-346) and shows
//! as a card with the URL and the loopback note (UI-345, UI-347); a `/dirs`-like local command's output is a
//! command-output card (UI-345).

use e2e::Journey;
use omfp_core::wire::RpcNotification;
use omfp_daemon::agent::FakeLauncher;
use serde_json::json;

const LOGIN: &str = "https://login.example.com/device?code=E2E27";
const TOAST: &str = "[data-testid=opened-url-toast]";
const CARD: &str = "[data-testid=command-output]";

// spec: E2E-27, UI-345, UI-346, UI-347
#[tokio::test(flavor = "multi_thread")]
async fn an_opened_url_toasts_with_open_and_copy_and_a_local_command_shows_its_output()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    launcher.local_output("/dirs", &["Workspace roots:\n  /w/app"]);
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;
    let thread = journey.create_thread("login").await?;

    // omp asks the client to open a login page: the window offers it as a toast and in the transcript.
    let open_url: RpcNotification = serde_json::from_value(json!({
        "type": "extension_ui_request",
        "method": "open_url",
        "id": "open-1",
        "url": LOGIN,
    }))?;
    journey
        .fake_process(&launcher, &thread)
        .await?
        .emit(vec![open_url]);
    journey
        .until("the opened-URL toast", async |page| {
            let urls = page.attrs(TOAST, "data-url").await?;
            Ok((urls == [Some(LOGIN.to_owned())]).then_some(()))
        })
        .await?;
    assert_eq!(
        journey.text(&format!("{TOAST} .omfp-toast__text")).await?,
        "omp opened login.example.com in the browser"
    );
    assert_eq!(
        journey
            .count(&format!("{TOAST} [data-testid=opened-url-open]"))
            .await?,
        1
    );
    assert_eq!(
        journey
            .count(&format!("{TOAST} [data-testid=opened-url-copy]"))
            .await?,
        1
    );
    let card = format!("{CARD}[data-kind=open-url]");
    assert_eq!(
        journey
            .attrs(&format!("{card} [data-testid=opened-url]"), "data-url")
            .await?,
        [Some(LOGIN.to_owned())]
    );
    assert!(
        journey
            .text(&format!("{card} [data-testid=loopback-note]"))
            .await?
            .contains("loopback address"),
        "the card says where the OAuth callback goes"
    );
    journey.screenshot("opened-url-toast").await?;

    // A local command answers with its output: a command card with the text as omp wrote it.
    journey.send("/dirs").await?;
    journey
        .until("the command-output card", async |page| {
            let texts = page
                .texts(&format!(
                    "{CARD}[data-kind=command] [data-testid=command-output-text]"
                ))
                .await?;
            Ok((texts == ["Workspace roots:\n  /w/app"]).then_some(()))
        })
        .await?;
    journey.screenshot("command-output").await?;

    journey.finish("opened url").await
}
