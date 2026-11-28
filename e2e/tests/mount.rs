//! The UI mount journeys of M10 (AGT-060..063, UI-230..232) and M17 (UI-375, UI-376): a scripted agent mounts inline
//! code into the workspace's view and as a floating panel, drives the mounted widget by updating its props while the
//! user's state in it stays, and the user closes the panel; the user closes a tab an agent mounted and takes it back.
//! Takes `omfp-m10-mount.png` and `omfp-m17-close-mounted-tab.png` (in the run's screenshot directory). Run with
//! `just test-e2e`.

use e2e::Journey;
use fake_llm::{Reply, Step, ToolCall};
use serde_json::{Value, json};

/// A widget showing its `value` prop, with a click counter kept in React state (gone if the widget remounts).
const PROGRESS: &str = r#"// Progress of the agent's task; props: value.
import { useState } from "react";

export default function Progress({ value = 0 }: { value?: number }) {
  const [clicks, setClicks] = useState(0);
  return (
    <div data-testid="progress" data-value={value} data-clicks={clicks}>
      <button type="button" data-testid="progress-click" onClick={() => setClicks((n) => n + 1)}>
        Step {value} of 3
      </button>
    </div>
  );
}
"#;

/// A note for the floating panel.
const NOTE: &str = r#"export default function Note({ text }: { text: string }) {
  return <p data-testid="note">{text}</p>;
}
"#;

const PROGRESS_CSS: &str = "[data-testid=view] [data-widget=\"./widgets/progress\"][data-instance=progress] [data-testid=progress]";
const PANEL_CSS: &str = "[data-testid=floating-widget][data-panel=note]";

/// The text of the tool result answering `call_id`, from the first chat-completions request carrying it.
fn tool_result(requests: &[Value], call_id: &str) -> String {
    requests
        .iter()
        .filter_map(|request| request["messages"].as_array())
        .flatten()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == call_id)
        .map_or_else(
            || panic!("no result for {call_id} in {} requests", requests.len()),
            |message| message["content"].as_str().unwrap_or_default().to_owned(),
        )
}

/// The progress widget's `data-<name>`, once it shows `expected`.
async fn progress_shows(journey: &Journey, name: &str, expected: &str) -> anyhow::Result<()> {
    journey
        .until(
            &format!("the progress widget's {name} at {expected}"),
            async |page| {
                Ok((page
                    .attr(PROGRESS_CSS, &format!("data-{name}"))
                    .await?
                    .as_deref()
                    == Some(expected))
                .then_some(()))
            },
        )
        .await
}

// spec: AGT-060, AGT-061, AGT-062, AGT-063, UI-230, UI-231, UI-232
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_mounts_a_widget_into_the_view_and_a_floating_panel_and_drives_it()
-> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_mount_view",
            "omfp_ui_mount",
            json!({ "id": "progress", "code": PROGRESS, "place": "end", "props": { "value": 1 } }),
        )])),
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_mount_panel",
            "omfp_ui_mount",
            json!({
                "id": "note",
                "code": NOTE,
                "panel": true,
                "props": { "title": "Agent note", "text": "Building checkout v2" }
            }),
        )])),
        // Held until the user clicked the mounted widget.
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_update",
            "omfp_ui_update",
            json!({ "id": "progress", "props": { "value": 2 } }),
        )]))
        .gated(),
        Step::reply(Reply::text("Mounted the progress and a note.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("mount journey").await?;
    let folder = journey.workspace_folder(&thread).await?;

    journey.send("show me your progress").await?;
    journey.fake.wait_for_gate(2).await;
    progress_shows(&journey, "value", "1").await?;
    journey.element(PANEL_CSS).await?;
    assert_eq!(
        journey
            .text(&format!("{PANEL_CSS} [data-testid=floating-widget-title]"))
            .await?,
        "Agent note"
    );
    assert_eq!(
        journey
            .text(&format!("{PANEL_CSS} [data-testid=note]"))
            .await?,
        "Building checkout v2"
    );
    journey.click("[data-testid=progress-click]").await?;
    progress_shows(&journey, "clicks", "1").await?;
    journey.screenshot("m10-mount").await?;

    // The agent updates the props: the widget re-renders in place, the user's clicks stay.
    journey.fake.release_gate(2);
    journey.reply("Mounted the progress and a note.").await?;
    progress_shows(&journey, "value", "2").await?;
    progress_shows(&journey, "clicks", "1").await?;

    let requests = journey.fake.requests();
    let mounted = tool_result(&requests, "call_mount_view");
    assert!(
        mounted.starts_with(
            "Mounted ./widgets/progress as \"progress\" in view \"default\"; open windows show it now."
        ),
        "{mounted}"
    );
    let panel = tool_result(&requests, "call_mount_panel");
    assert!(
        panel.starts_with(
            "Mounted ./widgets/note as \"note\" as the floating panel \"note\"; open windows show it now."
        ),
        "{panel}"
    );
    let updated = tool_result(&requests, "call_update");
    assert!(updated.starts_with("Updated progress"), "{updated}");

    // The user closes the panel: it leaves views.json, so every window and the agent see it gone.
    journey
        .click(&format!("{PANEL_CSS} [data-testid=floating-widget-close]"))
        .await?;
    journey.gone(PANEL_CSS).await?;
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join(".omfp/views.json"))?)?;
    assert_eq!(manifest.get("panels"), None, "{manifest}");
    journey.finish("M10 mount").await
}

