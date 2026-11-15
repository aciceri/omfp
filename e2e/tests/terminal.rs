//! The reusable terminal journey (E2E-71, TRM-101, TRM-103, TRM-105): an agent mounts `omfp:terminal`, its PTY
//! prompts without echo, xterm.js sends keyboard input, and the command prints only the accepted secret length.

use e2e::Journey;
use fake_llm::{Reply, Step, ToolCall};
use serde_json::json;
use thirtyfour::prelude::Key;

const TERMINAL: &str =
    "[data-testid=view] [data-widget=\"omfp:terminal\"][data-instance=terminal] .xterm";

// spec: E2E-71, TRM-101, TRM-103, TRM-105
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_mounts_an_interactive_password_prompt() -> anyhow::Result<()> {
    let command = "stty -echo; printf 'Password: '; IFS= read -r secret; stty echo; printf '\\r\\nAccepted:%s\\r\\n' \"${#secret}\"";
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_mount_terminal",
            "omfp_ui_mount",
            json!({
                "id": "terminal",
                "widget": "omfp:terminal",
                "place": "end",
                "props": { "command": command, "title": "Password prompt" }
            }),
        )])),
        Step::reply(Reply::text("The terminal is open.")),
    ])
    .await?;
    journey.connected().await?;
    journey.create_thread("terminal journey").await?;

    journey.send("open the password prompt").await?;
    journey.reply("The terminal is open.").await?;
    journey
        .until("the PTY password prompt", async |page| {
            Ok(page
                .text(TERMINAL)
                .await?
                .contains("Password:")
                .then_some(()))
        })
        .await?;

    journey.click(TERMINAL).await?;
    journey.keys("secret").await?;
    journey.keys(Key::Enter).await?;
    journey
        .until("the command accepting hidden input", async |page| {
            Ok(page
                .text(TERMINAL)
                .await?
                .contains("Accepted:6")
                .then_some(()))
        })
        .await?;

    let screen = journey.text(TERMINAL).await?;
    assert!(screen.contains("Accepted:6"));
    assert!(!screen.contains("secret"));
    journey.screenshot("terminal-password").await?;
    journey.finish("terminal").await
}
