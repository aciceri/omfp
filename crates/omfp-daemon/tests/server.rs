//! Integration tests (L2) of the daemon over a real WebSocket: a daemon on port 0 with a temporary data directory,
//! the fixture-driven fake launcher and fake git, and a native protocol client.

mod client;

use std::path::Path;
use std::sync::Arc;

use clap::Parser;
use client::{Client, Follower};
use omfp_core::ThreadEvent;
use omfp_core::wire::{RpcAgentEvent, RpcNotification};
use omfp_daemon::agent::{FakeLauncher, FakeReply, FakeRun};
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, discover, start_with};
use omfp_protocol::{ServerFrame, TopicEvent};
use serde_json::{Value, json};
use tempfile::TempDir;

struct TestDaemon {
    handle: DaemonHandle,
    launcher: FakeLauncher,
    git: Arc<FakeGit>,
    dir: TempDir,
}

fn config(dir: &Path, args: &[&str]) -> Config {
    let data_dir = dir.to_str().unwrap();
    let workspaces = dir.join("workspaces");
    let cli = Cli::try_parse_from(
        [
            "daemon",
            "--bind",
            "127.0.0.1:0",
            "--data-dir",
            data_dir,
            "--workspaces-root",
            workspaces.to_str().unwrap(),
        ]
        .into_iter()
        .chain(args.iter().copied()),
    )
    .unwrap();
    Config::from_cli(cli, None, None).unwrap()
}

/// The ports of a test daemon over `dir`, its standalone omp at `<dir>/standalone-home` ([`standalone_home`]).
fn ports(dir: &Path, launcher: &FakeLauncher, git: &Arc<FakeGit>) -> Ports {
    Ports {
        launcher: Arc::new(launcher.clone()),
        git: Arc::clone(git) as _,
        omp_program: "omp".into(),
        omp_version: omfp_core::OMP_VERSION.into(),
        omp_env: Vec::new(),
        user_omp_env: omfp_testkit::absent_standalone_omp(dir),
        warnings: vec!["running anyway (--allow-omp-mismatch)".into()],
        clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
    }
}

async fn daemon(args: &[&str]) -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let launcher = FakeLauncher::new();
    let git = Arc::new(FakeGit::new());
    let handle = start_with(config(dir.path(), args), ports(dir.path(), &launcher, &git))
        .await
        .unwrap();
    TestDaemon {
        handle,
        launcher,
        git,
        dir,
    }
}

/// The home of the synthetic standalone omp `auth.importable`/`auth.import` read (DMN-229,
/// [`omfp_testkit::absent_standalone_omp`]): absent until a test creates its `.omp`, so nothing reads the developer's
/// own home.
fn standalone_home(dir: &Path) -> std::path::PathBuf {
    dir.join("standalone-home")
}

impl TestDaemon {
    async fn client(&self, name: &str) -> Client {
        Client::hello(self.handle.addr(), self.handle.token().expose(), name).await
    }
}

fn fixture(name: &str) -> FakeRun {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/threads")
        .join(omfp_core::OMP_VERSION)
        .join(format!("{name}.jsonl"));
    FakeRun::from_fixture(&path).unwrap()
}

async fn create_thread(client: &mut Client) -> String {
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "test thread" }),
        )
        .await;
    thread["id"].as_str().unwrap().to_owned()
}

// spec: DMN-020
#[tokio::test(flavor = "multi_thread")]
async fn thread_creation_requires_an_explicit_workspace_without_side_effects() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let error = client.call("threads.create", json!({})).await.unwrap_err();
    assert_eq!(error["code"], "bad_request");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("{ id:") && message.contains("{ new:"),
        "{message}"
    );
    assert_eq!(
        listed(&mut client, "threads.list", "threads").await,
        Vec::<String>::new()
    );
    assert_eq!(
        listed(&mut client, "workspaces.list", "workspaces").await,
        Vec::<String>::new()
    );

    let workspace = client
        .ok("workspaces.create", json!({ "name": "existing" }))
        .await;
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "id": workspace["id"] } }),
        )
        .await;
    assert_eq!(thread["workspaceId"], workspace["id"]);
    assert_eq!(
        listed(&mut client, "workspaces.list", "workspaces").await,
        [workspace["id"].as_str().unwrap()]
    );

    let new_thread = client
        .ok("threads.create", json!({ "workspace": { "new": {} } }))
        .await;
    assert_ne!(new_thread["workspaceId"], workspace["id"]);
    assert_eq!(
        listed(&mut client, "workspaces.list", "workspaces")
            .await
            .len(),
        2
    );
    assert_eq!(
        listed(&mut client, "threads.list", "threads").await.len(),
        2
    );
}

fn hello(token: &str, protocol: u32) -> String {
    json!({
        "type": "request", "id": "h1", "method": "system.hello",
        "params": { "token": token, "protocol": protocol, "client": "test" }
    })
    .to_string()
}

// spec: PRT-001
#[tokio::test(flavor = "multi_thread")]
async fn any_frame_before_hello_closes_the_connection_with_4401() {
    let daemon = daemon(&[]).await;
    for first in [
        json!({ "type": "request", "id": "1", "method": "repos.suggest" }).to_string(),
        "not json".to_owned(),
    ] {
        let mut client = Client::connect(daemon.handle.addr()).await;
        client.send_text(first).await;
        assert_eq!(client.close_code().await, Some(4401));
    }
}

// spec: PRT-002, PRT-001
#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_token_is_unauthorized_and_closes_the_connection() {
    let daemon = daemon(&[]).await;
    let mut client = Client::connect(daemon.handle.addr()).await;

    client.send_text(hello("not-the-token", 1)).await;

    let response = client.frame().await;
    assert_eq!(response["error"]["code"], "unauthorized");
    assert_eq!(client.close_code().await, Some(4401));
}

// spec: DMN-184
#[tokio::test(start_paused = true)]
async fn a_connection_without_hello_is_closed_after_10_seconds() {
    let daemon = daemon(&[]).await;
    let mut client = Client::connect(daemon.handle.addr()).await;
    let start = tokio::time::Instant::now();

    assert_eq!(client.close_code().await, Some(4401));
    assert!(start.elapsed() >= std::time::Duration::from_secs(10));
}

// spec: DMN-184
#[tokio::test(flavor = "multi_thread")]
async fn at_most_64_connections_wait_for_hello_at_once() {
    let daemon = daemon(&[]).await;
    let addr = daemon.handle.addr();
    let mut waiting = Vec::new();
    for _ in 0..omfp_daemon::server::UNAUTHENTICATED_CONNECTIONS {
        waiting.push(Client::connect(addr).await);
    }
    let refused = |result: Result<_, tokio_tungstenite::tungstenite::Error>| match result {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => response.status() == 503,
        _ => false,
    };
    assert!(refused(
        tokio_tungstenite::connect_async(client::url(addr)).await
    ));

    // A connection that authenticates frees its place, before its hello is answered.
    let first = &mut waiting[0];
    let hello = first
        .call(
            "system.hello",
            json!({ "token": daemon.handle.token().expose(), "protocol": 1, "client": "test" }),
        )
        .await;
    assert!(hello.is_ok(), "{hello:?}");
    let mut late = Client::hello(addr, daemon.handle.token().expose(), "late").await;
    late.ok("threads.list", json!({})).await;
    waiting.push(Client::connect(addr).await);
    assert!(refused(
        tokio_tungstenite::connect_async(client::url(addr)).await
    ));
}

/// Whether a browser at `origin` reaching the daemon with `host` upgrades and passes `system.hello`; a refused upgrade
/// is `403`.
async fn upgrades(daemon: &TestDaemon, origin: &str, host: &str) -> bool {
    match Client::connect_with_origin_and_host(daemon.handle.addr(), origin, host).await {
        Ok(mut client) => {
            client
                .send_text(hello(daemon.handle.token().expose(), 1))
                .await;
            assert_eq!(client.frame().await["ok"], true, "{origin}");
            true
        }
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), 403, "{origin}");
            false
        }
        Err(error) => panic!("{origin}: {error}"),
    }
}

// spec: PRT-003, PRT-070
#[tokio::test(flavor = "multi_thread")]
async fn without_a_list_only_the_daemons_own_origin_upgrades() {
    let daemon = daemon(&[]).await;
    let own = daemon.handle.addr().to_string();

    assert!(upgrades(&daemon, &format!("http://{own}"), &own).await);
    assert!(!upgrades(&daemon, "https://evil.example", &own).await);
    assert!(
        !upgrades(&daemon, "https://omfp.example.ts.net", &own).await,
        "a proxy that rewrites Host to the daemon's address needs the list"
    );
    assert!(
        upgrades(
            &daemon,
            "https://omfp.example.ts.net",
            "omfp.example.ts.net"
        )
        .await,
        "a proxy that passes the browser's Host on needs no list"
    );
}

// spec: PRT-003, PRT-070, DMN-140
#[tokio::test(flavor = "multi_thread")]
async fn upgrades_from_listed_origins_pass_and_others_are_rejected() {
    let daemon = daemon(&[
        "--allowed-origin",
        "https://omfp.example.ts.net/",
        "--allowed-origin",
        "http://100.64.0.7:7420",
    ])
    .await;
    let own = daemon.handle.addr().to_string();

    for origin in [
        format!("http://{own}"),
        "https://omfp.example.ts.net".to_owned(),
        "http://100.64.0.7:7420".to_owned(),
    ] {
        assert!(upgrades(&daemon, &origin, &own).await, "{origin}");
    }
    for origin in [
        "https://evil.example",
        "http://omfp.example.ts.net",
        "https://omfp.example.ts.net:8443",
        "https://x.omfp.example.ts.net",
        "http://100.64.0.7:7421",
        "null",
    ] {
        assert!(!upgrades(&daemon, origin, &own).await, "{origin}");
    }
}

// spec: PRT-005, PRT-004, PRT-032, PRT-042
#[tokio::test(flavor = "multi_thread")]
async fn hello_reports_versions_and_warnings_and_refuses_unknown_protocols() {
    let daemon = daemon(&[]).await;
    let token = daemon.handle.token().expose().to_owned();

    let mut client = Client::connect(daemon.handle.addr()).await;
    client.send_text(hello(&token, 1)).await;
    let accepted = client.frame().await;
    let base = accepted["result"]["widgetBase"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let key = base.strip_prefix("/widgets/").unwrap_or_default();
    assert!(
        key.len() == 32 && key.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "a 128-bit key: {base}"
    );
    assert_eq!(
        accepted["result"],
        json!({
            "protocol": 1,
            "daemon": env!("CARGO_PKG_VERSION"),
            "omp": omfp_core::OMP_VERSION,
            "warnings": ["running anyway (--allow-omp-mismatch)"],
            "widgetBase": base,
            "fileBase": accepted["result"]["fileBase"].as_str().expect("a file base"),
            "connection": accepted["result"]["connection"].as_u64().expect("a connection number"),
        })
    );

    let mut future = Client::connect(daemon.handle.addr()).await;
    future.send_text(hello(&token, 2)).await;
    let refused = future.frame().await;
    assert_eq!(refused["error"]["code"], "version_mismatch");
    assert_eq!(refused["error"]["details"]["supported"], json!([1]));
    assert_eq!(future.close_code().await, Some(4426));
}

// spec: DMN-004, DMN-005
#[tokio::test(flavor = "multi_thread")]
async fn the_library_daemon_serves_until_shutdown_then_stops_omp_and_closes_clients() {
    let daemon = daemon(&[]).await;
    let addr = daemon.handle.addr();
    assert!(
        daemon
            .handle
            .url()
            .ends_with(&format!("{addr}/#token={}", daemon.handle.token().expose()))
    );
    let mut client = daemon.client("test").await;
    let thread = create_thread(&mut client).await;
    client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    let process = daemon.launcher.process(0);

    let TestDaemon {
        handle,
        launcher: _launcher,
        dir,
        ..
    } = daemon;
    handle.shutdown().await.unwrap();

    assert!(process.is_closed(), "omp was stopped");
    assert_eq!(client.close_code().await, Some(1001));
    assert!(!dir.path().join("daemon.json").exists());
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

// spec: DMN-181, UI-004
#[tokio::test(flavor = "multi_thread")]
async fn the_token_url_is_printed_only_to_a_terminal() {
    let daemon = daemon(&[]).await;
    let addr = daemon.handle.addr();
    let token = daemon.handle.token().expose().to_owned();

    assert_eq!(
        daemon.handle.listening_line(true),
        format!("omfp daemon listening on http://{addr}/#token={token}")
    );
    let logged = daemon.handle.listening_line(false);
    assert!(!logged.contains(&token), "{logged}");
    assert!(
        logged.starts_with(&format!("omfp daemon listening on http://{addr}/ (")),
        "{logged}"
    );
    assert!(logged.contains("omfp desktop"), "{logged}");
    assert!(
        logged.contains(&daemon.dir.path().join("token").display().to_string()),
        "{logged}"
    );
}

// spec: DMN-180, DMN-003
#[tokio::test(flavor = "multi_thread")]
async fn the_data_directory_and_the_daemons_files_are_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    let launcher = FakeLauncher::new();
    let git = Arc::new(FakeGit::new());
    let handle = start_with(config(&data, &[]), ports(&data, &launcher, &git))
        .await
        .unwrap();

    let mode = |name: &str| {
        std::fs::metadata(data.join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode(""), 0o700);
    for file in ["token", "daemon.json", "daemon.lock", "omfp.db"] {
        assert_eq!(mode(file), 0o600, "{file}");
    }
    handle.shutdown().await.unwrap();
}

// spec: DMN-007
#[tokio::test(flavor = "multi_thread")]
async fn one_daemon_per_data_directory_published_in_daemon_json() {
    let first = daemon(&[]).await;
    let data_dir = first.dir.path().to_owned();

    let info = discover(&data_dir).unwrap().expect("a running daemon");
    assert_eq!(info.addr, first.handle.addr());
    assert_eq!(info.pid, std::process::id());
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir.join("daemon.json")).unwrap())
            .unwrap();
    assert_eq!(written["addr"], first.handle.addr().to_string());

    let fresh = || ports(&data_dir, &FakeLauncher::new(), &Arc::new(FakeGit::new()));
    let second = start_with(config(&data_dir, &[]), fresh()).await;
    let error = second.unwrap_err().to_string();
    assert!(error.contains("already running"), "{error}");
    assert!(error.contains(&first.handle.addr().to_string()), "{error}");

    first.handle.shutdown().await.unwrap();
    assert_eq!(discover(&data_dir).unwrap(), None);
    // The directory is free again.
    let again = start_with(config(&data_dir, &[]), fresh()).await.unwrap();
    again.shutdown().await.unwrap();
}

// spec: DMN-007
#[test]
fn a_stale_daemon_json_is_not_discovered() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("daemon.json"),
        r#"{"addr":"127.0.0.1:9","pid":1}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("daemon.lock"), "").unwrap();

    assert_eq!(discover(dir.path()).unwrap(), None);
    assert_eq!(discover(&dir.path().join("missing")).unwrap(), None);
}

// spec: ARC-004, ARC-005, ARC-001, ARC-011
#[tokio::test(flavor = "multi_thread")]
async fn two_clients_drive_one_omp_process_and_see_the_same_thread() {
    let daemon = daemon(&[]).await;
    daemon.launcher.push_run(fixture("text_reply"));
    let mut web = daemon.client("omfp/web").await;
    let mut desktop = daemon.client("omfp/desktop").await;
    let thread = create_thread(&mut web).await;
    let mut web_view = Follower::snapshot(&mut web, &thread, None).await;
    let mut desktop_view = Follower::snapshot(&mut desktop, &thread, None).await;

    let (first, second) = tokio::join!(
        web.call(
            "thread.prompt",
            json!({ "threadId": thread, "text": "say hi" })
        ),
        desktop.call("thread.state", json!({ "threadId": thread }))
    );
    first.unwrap();
    second.unwrap();
    let done = |state: &omfp_core::ThreadState| state.settled && !state.prompts.is_empty();
    web_view.until(&mut web, done).await;
    desktop_view.until(&mut desktop, done).await;

    // Only the daemon launched omp, once (ARC-001, ARC-005).
    assert_eq!(daemon.launcher.launches().len(), 1);
    assert_eq!(web_view.state, desktop_view.state);
    assert_eq!(
        (&web_view.epoch, web_view.seq),
        (&desktop_view.epoch, desktop_view.seq)
    );
    let latest = Follower::snapshot(&mut web, &thread, None).await;
    assert_eq!(latest.state, web_view.state);
    // Thread events carry omp's frames as `omp_rpc::wire` types (ARC-011).
    let frame = json!({
        "type": "event", "topic": format!("thread/{thread}"), "epoch": "e", "seq": 1,
        "event": { "kind": "omp", "frame": { "type": "session_settled" } }
    });
    let ServerFrame::Event(event) = serde_json::from_value(frame).unwrap() else {
        panic!("not an event");
    };
    let TopicEvent::Thread(ThreadEvent::Omp { frame }) = event.event else {
        panic!("not an omp frame");
    };
    let wire: &omp_rpc::RpcNotification = &frame;
    assert!(matches!(wire, omp_rpc::RpcNotification::SessionSettled(_)));
}

