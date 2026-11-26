//! The web frame journey of M17 (E2E-67, UI-670..UI-673, UI-679): a scripted agent mounts `omfp:web` on a page this
//! test serves on another loopback port; the page shows inside the frame, in its own origin, gets no referrer, cannot
//! reach the window nor navigate it, and Reload loads it again. Takes `omfp-web-frame.png` (in `/tmp` or
//! `OMFP_SCREENSHOT_DIR`). Run with `just test-e2e`.

use std::sync::{Arc, Mutex};

use e2e::Journey;
use fake_llm::{Reply, Step, ToolCall};
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// The framed page: it says what it could reach, then tries to navigate the window (its sandbox forbids it).
const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Framed</title></head>
<body style="font: 16px sans-serif; background: #fff; color: #222">
<h1 id="framed">Framed dev server</h1>
<script>
  const body = document.body;
  body.dataset.origin = location.origin;
  try { void parent.document.title; body.dataset.parent = "reachable"; } catch { body.dataset.parent = "isolated"; }
  try { top.location.href = "http://127.0.0.1:9/escaped"; } catch { body.dataset.top = "refused"; }
</script>
</body></html>
"#;

const FRAME_CSS: &str =
    "[data-testid=view] [data-widget=\"omfp:web\"][data-instance=preview] [data-testid=web-frame]";

/// Serves [`PAGE`] on `127.0.0.1:<port 0>` and records each request's head.
async fn serve_page() -> anyhow::Result<(String, Arc<Mutex<Vec<String>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/preview", listener.local_addr()?);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buffer = [0_u8; 1024];
                while !head.windows(4).any(|end| end == b"\r\n\r\n") {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => head.extend_from_slice(&buffer[..read]),
                    }
                }
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).into_owned());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{PAGE}",
                    PAGE.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    Ok((url, requests))
}

/// The page requests the server saw (favicon requests left out).
fn page_requests(requests: &Mutex<Vec<String>>) -> Vec<String> {
    requests
        .lock()
        .unwrap()
        .iter()
        .filter(|head| head.starts_with("GET /preview "))
        .cloned()
        .collect()
}

// spec: E2E-67, UI-670, UI-672, UI-673, UI-679
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_frames_a_local_page_that_cannot_reach_or_navigate_the_window()
-> anyhow::Result<()> {
    let (url, requests) = serve_page().await?;
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_mount_web",
            "omfp_ui_mount",
            json!({
                "id": "preview",
                "widget": "omfp:web",
                "place": "end",
                "props": { "url": url, "title": "Preview" }
            }),
        )])),
        Step::reply(Reply::text("The preview is open.")),
    ])
    .await?;
    journey.connected().await?;
    journey.create_thread("web frame journey").await?;
    let window = journey.driver.current_url().await?;

    journey.send("show me the dev server").await?;
    journey.reply("The preview is open.").await?;
    journey
        .until("the framed page's load", async |page| {
            Ok(
                (page.attr(FRAME_CSS, "data-loaded").await?.as_deref() == Some("true"))
                    .then_some(()),
            )
        })
        .await?;
    assert_eq!(
        journey
            .attr(&format!("{FRAME_CSS} [data-testid=web-frame-url]"), "title")
            .await?
            .as_deref(),
        Some(url.as_str())
    );
    let frame = format!("{FRAME_CSS} [data-testid=web-frame-frame]");
    assert_eq!(
        journey.attr(&frame, "title").await?.as_deref(),
        Some("Preview")
    );
    assert_eq!(
        journey.attr(&frame, "sandbox").await?.as_deref(),
        Some(
            "allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"
        )
    );

    // Inside the frame: the page in its own origin, cut off from the window.
    journey.element(&frame).await?.enter_frame().await?;
    let inside = journey
        .eval(
            "const b = document.body; return [document.querySelector('#framed').textContent, b.dataset.origin, \
             b.dataset.parent, document.referrer];",
            Vec::new(),
        )
        .await;
    journey.driver.enter_default_frame().await?;
    let origin = url.trim_end_matches("/preview").to_owned();
    assert_eq!(
        inside?,
        json!(["Framed dev server", origin, "isolated", ""])
    );
    journey.screenshot("web-frame").await?;

    // Reload loads the page again; the window never left the daemon's page.
    let before = page_requests(&requests).len();
    journey
        .click(&format!("{FRAME_CSS} [data-testid=web-frame-reload]"))
        .await?;
    journey
        .until("the page requested again", async |_| {
            Ok((page_requests(&requests).len() > before).then_some(()))
        })
        .await?;
    journey
        .until("the reloaded page's load", async |page| {
            Ok(
                (page.attr(FRAME_CSS, "data-loaded").await?.as_deref() == Some("true"))
                    .then_some(()),
            )
        })
        .await?;
    assert_eq!(journey.driver.current_url().await?, window);
    // No referrer leaves the window (the daemon's address and the thread in it).
    let heads = page_requests(&requests);
    assert!(heads.len() >= 2, "{heads:?}");
    assert!(
        heads
            .iter()
            .all(|head| !head.to_lowercase().contains("\r\nreferer:")),
        "{heads:?}"
    );
    journey.finish("E2E-67 web frame").await
}
