//! Switching views (UI-390, UI-392) against a daemon on the fake launcher, in headless Chromium: a workspace widget
//! shows a file through `useFile`, whose reads a script installed before the page's scripts holds back for
//! [`SLOW_MS`] (a slow call). The test switches away and back, then through enough views that the widget's view is
//! dropped, and back again, measuring from the click on the view's tab to the widget showing the file's text, and
//! what the widget shows the moment its view shows.
//!
//! The test asserts that both switches back show the text at once (the kept view; the window's last result of the
//! dropped one) and prints the timings (TST-108). The benchmark prints the timings only, so it also runs against a
//! bundle without kept views for a comparison; it is ignored, and `just bench` runs it by hand (TST-115), also on
//! another bundle (`OMFP_WEB_BUNDLE=<bundle> just bench`).

use std::path::Path;

use e2e::{Journey, row};
use omfp_daemon::agent::FakeLauncher;
use omfp_protocol::method::ThreadsCreate;
use omfp_protocol::{CreateThreadParams, CreateWorkspaceParams, ThreadWorkspace};
use serde_json::{Value, json};

/// How long the page holds back each read of the widget's file.
const SLOW_MS: u64 = 500;
/// The file the widget shows, and its text.
const FILE: &str = "notes.md";
const TEXT: &str = "ready";

/// Holds back every response to a `workspaces.read_file` of [`FILE`] for [`SLOW_MS`], installed before the page's
/// scripts: the client sets `socket.onmessage` and sends JSON request frames.
fn slow_calls_script() -> String {
    format!(
        r#"(() => {{
  const held = new Set();
  const send = WebSocket.prototype.send;
  WebSocket.prototype.send = function (data) {{
    if (typeof data === "string") {{
      const frame = JSON.parse(data);
      if (frame.type === "request" && frame.method === "workspaces.read_file" && frame.params.path === "{FILE}") {{
        held.add(frame.id);
      }}
    }}
    return send.call(this, data);
  }};
  const descriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, "onmessage");
  Object.defineProperty(WebSocket.prototype, "onmessage", {{
    configurable: true,
    get() {{ return descriptor.get.call(this); }},
    set(handler) {{
      descriptor.set.call(this, handler === null ? null : function (event) {{
        if (typeof event.data === "string") {{
          const frame = JSON.parse(event.data);
          if (frame.type === "response" && held.delete(frame.id)) {{
            setTimeout(() => handler.call(this, event), {SLOW_MS});
            return;
          }}
        }}
        handler.call(this, event);
      }});
    }},
  }});
}})();"#
    )
}

/// The widget: the file's text, empty until it is read.
const WIDGET: &str = r#"import { useFile } from "omfp/sdk";

export default function Slow() {
  const { file } = useFile("notes.md", { follow: false });
  return <p data-testid="slow-text">{file?.content ?? ""}</p>;
}
"#;

/// Views: `slow` holds the widget; `b`, `c` and `d` push it out of the kept views (two besides the shown one).
fn manifest() -> Value {
    json!({
        "default": "b",
        "views": {
            "slow": { "title": "Slow", "layout": ["row", {}, "omfp:chat", "./widgets/slow"] },
            "b": { "title": "B", "layout": ["row", {}, "omfp:chat", "omfp:workspace"] },
            "c": { "title": "C", "layout": ["row", {}, "omfp:chat", "omfp:changes"] },
            "d": { "title": "D", "layout": "omfp:chat" }
        }
    })
}

fn write_workspace(folder: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(folder.join(".omfp/widgets/slow"))?;
    std::fs::write(folder.join(".omfp/widgets/slow/index.tsx"), WIDGET)?;
    std::fs::write(folder.join(FILE), TEXT)?;
    std::fs::write(
        folder.join(".omfp/views.json"),
        serde_json::to_string_pretty(&manifest())?,
    )?;
    Ok(())
}