// spec: PRT-020, PRT-021, PRT-022
#[tokio::test(flavor = "multi_thread")]
async fn a_client_reconnecting_with_since_gets_the_missed_events_in_order() {
    let daemon = daemon(&[]).await;
    daemon.launcher.push_run(fixture("bash_tool_call"));
    let mut first = daemon.client("first").await;
    let thread = create_thread(&mut first).await;
    let mut view = Follower::snapshot(&mut first, &thread, None).await;
    first
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": "run echo" }),
        )
        .await;
    // Disconnect mid-stream.
    view.until(&mut first, |state| !state.tools.is_empty())
        .await;
    drop(first);

    let mut observer = daemon.client("observer").await;
    let mut latest = Follower::snapshot(&mut observer, &thread, None).await;
    latest
        .until(&mut observer, |state| {
            state.settled && !state.prompts.is_empty()
        })
        .await;

    let mut second = daemon.client("second").await;
    let result = second
        .ok(
            "subscribe",
            json!({ "topic": format!("thread/{thread}"), "since": { "epoch": view.epoch, "seq": view.seq } }),
        )
        .await;
    assert_eq!(
        result,
        json!({ "mode": "replay", "epoch": view.epoch, "fromSeq": view.seq + 1 })
    );
    while view.seq < latest.seq {
        let frame = second.event(&view.topic).await;
        view.apply(&frame);
    }
    assert_eq!(view.state, latest.state);
}

// spec: PRT-020, PRT-021
#[tokio::test(flavor = "multi_thread")]
async fn an_evicted_position_gets_a_snapshot_equal_to_the_daemon_state() {
    let daemon = daemon(&["--event-buffer", "4"]).await;
    daemon.launcher.push_run(fixture("text_reply"));
    let mut client = daemon.client("test").await;
    let thread = create_thread(&mut client).await;
    let mut view = Follower::snapshot(&mut client, &thread, None).await;
    client
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": "say hi" }),
        )
        .await;
    view.until(&mut client, |state| {
        state.settled && !state.prompts.is_empty()
    })
    .await;

    let mut late = daemon.client("late").await;
    let snapshot = Follower::snapshot(&mut late, &thread, Some((&view.epoch, 1))).await;

    assert_eq!((&snapshot.epoch, snapshot.seq), (&view.epoch, view.seq));
    assert_eq!(snapshot.state, view.state);
}

// spec: PRT-024
#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_stops_delivery_of_the_topic() {
    let daemon = daemon(&[]).await;
    daemon.launcher.push_run(fixture("text_reply"));
    let mut client = daemon.client("test").await;
    let mut observer = daemon.client("observer").await;
    let thread = create_thread(&mut client).await;
    let topic = format!("thread/{thread}");
    client.ok("subscribe", json!({ "topic": topic })).await;
    let mut latest = Follower::snapshot(&mut observer, &thread, None).await;

    client.ok("unsubscribe", json!({ "topic": topic })).await;
    client
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": "say hi" }),
        )
        .await;
    latest
        .until(&mut observer, |state| {
            state.settled && !state.prompts.is_empty()
        })
        .await;
    client.ok("repos.suggest", json!({})).await;

    let leaked: Vec<Value> = client
        .take_buffered()
        .into_iter()
        .filter(|frame| frame["topic"] == topic)
        .collect();
    assert_eq!(leaked, Vec::<Value>::new());
}

// spec: PRT-222, PRT-223
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_subscription_kept_unwatched_switches_to_watched_on_the_same_stream() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("omfp/web").await;
    let thread = create_thread(&mut client).await;
    let topic = format!("thread/{thread}");
    let supervisor = daemon.handle.supervisor();

    let subscribed = client
        .ok("subscribe", json!({ "topic": topic, "watch": false }))
        .await;
    assert_eq!(subscribed["mode"], "snapshot");
    assert_eq!(supervisor.watchers(&thread), 0, "kept, not watched");

    let shown = client
        .ok("watch", json!({ "topic": topic, "watch": true }))
        .await;
    assert_eq!(shown, json!({}), "no new snapshot");
    assert_eq!(supervisor.watchers(&thread), 1, "watched again");
    client
        .ok("watch", json!({ "topic": topic, "watch": false }))
        .await;
    assert_eq!(supervisor.watchers(&thread), 0, "kept again");
    client.ok("subscribe", json!({ "topic": topic })).await;
    assert_eq!(
        supervisor.watchers(&thread),
        1,
        "a subscribe watches by default"
    );

    client.ok("unsubscribe", json!({ "topic": topic })).await;
    let gone = client
        .call("watch", json!({ "topic": topic, "watch": true }))
        .await
        .unwrap_err();
    assert_eq!(gone["code"], "not_found", "{gone}");

    let index = client
        .call("subscribe", json!({ "topic": "index", "watch": false }))
        .await
        .unwrap_err();
    assert_eq!(index["code"], "bad_request", "{index}");
    client.ok("subscribe", json!({ "topic": "index" })).await;
    client
        .ok("watch", json!({ "topic": "index", "watch": true }))
        .await;
    let unwatched = client
        .call("watch", json!({ "topic": "index", "watch": false }))
        .await
        .unwrap_err();
    assert_eq!(unwatched["code"], "bad_request", "{unwatched}");
}

// spec: DMN-071, PRT-011
#[tokio::test(flavor = "multi_thread")]
async fn omp_errors_reach_the_client_with_omps_code() {
    let daemon = daemon(&[]).await;
    daemon.launcher.reply(
        "get_tree",
        FakeReply::Fail {
            message: "session is busy".into(),
            code: Some("session_busy".into()),
        },
    );
    let mut client = daemon.client("test").await;
    let thread = create_thread(&mut client).await;

    let error = client
        .call("thread.tree", json!({ "threadId": thread }))
        .await
        .unwrap_err();

    assert_eq!(error["code"], "omp_error");
    assert_eq!(error["ompCode"], "session_busy");
    let missing = client
        .call("thread.tree", json!({ "threadId": "01NOPE" }))
        .await
        .unwrap_err();
    assert_eq!(missing["code"], "not_found");
}

// spec: PRT-033, ARC-009
#[tokio::test(flavor = "multi_thread")]
async fn created_entities_record_the_creating_client() {
    let daemon = daemon(&[]).await;
    let mut script = daemon.client("script/nightly").await;
    let repo = daemon.external_repo("existing");

    let workspace = script
        .ok("workspaces.create", json!({ "link": repo }))
        .await;
    let thread = script
        .ok(
            "threads.create",
            json!({ "title": "from a script", "workspace": { "id": workspace["id"] } }),
        )
        .await;

    let expected = json!({ "createdBy": { "kind": "user" }, "client": "script/nightly" });
    assert_eq!(workspace["provenance"], expected);
    assert_eq!(thread["provenance"], expected);
    let mut ui = daemon.client("omfp/web").await;
    let index = ui.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["state"]["threads"][0]["provenance"], expected);
    assert_eq!(index["state"]["workspaces"][0]["provenance"], expected);
}

/// Params with every field any method needs; methods ignore the others (PRT-012).
fn superset_params(thread: &str, workspace: &str) -> Value {
    json!({
        "threadId": thread, "workspaceId": workspace, "repo": "01NOREPO", "query": "hi",
        "path": "/nonexistent", "title": "t", "text": "hello", "message": "hello",
        "name": "n", "provider": "fake", "modelId": "scripted", "level": "low", "entryId": "e1",
        "subagentId": "s1", "op": "get", "question": "why?", "command": "true", "interactionId": "i1",
        "response": { "cancelled": true }, "topic": "index"
    })
}

// spec: ARC-007, PRT-033
#[tokio::test(flavor = "multi_thread")]
async fn every_method_is_served_to_a_script_client() {
    let daemon = daemon(&[]).await;
    let mut script = daemon.client("script/smoke").await;
    let thread = script
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "smoke" }),
        )
        .await;
    let params = superset_params(
        thread["id"].as_str().unwrap(),
        thread["workspaceId"].as_str().unwrap(),
    );

    let mut failures = Vec::new();
    for method in omfp_protocol::METHODS {
        if *method == "system.hello" {
            continue;
        }
        let response = script.request(method, params.clone()).await;
        let error = &response["error"];
        let unknown = error["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("unknown method"));
        if unknown || error["code"] == "internal" {
            failures.push(format!("{method}: {error}"));
        }
    }

    assert_eq!(failures, Vec::<String>::new());
}

impl TestDaemon {
    /// A fake repository outside every workspace.
    fn external_repo(&self, name: &str) -> std::path::PathBuf {
        let path = self.dir.path().join("src").join(name);
        std::fs::create_dir_all(&path).unwrap();
        self.git.add_repo(&path, "main");
        std::fs::canonicalize(path).unwrap()
    }

    /// Local commands (`/add-dir`, `/remove-dir`) the thread's first process got.
    fn local_commands(&self) -> Vec<String> {
        self.launcher
            .process(0)
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                omfp_daemon::agent::FakeCall::Prompt(prompt) if prompt.message.starts_with('/') => {
                    Some(prompt.message)
                }
                _ => None,
            })
            .collect()
    }
}

/// The next index event upserting workspace `id`.
async fn next_workspace(client: &mut Client, id: &Value) -> Value {
    loop {
        let event = client.event("index").await;
        if event["event"]["kind"] == "workspace_upserted"
            && event["event"]["workspace"]["id"] == *id
        {
            return event["event"]["workspace"].clone();
        }
    }
}

// spec: DMN-020, DMN-024, DMN-025, DMN-029, ARC-008
#[tokio::test(flavor = "multi_thread")]
async fn linking_a_repository_into_a_live_threads_workspace_adds_its_root_and_unlinking_removes_it()
{
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "Checkout v2" }),
        )
        .await;
    let id = thread["id"].clone();
    let workspace_id = thread["workspaceId"].clone();
    client.ok("thread.state", json!({ "threadId": id })).await;
    let target = daemon.external_repo("shop-frontend");

    let linked = client
        .ok(
            "workspaces.add_repo",
            json!({ "workspaceId": workspace_id, "source": { "kind": "link", "path": target } }),
        )
        .await;
    assert_eq!(linked["name"], "Checkout v2");
    assert!(
        linked["path"]
            .as_str()
            .unwrap()
            .ends_with("/workspaces/checkout-v2"),
        "{linked}"
    );
    assert_eq!(linked["threadIds"], json!([id]));
    let repo = &linked["repos"][0];
    assert_eq!(
        (&repo["name"], &repo["kind"], &repo["target"]),
        (&json!("shop-frontend"), &json!("link"), &json!(target))
    );
    let link = repo["path"].as_str().unwrap().to_owned();
    client
        .ok(
            "workspaces.remove_repo",
            json!({ "workspaceId": workspace_id, "name": "shop-frontend" }),
        )
        .await;

    assert_eq!(
        daemon.local_commands(),
        [format!("/add-dir {link}"), format!("/remove-dir {link}")]
    );
    assert!(target.is_dir(), "the link's target is never touched");
    let removal = client
        .call(
            "workspaces.remove",
            json!({ "workspaceId": workspace_id, "force": true }),
        )
        .await
        .unwrap_err();
    assert_eq!(
        removal["code"], "conflict",
        "a thread lives there (DMN-028)"
    );
}

// spec: DMN-023, DMN-029, DMN-022
#[tokio::test(flavor = "multi_thread")]
async fn a_repository_the_agent_links_appears_after_its_tool_run() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "agent work" }),
        )
        .await;
    let (id, workspace_id) = (thread["id"].clone(), thread["workspaceId"].clone());
    client.ok("thread.state", json!({ "threadId": id })).await;
    client.ok("subscribe", json!({ "topic": "index" })).await;
    let folder = client
        .ok("workspaces.rescan", json!({ "workspaceId": workspace_id }))
        .await["path"]
        .as_str()
        .unwrap()
        .to_owned();
    let link = Path::new(&folder).join("lib");
    std::os::unix::fs::symlink(daemon.external_repo("lib"), &link).unwrap();
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/threads")
        .join(omfp_core::OMP_VERSION)
        .join("bash_tool_call.jsonl");
    let tool_end = std::fs::read_to_string(fixture_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<ThreadEvent>(line).unwrap())
        .find_map(|event| match event {
            ThreadEvent::Omp {
                frame: frame @ RpcNotification::RpcAgentEvent(RpcAgentEvent::ToolExecutionEnd(_)),
            } => Some(frame),
            _ => None,
        })
        .unwrap();

    daemon.launcher.process(0).emit(vec![tool_end]);

    let workspace = next_workspace(&mut client, &workspace_id).await;
    assert_eq!(workspace["repos"][0]["name"], "lib");
    assert_eq!(workspace["repos"][0]["path"], json!(link));
    while daemon.local_commands().is_empty() {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        daemon.local_commands(),
        [format!("/add-dir {}", link.display())]
    );
}

// spec: DMN-020, DMN-023, DMN-028
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_open_in_a_window_is_polled_and_removing_it_unlinks_its_repositories() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let app = daemon.external_repo("app");
    let workspace = client.ok("workspaces.create", json!({ "link": app })).await;
    assert_eq!(workspace["name"], "app");
    assert_eq!(
        (
            &workspace["repos"][0]["name"],
            &workspace["repos"][0]["kind"]
        ),
        (&json!("app"), &json!("link"))
    );
    let id = workspace["id"].clone();
    let folder = std::path::PathBuf::from(workspace["path"].as_str().unwrap());
    client.ok("subscribe", json!({ "topic": "index" })).await;
    let topic = format!("workspace/{}", id.as_str().unwrap());
    client.ok("subscribe", json!({ "topic": topic })).await;

    // No request follows: only the poll of the watched workspace sees the new link.
    std::os::unix::fs::symlink(daemon.external_repo("lib"), folder.join("lib")).unwrap();
    let updated = next_workspace(&mut client, &id).await;
    let names: Vec<&str> = updated["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["app", "lib"]);

    client
        .ok("workspaces.remove", json!({ "workspaceId": id }))
        .await;
    assert!(!folder.exists());
    assert!(app.is_dir(), "a link's target is never touched");
}

// spec: DMN-026, UI-080
#[tokio::test(flavor = "multi_thread")]
async fn workspaces_status_publishes_the_fresh_status_of_every_repository_on_the_workspace_topic() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let app = daemon.external_repo("app");
    let workspace = client.ok("workspaces.create", json!({ "link": app })).await;
    let id = workspace["id"].as_str().unwrap().to_owned();
    let topic = format!("workspace/{id}");
    let subscribed = client.ok("subscribe", json!({ "topic": topic })).await;
    assert_eq!(
        subscribed["state"][0]["status"]["dirty"], false,
        "{subscribed}"
    );

    // An edit the cheap poll cannot see: `workspaces.status` (what `omfp:changes` calls after a tool run) publishes it.
    daemon.git.set_dirty(&app, true);
    let status = client
        .ok(
            "workspaces.status",
            json!({ "workspaceId": id, "repo": "app" }),
        )
        .await;
    assert_eq!(status["dirty"], true);
    let event = client.event(&topic).await;
    assert_eq!(event["event"]["kind"], "status", "{event}");
    assert_eq!(event["event"]["repos"][0]["name"], "app");
    assert_eq!(event["event"]["repos"][0]["status"]["dirty"], true);
}

// spec: DMN-089, DMN-090
#[tokio::test(flavor = "multi_thread")]
async fn a_written_file_republishes_the_workspace_status_and_a_stale_write_is_a_conflict() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let app = daemon.external_repo("app");
    std::fs::write(app.join("notes.md"), "one\n").unwrap();
    daemon.git.commit_file(&app, "notes.md", b"zero\n");
    let workspace = client.ok("workspaces.create", json!({ "link": app })).await;
    let id = workspace["id"].as_str().unwrap().to_owned();
    let topic = format!("workspace/{id}");
    client.ok("subscribe", json!({ "topic": topic })).await;

    let read = client
        .ok(
            "workspaces.read_file",
            json!({ "workspaceId": id, "path": "notes.md" }),
        )
        .await;
    assert_eq!(read["path"], "app/notes.md");
    assert_eq!(read["kind"], "text");
    assert_eq!(read["content"], "one\n");
    assert_eq!(read["repo"], "app");
    assert_eq!(read["head"], "zero\n");
    let hash = read["hash"].as_str().unwrap().to_owned();

    // What git would now report: the write's rescan publishes it, as after an agent's edit.
    daemon.git.set_dirty(&app, true);
    let written = client
        .ok(
            "workspaces.write_file",
            json!({ "workspaceId": id, "path": "app/notes.md", "content": "two\n", "expectedHash": hash }),
        )
        .await;
    assert_eq!(written["path"], "app/notes.md");
    assert_eq!(written["size"], 4);
    assert_eq!(
        std::fs::read_to_string(app.join("notes.md")).unwrap(),
        "two\n"
    );
    let event = client.event(&topic).await;
    assert_eq!(
        event["event"]["repos"][0]["status"]["dirty"], true,
        "{event}"
    );

    let stale = client
        .call(
            "workspaces.write_file",
            json!({ "workspaceId": id, "path": "app/notes.md", "content": "three\n", "expectedHash": hash }),
        )
        .await
        .unwrap_err();
    assert_eq!(stale["code"], "conflict", "{stale}");
    let exists = client
        .call(
            "workspaces.write_file",
            json!({ "workspaceId": id, "path": "app/notes.md", "content": "three\n" }),
        )
        .await
        .unwrap_err();
    assert_eq!(exists["code"], "conflict", "{exists}");
    assert_eq!(
        std::fs::read_to_string(app.join("notes.md")).unwrap(),
        "two\n"
    );

    let unchanged = client
        .ok(
            "workspaces.read_file",
            json!({ "workspaceId": id, "path": "app/notes.md", "knownHash": written["hash"] }),
        )
        .await;
    assert_eq!(unchanged["unchanged"], true);
    assert_eq!(unchanged.get("content"), None);
    for (path, code) in [
        ("../outside.md", "bad_request"),
        ("app", "bad_request"),
        ("missing.md", "not_found"),
    ] {
        let error = client
            .call(
                "workspaces.read_file",
                json!({ "workspaceId": id, "path": path }),
            )
            .await
            .unwrap_err();
        assert_eq!(error["code"], code, "{path}: {error}");
    }
    let no_folder = client
        .call(
            "workspaces.write_file",
            json!({ "workspaceId": id, "path": "app/docs/new.md", "content": "x" }),
        )
        .await
        .unwrap_err();
    assert_eq!(no_folder["code"], "not_found", "{no_folder}");
}

