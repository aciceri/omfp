//! The window while its index loads (UI-023, TST-111), against a daemon on the fake launcher, in headless Chromium: a
//! script installed before the page's holds back the answer to the `index` subscription until the test releases it,
//! so the window is connected and still without its index for as long as the test needs.

use e2e::Journey;
use omfp_daemon::agent::FakeLauncher;
use serde_json::json;
use thirtyfour::prelude::Key;

/// Holds back the response to the `index` subscription until `releaseIndex()`: the client sets `socket.onmessage`
/// and sends JSON request frames.
const HOLD_INDEX: &str = r#"(() => {
  const held = new Set();
  const waiting = [];
  let released = false;
  window.releaseIndex = () => {
    released = true;
    for (const deliver of waiting.splice(0)) {
      deliver();
    }
  };
  const send = WebSocket.prototype.send;
  WebSocket.prototype.send = function (data) {
    if (typeof data === "string") {
      const frame = JSON.parse(data);
      if (frame.type === "request" && frame.method === "subscribe" && frame.params.topic === "index") {
        held.add(frame.id);
      }
    }
    return send.call(this, data);
  };
  const descriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, "onmessage");
  Object.defineProperty(WebSocket.prototype, "onmessage", {
    configurable: true,
    get() { return descriptor.get.call(this); },
    set(handler) {
      descriptor.set.call(this, handler === null ? null : function (event) {
        if (!released && typeof event.data === "string" && held.delete(JSON.parse(event.data).id)) {
          waiting.push(() => handler.call(this, event));
          return;
        }
        handler.call(this, event);
      });
    },
  });
})();"#;

// spec: UI-023, TST-111
#[tokio::test(flavor = "multi_thread")]
async fn a_command_line_opened_before_the_index_keeps_the_focus() -> anyhow::Result<()> {
    let journey = Journey::start_fake(FakeLauncher::new()).await?;
    journey
        .driver
        .cdp()
        .send_raw(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": HOLD_INDEX }),
        )
        .await?;
    journey.driver.refresh().await?;

    // Connected without its index: the column is busy, and `:` opens the command line.
    journey.element("[data-testid=view]").await?;
    journey.gone("[data-testid=connection-banner]").await?;
    assert_eq!(
        journey
            .count("[data-testid=sidebar][aria-busy=true]")
            .await?,
        1
    );
    journey.keys(":").await?;
    journey.until_focused("command-line-input").await?;

    // The index arrives without a workspace: the column shows "+ workspace" and leaves the focus on the command line.
    journey.eval("releaseIndex();", Vec::new()).await?;
    journey
        .gone("[data-testid=sidebar][aria-busy=true]")
        .await?;
    journey.element("[data-testid=add-workspace]").await?;
    assert_eq!(
        journey.focused().await?.as_deref(),
        Some("command-line-input")
    );
    journey.keys(Key::Escape).await?;
    journey.gone("[data-testid=command-line]").await?;

    // Settled now: `connected` holds and the command line runs.
    journey.connected().await?;
    journey.scheme("light").await?;
    journey.finish("the window while its index loads").await
}

// spec: UI-091, UI-510, UI-794
#[tokio::test(flavor = "multi_thread")]
async fn the_page_shows_the_daemons_theme_and_skin_before_its_index() -> anyhow::Result<()> {
    let journey = Journey::start_fake(FakeLauncher::new()).await?;
    let mut connection = journey.connect().await?;
    connection
        .call::<omfp_protocol::method::SettingsSet>(omfp_protocol::SetSettingsParams {
            daemon: None,
            ui: Some(omfp_protocol::UiSettings {
                theme: Some(omfp_protocol::BuiltinTheme::Graphite),
                skin: Some(omfp_protocol::Skin::Boring),
                ..omfp_protocol::UiSettings::default()
            }),
            keymap: None,
        })
        .await?;
    connection.close().await?;
    journey
        .driver
        .cdp()
        .send_raw(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": HOLD_INDEX }),
        )
        .await?;
    journey.driver.refresh().await?;

    // Connected without its index, the window already shows the daemon's theme and skin, served on `<html>`.
    journey.element("[data-testid=view]").await?;
    journey.gone("[data-testid=connection-banner]").await?;
    let appearance = async || -> anyhow::Result<_> {
        Ok((
            journey.attr("html", "data-theme").await?,
            journey.attr("html", "data-skin").await?,
        ))
    };
    let expected = (Some("graphite".to_owned()), Some("boring".to_owned()));
    assert_eq!(appearance().await?, expected);

    // The index brings the same settings: nothing changes.
    journey.eval("releaseIndex();", Vec::new()).await?;
    journey
        .gone("[data-testid=sidebar][aria-busy=true]")
        .await?;
    assert_eq!(appearance().await?, expected);
    journey.finish("the served theme before the index").await
}
