//! Live voice against the pinned omp (L3, M6, M7). The realtime model cannot run offline, so these pin down what omp
//! does without an `openai-codex` login and on the cheap error paths the daemon relies on (DMN-101, DMN-102,
//! DMN-104), what `live_start` does without a sound device (DMN-109: it opens the speaker before anything else, so
//! these tests take every sound device away from omp), and record the no-credential sequence as a reducer fixture
//! (TST-012, PRT-052).

mod client;
mod common;

use std::ffi::OsString;
use std::path::Path;

use clap::Parser;
use client::{Client, Follower};
use common::{Harness, Recorder, TIMEOUT, frame_type, start};
use omfp_core::{LiveOwner, ThreadEvent};
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omfp_testkit::{FAKE_MODEL, FAKE_PROVIDER, omp_bin};
use omp_rpc::{LiveConnectCommand, LiveMuteCommand, LiveStartCommand, LiveStopCommand};
use serde_json::json;
use tempfile::TempDir;

/// What omp answers without an `openai-codex` login (S-09).
const NO_CREDENTIAL: &str = "No Codex OAuth credential is available for a live call.";

/// A browser-like offer; omp checks only that it is not blank.
const OFFER: &str = "v=0\r\no=- 46117 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0 1\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=sendrecv\r\na=rtpmap:111 opus/48000/2\r\na=setup:actpass\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=mid:1\r\na=sctp-port:5000\r\n";

fn connect(offer: &str) -> LiveConnectCommand {
    LiveConnectCommand {
        sdp_offer: offer.into(),
        voice: None,
        instructions: None,
    }
}

fn command_error(error: omp_rpc::Error) -> String {
    match error {
        omp_rpc::Error::Command { error, .. } => error,
        other => panic!("expected omp's failure, got {other:?}"),
    }
}

/// The omp types of the recorded `live_*` frames.
fn live_types(events: &[ThreadEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(frame_type)
        .filter(|kind| kind.starts_with("live_"))
        .collect()
}

/// The start of omp's failure when `live_start` finds no speaker: it opens the host's speaker before it signals the
/// call, so no failure comes earlier (the missing login is checked after it).
const NO_SPEAKER: &str = "Failed to open the default speaker: ";

/// Leaves omp no sound device, so `live_start` never reaches the machine's real speaker or microphone: `PulseAudio`
/// points at a socket that does not exist (no autospawn with an explicit server) and ALSA reads an empty
/// configuration (no `default` device).
fn without_audio(env: &mut Vec<(OsString, Option<OsString>)>, root: &Path) {
    let alsa = root.join("asound.conf");
    std::fs::write(&alsa, "").unwrap();
    let mut set = |key: &str, value: OsString| env.push((key.into(), Some(value)));
    set(
        "PULSE_SERVER",
        format!("unix:{}", root.join("no-pulse").display()).into(),
    );
    set("PULSE_RUNTIME_PATH", root.join("pulse").into());
    set("ALSA_CONFIG_PATH", alsa.into());
}

// spec: DMN-109, DMN-104
#[tokio::test(flavor = "multi_thread")]
async fn live_start_without_a_sound_device() {
    let harness = Harness::new(Vec::new()).await;
    let mut spec = harness.spec(&[]);
    without_audio(&mut spec.env, harness.env.root());

    let (recorder, error) = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        let error = process
            .client()
            .call(&LiveStartCommand {
                voice: None,
                instructions: None,
            })
            .unwrap_err();
        // As for `live_connect`, omp repeats the `error` phase after `live_end`, before the response.
        recorder.wait_until("the end of the live session", |events| {
            live_types(events).len() >= 4
        });
        (recorder, command_error(error))
    })
    .await
    .unwrap();

    assert!(error.starts_with(NO_SPEAKER), "{error}");
    assert_eq!(
        live_types(&recorder.events()),
        ["live_phase", "live_phase", "live_end", "live_phase"]
    );
    let end = recorder.events().into_iter().find_map(|event| match event {
        ThreadEvent::Omp {
            frame: omp_rpc::RpcNotification::LiveEnd(end),
        } => end.error,
        _ => None,
    });
    assert_eq!(end, Some(error), "the session ended on the same failure");
    harness.assert_no_stray_network();
}

// spec: TST-012, PRT-052, DMN-104
#[tokio::test(flavor = "multi_thread")]
async fn live_connect_without_a_codex_login() {
    let harness = Harness::new(Vec::new()).await;
    let spec = harness.spec(&[]);

    let (recorder, error) = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        // omp's startup frames first (two `available_commands_update`, as in every fixture), so the daemon's event
        // below has a fixed place in the fixture.
        recorder.wait_until("the startup frames", |events| {
            Recorder::count(events, "available_commands_update") >= 2
        });
        // The daemon opens the session before it forwards the offer (DMN-100).
        recorder.push(ThreadEvent::LiveSession {
            owner: Some(LiveOwner {
                client: "contract-test".into(),
                connection: 1,
            }),
        });
        let error = process.client().call(&connect(OFFER)).unwrap_err();
        // omp repeats the `error` phase after `live_end`, before the response.
        recorder.wait_until("the end of the live session", |events| {
            live_types(events).len() >= 4
        });
        (recorder, command_error(error))
    })
    .await
    .unwrap();

    assert_eq!(error, NO_CREDENTIAL);
    assert_eq!(
        live_types(&recorder.events()),
        ["live_phase", "live_phase", "live_end", "live_phase"]
    );
    let end = recorder.events().into_iter().find_map(|event| match event {
        ThreadEvent::Omp {
            frame: omp_rpc::RpcNotification::LiveEnd(end),
        } => Some(end),
        _ => None,
    });
    assert_eq!(
        end.and_then(|end| end.error).as_deref(),
        Some(NO_CREDENTIAL)
    );
    recorder.record(&harness, "live_no_credential");
    harness.assert_no_stray_network();
}

