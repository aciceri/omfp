//! Integration tests (L2) of jobs (DMN-080, DMN-081, PRT-041) over a real WebSocket: a daemon on port 0 with a
//! temporary data directory, real `sh` processes in a workspace folder, and a native protocol client. Jobs that must
//! keep running wait for a gate file the test never creates, so nothing depends on timing.

mod client;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use client::Client;
use omfp_daemon::agent::FakeLauncher;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use serde_json::{Value, json};
use tempfile::TempDir;

struct TestDaemon {
    handle: DaemonHandle,
    dir: TempDir,
}

fn config(dir: &Path) -> Config {
    let workspaces = dir.join("workspaces");
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        dir.to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
    ])
    .unwrap();
    Config::from_cli(cli, None, None).unwrap()
}

async fn daemon() -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let ports = Ports {
        launcher: Arc::new(FakeLauncher::new()),
        git: Arc::new(FakeGit::new()),
        omp_program: "omp".into(),
        omp_version: omfp_core::OMP_VERSION.into(),
        omp_env: Vec::new(),
        warnings: Vec::new(),
        clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
    };
    let handle = start_with(config(dir.path()), ports).await.unwrap();
    TestDaemon { handle, dir }
}

impl TestDaemon {
    async fn client(&self, name: &str) -> Client {
        Client::hello(self.handle.addr(), self.handle.token().expose(), name).await
    }

    /// A path in the test's directory, outside every workspace.
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A shell loop running until `gate` exists.
    fn hold(&self, gate: &str) -> String {
        format!(
            "while [ ! -e '{}' ]; do sleep 0.01; done",
            self.path(gate).display()
        )
    }
}

/// A new workspace: its id and folder.
async fn workspace(client: &mut Client) -> (String, PathBuf) {
    let workspace = client.ok("workspaces.create", json!({})).await;
    (
        workspace["id"].as_str().unwrap().to_owned(),
        PathBuf::from(workspace["path"].as_str().unwrap()),
    )
}

async fn start(client: &mut Client, workspace_id: &str, command: &str) -> Value {
    client
        .ok(
            "jobs.start",
            json!({ "workspaceId": workspace_id, "command": command }),
        )
        .await
}

/// A job followed from its snapshot, events applied as the client side of PRT-041 does.
struct Followed {
    topic: String,
    epoch: String,
    job: Value,
    lines: Vec<(String, String)>,
    dropped: u64,
    /// Events received after the snapshot, in order.
    events: Vec<Value>,
}

impl Followed {
    async fn subscribe(client: &mut Client, job_id: &str) -> Self {
        let topic = format!("job/{job_id}");
        let result = client.ok("subscribe", json!({ "topic": topic })).await;
        assert_eq!(result["mode"], "snapshot");
        let state = &result["state"];
        Self {
            topic,
            epoch: result["epoch"].as_str().unwrap().to_owned(),
            job: state["job"].clone(),
            lines: state["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| {
                    (
                        line["stream"].as_str().unwrap().to_owned(),
                        line["text"].as_str().unwrap().to_owned(),
                    )
                })
                .collect(),
            dropped: state["dropped"].as_u64().unwrap(),
            events: Vec::new(),
        }
    }

    async fn next(&mut self, client: &mut Client) {
        let frame = client.event(&self.topic).await;
        let event = frame["event"].clone();
        match event["kind"].as_str().unwrap() {
            "job_output" => self.lines.push((
                event["stream"].as_str().unwrap().to_owned(),
                event["text"].as_str().unwrap().to_owned(),
            )),
            "job_exited" => {
                self.job["status"] = json!({
                    "state": "exited",
                    "code": event["code"],
                    "signal": event["signal"],
                    "endedAt": event["endedAt"],
                });
            }
            kind => panic!("unexpected job event {kind}"),
        }
        self.events.push(frame);
    }

    fn has_line(&self, text: &str) -> bool {
        self.lines.iter().any(|(_, line)| line == text)
    }

    async fn until_line(&mut self, client: &mut Client, text: &str) {
        while !self.has_line(text) {
            self.next(client).await;
        }
    }

    /// The final status.
    async fn until_exited(&mut self, client: &mut Client) -> Value {
        while self.job["status"]["state"] == "running" {
            self.next(client).await;
        }
        self.job["status"].clone()
    }

    fn texts(&self, stream: &str) -> Vec<&str> {
        self.lines
            .iter()
            .filter(|(from, _)| from == stream)
            .map(|(_, text)| text.as_str())
            .collect()
    }
}

