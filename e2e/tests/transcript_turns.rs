//! One name line per turn and a tool result's images (E2E-40), against a daemon on the fake launcher: a thread whose
//! history has a turn of two tool calls (a `read` of a PNG whose result carries the image, then an `eval`) and a reply,
//! then a second user message and its answer. Headless Chromium shows the `Assistant` name line (its `◆` drawn by the
//! stylesheet, UI-796) once per turn (UI-470, UI-471) and the PNG as a decoded thumbnail under the collapsed `read`
//! card (UI-472, UI-473); Enter on the thumbnail opens it at full size and Escape closes it, back to the thumbnail
//! (UI-474).

use e2e::Journey;
use omfp_core::wire::AgentMessage;
use omfp_daemon::agent::{FakeLauncher, FakeRun};
use serde_json::{Value, json};
use thirtyfour::prelude::*;

/// A 120 × 80 PNG of four colored bands.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAHgAAABQCAIAAABd+SbeAAAAoUlEQVR42u3QUQ2AQAwFsKlADVJQQFCCnTsZiJkEXLzso0kVtI71EVAKRItGtGjRFkSLRrRo0RZEi0a0aNGIFo3oodH31QSIFi0a0aJFKxAtGtGiRVsQLRrRokVbEC0a0VOj36cJEC1aNKJFi1YgWjSiRYu2IFo0okWLtiBaNKKnRvc+CRAtWjSiRYtWIFo0okWLtiBaNKJFi7YgWjSih/oBBZpDP2acdsAAAAAASUVORK5CYII=";
const ASSISTANT: &str = "[data-testid=message][data-role=assistant]";
const READ: &str = "[data-testid=tool-card][data-call=call_read]";
const THUMBNAIL: &str = "[data-testid=tool-card][data-call=call_read] [data-testid=tool-image]";
const DIALOG: &str = "[data-testid=tool-image-dialog]";

fn message(value: &Value) -> AgentMessage {
    serde_json::from_value(value.clone()).expect("a valid agent message")
}

fn assistant(content: &Value) -> Value {
    json!({
        "role": "assistant", "api": "openai-completions", "provider": "fake", "model": "scripted",
        "content": content, "stopReason": "toolUse", "timestamp": 0,
        "usage": {
            "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
            "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0 },
        },
    })
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "timestamp": 0, "content": [{ "type": "text", "text": text }] })
}

fn result(id: &str, tool: &str, content: &Value) -> Value {
    json!({
        "role": "toolResult", "toolCallId": id, "toolName": tool, "content": content, "isError": false,
        "timestamp": 0,
    })
}

/// Two turns: the first reads a PNG, runs an `eval` and replies; the second answers at once.
fn history() -> Vec<AgentMessage> {
    [
        user("Show me the plot."),
        assistant(&json!([
            { "type": "thinking", "thinking": "Reading the plot first." },
            { "type": "toolCall", "id": "call_read", "name": "read", "arguments": { "path": "plot.png" } },
        ])),
        result(
            "call_read",
            "read",
            &json!([
                { "type": "text", "text": "Read image file [image/png]" },
                { "type": "image", "data": PNG, "mimeType": "image/png" },
            ]),
        ),
        assistant(&json!([
            { "type": "thinking", "thinking": "Then its size." },
            { "type": "toolCall", "id": "call_eval", "name": "eval", "arguments": { "language": "py", "code": "print(120 * 80)" } },
        ])),
        result(
            "call_eval",
            "eval",
            &json!([{ "type": "text", "text": "9600" }]),
        ),
        assistant(&json!([{ "type": "text", "text": "Here it is: four bands, 120 × 80." }])),
        user("Thanks."),
        assistant(&json!([{ "type": "text", "text": "You're welcome." }])),
    ]
    .iter()
    .map(message)
    .collect()
}

// spec: E2E-40, UI-470, UI-471, UI-472, UI-473, UI-474
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_shows_one_assistant_name_line_and_a_tool_results_image_opens_full_size()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/turns"),
        FakeRun::fresh().with_history(history()),
    );
    journey.connected().await?;
    journey.create_thread("turns").await?;
    journey
        .until("the history's assistant messages", async |page| {
            Ok((page.count(ASSISTANT).await? == 4).then_some(()))
        })
        .await?;

    // One name line per turn: the first turn's later messages continue it.
    assert_eq!(
        journey
            .texts(&format!("{ASSISTANT} .omfp-msg__name"))
            .await?,
        ["Assistant", "Assistant"]
    );
    assert_eq!(
        journey.attrs(ASSISTANT, "data-continues").await?,
        ["false", "true", "true", "false"].map(|value| Some(value.to_owned()))
    );
    assert_eq!(journey.count(&format!("article{ASSISTANT}")).await?, 2);

    // The image shows, decoded, under the collapsed card, 10rem tall.
    assert_eq!(
        journey
            .attr(&format!("{READ} [data-testid=toggle]"), "aria-expanded")
            .await?,
        Some("false".to_owned())
    );
    let decoded = journey
        .until("the thumbnail to decode", async |page| {
            let value = page
                .eval(
                    "const b = document.querySelector(arguments[0]); const i = b && b.querySelector('img');
                     if (!i || !i.complete || i.naturalWidth === 0) return null;
                     const rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
                     return [i.naturalWidth, i.naturalHeight, b.getBoundingClientRect().height / rem];",
                    vec![json!(THUMBNAIL)],
                )
                .await?;
            Ok(serde_json::from_value::<Option<(u32, u32, f64)>>(value)?)
        })
        .await?;
    assert_eq!((decoded.0, decoded.1), (120, 80));
    assert!(
        (decoded.2 - 10.0).abs() < 0.05,
        "the thumbnail is 10rem tall, not {}rem",
        decoded.2
    );
    assert_eq!(
        journey.attr(THUMBNAIL, "aria-label").await?,
        Some("Open image from read plot.png".to_owned())
    );
    journey.screenshot("transcript-turns").await?;

    // Enter opens the full view; Escape closes it, back to the thumbnail.
    journey
        .eval(
            "document.querySelector(arguments[0]).focus();",
            vec![json!(THUMBNAIL)],
        )
        .await?;
    journey.keys(Key::Enter).await?;
    journey.element(DIALOG).await?;
    assert_eq!(
        journey
            .attr(&format!("{DIALOG} img[data-testid=image-after]"), "alt")
            .await?,
        Some("Image from read plot.png".to_owned())
    );
    journey.screenshot("tool-image-full").await?;
    journey.keys(Key::Escape).await?;
    journey.gone(DIALOG).await?;
    journey.until_focused("tool-image").await?;

    journey.finish("transcript turns").await
}