/// The views.json the daemon wrote for the workspace in `folder`.
fn views_json(folder: &std::path::Path) -> anyhow::Result<Value> {
    Ok(serde_json::from_str(&std::fs::read_to_string(
        folder.join(".omfp/views.json"),
    )?)?)
}

/// The references of the default view's side tabs in `manifest`.
fn side_tabs(manifest: &Value) -> Vec<Value> {
    manifest["views"]["default"]["layout"][3]
        .as_array()
        .map(|tabs| tabs[2..].to_vec())
        .unwrap_or_default()
}

// spec: E2E-79, UI-375, UI-376
#[tokio::test(flavor = "multi_thread")]
async fn the_user_closes_a_tab_an_agent_mounted_and_takes_it_back() -> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_mount_tab",
            "omfp_ui_mount",
            json!({ "id": "progress", "code": PROGRESS, "props": { "value": 1, "title": "Progress" } }),
        )])),
        Step::reply(Reply::text("Mounted the progress.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("close journey").await?;
    let folder = journey.workspace_folder(&thread).await?;
    journey.send("show me your progress").await?;
    journey.reply("Mounted the progress.").await?;

    // The mounted tab is the last side tab, with its own ×; the chat alone in the main slot has none.
    let tab = "[data-testid=view] [data-testid=view-tab][data-tab=\"layout:progress#0\"]";
    journey.element(tab).await?;
    assert_eq!(journey.text(tab).await?, "Progress");
    assert_eq!(
        journey
            .count("[data-testid=view] [data-testid=layout-tab-close]")
            .await?,
        2
    );
    journey.click(tab).await?;
    progress_shows(&journey, "value", "1").await?;
    let mounted = views_json(&folder)?;
    assert_eq!(side_tabs(&mounted).len(), 2, "{mounted}");
    journey.hover(tab).await?;
    journey.screenshot("m17-close-mounted-tab").await?;

    // Its × writes views.json without it, so the agent and every window see it gone.
    journey
        .click(&format!("{tab} [data-testid=layout-tab-close]"))
        .await?;
    journey.gone(tab).await?;
    let toast = "[data-testid=success] .omfp-toast__text";
    journey.element(toast).await?;
    assert_eq!(journey.text(toast).await?, "Closed Progress");
    let closed = views_json(&folder)?;
    assert_eq!(side_tabs(&closed), ["omfp:canvas"], "{closed}");

    // Undo writes the file back as it was: the tab returns.
    journey.click("[data-testid=success-undo]").await?;
    journey.element(tab).await?;
    assert_eq!(views_json(&folder)?, mounted);
    journey.finish("M17 close a mounted tab").await
}