// spec: DMN-080, PRT-041
#[tokio::test(flavor = "multi_thread")]
async fn a_job_publishes_its_lines_and_exit_then_replays_them_in_order() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, folder) = workspace(&mut client).await;
    let command = r"pwd; if read line; then echo stdin; else echo no-stdin; fi; printf 'one\ntwo\r\nbad \377\n'; echo oops >&2; printf last";

    let job = start(&mut client, &workspace_id, command).await;
    assert_eq!(job["workspaceId"], json!(workspace_id));
    assert_eq!(job["command"], command);
    assert_eq!(job["status"], json!({ "state": "running" }));
    assert!(job["startedAt"].as_i64().unwrap() > 0);
    let job_id = job["id"].as_str().unwrap();
    let mut followed = Followed::subscribe(&mut client, job_id).await;
    let status = followed.until_exited(&mut client).await;

    assert_eq!(status["code"], 0);
    assert_eq!(status["signal"], Value::Null);
    assert!(status["endedAt"].as_i64().unwrap() >= job["startedAt"].as_i64().unwrap());
    let folder = folder.display().to_string();
    assert_eq!(
        followed.texts("stdout"),
        [
            folder.as_str(),
            "no-stdin",
            "one",
            "two",
            "bad \u{FFFD}",
            "last"
        ]
    );
    assert_eq!(followed.texts("stderr"), ["oops"]);
    assert_eq!(followed.dropped, 0);

    // Another client catches up from the start: every event, in order, the exit last.
    let mut other = daemon.client("other").await;
    let topic = format!("job/{job_id}");
    let replay = other
        .ok(
            "subscribe",
            json!({ "topic": topic, "since": { "epoch": followed.epoch, "seq": 0 } }),
        )
        .await;
    assert_eq!(
        replay,
        json!({ "mode": "replay", "epoch": followed.epoch, "fromSeq": 1 })
    );
    let count = followed.lines.len() + 1;
    let mut replayed = Vec::new();
    for _ in 0..count {
        replayed.push(other.event(&topic).await);
    }
    let seqs: Vec<u64> = replayed
        .iter()
        .map(|frame| frame["seq"].as_u64().unwrap())
        .collect();
    let last = u64::try_from(count).unwrap();
    assert_eq!(seqs, (1..=last).collect::<Vec<_>>());
    let texts: Vec<&Value> = replayed
        .iter()
        .map(|frame| &frame["event"]["text"])
        .collect();
    let expected: Vec<Value> = followed
        .lines
        .iter()
        .map(|(_, text)| json!(text))
        .chain([Value::Null])
        .collect();
    assert_eq!(texts, expected.iter().collect::<Vec<_>>());
    assert_eq!(replayed[count - 1]["event"]["kind"], "job_exited");

    // A snapshot holds the same lines and the final status.
    let snapshot = Followed::subscribe(&mut daemon.client("late").await, job_id).await;
    assert_eq!(snapshot.lines, followed.lines);
    assert_eq!(snapshot.job["status"], status);
}

// spec: DMN-080
#[tokio::test(flavor = "multi_thread")]
async fn a_line_longer_than_64_kib_arrives_in_pieces() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, _) = workspace(&mut client).await;

    let job = start(
        &mut client,
        &workspace_id,
        r"head -c 70000 /dev/zero | tr '\0' a",
    )
    .await;
    let mut followed = Followed::subscribe(&mut client, job["id"].as_str().unwrap()).await;
    followed.until_exited(&mut client).await;

    let lengths: Vec<usize> = followed
        .texts("stdout")
        .iter()
        .map(|text| text.len())
        .collect();
    assert_eq!(lengths, [64 * 1024, 70000 - 64 * 1024]);
}