// spec: DMN-310, DMN-313, PRT-195, PRT-196
#[tokio::test(flavor = "multi_thread")]
async fn an_absolute_path_outside_the_workspace_is_read_but_never_written() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let workspace = client.ok("workspaces.create", json!({})).await;
    let id = workspace["id"].as_str().unwrap().to_owned();
    let elsewhere = tempfile::tempdir().unwrap();
    let readme = elsewhere.path().join("README.md");
    std::fs::write(&readme, "# Elsewhere\n").unwrap();
    let path = readme.to_str().unwrap();

    let read = client
        .ok(
            "workspaces.read_file",
            json!({ "workspaceId": id, "path": path }),
        )
        .await;
    assert_eq!(read["path"], path);
    assert_eq!(read["content"], "# Elsewhere\n");
    assert_eq!(read["outside"], true);
    assert_eq!(read.get("repo"), None);

    let refused = client
        .call(
            "workspaces.write_file",
            json!({ "workspaceId": id, "path": path, "content": "x", "expectedHash": read["hash"] }),
        )
        .await
        .unwrap_err();
    assert_eq!(refused["code"], "bad_request", "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("it can be read, not written"),
        "{refused}"
    );
    assert_eq!(std::fs::read_to_string(&readme).unwrap(), "# Elsewhere\n");
    let found = client
        .ok(
            "workspaces.find_file",
            json!({ "workspaceId": id, "query": path }),
        )
        .await;
    assert_eq!(found["candidates"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_index_follows_thread_process_changes() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let thread = create_thread(&mut client).await;
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["mode"], "snapshot");
    assert_eq!(index["state"]["threads"][0]["process"]["state"], "stopped");

    client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;

    loop {
        let event = client.event("index").await;
        if event["event"]["kind"] == "thread_upserted"
            && event["event"]["thread"]["process"]["state"] == "running"
        {
            break;
        }
    }
}

// spec: DMN-129
#[tokio::test(flavor = "multi_thread")]
async fn the_thread_list_watch_follows_the_index() {
    let daemon = daemon(&[]).await;
    let mut threads = daemon.handle.threads();
    assert_eq!(threads.borrow_and_update().len(), 0);
    let mut client = daemon.client("test").await;
    let thread = create_thread(&mut client).await;
    until(&mut threads, |list| list.iter().any(|t| t.id == thread)).await;

    client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    until(&mut threads, |list| {
        list.iter()
            .any(|t| t.id == thread && t.process == omfp_core::ProcessState::Running)
    })
    .await;

    client
        .ok("threads.delete", json!({ "threadId": thread }))
        .await;
    until(&mut threads, |list| list.iter().all(|t| t.id != thread)).await;
}

/// Waits (at most 10 s) until the thread list satisfies `done`.
async fn until(
    threads: &mut tokio::sync::watch::Receiver<Vec<omfp_protocol::ThreadSummary>>,
    done: impl FnMut(&Vec<omfp_protocol::ThreadSummary>) -> bool,
) {
    tokio::time::timeout(std::time::Duration::from_secs(10), threads.wait_for(done))
        .await
        .unwrap()
        .unwrap();
}

/// The next `thread_upserted` of `thread` on the index.
async fn next_upsert(client: &mut Client, thread: &str) -> Value {
    loop {
        let event = client.event("index").await;
        if event["event"]["kind"] == "thread_upserted" && event["event"]["thread"]["id"] == thread {
            return event["event"]["thread"].clone();
        }
    }
}

// spec: UI-026
#[tokio::test(flavor = "multi_thread")]
async fn archive_unarchive_and_delete_update_the_index() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let thread = create_thread(&mut client).await;
    client.ok("subscribe", json!({ "topic": "index" })).await;

    client
        .ok("threads.archive", json!({ "threadId": thread }))
        .await;
    assert!(next_upsert(&mut client, &thread).await["archivedAt"].is_i64());

    let result = client
        .ok("threads.unarchive", json!({ "threadId": thread }))
        .await;
    assert_eq!(result, json!({}));
    assert_eq!(
        next_upsert(&mut client, &thread).await.get("archivedAt"),
        None
    );

    client
        .ok("threads.delete", json!({ "threadId": thread }))
        .await;
    loop {
        let event = client.event("index").await;
        if event["event"]["kind"] == "thread_removed" {
            assert_eq!(event["event"]["threadId"], thread);
            break;
        }
    }
    let error = client
        .call("threads.unarchive", json!({ "threadId": thread }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");
}

// spec: UI-676
#[tokio::test(flavor = "multi_thread")]
async fn the_web_ui_is_served_from_the_static_directory() {
    let site = tempfile::tempdir().unwrap();
    std::fs::write(site.path().join("index.html"), "<main>omfp</main>").unwrap();
    let daemon = daemon(&["--static-dir", site.path().to_str().unwrap()]).await;

    let mut stream = tokio::net::TcpStream::connect(daemon.handle.addr())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::write_all(
        &mut stream,
        b"GET /threads/x HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await
    .unwrap();
    let mut response = String::new();
    tokio::io::AsyncReadExt::read_to_string(&mut stream, &mut response)
        .await
        .unwrap();

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("text/html"));
    assert!(response.ends_with("<main>omfp</main>"));
    // No policy of the daemon's keeps the UI's web frames (omfp:web) from loading other sites.
    let head = response.split_once("\r\n\r\n").unwrap().0.to_lowercase();
    assert!(!head.contains("content-security-policy"), "{head}");
}

/// The raw HTTP/1.1 response to `GET path`, without credentials.
async fn http_get(daemon: &TestDaemon, path: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(daemon.handle.addr())
        .await
        .unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    tokio::io::AsyncWriteExt::write_all(&mut stream, request.as_bytes())
        .await
        .unwrap();
    let mut response = String::new();
    tokio::io::AsyncReadExt::read_to_string(&mut stream, &mut response)
        .await
        .unwrap();
    response
}

/// The opening tag of `<html>` in the body of `GET path`, after checking it is HTML that is never cached.
async fn index_html_tag(daemon: &TestDaemon, path: &str) -> String {
    let response = http_get(daemon, path).await;
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let head = head.to_lowercase();
    assert!(head.starts_with("http/1.1 200"), "{response}");
    assert!(head.contains("content-type: text/html"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
    let start = body.find("<html").unwrap();
    let end = start + body[start..].find('>').unwrap();
    body[start..=end].to_owned()
}

// spec: DMN-008, UI-091, UI-510, UI-794
#[tokio::test(flavor = "multi_thread")]
async fn index_html_carries_the_configured_theme_skin_density_motion_and_font_size() {
    let site = tempfile::tempdir().unwrap();
    std::fs::write(
        site.path().join("index.html"),
        "<!doctype html>\n<html lang=\"en\">\n<head></head>\n<body></body>\n</html>\n",
    )
    .unwrap();
    let daemon = daemon(&["--static-dir", site.path().to_str().unwrap()]).await;
    let mut client = daemon.client("omfp/web").await;
    assert_eq!(
        index_html_tag(&daemon, "/").await,
        "<html lang=\"en\">",
        "the defaults need no attribute"
    );

    client
        .ok(
            "settings.set",
            json!({ "ui": { "theme": "graphite", "skin": "boring", "density": "normal", "motion": "reduced", "fontSize": 20 } }),
        )
        .await;
    let tag = "<html data-theme=\"graphite\" data-skin=\"boring\" data-density=\"normal\" \
               data-motion=\"reduced\" style=\"font-size:20px\" lang=\"en\">";
    assert_eq!(index_html_tag(&daemon, "/").await, tag);
    assert_eq!(index_html_tag(&daemon, "/index.html").await, tag);
    assert_eq!(index_html_tag(&daemon, "/threads/x").await, tag);
    // The gallery picks its own appearance from its query (UI-173).
    assert_eq!(index_html_tag(&daemon, "/kit").await, "<html lang=\"en\">");
    assert_eq!(index_html_tag(&daemon, "/kit/").await, "<html lang=\"en\">");

    // `matugen` is Synth under the user theme, `accent` Synth without the host's accent: the stylesheet's default
    // until the window knows better.
    for theme in ["matugen", "accent"] {
        client
            .ok("settings.set", json!({ "ui": { "theme": theme } }))
            .await;
        assert_eq!(
            index_html_tag(&daemon, "/").await,
            "<html data-skin=\"boring\" data-density=\"normal\" data-motion=\"reduced\" \
             style=\"font-size:20px\" lang=\"en\">",
            "{theme}"
        );
    }
}

/// The body of `GET /theme.css`, after checking its headers.
async fn theme_css(daemon: &TestDaemon) -> String {
    let response = http_get(daemon, "/theme.css").await;
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let head = head.to_lowercase();
    assert!(head.starts_with("http/1.1 200"), "{response}");
    assert!(head.contains("content-type: text/css"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
    body.to_owned()
}

/// The theme of the next `theme_changed` on the index.
async fn next_theme(client: &mut Client) -> Value {
    loop {
        let event = client.event("index").await;
        if event["event"]["kind"] == "theme_changed" {
            return event["event"]["theme"].clone();
        }
    }
}

// spec: DMN-075
#[tokio::test(flavor = "multi_thread")]
async fn a_theme_set_by_one_client_reaches_every_client_and_theme_css() {
    let daemon = daemon(&[]).await;
    let mut web = daemon.client("omfp/web").await;
    let mut desktop = daemon.client("omfp/desktop").await;
    let index = desktop.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["state"]["theme"], json!({ "css": null }));
    web.ok("settings.set", json!({ "ui": { "theme": "matugen" } }))
        .await;
    assert_eq!(theme_css(&daemon).await, "");
    let css = ":root { --omfp-primary: #6750a4; }";

    assert_eq!(web.ok("theme.set", json!({ "css": css })).await, json!({}));

    assert_eq!(next_theme(&mut desktop).await, json!({ "css": css }));
    assert_eq!(web.ok("theme.get", json!({})).await, json!({ "css": css }));
    assert_eq!(theme_css(&daemon).await, css);
    let error = web
        .call(
            "theme.set",
            json!({ "css": "a".repeat(omfp_daemon::theme::MAX_THEME_BYTES + 1) }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "bad_request");
    assert_eq!(
        theme_css(&daemon).await,
        css,
        "a refused theme changes nothing"
    );

    web.ok("theme.set", json!({ "css": null })).await;
    assert_eq!(next_theme(&mut desktop).await, json!({ "css": null }));
    assert_eq!(theme_css(&daemon).await, "");
}

// spec: DMN-075, DMN-240
#[tokio::test(flavor = "multi_thread")]
async fn theme_css_serves_the_user_theme_only_while_the_theme_is_matugen() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("omfp/web").await;
    let css = ":root { --omfp-primary: #6750a4; }";
    client.ok("theme.set", json!({ "css": css })).await;
    assert_eq!(
        theme_css(&daemon).await,
        "",
        "the default theme (Synth) does not show the user theme"
    );

    client
        .ok("settings.set", json!({ "ui": { "theme": "matugen" } }))
        .await;
    assert_eq!(theme_css(&daemon).await, css);

    client
        .ok("settings.set", json!({ "ui": { "theme": "nord" } }))
        .await;
    assert_eq!(theme_css(&daemon).await, "");
    assert_eq!(
        client.ok("theme.get", json!({})).await,
        json!({ "css": css }),
        "the stored theme is kept"
    );
}

/// Replaces `file`'s content at once (a new file renamed over it), as editors and theme generators save: a plain
/// `std::fs::write` truncates the file first, so the daemon's poll could read it half-written and push that.
fn replace_file(file: &std::path::Path, content: &str) {
    let next = file.with_extension("next");
    std::fs::write(&next, content).unwrap();
    std::fs::rename(&next, file).unwrap();
}

// spec: DMN-075
#[tokio::test(flavor = "multi_thread")]
async fn the_theme_file_is_read_at_startup_and_its_changes_are_pushed() {
    let config = tempfile::tempdir().unwrap();
    let file = config.path().join("theme.css");
    std::fs::write(&file, "body { color: red; }").unwrap();
    let daemon = daemon(&["--theme-file", file.to_str().unwrap()]).await;
    let mut client = daemon.client("test").await;
    client
        .ok("settings.set", json!({ "ui": { "theme": "matugen" } }))
        .await;
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(
        index["state"]["theme"],
        json!({ "css": "body { color: red; }" })
    );

    replace_file(&file, "body { color: blue; }");
    assert_eq!(
        next_theme(&mut client).await,
        json!({ "css": "body { color: blue; }" })
    );
    // The last change wins, from the method or from the file.
    client.ok("theme.set", json!({ "css": "body {}" })).await;
    assert_eq!(next_theme(&mut client).await, json!({ "css": "body {}" }));
    replace_file(&file, "body { color: green; }");
    assert_eq!(
        next_theme(&mut client).await,
        json!({ "css": "body { color: green; }" })
    );
    assert_eq!(theme_css(&daemon).await, "body { color: green; }");
}

// spec: DMN-073
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_created_without_a_title_is_named_from_its_first_message() {
    let daemon = daemon(&[]).await;
    daemon.launcher.push_run(fixture("text_reply"));
    daemon.launcher.push_title(Ok("Greeting the room"));
    let titles = daemon.launcher.gate_titles();
    let mut client = daemon.client("omfp/web").await;
    client.ok("subscribe", json!({ "topic": "index" })).await;

    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "prompt": "say hi to everyone in the room please" }),
        )
        .await;

    assert_eq!(
        (&thread["title"], &thread["titleSource"]),
        (&json!("say hi to everyone in the…"), &json!("placeholder"))
    );
    let id = thread["id"].as_str().unwrap().to_owned();
    let workspace = client
        .ok(
            "workspaces.rescan",
            json!({ "workspaceId": thread["workspaceId"] }),
        )
        .await;
    assert_eq!(workspace["name"], "say hi to everyone in the");
    titles.open();
    loop {
        let summary = next_upsert(&mut client, &id).await;
        if summary["titleSource"] == "generated" {
            assert_eq!(summary["title"], "Greeting the room");
            break;
        }
    }
    let renamed = client
        .ok("threads.update", json!({ "threadId": id, "title": "Mine" }))
        .await;
    assert_eq!(
        (&renamed["title"], &renamed["titleSource"]),
        (&json!("Mine"), &json!("set"))
    );
    let created = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "Given", "prompt": "hello" }),
        )
        .await;
    assert_eq!(created["titleSource"], "set");
    assert_eq!(daemon.launcher.title_requests().len(), 1);
}

// spec: DMN-074
#[tokio::test(flavor = "multi_thread")]
async fn new_threads_run_in_yolo_mode_until_another_mode_is_set() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let thread = create_thread(&mut client).await;

    client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    client
        .ok(
            "threads.update",
            json!({ "threadId": thread, "approvalMode": "always-ask" }),
        )
        .await;
    client
        .ok("thread.restart", json!({ "threadId": thread }))
        .await;

    let modes: Vec<Option<String>> = daemon
        .launcher
        .launches()
        .into_iter()
        .map(|launch| launch.spec.approval_mode)
        .collect();
    assert_eq!(
        modes,
        [Some("yolo".to_owned()), Some("always-ask".to_owned())]
    );
}

// spec: DMN-210
#[tokio::test(flavor = "multi_thread")]
async fn approval_modes_are_omps_values_and_others_are_refused_before_anything_changes() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let modes = ["always-ask", "always-ask-cached", "write", "yolo"];
    for mode in modes {
        let thread = client
            .ok(
                "threads.create",
                json!({ "workspace": { "new": {} }, "approvalMode": mode, "prompt": "hello" }),
            )
            .await;
        assert!(thread["id"].is_string());
    }
    let launched: Vec<Option<String>> = daemon
        .launcher
        .launches()
        .into_iter()
        .map(|launch| launch.spec.approval_mode)
        .collect();
    assert_eq!(launched, modes.map(|mode| Some(mode.to_owned())));

    let thread = create_thread(&mut client).await;
    for (method, params) in [
        (
            "threads.create",
            json!({ "workspace": { "new": {} }, "approvalMode": "ask", "prompt": "hello" }),
        ),
        (
            "threads.update",
            json!({ "threadId": thread, "approvalMode": "ask" }),
        ),
    ] {
        let error = client.call(method, params).await.unwrap_err();
        assert_eq!(error["code"], "bad_request", "{error}");
        let message = error["message"].as_str().unwrap();
        assert!(modes.iter().all(|mode| message.contains(mode)), "{message}");
    }
    assert_eq!(
        listed(&mut client, "threads.list", "threads").await.len(),
        5
    );
    assert_eq!(daemon.launcher.launches().len(), 4);
}

/// The ids of `list`'s `key` array (`threads.list`, `workspaces.list`).
async fn listed(client: &mut Client, method: &str, key: &str) -> Vec<String> {
    client.ok(method, json!({})).await[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap().to_owned())
        .collect()
}

const OMP_USAGE_ERROR: &str = "Error: Invalid --approval-mode value: \"ask\". Expected one of: always-ask, \
     always-ask-cached, write, yolo.";

