//! The MCP empty state must point at this daemon's isolated profile, including with a custom data directory.

use e2e::Journey;
use omfp_daemon::agent::FakeLauncher;
use thirtyfour::By;
use thirtyfour::prelude::ElementQueryable as _;

// spec: ARC-021, UI-294
#[tokio::test(flavor = "multi_thread")]
async fn mcp_configuration_help_uses_the_daemons_profile_not_standalone_omp() -> anyhow::Result<()>
{
    let journey = Journey::start_fake(FakeLauncher::new()).await?;
    journey.connected().await?;
    journey.create_asking_thread("isolated profile").await?;
    journey.command("widget add omfp:mcp").await?;
    journey
        .driver
        .query(By::XPath(
            "//*[@data-testid='view-tab' and contains(., 'MCP servers')]",
        ))
        .first()
        .await?
        .click()
        .await?;
    journey.element("[data-testid=mcp-none]").await?;
    let help = journey.text("[data-testid=mcp-none]").await?;
    let expected = journey.data_dir().join("omp-profile/agent/mcp.json");
    assert!(
        help.contains(expected.to_str().expect("temporary path is UTF-8")),
        "{help}"
    );
    assert!(!help.contains("~/.omp/agent"), "{help}");
    journey.fits("[data-testid=mcp-none]").await?;
    journey.screenshot("isolated-mcp-profile").await?;
    journey.finish("isolated-mcp-profile").await
}