// spec: DMN-080, PRT-041
#[tokio::test(flavor = "multi_thread")]
async fn the_exit_reports_the_code_or_the_signal() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, _) = workspace(&mut client).await;

    for (command, code, signal) in [
        ("exit 3", json!(3), Value::Null),
        ("kill -9 $$", Value::Null, json!(9)),
    ] {
        let job = start(&mut client, &workspace_id, command).await;
        let mut followed = Followed::subscribe(&mut client, job["id"].as_str().unwrap()).await;
        let status = followed.until_exited(&mut client).await;
        assert_eq!(
            (&status["code"], &status["signal"]),
            (&code, &signal),
            "{command}"
        );
    }
}

// spec: DMN-080
#[tokio::test(flavor = "multi_thread")]
async fn a_job_exits_only_once_both_outputs_closed() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, _) = workspace(&mut client).await;
    // The shell exits at once; a background process keeps stdout open until the gate opens.
    let command = format!("({}; echo late) & echo started", daemon.hold("gate"));

    let job = start(&mut client, &workspace_id, &command).await;
    let mut followed = Followed::subscribe(&mut client, job["id"].as_str().unwrap()).await;
    followed.until_line(&mut client, "started").await;
    assert_eq!(followed.job["status"]["state"], "running");
    std::fs::write(daemon.path("gate"), "").unwrap();
    followed.until_exited(&mut client).await;

    assert_eq!(followed.texts("stdout"), ["started", "late"]);
}

// spec: DMN-081
#[tokio::test(flavor = "multi_thread")]
async fn a_job_outlives_the_client_that_started_it_and_every_client_sees_it() {
    let daemon = daemon().await;
    let mut starter = daemon.client("starter").await;
    let (workspace_id, _) = workspace(&mut starter).await;
    let command = format!("{}; echo done", daemon.hold("gate"));
    let job = start(&mut starter, &workspace_id, &command).await;
    drop(starter);

    let mut watcher = daemon.client("watcher").await;
    let mut followed = Followed::subscribe(&mut watcher, job["id"].as_str().unwrap()).await;
    assert_eq!(followed.job["status"]["state"], "running");
    std::fs::write(daemon.path("gate"), "").unwrap();

    assert_eq!(followed.until_exited(&mut watcher).await["code"], 0);
    assert_eq!(followed.texts("stdout"), ["done"]);
}

// spec: DMN-081
#[tokio::test(flavor = "multi_thread")]
async fn kill_terminates_the_whole_process_group() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, _) = workspace(&mut client).await;
    // A background member of the group holds stdout: the job finishes only if it dies too.
    let command = format!("({}) & echo ready; wait", daemon.hold("gate"));
    let job = start(&mut client, &workspace_id, &command).await;
    let job_id = job["id"].as_str().unwrap();
    let mut followed = Followed::subscribe(&mut client, job_id).await;
    followed.until_line(&mut client, "ready").await;

    assert_eq!(
        client.ok("jobs.kill", json!({ "jobId": job_id })).await,
        json!({})
    );
    let status = followed.until_exited(&mut client).await;

    assert_eq!(
        (&status["code"], &status["signal"]),
        (&Value::Null, &json!(15))
    );
    // Killing a finished job does nothing.
    assert_eq!(
        client.ok("jobs.kill", json!({ "jobId": job_id })).await,
        json!({})
    );
}

