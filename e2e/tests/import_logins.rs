//! Reusing standalone omp's logins (E2E-74), against the pinned omp: omfp's profile is a fresh install while the
//! user's own omp holds an Anthropic API key (an inert value, stored by omp's own `omp auth import`). The welcome
//! offers to copy it (UI-505); pressing the offer copies it through omp's export and import (UI-506, DMN-229, DMN-230,
//! PRT-136, PRT-137), and the window shows its view once omp can use a model.

use e2e::{Journey, fresh_install, standalone_omp};
use omfp_protocol::Empty;
use omfp_protocol::method::AuthStatus;
use serde_json::json;

const WELCOME: &str = "[data-testid=welcome]";
const IMPORT: &str = "[data-testid=welcome-import]";
const VIEW: &str = "[data-testid=view]";

// spec: E2E-74, UI-505, UI-506, DMN-229, DMN-230, PRT-136, PRT-137
#[tokio::test(flavor = "multi_thread")]
async fn the_welcome_copies_standalone_omps_logins_and_then_shows_the_view() -> anyhow::Result<()> {
    let journey = Journey::start_set_up(Vec::new(), |env, ports| {
        fresh_install(env)?;
        ports.user_omp_env = standalone_omp(
            env,
            &json!({
                "version": 1,
                "credentials": [{
                    "provider": "anthropic",
                    "credential": { "type": "api_key", "key": "inert-import-key", "source": "login" }
                }]
            }),
        )?;
        Ok(())
    })
    .await?;
    journey.element(WELCOME).await?;
    journey.gone("[data-testid=connection-banner]").await?;
    journey.settled(IMPORT).await?;
    assert_eq!(
        journey.count(VIEW).await?,
        0,
        "the welcome stands for the view"
    );
    assert_eq!(journey.text(IMPORT).await?, "Use my omp logins (anthropic)");
    assert_eq!(
        journey.text("[data-testid=welcome-import-note]").await?,
        "Copies only omp's logins, nothing else.",
        "an API key needs no browser-login caveat"
    );
    journey.screenshot("import-logins-welcome").await?;

    journey.click(IMPORT).await?;
    journey.element("[data-testid=success]").await?;
    assert_eq!(
        journey.text("[data-testid=success]").await?,
        "Copied omp's logins: anthropic"
    );
    journey.gone(WELCOME).await?;
    journey.element(VIEW).await?;

    let mut connection = journey.connect().await?;
    let status = connection.call::<AuthStatus>(Empty {}).await?;
    connection.close().await?;
    assert!(
        status.models > 0,
        "omp can use the copied login: {status:?}"
    );
    journey.finish("import-logins").await
}
