//! The first run (E2E-43), against a daemon on the fake launcher: when omp can use no model the window welcomes with
//! `Log in to a provider` instead of the view, and a login through the picker shows the view once omp can use models
//! (UI-500..UI-503, DMN-235, DMN-236); when omp has a model the window shows its normal frame (UI-504). A login another
//! connection started (an agent's) shows its page and progress in the window too (E2E-62).

use e2e::Journey;
use omfp_daemon::agent::{FakeLauncher, FakeReply};
use serde_json::json;

const WELCOME: &str = "[data-testid=welcome]";
const VIEW: &str = "[data-testid=view]";

/// The provider picker's first three rows, checked in the page: each holds its whole text (so the hover, selection and
/// focus backgrounds cover it), its id and status line sits under its name, no row overlaps the one before, and the
/// long name of the third wraps rather than being cut (UI-382). Evaluates to the problems found.
const PROVIDER_ROW_PROBLEMS: &str = "
    const rows = [...document.querySelectorAll('[data-testid=login-picker] [data-testid=login-provider]')].slice(0, 3);
    const problems = [];
    const inside = (inner, outer) => inner.left >= outer.left - 0.5 && inner.right <= outer.right + 0.5
      && inner.top >= outer.top - 0.5 && inner.bottom <= outer.bottom + 0.5;
    rows.forEach((row, index) => {
      const box = row.getBoundingClientRect();
      for (const part of row.querySelectorAll('*')) {
        const rect = part.getBoundingClientRect();
        if (rect.width > 0 && rect.height > 0 && !inside(rect, box)) {
          problems.push(`row ${index}: ${part.className.baseVal ?? part.className} spills out`);
        }
      }
      const name = row.querySelector('[data-testid=login-provider-name]').getBoundingClientRect();
      const meta = row.querySelector('[data-testid=login-provider-meta]').getBoundingClientRect();
      if (meta.top < name.bottom - 0.5) problems.push(`row ${index}: the id line overlaps the name`);
      if (row.scrollWidth > row.clientWidth) problems.push(`row ${index}: overflows sideways`);
      const previous = rows[index - 1]?.getBoundingClientRect();
      if (previous && box.top < previous.bottom - 0.5) problems.push(`row ${index}: overlaps row ${index - 1}`);
    });
    const long = rows[2].querySelector('[data-testid=login-provider-name]');
    const lines = long.getBoundingClientRect().height / parseFloat(getComputedStyle(long).lineHeight);
    if (lines < 1.5) problems.push(`the long name did not wrap (${lines} lines)`);
    return problems;";

/// Waits until `css` matches exactly `count` elements.
async fn until_count(journey: &Journey, css: &str, count: usize) -> anyhow::Result<()> {
    journey
        .until(&format!("{count} × {css}"), async |page| {
            Ok((page.count(css).await? == count).then_some(()))
        })
        .await
}

// spec: E2E-43, UI-382, UI-500, UI-501, UI-502, UI-503, DMN-235, DMN-236
#[tokio::test(flavor = "multi_thread")]
async fn without_a_usable_model_the_window_welcomes_with_the_login_and_shows_the_view_after_it()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    launcher.set_usable_models(0);
    // Long names and ids, a logged-in and an unavailable provider right after Anthropic: their rows must stay legible.
    let mut providers = vec![
        json!({
            "id": "anthropic", "name": "Anthropic (Claude Pro/Max)", "available": true, "authenticated": false
        }),
        json!({
            "id": "openai-codex", "name": "ChatGPT Plus/Pro (Codex Subscription)",
            "available": true, "authenticated": true
        }),
        json!({
            "id": "zai-coding-plan-international-endpoint-with-a-long-identifier",
            "name": "Z.AI Coding Plan (International, GLM-4.6 and GLM-4.5 Air through the coding endpoint)",
            "available": false, "authenticated": false
        }),
    ];
    providers.extend((0..48).map(|i| {
        json!({
            "id": format!("provider-{i}"), "name": format!("Provider {i}"),
            "available": true, "authenticated": false
        })
    }));
    launcher.reply(
        "get_login_providers",
        FakeReply::Ok(json!({ "providers": providers })),
    );
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.element(WELCOME).await?;
    journey.gone("[data-testid=connection-banner]").await?;

    assert_eq!(
        journey.count(VIEW).await?,
        0,
        "the welcome stands for the view"
    );
    assert_eq!(
        journey.text("[data-testid=welcome-login]").await?,
        "Log in to a provider"
    );
    assert_eq!(
        journey.count("[data-testid=left-column]").await?,
        1,
        "the threads column stays"
    );
    assert_eq!(launcher.model_checks().len(), 1);
    journey.screenshot("first-run-welcome").await?;

    // The login through the picker; afterwards omp can use models.
    journey.driver.set_window_rect(0, 0, 1280, 800).await?;
    launcher.set_usable_models(3);
    journey.click("[data-testid=welcome-login]").await?;
    journey.element("[data-testid=login-provider]").await?;
    let geometry = journey
        .eval(
            "const dialog = document.querySelector('[data-testid=login-picker]');
             const list = dialog.querySelector('[data-testid=login-providers]');
             const cancel = dialog.querySelector('[data-testid=login-picker-cancel]');
             const box = dialog.getBoundingClientRect(), action = cancel.getBoundingClientRect();
             return [box.top, box.bottom, action.bottom, list.scrollHeight > list.clientHeight, innerHeight];",
            Vec::new(),
        )
        .await?;
    let [top, bottom, action, scrolling, height] =
        geometry.as_array().expect("dialog geometry").as_slice()
    else {
        unreachable!("dialog geometry")
    };
    let height = height.as_f64().expect("viewport height");
    assert!(top.as_f64().unwrap_or(-1.0) >= 0.0, "{geometry}");
    assert!(bottom.as_f64().unwrap_or(f64::MAX) <= height, "{geometry}");
    assert!(action.as_f64().unwrap_or(f64::MAX) <= height, "{geometry}");
    assert_eq!(scrolling, &json!(true), "{geometry}");
    let problems = journey.eval(PROVIDER_ROW_PROBLEMS, Vec::new()).await?;
    assert_eq!(problems, json!([]), "provider rows");
    journey.screenshot("first-run-picker").await?;
    journey
        .click("[data-testid=login-picker] [data-testid=login-provider]")
        .await?;
    until_count(&journey, VIEW, 1).await?;
    journey.gone("[data-testid=login-picker]").await?;
    assert_eq!(journey.count(WELCOME).await?, 0);
    assert_eq!(
        journey.text("[data-testid=success]").await?,
        "Logged in to Anthropic (Claude Pro/Max)"
    );
    journey.screenshot("first-run-logged-in").await?;
    journey.finish("first run without a model").await
}