// spec: DMN-081
#[tokio::test(flavor = "multi_thread")]
async fn kill_escalates_to_sigkill_when_the_shell_survives_sigterm() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, _) = workspace(&mut client).await;
    let command = format!("trap 'echo term' TERM; echo ready; {}", daemon.hold("gate"));
    let job = start(&mut client, &workspace_id, &command).await;
    let job_id = job["id"].as_str().unwrap();
    let mut followed = Followed::subscribe(&mut client, job_id).await;
    followed.until_line(&mut client, "ready").await;

    client.ok("jobs.kill", json!({ "jobId": job_id })).await;
    followed.until_line(&mut client, "term").await;
    let status = followed.until_exited(&mut client).await;

    assert_eq!(
        (&status["code"], &status["signal"]),
        (&Value::Null, &json!(9))
    );
}

// spec: DMN-080, DMN-081, PRT-041
#[tokio::test(flavor = "multi_thread")]
async fn unknown_workspaces_jobs_and_empty_commands_are_refused() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, _) = workspace(&mut client).await;

    let error = client
        .call(
            "jobs.start",
            json!({ "workspaceId": "nope", "command": "true" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");
    let error = client
        .call(
            "jobs.start",
            json!({ "workspaceId": workspace_id, "command": " \n" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "bad_request");
    let error = client
        .call("jobs.kill", json!({ "jobId": "nope" }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");
    let error = client
        .call("subscribe", json!({ "topic": "job/nope" }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");
}

// spec: DMN-081
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_workspace_kills_its_jobs_before_deleting_the_folder_and_forgets_them() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, folder) = workspace(&mut client).await;
    let command = format!(
        r#"trap 'test -d "$PWD" && echo folder-present; exit 0' TERM; echo ready; {}"#,
        daemon.hold("gate")
    );
    let job = start(&mut client, &workspace_id, &command).await;
    let job_id = job["id"].as_str().unwrap();
    let mut followed = Followed::subscribe(&mut client, job_id).await;
    followed.until_line(&mut client, "ready").await;

    client
        .ok("workspaces.remove", json!({ "workspaceId": workspace_id }))
        .await;

    assert!(!folder.exists());
    let status = followed.until_exited(&mut client).await;
    assert_eq!(status["code"], 0);
    assert!(followed.has_line("folder-present"));
    let error = client
        .call("jobs.kill", json!({ "jobId": job_id }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");
    let error = client
        .call("subscribe", json!({ "topic": format!("job/{job_id}") }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");
}

// spec: DMN-081
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_workspace_removal_keeps_its_jobs_running() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, folder) = workspace(&mut client).await;
    client
        .ok(
            "threads.create",
            json!({ "title": "stays", "workspace": { "id": workspace_id } }),
        )
        .await;
    let command = format!(
        "trap 'echo term' TERM; echo ready; {}; echo done",
        daemon.hold("gate")
    );
    let job = start(&mut client, &workspace_id, &command).await;
    let mut followed = Followed::subscribe(&mut client, job["id"].as_str().unwrap()).await;
    followed.until_line(&mut client, "ready").await;

    client
        .call("workspaces.remove", json!({ "workspaceId": workspace_id }))
        .await
        .unwrap_err();

    assert!(folder.is_dir());
    std::fs::write(daemon.path("gate"), "").unwrap();
    assert_eq!(followed.until_exited(&mut client).await["code"], 0);
    assert_eq!(followed.texts("stdout"), ["ready", "done"]);
}

// spec: DMN-081
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_kills_every_running_job() {
    let daemon = daemon().await;
    let mut client = daemon.client("test").await;
    let (workspace_id, _) = workspace(&mut client).await;
    let marker = daemon.path("stopped");
    let command = format!(
        "trap 'echo stopped > {}; exit 0' TERM; echo ready; {}",
        marker.display(),
        daemon.hold("gate")
    );
    let job = start(&mut client, &workspace_id, &command).await;
    let mut followed = Followed::subscribe(&mut client, job["id"].as_str().unwrap()).await;
    followed.until_line(&mut client, "ready").await;

    // The temporary directory outlives the shutdown.
    let TestDaemon { handle, dir: _dir } = daemon;
    handle.shutdown().await.unwrap();

    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "stopped\n");
}