// spec: DMN-211, DMN-212
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_whose_first_omp_start_fails_is_not_created() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;

    // In a new workspace: the thread, its directory and the workspace made for it are gone.
    daemon
        .launcher
        .fail_next_launch(&format!("{OMP_USAGE_ERROR}\n"));
    let error = client
        .call(
            "threads.create",
            json!({ "workspace": { "new": {} }, "prompt": "hello" }),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error,
        json!({
            "code": "thread_failed",
            "message": "omp failed to start: omp did not become ready: server closed",
            "details": { "stderrTail": OMP_USAGE_ERROR },
        })
    );
    assert_eq!(
        listed(&mut client, "threads.list", "threads").await,
        Vec::<String>::new()
    );
    assert_eq!(
        listed(&mut client, "workspaces.list", "workspaces").await,
        Vec::<String>::new()
    );
    let thread_dirs =
        std::fs::read_dir(daemon.dir.path().join("threads")).map_or(0, Iterator::count);
    assert_eq!(thread_dirs, 0);

    // In an existing workspace: the workspace stays.
    let workspace = client
        .ok("workspaces.create", json!({ "name": "kept" }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    daemon.launcher.fail_next_launch("fatal: out of memory\n");
    let error = client
        .call(
            "threads.create",
            json!({ "workspace": { "id": workspace }, "prompt": "hello" }),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error["details"],
        json!({ "stderrTail": "fatal: out of memory" })
    );
    assert_eq!(
        listed(&mut client, "threads.list", "threads").await,
        Vec::<String>::new()
    );
    assert_eq!(
        listed(&mut client, "workspaces.list", "workspaces").await,
        [workspace.as_str()]
    );

    // Calling again creates it.
    daemon.launcher.push_run(fixture("text_reply"));
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "id": workspace }, "prompt": "hello" }),
        )
        .await;
    assert_eq!(
        listed(&mut client, "threads.list", "threads").await,
        [thread["id"].as_str().unwrap().to_owned()]
    );
}

/// The text and images of the first `prompt` launch `index`'s omp received.
fn first_prompt(daemon: &TestDaemon, index: usize) -> (String, Value) {
    daemon
        .launcher
        .process(index)
        .calls()
        .into_iter()
        .find_map(|call| match call {
            omfp_daemon::agent::FakeCall::Prompt(prompt) => {
                Some((prompt.message, serde_json::to_value(prompt.images).unwrap()))
            }
            _ => None,
        })
        .expect("a prompt")
}

// spec: DMN-211
#[tokio::test(flavor = "multi_thread")]
async fn the_first_messages_images_reach_the_agent_with_or_without_text() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let png = json!({ "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" });

    client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "prompt": "what is this?", "images": [png] }),
        )
        .await;
    assert_eq!(
        first_prompt(&daemon, 0),
        ("what is this?".to_owned(), json!([png]))
    );

    // Images alone are a first message too.
    client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "images": [png] }),
        )
        .await;
    assert_eq!(first_prompt(&daemon, 1), (String::new(), json!([png])));

    // A first message of images only that cannot start omp leaves nothing behind either.
    daemon.launcher.fail_next_launch("fatal: out of memory\n");
    let error = client
        .call(
            "threads.create",
            json!({ "workspace": { "new": {} }, "images": [png] }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "thread_failed", "{error}");
    assert_eq!(
        listed(&mut client, "threads.list", "threads").await.len(),
        2
    );
    assert_eq!(
        listed(&mut client, "workspaces.list", "workspaces")
            .await
            .len(),
        2
    );
}

// spec: DMN-212
#[tokio::test(flavor = "multi_thread")]
async fn a_later_start_failure_keeps_the_thread_failed_with_omps_stderr_until_a_retry() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let thread = create_thread(&mut client).await;

    daemon
        .launcher
        .fail_next_launch("booting\nfatal: out of memory\n");
    let error = client
        .call(
            "thread.prompt",
            json!({ "threadId": thread, "text": "hello" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "thread_failed");
    assert_eq!(
        error["details"],
        json!({ "stderrTail": "booting\nfatal: out of memory" })
    );
    // `thread.state` would start omp: the topic's snapshot does not.
    let view = Follower::snapshot(&mut client, &thread, None).await;
    let omfp_core::ProcessState::Failed { stderr_tail, .. } = &view.state.process else {
        panic!("{:?}", view.state.process)
    };
    assert_eq!(stderr_tail, "booting\nfatal: out of memory");

    daemon.launcher.push_run(fixture("text_reply"));
    client
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": "hello" }),
        )
        .await;
    assert_eq!(daemon.launcher.launches().len(), 1);
}

// spec: PRT-125, PRT-126, DMN-213, DMN-214
#[tokio::test(flavor = "multi_thread")]
async fn thread_session_answers_the_sessions_small_facts_without_starting_omp() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let created = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "facts", "model": "fake/scripted", "thinkingLevel": "high", "approvalMode": "write" }),
        )
        .await;
    let thread = created["id"].as_str().unwrap().to_owned();
    let workspaces = client.ok("workspaces.list", json!({})).await;
    let cwd = workspaces["workspaces"][0]["path"].clone();
    let session_dir = std::path::absolute(
        daemon
            .dir
            .path()
            .join("threads")
            .join(&thread)
            .join("sessions"),
    )
    .unwrap();
    let facts = |extra: Value| {
        let mut facts = json!({
            "threadId": thread, "workspaceId": created["workspaceId"], "cwd": cwd, "roots": [],
            "sessionDir": session_dir, "model": "fake/scripted", "thinkingLevel": "high", "approvalMode": "write",
            "needsAttention": false,
        });
        facts
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        facts
    };

    // Before the first start: no session yet, and asking does not start omp.
    let session = client
        .ok("thread.session", json!({ "threadId": thread }))
        .await;
    assert_eq!(
        session,
        facts(json!({ "process": { "state": "stopped" }, "settled": true }))
    );
    assert_eq!(daemon.launcher.launches().len(), 0);

    // While omp runs: its usage (omp `get_session_stats`) and the transcript the daemon holds.
    daemon.launcher.reply(
        "get_session_stats",
        FakeReply::Ok(json!({
            "sessionId": "s", "userMessages": 1, "assistantMessages": 1, "toolCalls": 0, "toolResults": 0,
            "totalMessages": 2, "premiumRequests": 0, "cost": 0.25,
            "tokens": { "input": 1200, "output": 300, "cacheRead": 800, "cacheWrite": 100, "total": 2400, "reasoning": 0 },
            "contextUsage": { "tokens": 2000, "contextWindow": 200_000, "percent": 1.0 }
        })),
    );
    client
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": "hello" }),
        )
        .await;
    let session = client
        .ok("thread.session", json!({ "threadId": thread }))
        .await;
    let session_id = session["sessionId"]
        .as_str()
        .expect("the session id")
        .to_owned();
    assert_eq!(
        session,
        facts(json!({
            "sessionId": session_id, "process": { "state": "running" }, "settled": true, "messageCount": 0,
            "usage": {
                "cost": 0.25,
                "tokens": { "input": 1200, "output": 300, "cacheRead": 800, "cacheWrite": 100, "total": 2400 },
                "context": { "tokens": 2000, "contextWindow": 200_000, "percent": 1.0 }
            }
        }))
    );

    // Stopped: no usage; the session file is found in the session directory by the session id.
    let file = session_dir.join(format!("2026-11-13T10-00-00-000Z_{session_id}.jsonl"));
    std::fs::write(&file, "").unwrap();
    client
        .ok("thread.stop", json!({ "threadId": thread }))
        .await;
    let session = client
        .ok("thread.session", json!({ "threadId": thread }))
        .await;
    assert_eq!(
        session,
        facts(json!({
            "sessionId": session_id, "sessionFile": file, "process": { "state": "stopped" }, "settled": true,
            "messageCount": 0
        }))
    );
}

// spec: PRT-211, DMN-337, DMN-338, DMN-339
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_whose_saved_model_is_unusable_reads_its_history_and_takes_another_model() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let thread = create_thread(&mut client).await;
    let session_dir = daemon
        .dir
        .path()
        .join("threads")
        .join(&thread)
        .join("sessions");
    std::fs::create_dir_all(&session_dir).unwrap();
    let lines = [
        json!({ "type": "session", "version": 3, "id": "S-old", "timestamp": "2026-11-11T23:25:36.429Z" }),
        json!({ "type": "model_change", "id": "m1", "parentId": null, "timestamp": "2026-11-11T23:25:36.489Z",
                "model": "anthropic/claude-opus-5-5" }),
        json!({ "type": "message", "id": "u1", "parentId": "m1", "timestamp": "2026-11-11T23:25:36.751Z",
                "message": { "role": "user", "content": "old question", "timestamp": 1 } }),
    ];
    let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
    std::fs::write(
        session_dir.join("2026-11-11T23-25-36-429Z_S-old.jsonl"),
        text.join("\n"),
    )
    .unwrap();

    daemon.launcher.fail_next_launch(
        "SessionModelRestoreError: Could not restore model anthropic/claude-opus-5-5\n",
    );
    let error = client
        .call("thread.state", json!({ "threadId": thread }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "thread_failed");
    let session = client
        .ok("thread.session", json!({ "threadId": thread }))
        .await;
    assert_eq!(session["sessionModel"], "anthropic/claude-opus-5-5");
    assert_eq!(
        session["messageCount"], 1,
        "the history of the session file"
    );
    assert_eq!(session["model"], Value::Null, "nothing chosen for the user");

    client
        .ok(
            "thread.set_model",
            json!({ "threadId": thread, "provider": "fake", "modelId": "scripted" }),
        )
        .await;
    let session = client
        .ok("thread.session", json!({ "threadId": thread }))
        .await;
    assert_eq!(session["model"], "fake/scripted");
    assert_eq!(session["sessionModel"], Value::Null);
    assert_eq!(session["process"]["state"], "running");
    let launches = daemon.launcher.launches();
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].spec.model.as_deref(), Some("fake/scripted"));
}

// spec: DMN-070, DMN-358, DMN-359, DMN-214
#[tokio::test(flavor = "multi_thread")]
async fn showing_a_stopped_thread_starts_no_omp_and_a_prompt_does() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("omfp/web").await;
    let thread = create_thread(&mut client).await;
    let session_dir = daemon
        .dir
        .path()
        .join("threads")
        .join(&thread)
        .join("sessions");
    let lines = [
        json!({ "type": "session", "version": 3, "id": "S-old", "timestamp": "2026-11-11T23:25:36.429Z" }),
        json!({ "type": "message", "id": "u1", "parentId": null, "timestamp": "2026-11-11T23:25:36.751Z",
                "message": { "role": "user", "content": "old question", "timestamp": 1 } }),
    ];
    let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
    std::fs::write(
        session_dir.join("2026-11-11T23-25-36-429Z_S-old.jsonl"),
        text.join("\n"),
    )
    .unwrap();
    let topic = format!("thread/{thread}");

    // What a window does to show a thread, keep it and offer its `/` commands.
    let shown = client.ok("subscribe", json!({ "topic": topic })).await;
    let mut other = daemon.client("omfp/web").await;
    other
        .ok("subscribe", json!({ "topic": topic, "watch": false }))
        .await;
    let commands = client
        .ok("thread.commands", json!({ "threadId": thread }))
        .await;
    let session = client
        .ok("thread.session", json!({ "threadId": thread }))
        .await;
    client
        .ok("thread.mcp_servers", json!({ "threadId": thread }))
        .await;

    assert_eq!(daemon.launcher.launches().len(), 0, "nothing started omp");
    assert_eq!(
        shown["state"]["transcript"].as_array().map(Vec::len),
        Some(1),
        "the history of the session file: {shown}"
    );
    assert_eq!(commands, json!({ "commands": [] }));
    assert_eq!(session["process"]["state"], "stopped");

    client
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": "hello" }),
        )
        .await;
    assert_eq!(daemon.launcher.launches().len(), 1, "a prompt starts omp");
}

// spec: DMN-029
#[tokio::test(flavor = "multi_thread")]
async fn a_repository_linked_while_omp_starts_is_a_root_before_the_next_prompt() {
    let daemon = daemon(&[]).await;
    let launches = daemon.launcher.gate_launches();
    let mut client = daemon.client("test").await;
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "racing" }),
        )
        .await;
    let (id, workspace_id) = (thread["id"].clone(), thread["workspaceId"].clone());
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    let folder = index["state"]["workspaces"][0]["path"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut starter = daemon.client("starter").await;
    let starting = tokio::spawn({
        let id = id.clone();
        async move { starter.ok("thread.state", json!({ "threadId": id })).await }
    });
    // omp's flags come from a scan made before the link exists.
    launches.arrived(1).await;
    let link = Path::new(&folder).join("shop");
    std::os::unix::fs::symlink(daemon.external_repo("shop"), &link).unwrap();
    let mut linker = daemon.client("linker").await;
    let rescanning = tokio::spawn({
        let workspace_id = workspace_id.clone();
        async move {
            linker
                .ok("workspaces.rescan", json!({ "workspaceId": workspace_id }))
                .await
        }
    });
    // The rescan publishes the link, then syncs the roots of the thread whose omp still starts; then omp comes up.
    loop {
        let workspace = next_workspace(&mut client, &workspace_id).await;
        if workspace["repos"][0]["path"] == json!(link) {
            break;
        }
    }
    launches.open();
    starting.await.unwrap();
    rescanning.await.unwrap();
    let link = link.display().to_string();

    client
        .ok("thread.prompt", json!({ "threadId": id, "text": "hello" }))
        .await;

    let prompts: Vec<String> = daemon
        .launcher
        .process(0)
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            omfp_daemon::agent::FakeCall::Prompt(prompt) => Some(prompt.message),
            _ => None,
        })
        .collect();
    assert_eq!(prompts, [format!("/add-dir {link}"), "hello".to_owned()]);
}

/// The settings of the next `settings_changed` on the index.
async fn next_settings(client: &mut Client) -> Value {
    loop {
        let event = client.event("index").await;
        if event["event"]["kind"] == "settings_changed" {
            return event["event"]["settings"].clone();
        }
    }
}

// spec: DMN-009, UI-097
#[tokio::test(flavor = "multi_thread")]
async fn settings_come_from_the_config_file_and_settings_set_writes_it_back() {
    let config = tempfile::tempdir().unwrap();
    let file = config.path().join("config.toml");
    std::fs::write(
        &file,
        "# Mine.\n[ui]\ndensity = \"normal\" # roomy\n\n[keymap]\n\"Space t\" = \"theme.set\"\n",
    )
    .unwrap();
    let daemon = daemon(&["--config", file.to_str().unwrap()]).await;
    let mut web = daemon.client("omfp/web").await;
    let mut desktop = daemon.client("omfp/desktop").await;
    let index = desktop.ok("subscribe", json!({ "topic": "index" })).await;
    let expected = json!({
        "ui": { "density": "normal" },
        "keymap": [{ "mode": "normal", "keys": "Space t", "command": "theme.set" }],
        "error": null
    });
    assert_eq!(index["state"]["settings"], expected);
    assert_eq!(web.ok("settings.get", json!({})).await, expected);

    let set = web
        .ok(
            "settings.set",
            json!({
                "ui": { "density": "compact", "fonts": { "code": "Iosevka, monospace" } },
                "keymap": [{ "mode": "insert", "keys": "Ctrl-j", "command": "nav.next" }]
            }),
        )
        .await;

    let expected = json!({
        "ui": { "density": "compact", "fonts": { "code": "Iosevka, monospace" } },
        "keymap": [
            { "mode": "normal", "keys": "Space t", "command": "theme.set" },
            { "mode": "insert", "keys": "Ctrl-j", "command": "nav.next" }
        ],
        "error": null
    });
    assert_eq!(set, expected);
    assert_eq!(next_settings(&mut desktop).await, expected);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.starts_with("# Mine.\n[ui]\ndensity = \"compact\" # roomy\n"),
        "{text}"
    );
    assert!(
        text.contains("[keymap.insert]\nCtrl-j = \"nav.next\""),
        "{text}"
    );
    let error = web
        .call(
            "settings.set",
            json!({ "keymap": [{ "mode": "normal", "keys": "", "command": "x" }] }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "bad_request");
}

// spec: DMN-009, UI-097
#[tokio::test(flavor = "multi_thread")]
async fn config_file_changes_reach_every_window_and_an_invalid_file_keeps_the_settings() {
    let config = tempfile::tempdir().unwrap();
    let file = config.path().join("config.toml");
    let daemon = daemon(&["--config", file.to_str().unwrap()]).await;
    let mut client = daemon.client("test").await;
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(
        index["state"]["settings"],
        json!({ "ui": {}, "keymap": [], "error": null }),
        "no file yet: the defaults"
    );

    replace_file(&file, "[ui]\nnotifications = \"never\"\n");
    assert_eq!(
        next_settings(&mut client).await,
        json!({ "ui": { "notifications": "never" }, "keymap": [], "error": null })
    );
    replace_file(&file, "[ui]\nnotifications = \"sometimes\"\n");
    let broken = next_settings(&mut client).await;
    assert_eq!(
        broken["ui"],
        json!({ "notifications": "never" }),
        "the previous settings stay"
    );
    let error = broken["error"].as_str().unwrap();
    assert!(
        error.contains("sometimes") && error.contains("config.toml"),
        "{error}"
    );
    // `[daemon]` changes are not applied live (the log asks for a restart); `[ui]` ones are.
    replace_file(
        &file,
        "[daemon]\nmax_processes = 2\n\n[ui]\ndensity = \"normal\"\n",
    );
    assert_eq!(
        next_settings(&mut client).await,
        json!({ "ui": { "density": "normal" }, "keymap": [], "error": null })
    );
}

// spec: DMN-009
#[tokio::test(flavor = "multi_thread")]
async fn without_a_config_file_settings_live_in_memory() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    client
        .ok("settings.set", json!({ "ui": { "density": "normal" } }))
        .await;
    let set = client
        .ok(
            "settings.set",
            json!({ "keymap": [{ "mode": "cmdline", "keys": "Ctrl-p", "command": "" }] }),
        )
        .await;
    assert_eq!(
        set,
        json!({
            "ui": { "density": "normal" },
            "keymap": [{ "mode": "cmdline", "keys": "Ctrl-p", "command": "" }],
            "error": null
        })
    );
    assert_eq!(client.ok("settings.get", json!({})).await, set);
}

