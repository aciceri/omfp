//! The MCP empty state names the workspace's own config: MCP servers belong to the workspace, so neither this
//! daemon's isolated profile nor standalone omp's user config is offered, also with a custom data directory.

use e2e::Journey;
use omfp_daemon::agent::FakeLauncher;
use thirtyfour::By;
use thirtyfour::prelude::ElementQueryable as _;

// spec: ARC-021, UI-294
#[tokio::test(flavor = "multi_thread")]
async fn mcp_configuration_help_names_the_workspace_config_only() -> anyhow::Result<()> {
    let journey = Journey::start_fake(FakeLauncher::new()).await?;
    journey.connected().await?;
    journey.create_asking_thread("workspace mcp").await?;
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
    assert!(help.contains("/.omp/mcp.json"), "{help}");
    let profile = journey.data_dir().join("omp-profile");
    assert!(
        !help.contains(profile.to_str().expect("temporary path is UTF-8")),
        "{help}"
    );
    assert!(!help.contains("~/.omp/agent"), "{help}");
    journey.fits("[data-testid=mcp-none]").await?;
    journey.screenshot("workspace-mcp-config").await?;
    journey.finish("workspace-mcp-config").await
}