// spec: DMN-101, DMN-102
#[tokio::test(flavor = "multi_thread")]
async fn the_cheap_live_errors_of_omp() {
    let harness = Harness::new(Vec::new()).await;
    let spec = harness.spec(&[]);

    tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        let client = process.client();

        let blank = command_error(client.call(&connect("  ")).unwrap_err());
        assert_eq!(blank, "live_connect requires a non-empty sdpOffer");
        let mute = command_error(
            client
                .call(&LiveMuteCommand { muted: Some(true) })
                .unwrap_err(),
        );
        assert_eq!(mute, "No live session is active");
        client
            .call(&LiveStopCommand {})
            .expect("live_stop succeeds without a session");

        // omp writes a session's frames before its response, so after one session's end every live frame so far
        // is that session's: the commands above started none.
        let error = command_error(client.call(&connect(OFFER)).unwrap_err());
        assert_eq!(error, NO_CREDENTIAL);
        recorder.wait_until("the session's end", |events| live_types(events).len() >= 4);
        assert_eq!(
            live_types(&recorder.events()),
            ["live_phase", "live_phase", "live_end", "live_phase"]
        );
    })
    .await
    .unwrap();
    harness.assert_no_stray_network();
}

struct Daemon {
    handle: DaemonHandle,
    _data: TempDir,
}

impl Daemon {
    async fn start(harness: &Harness) -> Self {
        let data = tempfile::tempdir().unwrap();
        let omp = omp_bin();
        let workspaces = data.path().join("workspaces");
        let cli = Cli::try_parse_from([
            "daemon",
            "--bind",
            "127.0.0.1:0",
            "--data-dir",
            data.path().to_str().unwrap(),
            "--workspaces-root",
            workspaces.to_str().unwrap(),
            "--omp-bin",
            &omp,
            "--no-title-generation",
        ])
        .unwrap();
        let config = Config::from_cli(cli, None, None).unwrap();
        let mut ports = Ports::resolve(&config).unwrap();
        let mut env = harness.env.env_overrides();
        without_audio(&mut env, harness.env.root());
        ports.omp_env = env;
        let handle = start_with(config, ports).await.unwrap();
        Self {
            handle,
            _data: data,
        }
    }

    async fn client(&self, name: &str) -> Client {
        Client::hello(self.handle.addr(), self.handle.token().expose(), name).await
    }
}

// spec: DMN-100, DMN-101, DMN-104
#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_names_the_missing_codex_login() {
    let harness = Harness::new(Vec::new()).await;
    let daemon = Daemon::start(&harness).await;
    let mut web = daemon.client("omfp/web").await;
    let thread = web
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "live", "model": format!("{FAKE_PROVIDER}/{FAKE_MODEL}") }),
        )
        .await;
    let thread = thread["id"].as_str().unwrap().to_owned();
    let mut view = Follower::snapshot(&mut web, &thread, None).await;

    let error = tokio::time::timeout(
        TIMEOUT,
        web.call(
            "thread.live_connect",
            json!({ "threadId": thread, "sdpOffer": OFFER }),
        ),
    )
    .await
    .expect("omp answers at once without a login")
    .unwrap_err();

    assert_eq!(error["code"], "omp_error", "{error}");
    assert_eq!(
        error["message"],
        "live voice needs an `openai-codex` login in omp: run `:login openai-codex`, then try again"
    );
    assert_eq!(error["details"]["ompError"], NO_CREDENTIAL);
    view.until(&mut web, |state| {
        state.live.is_none() && state.live_error.is_some()
    })
    .await;
    assert_eq!(view.state.live_error.as_deref(), error["message"].as_str());
    // omp's `live_end` freed the thread: the next try reaches omp again and fails the same way.
    let again = web
        .call(
            "thread.live_connect",
            json!({ "threadId": thread, "sdpOffer": OFFER }),
        )
        .await
        .unwrap_err();
    assert_eq!(again["code"], "omp_error", "{again}");
    harness.assert_no_stray_network();
}

// spec: DMN-109, DMN-104
#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_relays_a_failed_live_start() {
    let harness = Harness::new(Vec::new()).await;
    let daemon = Daemon::start(&harness).await;
    let mut desktop = daemon.client("omfp/desktop").await;
    let thread = desktop
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "live", "model": format!("{FAKE_PROVIDER}/{FAKE_MODEL}") }),
        )
        .await;
    let thread = thread["id"].as_str().unwrap().to_owned();
    let mut view = Follower::snapshot(&mut desktop, &thread, None).await;

    let error = tokio::time::timeout(
        TIMEOUT,
        desktop.call("thread.live_start", json!({ "threadId": thread })),
    )
    .await
    .expect("omp answers at once without a sound device")
    .unwrap_err();

    assert_eq!(error["code"], "omp_error", "{error}");
    let omp = error["details"]["ompError"].as_str().unwrap().to_owned();
    assert!(omp.starts_with(NO_SPEAKER), "{omp}");
    assert_eq!(error["message"], format!("live voice failed: {omp}"));
    view.until(&mut desktop, |state| {
        state.live.is_none() && state.live_error.is_some()
    })
    .await;
    assert_eq!(view.state.live_error.as_deref(), error["message"].as_str());
    // omp's `live_end` freed the thread: the next try reaches omp again.
    let again = desktop
        .call("thread.live_start", json!({ "threadId": thread }))
        .await
        .unwrap_err();
    assert_eq!(again["details"]["ompError"], omp, "{again}");
    harness.assert_no_stray_network();
}
