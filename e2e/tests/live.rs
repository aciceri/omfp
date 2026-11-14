//! Live voice (E2E-14, M6) in headless Chromium with a fake microphone, against a daemon on the fake launcher: its
//! canned SDP answer is no real peer, so the peer never connects and the panel stays `connecting` (the states after
//! `connected` are the component tests' job, UI-181..UI-184). The journey checks the UI states and the commands omp
//! receives, not audio. Screenshots of the panel states land in the run's screenshot directory
//! (`omfp-live-*.png`).
//! A second journey plays the desktop's path (UI-188): a page without WebRTC on the daemon's machine (`#local=1`)
//! starts omp's own session (`live_start`).

use e2e::Journey;
use omfp_daemon::agent::{
    FakeCall, FakeLauncher, FakeLive, FakeProcess, OmpCall, live_levels, live_phase,
};
use omfp_protocol::LiveConnectParams;
use omfp_protocol::method::ThreadLiveConnect;
use thirtyfour::prelude::*;

const PANEL: &str = "[data-testid=live-panel]";
const LOGIN: &str =
    "live voice needs an `openai-codex` login in omp: run `:login openai-codex`, then try again";

/// The live commands the thread's omp received, in order: `connect`, `start`, `mute <bool>`, `stop`.
fn live_calls(process: &FakeProcess) -> Vec<String> {
    process
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            FakeCall::Call(OmpCall::LiveConnect(_)) => Some("connect".to_owned()),
            FakeCall::Call(OmpCall::LiveStart(_)) => Some("start".to_owned()),
            FakeCall::Call(OmpCall::LiveMute(command)) => {
                Some(format!("mute {}", command.muted.unwrap_or_default()))
            }
            FakeCall::Call(OmpCall::LiveStop) => Some("stop".to_owned()),
            _ => None,
        })
        .collect()
}

/// Waits until omp received exactly `expected` live commands.
async fn until_calls(
    journey: &Journey,
    launcher: &FakeLauncher,
    thread: &str,
    expected: &[&str],
) -> anyhow::Result<()> {
    let process = journey.fake_process(launcher, thread).await?;
    journey
        .until(&format!("omp to receive {expected:?}"), async |_| {
            Ok((live_calls(&process) == expected).then_some(()))
        })
        .await
}

/// Waits until the panel shows `view` (`own`, `elsewhere`, `error`) with status `status`.
async fn until_panel(journey: &Journey, view: &str, status: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("the {view} panel {status}"), async |page| {
            let views = page.attr(PANEL, "data-view").await?;
            let statuses = page.attr(PANEL, "data-status").await?;
            Ok(
                (views.as_deref() == Some(view) && statuses.as_deref() == Some(status))
                    .then_some(()),
            )
        })
        .await
}

// spec: E2E-14
#[tokio::test(flavor = "multi_thread")]
async fn live_voice_from_the_mic_button_mute_end_and_the_login_message() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    launcher.script_live(FakeLive::connect(FakeLive::call_frames()));
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;
    let thread = journey.create_thread("voice").await?;

    // The mic button opens the fake microphone and a peer; the daemon relays the offer to omp.
    journey.click("[data-testid=live-mic]").await?;
    until_calls(&journey, &launcher, &thread, &["connect"]).await?;
    journey
        .until("the scripted transcript", async |page| {
            let lines = page.texts(&format!("{PANEL} .omfp-live__line")).await?;
            Ok((lines == ["Ciao, mi senti? Dimmi solo sì", "Sì."]).then_some(()))
        })
        .await?;
    // omp listens, but the canned answer never connects the peer: the panel keeps `connecting` (UI-181).
    until_panel(&journey, "own", "connecting").await?;
    assert_eq!(
        journey
            .attr("[data-testid=live-mic]", "aria-pressed")
            .await?
            .as_deref(),
        Some("true")
    );
    journey.until_focused("live-panel").await?;
    journey.screenshot("live-connecting").await?;

    // Space mutes: the local track and `live_mute` (omp moves to `muted`).
    journey.keys(" ").await?;
    until_calls(&journey, &launcher, &thread, &["connect", "mute true"]).await?;
    journey
        .until("the unmute hint", async |page| {
            let muted = page.attr(PANEL, "data-muted").await?;
            let hints = page.text(&format!("{PANEL} .omfp-live__status")).await?;
            Ok((muted.as_deref() == Some("true") && hints.contains("unmute")).then_some(()))
        })
        .await?;
    journey.screenshot("live-muted").await?;

    // Esc ends the session: `live_stop`, the panel goes with omp's `live_end`.
    journey.keys(Key::Escape).await?;
    until_calls(
        &journey,
        &launcher,
        &thread,
        &["connect", "mute true", "stop"],
    )
    .await?;
    journey.gone(PANEL).await?;

    // A session another client started shows its state and an end button.
    let mut other = journey.connect().await?;
    other
        .call::<ThreadLiveConnect>(LiveConnectParams {
            thread_id: thread.clone(),
            sdp_offer: "v=0\r\na=setup:actpass\r\n".to_owned(),
        })
        .await?;
    until_panel(&journey, "elsewhere", "listening").await?;
    assert!(
        journey.text(PANEL).await?.contains("in another window"),
        "the panel names the other window"
    );
    journey.screenshot("live-elsewhere").await?;
    journey.click("[data-testid=live-end]").await?;
    until_calls(
        &journey,
        &launcher,
        &thread,
        &["connect", "mute true", "stop", "connect", "stop"],
    )
    .await?;
    journey.gone(PANEL).await?;
    other.close().await?;

    // Without an `openai-codex` login omp fails the call: the panel shows the login message (UI-186).
    launcher.script_live(FakeLive::no_credential());
    journey.click("[data-testid=live-mic]").await?;
    journey
        .until("the login message", async |page| {
            let alert = page.text(&format!("{PANEL} [role=alert]")).await?;
            Ok((alert == LOGIN).then_some(()))
        })
        .await?;
    until_panel(&journey, "error", "error").await?;
    journey.screenshot("live-error").await?;

    journey.finish("live voice").await
}

// spec: E2E-14, UI-188, UI-189
#[tokio::test(flavor = "multi_thread")]
async fn without_webrtc_on_the_daemons_machine_the_mic_starts_omps_own_session()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    launcher.script_live(FakeLive::connect(vec![
        live_levels(0.2, 0.3),
        live_phase(omfp_core::wire::LivePhase::Speaking),
    ]));
    let journey = Journey::start_fake(launcher.clone()).await?;
    journey.connected().await?;
    // The desktop's page: `#local=1` (the token is stored by now), and no WebRTC as in WebKitGTK.
    journey
        .eval("location.hash = 'local=1'; location.reload();", Vec::new())
        .await?;
    journey.connected().await?;
    journey
        .eval("delete window.RTCPeerConnection;", Vec::new())
        .await?;
    let thread = journey.create_thread("desktop voice").await?;

    journey.click("[data-testid=live-mic]").await?;
    until_calls(&journey, &launcher, &thread, &["start"]).await?;
    // omp's own phases, `speaking` included (UI-189).
    until_panel(&journey, "own", "speaking").await?;
    journey.until_focused("live-panel").await?;
    journey.screenshot("live-host-speaking").await?;

    journey.keys(" ").await?;
    until_calls(&journey, &launcher, &thread, &["start", "mute true"]).await?;
    journey.keys(Key::Escape).await?;
    until_calls(
        &journey,
        &launcher,
        &thread,
        &["start", "mute true", "stop"],
    )
    .await?;
    journey.gone(PANEL).await?;

    journey.finish("live voice on the daemon's machine").await
}