// spec: DMN-239, DMN-240, UI-510, DMN-238
#[tokio::test(flavor = "multi_thread")]
async fn the_built_in_theme_is_a_ui_setting_written_back_and_independent_of_the_user_theme() {
    let config = tempfile::tempdir().unwrap();
    let file = config.path().join("config.toml");
    std::fs::write(&file, "[ui]\ntheme = \"nord\" # cold\n").unwrap();
    let daemon = daemon(&["--config", file.to_str().unwrap()]).await;
    let mut web = daemon.client("omfp/web").await;
    let mut desktop = daemon.client("omfp/desktop").await;
    let index = desktop.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["state"]["settings"]["ui"], json!({ "theme": "nord" }));
    let css = ":root { --omfp-primary: #6750a4; }";
    web.ok("theme.set", json!({ "css": css })).await;
    assert_eq!(next_theme(&mut desktop).await, json!({ "css": css }));

    let set = web
        .ok("settings.set", json!({ "ui": { "theme": "rose-pine" } }))
        .await;

    assert_eq!(set["ui"], json!({ "theme": "rose-pine" }));
    assert_eq!(next_settings(&mut desktop).await["ui"], set["ui"]);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "[ui]\ntheme = \"rose-pine\" # cold\n"
    );
    // The user theme stays, kept for `matugen`, the theme that applies it.
    assert_eq!(web.ok("theme.get", json!({})).await, json!({ "css": css }));
    let set = web
        .ok("settings.set", json!({ "ui": { "theme": "matugen" } }))
        .await;
    assert_eq!(set["ui"], json!({ "theme": "matugen" }));
    assert_eq!(next_settings(&mut desktop).await["ui"], set["ui"]);
    assert_eq!(web.ok("theme.get", json!({})).await, json!({ "css": css }));
    web.ok("theme.set", json!({ "css": null })).await;
    assert_eq!(
        web.ok("settings.get", json!({})).await["ui"],
        json!({ "theme": "matugen" })
    );
    let error = web
        .call("settings.set", json!({ "ui": { "theme": "dracula" } }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "bad_request");
    replace_file(&file, "[ui]\ntheme = \"tokyo-night\"\n");
    assert_eq!(
        next_settings(&mut desktop).await["ui"],
        json!({ "theme": "tokyo-night" })
    );
}

/// Authenticates as `name` and returns the connection with its number (PRT-051).
async fn numbered(daemon: &TestDaemon, name: &str) -> (Client, u64) {
    let mut client = Client::connect(daemon.handle.addr()).await;
    let hello = client
        .ok(
            "system.hello",
            json!({ "token": daemon.handle.token().expose(), "protocol": 1, "client": name }),
        )
        .await;
    (
        client,
        hello["connection"]
            .as_u64()
            .expect("hello names the connection"),
    )
}

const OFFER: &str = "v=0\r\na=setup:actpass\r\na=mid:0\r\n";

// spec: PRT-051, DMN-100
#[tokio::test(flavor = "multi_thread")]
async fn live_connect_relays_the_offer_and_records_the_owning_connection() {
    let daemon = daemon(&[]).await;
    let (mut first, first_number) = numbered(&daemon, "omfp/web").await;
    let (mut second, second_number) = numbered(&daemon, "omfp/web").await;
    assert_ne!(first_number, second_number);
    let thread = create_thread(&mut first).await;

    let answer = first
        .ok(
            "thread.live_connect",
            json!({ "threadId": thread, "sdpOffer": OFFER }),
        )
        .await;

    assert_eq!(
        answer,
        json!({ "sdpAnswer": omfp_daemon::agent::answer_for(OFFER), "voice": "sol" })
    );
    let mut follower = Follower::snapshot(&mut second, &thread, None).await;
    follower
        .until(&mut second, |state| {
            state.live.as_ref().is_some_and(|live| live.phase.is_some())
        })
        .await;
    let live = follower.state.live.clone().unwrap();
    assert_eq!(live.owner.connection, first_number);
    assert_eq!(live.owner.client, "omfp/web");
}

// spec: DMN-103
#[tokio::test(flavor = "multi_thread")]
async fn closing_the_owning_connection_stops_its_live_session() {
    let daemon = daemon(&[]).await;
    let (mut owner, _) = numbered(&daemon, "omfp/web").await;
    let (mut watcher, _) = numbered(&daemon, "omfp/web").await;
    let thread = create_thread(&mut owner).await;
    owner
        .ok(
            "thread.live_connect",
            json!({ "threadId": thread, "sdpOffer": OFFER }),
        )
        .await;
    let mut follower = Follower::snapshot(&mut watcher, &thread, None).await;

    drop(owner);

    follower
        .until(&mut watcher, |state| state.live.is_none())
        .await;
    let process = daemon.launcher.process(0);
    assert!(!process.live_active());
    assert!(process.calls().iter().any(|call| matches!(
        call,
        omfp_daemon::agent::FakeCall::Call(omfp_daemon::agent::OmpCall::LiveStop)
    )));
}

// spec: DMN-109, PRT-053, DMN-110, PRT-054
#[tokio::test(flavor = "multi_thread")]
async fn live_start_relays_the_voice_and_levels_reach_every_subscriber_as_transient_frames() {
    use omfp_daemon::agent::{FakeLive, live_levels, live_phase};
    let daemon = daemon(&[]).await;
    let (mut desktop, desktop_number) = numbered(&daemon, "omfp/desktop").await;
    let (mut web, _) = numbered(&daemon, "omfp/web").await;
    let thread = create_thread(&mut desktop).await;
    let topic = format!("thread/{thread}");
    let mut follower = Follower::snapshot(&mut web, &thread, None).await;
    daemon.launcher.script_live(FakeLive::connect(vec![
        live_levels(0.25, 0.5),
        live_phase(omp_rpc::LivePhase::Speaking),
    ]));

    let started = desktop
        .ok("thread.live_start", json!({ "threadId": thread }))
        .await;

    assert_eq!(started, json!({ "voice": "sol" }));
    assert_eq!(
        web.transient(&topic).await,
        json!({
            "type": "transient",
            "topic": topic,
            "event": { "kind": "live_levels", "input": 0.25, "output": 0.5 }
        })
    );
    follower
        .until(&mut web, |state| {
            state
                .live
                .as_ref()
                .and_then(|live| live.phase)
                .is_some_and(|phase| phase == omp_rpc::LivePhase::Speaking)
        })
        .await;
    let live = follower.state.live.clone().unwrap();
    assert_eq!(live.owner.connection, desktop_number);
    assert_eq!(live.owner.client, "omfp/desktop");
}

// spec: DMN-109, DMN-103, DMN-101
#[tokio::test(flavor = "multi_thread")]
async fn a_live_start_session_is_the_threads_one_and_stops_with_its_connection() {
    let daemon = daemon(&[]).await;
    let (mut owner, _) = numbered(&daemon, "omfp/desktop").await;
    let (mut watcher, _) = numbered(&daemon, "omfp/web").await;
    let thread = create_thread(&mut owner).await;
    owner
        .ok("thread.live_start", json!({ "threadId": thread }))
        .await;
    let busy = watcher
        .call(
            "thread.live_connect",
            json!({ "threadId": thread, "sdpOffer": OFFER }),
        )
        .await
        .unwrap_err();
    assert_eq!(busy["code"], "conflict", "{busy}");
    let mut follower = Follower::snapshot(&mut watcher, &thread, None).await;

    drop(owner);

    follower
        .until(&mut watcher, |state| state.live.is_none())
        .await;
    assert!(!daemon.launcher.process(0).live_active());
}

// spec: DMN-060
#[tokio::test(flavor = "multi_thread")]
async fn open_file_reaches_the_window_last_used_on_the_thread_and_fails_without_one() {
    let daemon = daemon(&[]).await;
    daemon.launcher.push_run(fixture("text_reply"));
    let (mut web, web_number) = numbered(&daemon, "omfp/web").await;
    let (mut desktop, desktop_number) = numbered(&daemon, "omfp/desktop").await;
    let thread = create_thread(&mut web).await;
    let supervisor = daemon.handle.supervisor();
    assert_eq!(
        supervisor.open_file(&thread, "notes.md", Some(3)),
        Err(omfp_daemon::supervisor::NO_WINDOW.to_owned()),
        "no window watches the thread yet"
    );
    let opened = |path: &'static str| {
        move |state: &omfp_core::ThreadState| {
            state
                .open_file
                .as_ref()
                .is_some_and(|request| request.path == path)
        }
    };

    let mut web_view = Follower::snapshot(&mut web, &thread, None).await;
    let mut desktop_view = Follower::snapshot(&mut desktop, &thread, None).await;
    supervisor.open_file(&thread, "notes.md", Some(3)).unwrap();
    web_view.until(&mut web, opened("notes.md")).await;
    let request = web_view.state.open_file.clone().unwrap();
    assert_eq!(
        (request.connection, request.line),
        (desktop_number, Some(3)),
        "the window that subscribed last"
    );

    web.ok(
        "thread.prompt",
        json!({ "threadId": thread, "text": "say hi" }),
    )
    .await;
    supervisor.open_file(&thread, "src/lib.rs", None).unwrap();
    desktop_view.until(&mut desktop, opened("src/lib.rs")).await;
    assert_eq!(
        desktop_view.state.open_file.clone().unwrap().connection,
        web_number,
        "the window that prompted last"
    );

    for client in [&mut web, &mut desktop] {
        client
            .ok(
                "unsubscribe",
                json!({ "topic": format!("thread/{thread}") }),
            )
            .await;
    }
    assert_eq!(
        supervisor.open_file(&thread, "notes.md", None),
        Err(omfp_daemon::supervisor::NO_WINDOW.to_owned())
    );
}

// spec: DMN-101, DMN-102, DMN-104
#[tokio::test(flavor = "multi_thread")]
async fn live_errors_have_their_codes_and_readable_messages() {
    let daemon = daemon(&[]).await;
    let (mut client, _) = numbered(&daemon, "omfp/web").await;
    let thread = create_thread(&mut client).await;

    let blank = client
        .call(
            "thread.live_connect",
            json!({ "threadId": thread, "sdpOffer": "" }),
        )
        .await
        .unwrap_err();
    assert_eq!(blank["code"], "bad_request", "{blank}");
    let mute = client
        .call(
            "thread.live_mute",
            json!({ "threadId": thread, "muted": true }),
        )
        .await
        .unwrap_err();
    assert_eq!(mute["code"], "conflict", "{mute}");
    assert_eq!(
        client
            .ok("thread.live_stop", json!({ "threadId": thread }))
            .await,
        json!({})
    );

    daemon
        .launcher
        .script_live(omfp_daemon::agent::FakeLive::no_credential());
    let login = client
        .call(
            "thread.live_connect",
            json!({ "threadId": thread, "sdpOffer": OFFER }),
        )
        .await
        .unwrap_err();
    assert_eq!(login["code"], "omp_error", "{login}");
    assert_eq!(
        login["message"],
        "live voice needs an `openai-codex` login in omp: run `:login openai-codex`, then try again"
    );
    assert_eq!(
        login["details"]["ompError"],
        omfp_daemon::agent::NO_CREDENTIAL
    );

    daemon
        .launcher
        .script_live(omfp_daemon::agent::FakeLive::default());
    let mut follower = Follower::snapshot(&mut client, &thread, None).await;
    follower
        .until(&mut client, |state| {
            state.live.is_none() && state.live_error.is_some()
        })
        .await;
    client
        .ok(
            "thread.live_connect",
            json!({ "threadId": thread, "sdpOffer": OFFER }),
        )
        .await;
    let second = client
        .call(
            "thread.live_connect",
            json!({ "threadId": thread, "sdpOffer": OFFER }),
        )
        .await
        .unwrap_err();
    assert_eq!(second["code"], "conflict", "{second}");
    assert_eq!(
        client
            .ok(
                "thread.live_mute",
                json!({ "threadId": thread, "muted": true })
            )
            .await,
        json!({ "muted": true })
    );
}

/// What `get_login_providers` answers in the login tests.
fn login_providers() -> FakeReply {
    FakeReply::Ok(json!({ "providers": [
        { "id": "anthropic", "name": "Anthropic (Claude Pro/Max)", "available": true, "authenticated": false },
        { "id": "kimi-code", "name": "Kimi Code", "available": true, "authenticated": true },
        { "id": "cursor", "name": "Cursor", "available": false, "authenticated": false }
    ]}))
}

/// Sends a request without waiting for its response; returns its id.
async fn send_request(client: &mut Client, id: &str, method: &str, params: Value) {
    client
        .send_text(
            json!({ "type": "request", "id": id, "method": method, "params": params }).to_string(),
        )
        .await;
}

/// The next frame of `client` that is a window request, with its id.
async fn next_window_request(client: &mut Client) -> (String, Value) {
    loop {
        let frame = client.frame().await;
        if frame["type"] == "window_request" {
            return (
                frame["id"].as_str().unwrap().to_owned(),
                frame["request"].clone(),
            );
        }
    }
}

/// The response to request `id`, from the frames `client` received.
async fn response_to(client: &mut Client, id: &str) -> Value {
    loop {
        let frame = client.frame().await;
        if frame["type"] == "response" && frame["id"] == id {
            return frame;
        }
    }
}

