//! The chat's images open zoomable (E2E-75), against a daemon on the fake launcher: a thread whose history has a user
//! message with a 1600 × 1000 image attached and a `read` of the same image. Headless Chromium shows the message's image
//! shrunk into the transcript (at most 32rem); a click opens it in a dialog as large as the window, fitted, then at its
//! own pixels and zoomed by Ctrl with the wheel; Escape goes back to it. The `read` card's thumbnail opens the same
//! viewer, whose zoom buttons zoom it. Shots `omfp-chat-image-{transcript,fitted,actual,wheel,tool}.png`.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use e2e::Journey;
use omfp_core::wire::AgentMessage;
use omfp_daemon::agent::{FakeLauncher, FakeRun};
use serde_json::{Value, json};
use thirtyfour::prelude::*;

const WIDTH: u32 = 1600;
const HEIGHT: u32 = 1000;
const OPEN: &str = "[data-testid=message-image-open]";
const MESSAGE_DIALOG: &str = "[data-testid=message-image-dialog]";
const THUMBNAIL: &str = "[data-testid=tool-card][data-call=call_read] [data-testid=tool-image]";
const TOOL_DIALOG: &str = "[data-testid=tool-image-dialog]";

/// An SVG `WIDTH` × `HEIGHT`: a grid every 100 px with each cell's coordinates in small text, readable only zoomed.
fn picture() -> String {
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" font-family="monospace" font-size="11"><rect width="100%" height="100%" fill="#f4f1e8"/>"##
    );
    for x in (0..WIDTH).step_by(100) {
        for y in (0..HEIGHT).step_by(100) {
            let fill = if (x + y) % 200 == 0 {
                "#2f6fdf"
            } else {
                "#df6f2f"
            };
            let _ = write!(
                svg,
                r##"<rect x="{x}" y="{y}" width="100" height="100" fill="{fill}" fill-opacity="0.18" stroke="#333"/><text x="{}" y="{}">{x},{y}</text>"##,
                x + 8,
                y + 20
            );
        }
    }
    svg.push_str("</svg>");
    STANDARD.encode(svg)
}

fn message(value: &Value) -> AgentMessage {
    serde_json::from_value(value.clone()).expect("a valid agent message")
}

/// A user message with the picture attached, then a `read` of it and a reply.
fn history() -> Vec<AgentMessage> {
    let data = picture();
    let image = json!({ "type": "image", "data": data, "mimeType": "image/svg+xml" });
    let usage = json!({
        "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
        "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0 },
    });
    let assistant = |content: Value, stop: &str| {
        json!({
            "role": "assistant", "api": "openai-completions", "provider": "fake", "model": "scripted",
            "content": content, "stopReason": stop, "timestamp": 0, "usage": usage,
        })
    };
    [
        json!({ "role": "user", "timestamp": 0, "content": [
            { "type": "text", "text": "What does this grid say?" },
            image,
        ] }),
        assistant(
            json!([{ "type": "toolCall", "id": "call_read", "name": "read", "arguments": { "path": "grid.svg" } }]),
            "toolUse",
        ),
        json!({
            "role": "toolResult", "toolCallId": "call_read", "toolName": "read", "isError": false, "timestamp": 0,
            "content": [{ "type": "text", "text": "Read image file [image/svg+xml]" }, image],
        }),
        assistant(json!([{ "type": "text", "text": "A grid of 16 × 10 labelled cells." }]), "stop"),
    ]
    .iter()
    .map(message)
    .collect()
}

/// The viewer's scale in `dialog`, in percent.
async fn zoom(journey: &Journey, dialog: &str) -> anyhow::Result<u32> {
    let text = journey
        .text(&format!("{dialog} [data-testid=image-zoom]"))
        .await?;
    Ok(text.trim_end_matches('%').parse()?)
}

/// Waits for the image in `dialog` to load; `[natural width, rendered width]`.
async fn loaded(journey: &Journey, dialog: &str) -> anyhow::Result<(f64, f64)> {
    journey
        .until("the full view's image to load", async |page| {
            let value = page
                .eval(
                    "const i = document.querySelector(arguments[0]);
                     if (!i || i.dataset.loaded !== 'true' || !i.complete) return null;
                     return [i.naturalWidth, i.getBoundingClientRect().width];",
                    vec![json!(format!("{dialog} img[data-testid=image-after]"))],
                )
                .await?;
            Ok(serde_json::from_value::<Option<(f64, f64)>>(value)?)
        })
        .await
}

/// The message's image in the transcript, once decoded: `[natural width, width in px, width in rem]`.
async fn inline_image(journey: &Journey) -> anyhow::Result<(f64, f64, f64)> {
    journey
        .until("the message's image to decode", async |page| {
            let value = page
                .eval(
                    "const i = document.querySelector('[data-testid=message-image]');
                     if (!i || !i.complete || i.naturalWidth === 0) return null;
                     const rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
                     const width = i.getBoundingClientRect().width;
                     return [i.naturalWidth, width, width / rem];",
                    Vec::new(),
                )
                .await?;
            Ok(serde_json::from_value::<Option<(f64, f64, f64)>>(value)?)
        })
        .await
}