/// Starts the journey with the slow reads, a thread in a new workspace holding the views, and the thread selected.
async fn start() -> anyhow::Result<Journey> {
    let journey = Journey::start_fake(FakeLauncher::new()).await?;
    let cdp = journey.driver.cdp();
    cdp.send_raw(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": slow_calls_script() }),
    )
    .await?;
    journey.driver.refresh().await?;
    journey.connected().await?;
    let mut connection = journey.connect().await?;
    let thread = connection
        .call::<ThreadsCreate>(CreateThreadParams {
            title: None,
            workspace: ThreadWorkspace::New(CreateWorkspaceParams {
                name: Some("views".to_owned()),
                link: None,
            }),
            model: None,
            thinking_level: None,
            approval_mode: None,
            prompt: None,
            images: Vec::new(),
        })
        .await?;
    connection.close().await?;
    journey.click(&row(&thread.id)).await?;
    write_workspace(&journey.workspace_folder(&thread.id).await?)?;
    journey
        .element("[data-testid=mode-view][data-view=slow]")
        .await?;
    Ok(journey)
}

/// Clicks the tab of view `id` and waits until it shows.
async fn switch(journey: &Journey, id: &str) -> anyhow::Result<()> {
    journey
        .click(&format!("[data-testid=mode-view][data-view={id}]"))
        .await?;
    journey
        .element(&format!("[data-testid=view][data-view={id}]"))
        .await?;
    Ok(())
}

/// One switch to `slow`: the ms from the tab's click to the widget showing [`TEXT`], and the widget's text the moment
/// the view showed.
struct Switch {
    ms: f64,
    at_show: String,
}

async fn switch_timed(journey: &Journey) -> anyhow::Result<Switch> {
    let script = "const [tabCss, viewCss, done] = arguments;
        const started = performance.now();
        let atShow = null;
        const check = () => {
          const view = document.querySelector(viewCss);
          if (view === null) { return false; }
          const text = view.querySelector('[data-testid=slow-text]')?.textContent ?? '';
          if (atShow === null) { atShow = text; }
          if (text === '') { return false; }
          done([performance.now() - started, atShow]);
          return true;
        };
        const observer = new MutationObserver(() => { if (check()) { observer.disconnect(); } });
        observer.observe(document.body, { childList: true, subtree: true, characterData: true, attributes: true });
        document.querySelector(tabCss).click();";
    let value = journey
        .driver
        .execute_async(
            script,
            vec![
                json!("[data-testid=mode-view][data-view=slow]"),
                json!("[data-testid=view][data-view=slow]"),
            ],
        )
        .await?
        .json()
        .clone();
    let (ms, at_show): (f64, String) = serde_json::from_value(value)?;
    Ok(Switch { ms, at_show })
}

/// The first show of `slow` (its module loads, its file is read), then `rounds` switches back from `b` (the view
/// kept) and `rounds` after `b`, `c` and `d` (the view dropped), each printed.
async fn run(journey: &Journey, rounds: usize) -> anyhow::Result<(Vec<Switch>, Vec<Switch>)> {
    let first = switch_timed(journey).await?;
    eprintln!(
        "first show: {:.0} ms to the text (slow read {SLOW_MS} ms)",
        first.ms
    );
    let mut kept = Vec::new();
    for round in 1..=rounds {
        switch(journey, "b").await?;
        let back = switch_timed(journey).await?;
        eprintln!(
            "switch back {round} (kept view): {:.0} ms to the text, {:?} when shown",
            back.ms, back.at_show
        );
        kept.push(back);
    }
    let mut dropped = Vec::new();
    for round in 1..=rounds {
        for id in ["b", "c", "d"] {
            switch(journey, id).await?;
        }
        let back = switch_timed(journey).await?;
        eprintln!(
            "switch back {round} (view dropped): {:.0} ms to the text, {:?} when shown",
            back.ms, back.at_show
        );
        dropped.push(back);
    }
    Ok((kept, dropped))
}

// spec: TST-108, UI-390, UI-392
#[tokio::test(flavor = "multi_thread")]
async fn switching_back_shows_the_widget_with_its_data_at_once() -> anyhow::Result<()> {
    let journey = start().await?;
    let (kept, dropped) = run(&journey, 2).await?;
    for back in kept.iter().chain(&dropped) {
        assert_eq!(
            back.at_show, TEXT,
            "the text shows with the view, not after the {SLOW_MS} ms read"
        );
    }
    journey.finish("view switch").await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark: prints timings, asserts nothing (`just bench`)"]
async fn benchmark_view_switch() -> anyhow::Result<()> {
    let journey = start().await?;
    run(&journey, 5).await?;
    journey.finish("view switch benchmark").await
}