// spec: E2E-43, UI-504
#[tokio::test(flavor = "multi_thread")]
async fn with_a_usable_model_the_window_shows_its_normal_frame() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;

    journey
        .until("the window's auth.status", async |_| {
            Ok((launcher.model_checks().len() == 1).then_some(()))
        })
        .await?;
    until_count(&journey, VIEW, 1).await?;
    assert_eq!(journey.count(WELCOME).await?, 0);
    journey.screenshot("first-run-with-a-model").await?;
    journey.finish("first run with a model").await
}

// spec: E2E-62, UI-640, DMN-290
#[tokio::test(flavor = "multi_thread")]
async fn a_login_another_connection_started_shows_its_page_and_progress_in_the_window()
-> anyhow::Result<()> {
    use omfp_core::wire::{ExtensionUiRequest, NotifyUiRequest, OpenUrlUiRequest, RpcNotification};
    const URL: &str = "https://auth.kimi.com/device?code=E2E62";
    let launcher = FakeLauncher::new();
    launcher.reply(
        "get_login_providers",
        FakeReply::Ok(json!({ "providers": [
            { "id": "kimi-code", "name": "Kimi Code", "available": true, "authenticated": false }
        ]})),
    );
    let gate = launcher.gate("login");
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;

    // An agent's `auth.login` (its own connection, as `omfp_call` has): its flow's requests go to that connection.
    let mut agent = journey.connect().await?;
    let login = tokio::spawn(async move {
        let logged_in = agent
            .call::<omfp_protocol::method::AuthLogin>(omfp_protocol::ProviderRef {
                provider_id: "kimi-code".to_owned(),
            })
            .await;
        agent.close().await?;
        logged_in
    });
    gate.arrived(1).await;
    let omp = launcher.process(launcher.launches().len() - 1);
    let ui = |request| RpcNotification::ExtensionUiRequest(request);
    omp.emit(vec![
        ui(ExtensionUiRequest::OpenUrl(OpenUrlUiRequest {
            id: "u1".into(),
            url: URL.into(),
            launch_url: None,
            instructions: Some("Enter E2E62".into()),
        })),
        ui(ExtensionUiRequest::Notify(NotifyUiRequest {
            id: "u2".into(),
            message: "Waiting for the browser…".into(),
            notify_type: None,
        })),
    ]);

    // The window shows it as it shows its own: the progress with Cancel, the page with Open and Copy.
    journey
        .until("the login's progress", async |page| {
            Ok(page
                .texts("[data-testid=login-progress-message]")
                .await?
                .contains(&"Waiting for the browser…".to_owned())
                .then_some(()))
        })
        .await?;
    assert!(
        journey
            .text("[data-testid=login-progress]")
            .await?
            .contains("Logging in to Kimi Code… finish in your browser")
    );
    let toast = format!("[data-testid=opened-url-toast][data-url='{URL}']");
    assert!(
        journey
            .text(&toast)
            .await?
            .contains("Log in to Kimi Code at auth.kimi.com")
    );
    assert_eq!(
        journey
            .text(&format!("{toast} [data-testid=login-instructions]"))
            .await?,
        "Enter E2E62"
    );
    assert_eq!(
        journey
            .count(&format!(
                "{toast} [data-testid=opened-url-open], {toast} [data-testid=opened-url-copy]"
            ))
            .await?,
        2
    );
    journey.screenshot("agent-login").await?;

    // It ends: its toasts go.
    gate.open();
    login.await??;
    journey.gone("[data-testid=login-progress]").await?;
    journey.gone("[data-testid=opened-url-toast]").await?;
    journey.finish("a login started elsewhere").await
}