/// Waits until the fake launched `count` processes.
async fn launched(daemon: &TestDaemon, count: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while daemon.launcher.launches().len() < count {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no launch {count}: {}", daemon.launcher.launches().len()));
}

// spec: PRT-129, DMN-219
#[tokio::test(flavor = "multi_thread")]
async fn providers_come_from_a_short_lived_omp_started_with_a_catalog_model() {
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let mut client = daemon.client("test").await;

    let providers = client.ok("auth.providers", json!({})).await;

    assert_eq!(providers["providers"].as_array().unwrap().len(), 3);
    assert_eq!(
        providers["providers"][0],
        json!({ "id": "anthropic", "name": "Anthropic (Claude Pro/Max)", "available": true, "authenticated": false })
    );
    let launch = &daemon.launcher.launches()[0];
    assert_eq!(
        launch.spec.model.as_deref(),
        Some(omfp_daemon::supervisor::LOGIN_MODEL)
    );
    let login = daemon.dir.path().join("login");
    assert_eq!(launch.spec.cwd, login.join("work"));
    assert_eq!(launch.spec.session_dir, login.join("sessions"));
    assert_eq!(launch.spec.add_dirs, Vec::<std::path::PathBuf>::new());
    assert_eq!(launch.spec.append_system_prompt, None);
    assert_eq!(launch.tools, Vec::<String>::new());
    assert!(launch.process.is_closed(), "the login omp is stopped");
}

// spec: DMN-219
#[tokio::test(flavor = "multi_thread")]
async fn a_login_omp_that_will_not_start_with_the_catalog_model_starts_without_it() {
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    daemon
        .launcher
        .fail_next_launch("Model \"anthropic/claude-sonnet-4-5\" not found");
    let mut client = daemon.client("test").await;

    client.ok("auth.providers", json!({})).await;

    let launches = daemon.launcher.launches();
    assert_eq!(launches.len(), 1, "the failed start is not a launch");
    assert_eq!(launches[0].spec.model, None);

    daemon
        .launcher
        .fail_next_launch("Model \"anthropic/claude-sonnet-4-5\" not found");
    daemon
        .launcher
        .fail_next_launch("No default model selected. Use /login, …");
    let error = client.call("auth.providers", json!({})).await.unwrap_err();
    assert_eq!(error["code"], "thread_failed", "{error}");
    assert_eq!(
        error["details"]["stderrTail"],
        "No default model selected. Use /login, …"
    );
}

// spec: PRT-130, PRT-131, DMN-220, DMN-221, DMN-224
#[tokio::test(flavor = "multi_thread")]
async fn a_login_shows_its_flow_in_the_calling_window_then_reloads_the_running_threads() {
    use omfp_core::wire::{
        ExtensionUiRequest, ExtensionUiResponse, InputUiRequest, NotifyUiRequest, OpenUrlUiRequest,
    };
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let gate = daemon.launcher.gate("login");
    let mut window = daemon.client("test").await;
    let thread = create_thread(&mut window).await;
    window
        .ok("thread.state", json!({ "threadId": thread }))
        .await;

    send_request(
        &mut window,
        "l1",
        "auth.login",
        json!({ "providerId": "kimi-code" }),
    )
    .await;
    gate.arrived(1).await;
    let omp = daemon.launcher.process(1);
    let ui = |request| RpcNotification::ExtensionUiRequest(request);
    omp.emit(vec![
        ui(ExtensionUiRequest::OpenUrl(OpenUrlUiRequest {
            id: "u1".into(),
            url: "https://auth.kimi.com/device?code=ABCD".into(),
            launch_url: Some("http://127.0.0.1:1455/launch".into()),
            instructions: Some("Enter ABCD".into()),
        })),
        ui(ExtensionUiRequest::Notify(NotifyUiRequest {
            id: "u2".into(),
            message: "Waiting for the browser…".into(),
            notify_type: None,
        })),
        ui(ExtensionUiRequest::Input(InputUiRequest {
            id: "u3".into(),
            title: "Paste the authorization code".into(),
            placeholder: Some("code#state".into()),
            timeout: Some(600_000),
            secret: None,
        })),
    ]);

    let (_, url) = next_window_request(&mut window).await;
    assert_eq!(
        url,
        json!({
            "kind": "login_url", "providerId": "kimi-code", "name": "Kimi Code",
            "url": "https://auth.kimi.com/device?code=ABCD", "launchUrl": "http://127.0.0.1:1455/launch",
            "instructions": "Enter ABCD"
        })
    );
    let (_, progress) = next_window_request(&mut window).await;
    assert_eq!(progress["kind"], "login_progress");
    assert_eq!(progress["message"], "Waiting for the browser…");
    let (input_id, input) = next_window_request(&mut window).await;
    assert_eq!(
        input,
        json!({
            "kind": "login_input", "providerId": "kimi-code", "name": "Kimi Code",
            "message": "Paste the authorization code", "placeholder": "code#state"
        })
    );
    window
        .ok(
            "windows.answer",
            json!({ "requestId": input_id, "answer": { "kind": "login_input", "value": "abc#xyz" } }),
        )
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !omp.calls().iter().any(|call| {
            matches!(
                call,
                omfp_daemon::agent::FakeCall::Respond(ExtensionUiResponse::ValueUiResponse(value))
                    if value.id == "u3" && value.value == "abc#xyz"
            )
        }) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the code reaches omp");
    gate.open();

    let response = response_to(&mut window, "l1").await;
    assert_eq!(
        response["result"],
        json!({ "providerId": "kimi-code", "name": "Kimi Code" }),
        "{response}"
    );
    assert!(omp.is_closed(), "the login omp is stopped");
    // The idle thread restarts to load the models the credential unlocked.
    launched(&daemon, 3).await;
    assert_eq!(
        daemon.launcher.launches()[2].spec.session_dir,
        daemon.launcher.launches()[0].spec.session_dir
    );
    assert!(daemon.launcher.process(0).is_closed());
}

// spec: PRT-130, DMN-220
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_or_unavailable_provider_is_refused() {
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let mut client = daemon.client("test").await;

    let unknown = client
        .call("auth.login", json!({ "providerId": "nope" }))
        .await
        .unwrap_err();
    assert_eq!(unknown["code"], "bad_request");
    let message = unknown["message"].as_str().unwrap();
    assert!(
        message.contains("anthropic, kimi-code, cursor"),
        "{message}"
    );
    let unavailable = client
        .call("auth.login", json!({ "providerId": "cursor" }))
        .await
        .unwrap_err();
    assert_eq!(unavailable["code"], "bad_request", "{unavailable}");
    for launch in daemon.launcher.launches() {
        assert!(launch.process.is_closed());
    }
}

// spec: PRT-130, DMN-220
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_login_reports_omps_words() {
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    daemon.launcher.reply(
        "login",
        FakeReply::Fail {
            message:
                "Provider 'anthropic' requires secret input, which is not supported in RPC mode."
                    .into(),
            code: None,
        },
    );
    let mut client = daemon.client("test").await;

    let error = client
        .call("auth.login", json!({ "providerId": "anthropic" }))
        .await
        .unwrap_err();

    assert_eq!(error["code"], "omp_error", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("requires secret input"),
        "{error}"
    );
}

// spec: PRT-132, DMN-222
#[tokio::test(flavor = "multi_thread")]
async fn a_login_is_cancelled_by_auth_login_cancel_by_the_next_login_and_by_closing_its_window() {
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let gate = daemon.launcher.gate("login");
    let mut window = daemon.client("test").await;

    assert_eq!(
        window.ok("auth.login_cancel", json!({})).await,
        json!({ "providerId": null })
    );
    send_request(
        &mut window,
        "l1",
        "auth.login",
        json!({ "providerId": "anthropic" }),
    )
    .await;
    gate.arrived(1).await;
    let mut other = daemon.client("other").await;
    assert_eq!(
        other.ok("auth.login_cancel", json!({})).await,
        json!({ "providerId": "anthropic" })
    );
    assert!(daemon.launcher.process(0).is_closed(), "its omp is stopped");

    // A second login cancels the first: omp's callback listeners take fixed ports.
    send_request(
        &mut window,
        "l2",
        "auth.login",
        json!({ "providerId": "anthropic" }),
    )
    .await;
    gate.arrived(2).await;
    send_request(
        &mut window,
        "l3",
        "auth.login",
        json!({ "providerId": "kimi-code" }),
    )
    .await;
    gate.arrived(3).await;
    assert!(daemon.launcher.process(1).is_closed());
    assert!(!daemon.launcher.process(2).is_closed());

    // Closing the window cancels the login it shows; another window's login stays.
    drop(window);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !daemon.launcher.process(2).is_closed() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("closing the window stops its login");
    gate.open();

    let mut window = daemon.client("again").await;
    send_request(
        &mut window,
        "l4",
        "auth.login",
        json!({ "providerId": "anthropic" }),
    )
    .await;
    let response = response_to(&mut window, "l4").await;
    assert_eq!(response["ok"], true, "{response}");
    let cancelled = other.call("auth.login_cancel", json!({})).await.unwrap();
    assert_eq!(cancelled, json!({ "providerId": null }));
}

// spec: PRT-132, DMN-222
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_login_fails_as_cancelled() {
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let gate = daemon.launcher.gate("login");
    let mut window = daemon.client("test").await;
    send_request(
        &mut window,
        "l1",
        "auth.login",
        json!({ "providerId": "anthropic" }),
    )
    .await;
    gate.arrived(1).await;

    let mut other = daemon.client("other").await;
    other.ok("auth.login_cancel", json!({})).await;
    gate.open();

    let response = response_to(&mut window, "l1").await;
    assert_eq!(response["error"]["code"], "conflict", "{response}");
    assert_eq!(
        response["error"]["message"],
        "the login to Anthropic (Claude Pro/Max) was cancelled"
    );
}

/// The `login` of the next `login_changed` event on the `index` topic (DMN-290).
async fn login_changed(client: &mut Client) -> Value {
    loop {
        let frame = client.event("index").await;
        if frame["event"]["kind"] == "login_changed" {
            return frame["event"]["login"].clone();
        }
    }
}

// spec: DMN-290
#[tokio::test(flavor = "multi_thread")]
async fn every_window_sees_the_running_login_its_page_and_its_progress_on_the_index() {
    use omfp_core::wire::{ExtensionUiRequest, NotifyUiRequest, OpenUrlUiRequest};
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let gate = daemon.launcher.gate("login");
    let mut caller = daemon.client("agent").await;
    let mut other = daemon.client("other").await;
    let index = other.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["state"]["login"], Value::Null, "no login runs");

    send_request(
        &mut caller,
        "l1",
        "auth.login",
        json!({ "providerId": "kimi-code" }),
    )
    .await;
    gate.arrived(1).await;
    let started = json!({
        "providerId": "kimi-code", "name": "Kimi Code", "url": null, "instructions": null, "progress": null
    });
    assert_eq!(login_changed(&mut other).await, started);
    let ui = |request| RpcNotification::ExtensionUiRequest(request);
    daemon.launcher.process(0).emit(vec![
        ui(ExtensionUiRequest::OpenUrl(OpenUrlUiRequest {
            id: "u1".into(),
            url: "https://auth.kimi.com/device?code=ABCD".into(),
            launch_url: None,
            instructions: Some("Enter ABCD".into()),
        })),
        ui(ExtensionUiRequest::Notify(NotifyUiRequest {
            id: "u2".into(),
            message: "Waiting for the browser…".into(),
            notify_type: None,
        })),
    ]);

    let opened = json!({
        "providerId": "kimi-code", "name": "Kimi Code", "url": "https://auth.kimi.com/device?code=ABCD",
        "instructions": "Enter ABCD", "progress": null
    });
    assert_eq!(login_changed(&mut other).await, opened);
    let mut waiting = opened.clone();
    waiting["progress"] = json!("Waiting for the browser…");
    assert_eq!(login_changed(&mut other).await, waiting);
    // A window opened meanwhile reads it from the snapshot.
    let mut late = daemon.client("late").await;
    let index = late.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["state"]["login"], waiting);

    gate.open();
    let response = response_to(&mut caller, "l1").await;
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(login_changed(&mut other).await, Value::Null, "it ended");
}

// spec: PRT-133, DMN-223
#[tokio::test(flavor = "multi_thread")]
async fn logout_removes_every_stored_credential_of_the_provider() {
    let daemon = daemon(&[]).await;
    daemon.launcher.reply(
        "get_logout_accounts",
        FakeReply::Ok(json!({ "accounts": [
            { "credentialId": 3, "provider": "anthropic", "label": "a@example.com", "detail": "oauth #3",
              "type": "oauth", "active": true },
            { "credentialId": 7, "provider": "anthropic", "label": "API key #7", "detail": "stored API key #7",
              "type": "api_key", "active": false }
        ]})),
    );
    daemon.launcher.reply(
        "logout",
        FakeReply::Ok(json!({ "remainingSource": "ANTHROPIC_API_KEY" })),
    );
    let mut client = daemon.client("test").await;

    let result = client
        .ok("auth.logout", json!({ "providerId": "anthropic" }))
        .await;

    assert_eq!(
        result,
        json!({ "providerId": "anthropic", "removed": 2, "remainingSource": "ANTHROPIC_API_KEY" })
    );
    let process = daemon.launcher.process(0);
    let removed: Vec<i64> = process
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            omfp_daemon::agent::FakeCall::Call(omfp_daemon::agent::OmpCall::Logout(logout)) => {
                Some(logout.credential_id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(removed, [3, 7]);
    assert!(process.is_closed());
}

/// The count of the next `models_changed` event on the `index` topic (PRT-141).
async fn models_changed(client: &mut Client) -> Value {
    loop {
        let frame = client.event("index").await;
        if frame["event"]["kind"] == "models_changed" {
            return frame["event"]["models"].clone();
        }
    }
}

// spec: PRT-140, PRT-141, DMN-235
#[tokio::test(flavor = "multi_thread")]
async fn the_usable_models_are_checked_once_by_a_login_omp_and_published_on_the_index() {
    let daemon = daemon(&[]).await;
    daemon.launcher.set_usable_models(0);
    let mut client = daemon.client("test").await;
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["state"]["models"], Value::Null, "nobody asked yet");

    let status = client.ok("auth.status", json!({})).await;

    assert_eq!(status, json!({ "models": 0 }));
    assert_eq!(models_changed(&mut client).await, 0);
    assert_eq!(
        client.ok("auth.status", json!({})).await,
        json!({ "models": 0 })
    );
    let checks = daemon.launcher.model_checks();
    assert_eq!(checks.len(), 1, "the count is kept");
    assert_eq!(
        checks[0].model.as_deref(),
        Some(omfp_daemon::supervisor::LOGIN_MODEL)
    );
    assert_eq!(checks[0].cwd, daemon.dir.path().join("login").join("work"));
    assert_eq!(daemon.launcher.launches().len(), 0, "a check is no launch");
    let mut late = daemon.client("late").await;
    let index = late.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["state"]["models"], 0);
}

// spec: DMN-235
#[tokio::test(flavor = "multi_thread")]
async fn a_check_that_cannot_start_omp_fails_with_its_stderr_and_the_next_one_checks_again() {
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .fail_next_launch("Model \"anthropic/claude-sonnet-4-5\" not found");
    daemon.launcher.fail_next_launch("fatal: out of memory");
    let mut client = daemon.client("test").await;

    let error = client.call("auth.status", json!({})).await.unwrap_err();

    assert_eq!(error["code"], "thread_failed", "{error}");
    assert_eq!(error["details"]["stderrTail"], "fatal: out of memory");
    assert_eq!(daemon.launcher.model_checks().len(), 0);
    assert_eq!(
        client.ok("auth.status", json!({})).await,
        json!({ "models": 1 })
    );
}

// spec: PRT-226, PRT-227, DMN-361
#[tokio::test(flavor = "multi_thread")]
async fn word_completion_answers_omps_suffix_without_a_thread() {
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("predict_word", FakeReply::Ok(json!({ "suffix": "sts" })));
    let mut client = daemon.client("test").await;

    let predicted = client
        .ok(
            "prompts.predict_word",
            json!({ "text": "Run the te", "cursor": 10 }),
        )
        .await;
    let feedback = client
        .ok(
            "prompts.predict_word_feedback",
            json!({ "text": "Run the te", "cursor": 10, "suggestion": "sts", "accepted": false }),
        )
        .await;

    assert_eq!(predicted, json!({ "suffix": "sts" }));
    assert_eq!(feedback, json!({}));
    let launches = daemon.launcher.launches();
    assert_eq!(launches.len(), 1, "one predictor omp, no thread");
    assert_eq!(
        launches[0].spec.cwd,
        daemon.dir.path().join("predict").join("work")
    );
}

// spec: PRT-226, DMN-362
#[tokio::test(flavor = "multi_thread")]
async fn word_completion_without_omp_fails_with_its_stderr() {
    let daemon = daemon(&[]).await;
    daemon.launcher.fail_next_launch("first attempt");
    daemon.launcher.fail_next_launch("fatal: out of memory");
    let mut client = daemon.client("test").await;

    let error = client
        .call("prompts.predict_word", json!({ "text": "a", "cursor": 1 }))
        .await
        .unwrap_err();

    assert_eq!(error["code"], "thread_failed", "{error}");
    assert_eq!(error["details"]["stderrTail"], "fatal: out of memory");
}

// spec: DMN-236
#[tokio::test(flavor = "multi_thread")]
async fn a_login_or_a_logout_checks_the_usable_models_again_once_someone_asked() {
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    daemon.launcher.set_usable_models(0);
    let mut window = daemon.client("test").await;
    window.ok("subscribe", json!({ "topic": "index" })).await;
    let anthropic = json!({ "providerId": "anthropic" });
    window.ok("auth.login", anthropic.clone()).await;
    assert_eq!(daemon.launcher.model_checks().len(), 0, "nobody asked yet");

    assert_eq!(
        window.ok("auth.status", json!({})).await,
        json!({ "models": 0 })
    );
    assert_eq!(models_changed(&mut window).await, 0);
    daemon.launcher.set_usable_models(4);
    window.ok("auth.login", anthropic.clone()).await;
    assert_eq!(models_changed(&mut window).await, 4);
    daemon.launcher.set_usable_models(0);
    window.ok("auth.logout", anthropic).await;
    assert_eq!(models_changed(&mut window).await, 0);
    assert_eq!(daemon.launcher.model_checks().len(), 3);
}

/// An `omp auth export` document: an API key of `openai`, an MCP server's credential, an OAuth login of
/// `anthropic` and a second, OAuth login of `openai`.
fn omp_export() -> String {
    json!({ "version": 1, "credentials": [
        { "provider": "openai", "credential": { "type": "api_key", "key": "sk-inert" } },
        { "provider": "mcp:fixture", "credential": { "type": "oauth", "access": "a", "refresh": "r", "expires": 1 } },
        { "provider": "anthropic", "credential": { "type": "oauth", "access": "a", "refresh": "r", "expires": 1 } },
        { "provider": "openai", "credential": { "type": "oauth", "access": "b", "refresh": "s", "expires": 2 } },
    ] })
    .to_string()
}

/// The last value `key` has in `env`: `None` when absent, `null` when unset, else the value as a string.
fn env_value(env: &[(std::ffi::OsString, Option<std::ffi::OsString>)], key: &str) -> Option<Value> {
    env.iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| {
            value
                .as_ref()
                .map_or(Value::Null, |value| json!(value.to_string_lossy()))
        })
}

impl TestDaemon {
    /// Makes the synthetic standalone omp exist: its credential store `~/.omp/agent/agent.db`, a WAL-mode SQLite
    /// database with a `standalone` table.
    fn install_standalone_omp(&self) {
        let agent = standalone_home(self.dir.path()).join(".omp").join("agent");
        std::fs::create_dir_all(&agent).unwrap();
        let db = rusqlite::Connection::open(agent.join("agent.db")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE standalone (n INTEGER);")
            .unwrap();
    }

    /// The snapshot directories left in `<data-dir>/login/`.
    fn snapshots(&self) -> Vec<std::path::PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.dir.path().join("login")) else {
            return Vec::new();
        };
        entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("snapshot-"))
            })
            .collect()
    }
}

/// Whether the SQLite database `bytes` has a table `table`.
fn has_table(bytes: &[u8], table: &str) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("copy.db");
    std::fs::write(&path, bytes).unwrap();
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