/// Opens the message's image (`inline_width` px wide in the transcript): as large as the window, fitted, at 100 %, then
/// zoomed by Ctrl with the wheel; Escape goes back to it.
async fn zooms_the_message_image(journey: &Journey, inline_width: f64) -> anyhow::Result<()> {
    journey.click(OPEN).await?;
    journey.settled(MESSAGE_DIALOG).await?;
    let (natural, fitted_width) = loaded(journey, MESSAGE_DIALOG).await?;
    assert_eq!(natural, f64::from(WIDTH));
    let size: (f64, f64) = serde_json::from_value(
        journey
            .eval(
                "const r = document.querySelector(arguments[0]).getBoundingClientRect();
                 return [r.width / innerWidth, r.height / innerHeight];",
                vec![json!(MESSAGE_DIALOG)],
            )
            .await?,
    )?;
    assert!(
        size.0 > 0.9 && size.1 > 0.9,
        "as large as the window: {size:?}"
    );
    assert_eq!(
        journey
            .attr(
                &format!("{MESSAGE_DIALOG} [data-testid=image-fit]"),
                "aria-pressed"
            )
            .await?,
        Some("true".to_owned())
    );
    let fitted = zoom(journey, MESSAGE_DIALOG).await?;
    assert!(
        fitted < 100 && fitted_width > inline_width,
        "fitted at {fitted}%, {fitted_width}px wide: wider than the transcript's {inline_width}px"
    );
    journey.screenshot("chat-image-fitted").await?;

    // 100 %: its own pixels.
    journey
        .click(&format!("{MESSAGE_DIALOG} [data-testid=image-actual]"))
        .await?;
    assert_eq!(zoom(journey, MESSAGE_DIALOG).await?, 100);
    let (_, actual) = loaded(journey, MESSAGE_DIALOG).await?;
    assert!(
        (actual - f64::from(WIDTH)).abs() < 1.0,
        "its own pixels: {actual}"
    );
    journey.screenshot("chat-image-actual").await?;

    // Ctrl with the wheel zooms the image (×e per 500 px up): the viewer takes the event (prevents the default) and
    // the page's own zoom never sees it.
    let prevented = journey
        .eval(
            "const viewport = document.querySelector(arguments[0]);
             const box = viewport.getBoundingClientRect();
             const wheel = new WheelEvent('wheel', { bubbles: true, cancelable: true, ctrlKey: true, deltaY: -500,
               clientX: box.left + 200, clientY: box.top + 150 });
             viewport.dispatchEvent(wheel);
             return wheel.defaultPrevented;",
            vec![json!(format!(
                "{MESSAGE_DIALOG} [data-testid=image-viewport]"
            ))],
        )
        .await?;
    assert_eq!(prevented, json!(true));
    journey
        .until("the wheel's zoom", async |page| {
            Ok((zoom(page, MESSAGE_DIALOG).await? == 272).then_some(()))
        })
        .await?;
    journey.screenshot("chat-image-wheel").await?;

    journey.keys(Key::Escape).await?;
    journey.gone(MESSAGE_DIALOG).await?;
    journey.until_focused("message-image-open").await
}

/// Opens the `read` card's thumbnail in the same viewer, whose buttons zoom it; Escape goes back to the thumbnail.
async fn zooms_the_result_image(journey: &Journey) -> anyhow::Result<()> {
    journey.click(THUMBNAIL).await?;
    journey.settled(TOOL_DIALOG).await?;
    loaded(journey, TOOL_DIALOG).await?;
    assert_eq!(
        journey
            .attr(
                &format!("{TOOL_DIALOG} img[data-testid=image-after]"),
                "alt"
            )
            .await?,
        Some("Image from read grid.svg".to_owned())
    );
    let before = zoom(journey, TOOL_DIALOG).await?;
    for _ in 0..2 {
        journey
            .click(&format!("{TOOL_DIALOG} [data-testid=image-zoom-in]"))
            .await?;
    }
    let after = zoom(journey, TOOL_DIALOG).await?;
    assert!(after > before, "zoomed in: {before}% → {after}%");
    journey.screenshot("chat-image-tool").await?;
    journey.keys(Key::Escape).await?;
    journey.gone(TOOL_DIALOG).await?;
    journey.until_focused("tool-image").await
}

// spec: E2E-75, UI-474, UI-475, UI-476, UI-549, UI-550
#[tokio::test(flavor = "multi_thread")]
async fn the_chats_images_open_zoomable_as_large_as_the_window() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/images"),
        FakeRun::fresh().with_history(history()),
    );
    journey.connected().await?;
    journey.create_thread("images").await?;

    // The message's image, shrunk into the transcript: 32rem at most, a button that says what it does.
    let (natural, width, rem) = inline_image(&journey).await?;
    assert_eq!(natural, f64::from(WIDTH));
    assert!(rem <= 32.0 + 0.05, "at most 32rem wide, not {rem}rem");
    assert_eq!(
        journey.attr(OPEN, "aria-label").await?,
        Some("Open the attached image".to_owned())
    );
    journey.screenshot("chat-image-transcript").await?;

    zooms_the_message_image(&journey, width).await?;
    zooms_the_result_image(&journey).await?;
    journey.finish("chat images").await
}