// spec: PRT-136, DMN-229
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // One scenario: no store, then a store, its snapshot and the export's spec.
async fn omps_logins_are_read_from_a_snapshot_of_its_store_only_when_the_store_exists() {
    let daemon = daemon(&[]).await;
    daemon.launcher.set_export(Ok(&omp_export()));
    let mut client = daemon.client("test").await;

    assert_eq!(
        client.ok("auth.importable", json!({})).await,
        json!({ "providers": [] })
    );
    let agent = standalone_home(daemon.dir.path())
        .join(".omp")
        .join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    assert_eq!(
        client.ok("auth.importable", json!({})).await,
        json!({ "providers": [] })
    );
    assert!(
        daemon.launcher.exports().is_empty(),
        "no omp runs without standalone omp's credential store"
    );

    daemon.install_standalone_omp();
    let listing = || {
        let mut names: Vec<_> = std::fs::read_dir(&agent)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let before = listing();
    let bytes = std::fs::read(agent.join("agent.db")).unwrap();
    assert_eq!(
        client.ok("auth.importable", json!({})).await,
        json!({ "providers": [
            { "providerId": "openai", "oauth": true },
            { "providerId": "anthropic", "oauth": true },
        ] })
    );
    assert_eq!(
        listing(),
        before,
        "nothing written beside standalone omp's store"
    );
    assert_eq!(std::fs::read(agent.join("agent.db")).unwrap(), bytes);
    let exports = daemon.launcher.exports();
    assert_eq!(exports.len(), 1);
    let spec = &exports[0];
    let dir = daemon.dir.path();
    assert_eq!(spec.program, std::path::PathBuf::from("omp"));
    assert_eq!(spec.cwd, dir.join("login").join("work"));
    assert!(spec.cwd.is_dir());
    let Some(Value::String(snapshot)) = env_value(&spec.env, "PI_CODING_AGENT_DIR") else {
        panic!("the export runs on a snapshot: {:?}", spec.env);
    };
    let snapshot = std::path::PathBuf::from(snapshot);
    assert_eq!(snapshot.parent(), Some(dir.join("login").as_path()));
    assert_eq!(
        env_value(&spec.env, "OMP_CODING_AGENT_DIR"),
        Some(json!(snapshot.to_string_lossy()))
    );
    assert_eq!(env_value(&spec.env, "OMP_LOG_LEVEL"), Some(json!("error")));
    assert_eq!(
        env_value(&spec.env, "OMP_PROFILE"),
        Some(json!("default")),
        "omfp's profile environment, not the user's"
    );
    assert_eq!(
        env_value(&spec.env, "XDG_DATA_HOME"),
        Some(json!(
            dir.join("omp-profile")
                .join("xdg")
                .join("data")
                .to_string_lossy()
        ))
    );
    assert_ne!(
        env_value(&spec.env, "HOME"),
        Some(json!(standalone_home(dir).to_string_lossy()))
    );
    let stores = daemon.launcher.export_stores();
    assert!(
        has_table(
            stores[0].as_deref().expect("a copy of the store"),
            "standalone"
        ),
        "the snapshot is standalone omp's store"
    );
    assert!(
        !snapshot.exists(),
        "the snapshot is removed after the export"
    );
    assert_eq!(daemon.snapshots(), Vec::<std::path::PathBuf>::new());
    assert!(
        daemon.launcher.imports().is_empty(),
        "reading copies nothing"
    );
    daemon.launcher.set_export(Ok(
        r#"{"version":1,"credentials":[{"provider":"groq","credential":{"type":"api_key","key":"k"}}]}"#,
    ));
    assert_eq!(
        client.ok("auth.importable", json!({})).await,
        json!({ "providers": [{ "providerId": "groq", "oauth": false }] })
    );
}

// spec: PRT-136, DMN-229
#[tokio::test(flavor = "multi_thread")]
async fn an_export_omfp_cannot_read_or_a_failed_one_is_omps_error() {
    let daemon = daemon(&[]).await;
    daemon.install_standalone_omp();
    let mut client = daemon.client("test").await;

    daemon
        .launcher
        .set_export(Ok(r#"{"version":2,"credentials":[]}"#));
    let error = client.call("auth.importable", json!({})).await.unwrap_err();
    assert_eq!(error["code"], "omp_error", "{error}");
    assert_eq!(
        error["message"],
        "omp `auth export` failed: its login export is version 2; omfp reads version 1"
    );

    daemon
        .launcher
        .set_export(Err("Auth broker is configured; export is not available"));
    let error = client.call("auth.import", json!({})).await.unwrap_err();
    assert_eq!(error["code"], "omp_error", "{error}");
    assert_eq!(
        error["message"],
        "omp `auth export` failed: Auth broker is configured; export is not available"
    );
    assert_eq!(
        daemon.launcher.imports().len(),
        0,
        "no import after a failed export"
    );
    assert_eq!(
        daemon.snapshots(),
        Vec::<std::path::PathBuf>::new(),
        "the snapshot is removed after a failed export too"
    );
}

// spec: PRT-137, DMN-230, DMN-224, DMN-236
#[tokio::test(flavor = "multi_thread")]
async fn copying_imports_the_provider_logins_into_the_profile_then_reloads_threads_and_models() {
    let daemon = daemon(&[]).await;
    daemon.install_standalone_omp();
    daemon.launcher.set_export(Ok(&omp_export()));
    daemon.launcher.set_usable_models(0);
    let mut window = daemon.client("test").await;
    window.ok("subscribe", json!({ "topic": "index" })).await;
    let thread = create_thread(&mut window).await;
    window
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    launched(&daemon, 1).await;
    assert_eq!(
        window.ok("auth.status", json!({})).await,
        json!({ "models": 0 })
    );
    assert_eq!(models_changed(&mut window).await, 0);
    daemon.launcher.set_usable_models(3);

    let copied = window.ok("auth.import", json!({})).await;

    assert_eq!(
        copied,
        json!({ "providers": [
            { "providerId": "openai", "oauth": true },
            { "providerId": "anthropic", "oauth": true },
        ] })
    );
    let imports = daemon.launcher.imports();
    assert_eq!(imports.len(), 1);
    let (spec, document) = &imports[0];
    let document: Value = serde_json::from_str(document).unwrap();
    let providers: Vec<&str> = document["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["provider"].as_str().unwrap())
        .collect();
    assert_eq!(document["version"], 1);
    assert_eq!(
        providers,
        ["openai", "anthropic", "openai"],
        "no MCP credential"
    );
    assert_eq!(document["credentials"][0]["credential"]["key"], "sk-inert");
    let dir = daemon.dir.path();
    assert_eq!(spec.cwd, dir.join("login").join("work"));
    assert_eq!(
        env_value(&spec.env, "PI_CODING_AGENT_DIR"),
        Some(json!(
            dir.join("omp-profile").join("agent").to_string_lossy()
        )),
        "imported into omfp's profile"
    );
    assert_eq!(env_value(&spec.env, "OMP_PROFILE"), Some(json!("default")));
    assert_eq!(daemon.launcher.exports().len(), 1, "a fresh export");
    assert_eq!(models_changed(&mut window).await, 3);
    // The idle thread restarts to load the models the logins unlocked, as after a login.
    launched(&daemon, 2).await;
    assert_eq!(
        daemon.launcher.launches()[1].spec.session_dir,
        daemon.launcher.launches()[0].spec.session_dir
    );
    assert!(daemon.launcher.process(0).is_closed());
}

// spec: PRT-137, DMN-230
#[tokio::test(flavor = "multi_thread")]
async fn nothing_to_copy_runs_no_import_and_a_failed_import_is_omps_error() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    assert_eq!(
        client.ok("auth.import", json!({})).await,
        json!({ "providers": [] })
    );
    assert!(daemon.launcher.exports().is_empty(), "no standalone omp");

    daemon.install_standalone_omp();
    daemon.launcher.set_export(Ok(
        r#"{"version":1,"credentials":[{"provider":"mcp:fixture","credential":{"type":"oauth","access":"a"}}]}"#,
    ));
    assert_eq!(
        client.ok("auth.import", json!({})).await,
        json!({ "providers": [] })
    );
    assert_eq!(daemon.launcher.exports().len(), 1);
    assert!(
        daemon.launcher.imports().is_empty(),
        "only MCP credentials: no import"
    );

    daemon.launcher.set_export(Ok(&omp_export()));
    daemon.launcher.fail_imports("Auth broker is configured");
    let error = client.call("auth.import", json!({})).await.unwrap_err();
    assert_eq!(error["code"], "omp_error", "{error}");
    assert_eq!(
        error["message"],
        "omp `auth import` failed: Auth broker is configured"
    );
    assert!(!error.to_string().contains("sk-inert"), "{error}");
}

// spec: PRT-136, PRT-137, DMN-229, DMN-230
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_store_is_never_exported() {
    let daemon = daemon(&[]).await;
    daemon.launcher.set_export(Ok(&omp_export()));
    let mut client = daemon.client("test").await;
    let agent = standalone_home(daemon.dir.path())
        .join(".omp")
        .join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::write(
        agent.join("agent.db"),
        "sk-inert, not a database, long enough to have a header",
    )
    .unwrap();

    assert_eq!(
        client.ok("auth.importable", json!({})).await,
        json!({ "providers": [] })
    );
    let error = client.call("auth.import", json!({})).await.unwrap_err();
    assert_eq!(error["code"], "omp_error", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("standalone omp's login store cannot be read: "),
        "{error}"
    );
    assert!(!error.to_string().contains("sk-inert"), "{error}");
    assert!(daemon.launcher.exports().is_empty(), "no omp runs");
    assert_eq!(daemon.snapshots(), Vec::<std::path::PathBuf>::new());
}

// spec: DMN-229
#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_started_from_an_omfp_shell_sees_no_standalone_omp_even_through_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let launcher = FakeLauncher::new();
    let git = Arc::new(FakeGit::new());
    let mut ports = ports(dir.path(), &launcher, &git);
    // omfp's own profile, named through a symlink to the data directory.
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(dir.path(), &alias).unwrap();
    ports.user_omp_env.push((
        "PI_CODING_AGENT_DIR".into(),
        Some(alias.join("omp-profile").join("agent").into()),
    ));
    let handle = start_with(config(dir.path(), &[]), ports).await.unwrap();
    let daemon = TestDaemon {
        handle,
        launcher,
        git,
        dir,
    };
    let agent = daemon.dir.path().join("omp-profile").join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    rusqlite::Connection::open(agent.join("agent.db"))
        .unwrap()
        .execute_batch("CREATE TABLE standalone (n INTEGER);")
        .unwrap();
    daemon.launcher.set_export(Ok(&omp_export()));
    let mut client = daemon.client("test").await;

    assert_eq!(
        client.ok("auth.importable", json!({})).await,
        json!({ "providers": [] })
    );
    assert_eq!(
        client.ok("auth.import", json!({})).await,
        json!({ "providers": [] })
    );
    assert!(daemon.launcher.exports().is_empty(), "omfp's own profile");
}

// spec: PRT-137, DMN-230
#[tokio::test(flavor = "multi_thread")]
async fn copying_while_a_login_runs_is_a_conflict() {
    let daemon = daemon(&[]).await;
    daemon.install_standalone_omp();
    daemon.launcher.set_export(Ok(&omp_export()));
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let gate = daemon.launcher.gate("login");
    let mut window = daemon.client("test").await;
    send_request(
        &mut window,
        "l1",
        "auth.login",
        json!({ "providerId": "anthropic" }),
    )
    .await;
    gate.arrived(1).await;

    let mut other = daemon.client("other").await;
    let error = other.call("auth.import", json!({})).await.unwrap_err();
    assert_eq!(error["code"], "conflict", "{error}");
    assert_eq!(
        error["message"],
        "a login is running; copy omp's logins after it ends"
    );
    assert!(daemon.launcher.exports().is_empty(), "nothing ran");
    gate.open();
    let response = response_to(&mut window, "l1").await;
    assert_eq!(response["ok"], true, "{response}");

    assert_eq!(
        other.ok("auth.import", json!({})).await["providers"][0]["providerId"],
        "openai"
    );
}

// spec: DMN-230
#[tokio::test(flavor = "multi_thread")]
async fn a_login_begins_only_after_a_running_copy() {
    let daemon = daemon(&[]).await;
    daemon.install_standalone_omp();
    daemon.launcher.set_export(Ok(&omp_export()));
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let imports = daemon.launcher.gate_imports();
    let listed = daemon.launcher.gate("get_login_providers");
    let login = daemon.launcher.gate("login");
    let mut window = daemon.client("test").await;
    window.ok("subscribe", json!({ "topic": "index" })).await;
    send_request(&mut window, "i1", "auth.import", json!({})).await;
    imports.arrived(1).await;

    // The login lists the providers, then waits for the copy to begin.
    let mut other = daemon.client("other").await;
    send_request(
        &mut other,
        "l1",
        "auth.login",
        json!({ "providerId": "anthropic" }),
    )
    .await;
    listed.arrived(1).await;
    listed.open();
    imports.open();

    // The copy stored its logins before the login began (the lock is released before the copy's answer is written,
    // so the two frames may come in either order).
    let (mut answered, mut began) = (false, false);
    while !(answered && began) {
        let frame = window.frame().await;
        if frame["type"] == "response" && frame["id"] == "i1" {
            assert_eq!(frame["ok"], true, "{frame}");
            answered = true;
        } else if frame["type"] == "event"
            && frame["event"]["kind"] == "login_changed"
            && !frame["event"]["login"].is_null()
        {
            assert_eq!(
                daemon.launcher.imports().len(),
                1,
                "the login began only after the copy stored its logins"
            );
            began = true;
        }
    }
    login.open();
    let response = response_to(&mut other, "l1").await;
    assert_eq!(response["ok"], true, "{response}");
}

// spec: DMN-237
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_start_contradicting_the_usable_models_checks_them_again() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    client.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(
        client.ok("auth.status", json!({})).await,
        json!({ "models": 1 })
    );
    assert_eq!(models_changed(&mut client).await, 1);
    let thread = create_thread(&mut client).await;

    // The credential went away behind the daemon's back: omp refuses to start.
    daemon.launcher.set_usable_models(0);
    daemon.launcher.fail_next_launch(
        "No default model selected. Use /login, set an API key environment variable, …",
    );
    let error = client
        .call("thread.state", json!({ "threadId": thread }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "thread_failed", "{error}");
    assert_eq!(models_changed(&mut client).await, 0);

    // A key came back: the thread starts without a model of its own.
    daemon.launcher.set_usable_models(2);
    client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    assert_eq!(models_changed(&mut client).await, 2);
    assert_eq!(daemon.launcher.model_checks().len(), 3);
}

/// The ids `method` lists (`workspaces.list`, `threads.list` with archived threads) by their `position`.
async fn by_position(client: &mut Client, method: &str, key: &str) -> Vec<(String, u64)> {
    let listed = client.ok(method, json!({ "includeArchived": true })).await;
    let mut entries: Vec<(String, u64)> = listed[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["name"]
                    .as_str()
                    .or(entry["title"].as_str())
                    .unwrap()
                    .to_owned(),
                entry["position"].as_u64().unwrap(),
            )
        })
        .collect();
    entries.sort_by_key(|(_, position)| *position);
    entries
}

/// A thread titled `title` in workspace `workspace`.
async fn thread_in(client: &mut Client, title: &str, workspace: &Value) -> String {
    let thread = client
        .ok(
            "threads.create",
            json!({ "title": title, "workspace": { "id": workspace } }),
        )
        .await;
    thread["id"].as_str().unwrap().to_owned()
}

// spec: DMN-280, DMN-281, DMN-284, PRT-170, PRT-172
#[tokio::test(flavor = "multi_thread")]
async fn workspaces_are_reordered_for_every_window() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let mut ids = Vec::new();
    for name in ["zeta", "alpha", "beta"] {
        ids.push(
            client
                .ok("workspaces.create", json!({ "name": name }))
                .await["id"]
                .clone(),
        );
    }
    // New workspaces go last, whatever their name.
    let order = |names: [&str; 3]| -> Vec<(String, u64)> {
        names
            .iter()
            .zip(0..)
            .map(|(name, position)| ((*name).to_owned(), position))
            .collect()
    };
    assert_eq!(
        by_position(&mut client, "workspaces.list", "workspaces").await,
        order(["zeta", "alpha", "beta"])
    );
    let mut other = daemon.client("other window").await;
    other.ok("subscribe", json!({ "topic": "index" })).await;

    let answer = client
        .ok(
            "workspaces.reorder",
            json!({ "workspaceId": ids[2], "beforeId": ids[0] }),
        )
        .await;

    assert_eq!(answer, json!({}));
    let moved = next_workspace(&mut other, &ids[2]).await;
    assert_eq!(moved["position"], 0);
    assert_eq!(
        by_position(&mut other, "workspaces.list", "workspaces").await,
        order(["beta", "zeta", "alpha"])
    );
    client
        .ok("workspaces.reorder", json!({ "workspaceId": ids[2] }))
        .await;
    assert_eq!(
        by_position(&mut client, "workspaces.list", "workspaces").await,
        order(["zeta", "alpha", "beta"])
    );
}

// spec: DMN-282, DMN-284, PRT-171, PRT-172
#[tokio::test(flavor = "multi_thread")]
async fn threads_are_reordered_without_looking_updated() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let workspace = client
        .ok("workspaces.create", json!({ "name": "shop" }))
        .await["id"]
        .clone();
    let first = thread_in(&mut client, "first", &workspace).await;
    let second = thread_in(&mut client, "second", &workspace).await;
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    let updated_at = |index: &Value| {
        index["state"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|thread| thread["updatedAt"].clone())
            .collect::<Vec<_>>()
    };
    let before = updated_at(&index);

    client
        .ok(
            "threads.reorder",
            json!({ "threadId": second, "beforeId": first }),
        )
        .await;

    let moved = next_upsert(&mut client, &second).await;
    assert_eq!(moved["position"], 0);
    assert_eq!(
        by_position(&mut client, "threads.list", "threads").await,
        [("second".to_owned(), 0), ("first".to_owned(), 1)]
    );
    // A new place is not news: nothing turns unread (UI-020).
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(updated_at(&index), before);
}

// spec: DMN-283, PRT-171
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_moved_to_another_workspace_restarts_its_omp_there() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let shop = client
        .ok("workspaces.create", json!({ "name": "shop" }))
        .await;
    let docs = client
        .ok("workspaces.create", json!({ "name": "docs" }))
        .await;
    let running = thread_in(&mut client, "running", &shop["id"]).await;
    let stopped = thread_in(&mut client, "stopped", &shop["id"]).await;
    let there = thread_in(&mut client, "there", &docs["id"]).await;
    client
        .ok("thread.state", json!({ "threadId": running }))
        .await;
    assert_eq!(daemon.launcher.launches().len(), 1);

    client
        .ok(
            "threads.reorder",
            json!({ "threadId": running, "workspaceId": docs["id"], "beforeId": there }),
        )
        .await;

    let launches = daemon.launcher.launches();
    assert_eq!(launches.len(), 2, "its omp started again");
    assert!(daemon.launcher.process(0).is_closed());
    assert_eq!(
        launches[1].spec.cwd.to_str(),
        docs["path"].as_str(),
        "in the new workspace's folder"
    );
    // A stopped thread just moves: it starts there next time.
    client
        .ok(
            "threads.reorder",
            json!({ "threadId": stopped, "workspaceId": docs["id"] }),
        )
        .await;
    assert_eq!(daemon.launcher.launches().len(), 2);
    let listed = client
        .ok("threads.list", json!({ "workspaceId": docs["id"] }))
        .await;
    let mut placed: Vec<(String, u64)> = listed["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|thread| {
            (
                thread["title"].as_str().unwrap().to_owned(),
                thread["position"].as_u64().unwrap(),
            )
        })
        .collect();
    placed.sort_by_key(|(_, position)| *position);
    assert_eq!(
        placed,
        [
            ("running".to_owned(), 0),
            ("there".to_owned(), 1),
            ("stopped".to_owned(), 2)
        ]
    );
}

// spec: DMN-283
#[tokio::test(flavor = "multi_thread")]
async fn a_working_thread_is_not_moved_to_another_workspace() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let shop = client
        .ok("workspaces.create", json!({ "name": "shop" }))
        .await["id"]
        .clone();
    let docs = client
        .ok("workspaces.create", json!({ "name": "docs" }))
        .await["id"]
        .clone();
    let thread = thread_in(&mut client, "busy", &shop).await;
    let topic = format!("thread/{thread}");
    client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    client.ok("subscribe", json!({ "topic": topic })).await;
    let event: omp_rpc::AgentStartEvent =
        serde_json::from_value(json!({ "type": "agent_start" })).unwrap();
    daemon
        .launcher
        .process(0)
        .emit(vec![RpcNotification::RpcAgentEvent(
            RpcAgentEvent::AgentStart(event),
        )]);
    loop {
        let event = client.event(&topic).await;
        if event["event"].to_string().contains("agent_start") {
            break;
        }
    }

    let refused = client
        .call(
            "threads.reorder",
            json!({ "threadId": thread, "workspaceId": docs }),
        )
        .await
        .unwrap_err();

    assert_eq!(refused["code"], "conflict", "{refused}");
    assert_eq!(daemon.launcher.launches().len(), 1);
    let listed = client
        .ok("threads.list", json!({ "workspaceId": shop }))
        .await;
    assert_eq!(listed["threads"][0]["id"], thread.as_str());
    // Within its workspace it still reorders.
    client
        .ok("threads.reorder", json!({ "threadId": thread }))
        .await;
}

// spec: DMN-285
#[tokio::test(flavor = "multi_thread")]
async fn a_reorder_before_what_another_window_moved_away_is_a_conflict() {
    let daemon = daemon(&[]).await;
    let mut window = daemon.client("window").await;
    let mut other = daemon.client("other window").await;
    let shop = window
        .ok("workspaces.create", json!({ "name": "shop" }))
        .await["id"]
        .clone();
    let docs = window
        .ok("workspaces.create", json!({ "name": "docs" }))
        .await["id"]
        .clone();
    let first = thread_in(&mut window, "first", &shop).await;
    let second = thread_in(&mut window, "second", &shop).await;

    // The other window moved `first` meanwhile; this window still drops `second` before it.
    other
        .ok(
            "threads.reorder",
            json!({ "threadId": first, "workspaceId": docs }),
        )
        .await;
    let stale = window
        .call(
            "threads.reorder",
            json!({ "threadId": second, "beforeId": first }),
        )
        .await
        .unwrap_err();
    assert_eq!(stale["code"], "conflict", "{stale}");
    let gone = window
        .call(
            "workspaces.reorder",
            json!({ "workspaceId": shop, "beforeId": "01NOWORKSPACE" }),
        )
        .await
        .unwrap_err();
    assert_eq!(gone["code"], "conflict", "{gone}");
    for (method, params) in [
        (
            "threads.reorder",
            json!({ "threadId": second, "beforeId": second }),
        ),
        (
            "workspaces.reorder",
            json!({ "workspaceId": shop, "beforeId": shop }),
        ),
    ] {
        let itself = window.call(method, params).await.unwrap_err();
        assert_eq!(itself["code"], "bad_request", "{itself}");
    }
    // Nothing changed: `second` is still alone in its workspace.
    let listed = window
        .ok("threads.list", json!({ "workspaceId": shop }))
        .await;
    assert_eq!(listed["threads"].as_array().unwrap().len(), 1);
    assert_eq!(listed["threads"][0]["id"], second.as_str());
    assert_eq!(listed["threads"][0]["position"], 0);
}

// spec: DMN-283, DMN-284
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_moved_to_another_workspace_keeps_its_updated_at() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let shop = client
        .ok("workspaces.create", json!({ "name": "shop" }))
        .await["id"]
        .clone();
    let docs = client
        .ok("workspaces.create", json!({ "name": "docs" }))
        .await["id"]
        .clone();
    let thread = thread_in(&mut client, "moved", &shop).await;
    let updated_at = async |client: &mut Client| {
        client.ok("threads.list", json!({})).await["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|summary| summary["id"] == thread.as_str())
            .map(|summary| (summary["workspaceId"].clone(), summary["updatedAt"].clone()))
            .unwrap()
    };
    let (_, before) = updated_at(&mut client).await;

    client
        .ok(
            "threads.reorder",
            json!({ "threadId": thread, "workspaceId": docs }),
        )
        .await;

    // The user moved it: not news, so no window marks it unread (UI-020).
    assert_eq!(updated_at(&mut client).await, (docs, before));
}

/// A started thread titled "Fix the cart" whose omp session file holds a user message `u1` and its answer `a1`: its
/// id and that file.
async fn thread_with_session(
    daemon: &TestDaemon,
    client: &mut Client,
) -> (String, std::path::PathBuf) {
    let created = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "Fix the cart", "model": "fake/scripted", "approvalMode": "write" }),
        )
        .await;
    let thread = created["id"].as_str().unwrap().to_owned();
    client
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": "hello" }),
        )
        .await;
    let session = client
        .ok("thread.session", json!({ "threadId": thread }))
        .await;
    let file = std::path::absolute(
        daemon
            .dir
            .path()
            .join("threads")
            .join(&thread)
            .join("sessions")
            .join(format!(
                "2026-11-14T10-00-00-000Z_{}.jsonl",
                session["sessionId"].as_str().unwrap()
            )),
    )
    .unwrap();
    let entries = [
        json!({ "type": "session", "id": session["sessionId"] }),
        json!({ "type": "message", "id": "u1", "parentId": null,
                "message": { "role": "user", "content": "Fix the cart\nand test it", "timestamp": 1 } }),
        json!({ "type": "message", "id": "a1", "parentId": "u1",
                "message": { "role": "assistant", "content": [{ "type": "text", "text": "Done" }], "timestamp": 2 } }),
    ];
    let text: Vec<String> = entries.iter().map(Value::to_string).collect();
    std::fs::write(&file, text.join("\n")).unwrap();
    (thread, file)
}

// spec: DMN-300, DMN-306, PRT-180
#[tokio::test(flavor = "multi_thread")]
async fn threads_fork_makes_a_new_thread_on_a_copy_of_the_session_and_leaves_the_source_alone() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let (source, file) = thread_with_session(&daemon, &mut client).await;
    let source_calls = daemon.launcher.process(0).calls().len();

    let fork = client
        .ok(
            "threads.fork",
            json!({ "threadId": source, "entryId": "u1" }),
        )
        .await;

    let id = fork["id"].as_str().unwrap().to_owned();
    assert_ne!(id, source);
    assert_eq!(
        (&fork["title"], &fork["titleSource"]),
        (&json!("Fix the cart (fork)"), &json!("placeholder"))
    );
    assert_eq!(
        fork["forkedFrom"],
        json!({ "threadId": source, "title": "Fix the cart", "entryId": "u1", "message": "Fix the cart" })
    );
    let listed = client.ok("threads.list", json!({})).await;
    let threads = listed["threads"].as_array().unwrap();
    let source_summary = threads.iter().find(|t| t["id"] == source).unwrap();
    assert_eq!(fork["workspaceId"], source_summary["workspaceId"]);
    assert_eq!(source_summary.get("forkedFrom"), None);
    // The fork runs its own omp with the source's settings, started on the copy and cut at the entry.
    let launch = daemon.launcher.launches().pop().unwrap();
    assert_eq!(launch.spec.fork.as_deref(), Some(file.as_path()));
    assert_eq!(
        (
            launch.spec.model.as_deref(),
            launch.spec.approval_mode.as_deref()
        ),
        (Some("fake/scripted"), Some("write"))
    );
    assert!(
        launch.process.calls().iter().any(|call| matches!(
            call,
            omfp_daemon::agent::FakeCall::Call(omfp_daemon::agent::OmpCall::Fork(fork))
                if fork.entry_id.as_deref() == Some("u1")
        )),
        "{:?}",
        launch.process.calls()
    );
    // The source's omp got nothing, its session file is unchanged.
    assert_eq!(daemon.launcher.process(0).calls().len(), source_calls);
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"a1\""));

    // The whole session, without an entry: no message in the lineage.
    let whole = client
        .ok("threads.fork", json!({ "threadId": source }))
        .await;
    assert_eq!(
        whole["forkedFrom"],
        json!({ "threadId": source, "title": "Fix the cart" })
    );
}

// spec: DMN-303, DMN-305, PRT-182
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_that_cannot_be_made_leaves_nothing_and_a_deleted_source_keeps_its_forks_lineage() {
    let daemon = daemon(&[]).await;
    let mut client = daemon.client("test").await;
    let (source, _) = thread_with_session(&daemon, &mut client).await;
    let fresh = create_thread(&mut client).await;
    let before = listed(&mut client, "threads.list", "threads").await;

    let error = client
        .call("threads.fork", json!({ "threadId": fresh }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "bad_request", "{error}");
    let error = client
        .call(
            "threads.fork",
            json!({ "threadId": source, "entryId": "nope" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found", "{error}");
    daemon.launcher.fail_next_launch("fatal: out of memory\n");
    let error = client
        .call(
            "threads.fork",
            json!({ "threadId": source, "entryId": "a1" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "thread_failed", "{error}");
    assert_eq!(listed(&mut client, "threads.list", "threads").await, before);
    let thread_dirs = std::fs::read_dir(daemon.dir.path().join("threads"))
        .unwrap()
        .count();
    assert_eq!(thread_dirs, before.len());

    let fork = client
        .ok(
            "threads.fork",
            json!({ "threadId": source, "entryId": "a1" }),
        )
        .await;
    assert_eq!(fork["forkedFrom"]["message"], "Done");
    client
        .ok("threads.delete", json!({ "threadId": source }))
        .await;
    let listed = client.ok("threads.list", json!({})).await;
    let kept = listed["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == fork["id"])
        .unwrap()
        .clone();
    assert_eq!(kept["forkedFrom"], fork["forkedFrom"]);
}

/// A stand-in for omp's OAuth callback listener on `127.0.0.1:0`: answers each connection with the next of `statuses`
/// and hands over the request line it received.
async fn callback_listener(
    statuses: &'static [u16],
) -> (u16, tokio::sync::mpsc::UnboundedReceiver<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (requests, received) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        for status in statuses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|end| end == b"\r\n\r\n") {
                let read = socket.read(&mut buffer).await.unwrap();
                assert_ne!(read, 0, "the request ends early");
                request.extend_from_slice(&buffer[..read]);
            }
            let text = String::from_utf8(request).unwrap();
            requests
                .send(text.lines().next().unwrap().to_owned())
                .unwrap();
            let reply =
                format!("HTTP/1.1 {status} X\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            socket.write_all(reply.as_bytes()).await.unwrap();
        }
    });
    (port, received)
}

/// An authorization URL whose `redirect_uri` is omp's listener on `port`, path `/callback`.
fn authorization_url(port: u16) -> String {
    format!(
        "https://auth.example.com/authorize?client_id=omp&redirect_uri=http%3A%2F%2Flocalhost%3A{port}%2Fcallback\
         &state=s1"
    )
}

// spec: PRT-212, DMN-340, DMN-341, DMN-342
#[tokio::test(flavor = "multi_thread")]
async fn a_pasted_callback_of_a_threads_mcp_reauth_reaches_omps_loopback_listener_once() {
    let daemon = daemon(&[]).await;
    let mut window = daemon.client("test").await;
    let thread = create_thread(&mut window).await;
    window
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    let mut follower = Follower::snapshot(&mut window, &thread, None).await;
    let (port, mut requests) = callback_listener(&[500, 200]).await;
    let pasted = format!("http://localhost:{port}/callback?code=c0de&state=s1#ignored");
    let relay = json!({ "threadId": thread, "url": pasted });

    let error = window
        .call("oauth.callback", relay.clone())
        .await
        .unwrap_err();
    assert_eq!(error["code"], "conflict", "nothing pending yet: {error}");

    daemon
        .launcher
        .process(0)
        .emit(vec![RpcNotification::CommandOutput(
            omfp_core::wire::CommandOutputEvent {
                text: format!(
                    "━━━ OAuth Authorization Required ━━━\nOpening your browser to authorize. If it did not \
                     open:\n{}",
                    authorization_url(port)
                ),
            },
        )]);
    follower
        .until(&mut window, |state| !state.notices.is_empty())
        .await;

    for other in [
        format!("http://localhost:{port}/other?code=c0de"),
        format!("http://example.com:{port}/callback?code=c0de"),
        format!("http://user@127.0.0.1:{port}/callback?code=c0de"),
        format!("https://localhost:{port}/callback?code=c0de"),
    ] {
        let error = window
            .call(
                "oauth.callback",
                json!({ "threadId": thread, "url": other }),
            )
            .await
            .unwrap_err();
        assert_eq!(error["code"], "bad_request", "{other}: {error}");
    }

    // omp's listener refused it (a state mismatch answers 500): the sign-in still waits.
    let error = window
        .call("oauth.callback", relay.clone())
        .await
        .unwrap_err();
    assert_eq!(error["code"], "omp_error", "{error}");
    assert_eq!(error["details"]["status"], 500, "{error}");
    assert_eq!(
        requests.recv().await.unwrap(),
        "GET /callback?code=c0de&state=s1 HTTP/1.1"
    );

    assert_eq!(
        window.ok("oauth.callback", relay.clone()).await,
        json!({ "status": 200 })
    );
    assert_eq!(
        requests.recv().await.unwrap(),
        "GET /callback?code=c0de&state=s1 HTTP/1.1"
    );

    // Completed: it waits no more.
    let error = window.call("oauth.callback", relay).await.unwrap_err();
    assert_eq!(error["code"], "conflict", "{error}");

    let error = window
        .call(
            "oauth.callback",
            json!({ "threadId": "nope", "url": pasted }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found", "{error}");
}

// spec: PRT-212, DMN-340
#[tokio::test(flavor = "multi_thread")]
async fn a_pasted_callback_without_a_thread_completes_the_running_login_and_none_after_it() {
    use omfp_core::wire::{ExtensionUiRequest, OpenUrlUiRequest};
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let gate = daemon.launcher.gate("login");
    let mut window = daemon.client("test").await;
    let (port, mut requests) = callback_listener(&[200]).await;
    let relay = json!({ "url": format!("http://127.0.0.1:{port}/callback?code=c0de&state=s1") });

    send_request(
        &mut window,
        "l1",
        "auth.login",
        json!({ "providerId": "anthropic" }),
    )
    .await;
    gate.arrived(1).await;
    daemon
        .launcher
        .process(0)
        .emit(vec![RpcNotification::ExtensionUiRequest(
            ExtensionUiRequest::OpenUrl(OpenUrlUiRequest {
                id: "u1".into(),
                url: authorization_url(port),
                launch_url: Some(format!("http://localhost:{port}/launch")),
                instructions: None,
            }),
        )]);
    let (_, url) = next_window_request(&mut window).await;
    assert_eq!(url["kind"], "login_url");

    assert_eq!(
        window.ok("oauth.callback", relay.clone()).await,
        json!({ "status": 200 })
    );
    assert_eq!(
        requests.recv().await.unwrap(),
        "GET /callback?code=c0de&state=s1 HTTP/1.1"
    );

    window.ok("auth.login_cancel", json!({})).await;
    let error = window.call("oauth.callback", relay).await.unwrap_err();
    assert_eq!(error["code"], "conflict", "{error}");
    gate.open();
}

// spec: DMN-340
#[tokio::test(flavor = "multi_thread")]
async fn a_login_omp_that_exits_unexpectedly_leaves_no_sign_in_waiting() {
    use omfp_core::wire::{ExtensionUiRequest, OpenUrlUiRequest};
    let daemon = daemon(&[]).await;
    daemon
        .launcher
        .reply("get_login_providers", login_providers());
    let gate = daemon.launcher.gate("login");
    let mut window = daemon.client("test").await;
    // Nothing listens on the port any more: before the guard forgot it, a relay would reach whatever took it.
    let (port, _) = callback_listener(&[]).await;
    let relay = json!({ "url": format!("http://localhost:{port}/callback?code=c0de&state=s1") });

    send_request(
        &mut window,
        "l1",
        "auth.login",
        json!({ "providerId": "anthropic" }),
    )
    .await;
    gate.arrived(1).await;
    let omp = daemon.launcher.process(0);
    omp.emit(vec![RpcNotification::ExtensionUiRequest(
        ExtensionUiRequest::OpenUrl(OpenUrlUiRequest {
            id: "u1".into(),
            url: authorization_url(port),
            launch_url: None,
            instructions: None,
        }),
    )]);
    next_window_request(&mut window).await;

    // The login omp crashes while omp's `login` call is still out.
    omp.exit();
    let forgotten = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let error = window
                .call("oauth.callback", relay.clone())
                .await
                .unwrap_err();
            if error["code"] == "conflict" {
                break error;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    // Released first: a blocked `login` call would hold the runtime's shutdown if the assertion fails.
    gate.open();
    let error = forgotten.expect("the crashed login's sign-in is forgotten");
    assert_eq!(error["message"], "no login waits for its callback");
}
