//! Integration tests (L2) of declared jobs (DMN-320..DMN-336, PRT-200..PRT-210) over a real WebSocket: a daemon on port
//! 0 with a temporary data directory, real `sh` processes in a workspace folder, a fake omp, and a native protocol
//! client. Runs that must keep running wait for a gate file the test never creates, so nothing depends on timing.

mod client;

use std::path::{Path, PathBuf};

use client::{Client, TestDaemon};
use omfp_daemon::agent::FakeRun;
use serde_json::{Value, json};

fn fixture(name: &str) -> FakeRun {
    FakeRun::from_fixture(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/threads")
            .join(omfp_core::OMP_VERSION)
            .join(format!("{name}.jsonl")),
    )
    .unwrap()
}

/// A new workspace: its id and folder.
async fn workspace(client: &mut Client) -> (String, PathBuf) {
    let workspace = client.ok("workspaces.create", json!({})).await;
    (
        workspace["id"].as_str().unwrap().to_owned(),
        PathBuf::from(workspace["path"].as_str().unwrap()),
    )
}

async fn define(client: &mut Client, workspace_id: &str, name: &str, job: Value) -> Value {
    client
        .ok(
            "jobs.define",
            json!({ "workspaceId": workspace_id, "name": name, "job": job }),
        )
        .await
}

async fn run(client: &mut Client, workspace_id: &str, job: &str) -> Result<Value, Value> {
    client
        .call(
            "jobs.run",
            json!({ "workspaceId": workspace_id, "job": job }),
        )
        .await
}

/// `jobs/<id>` followed from its snapshot.
struct Jobs {
    topic: String,
    state: Value,
}

impl Jobs {
    async fn subscribe(client: &mut Client, workspace_id: &str) -> Self {
        let topic = format!("jobs/{workspace_id}");
        let result = client.ok("subscribe", json!({ "topic": topic })).await;
        assert_eq!(result["mode"], "snapshot");
        Self {
            topic,
            state: result["state"].clone(),
        }
    }

    /// The next event, applied as the client reducer does (PRT-200).
    async fn next(&mut self, client: &mut Client) -> Value {
        let event = client.event(&self.topic).await["event"].clone();
        match event["kind"].as_str().unwrap() {
            "jobs_set" => {
                self.state = json!({ "jobs": event["jobs"], "error": event["error"] });
            }
            "job_updated" => {
                let jobs = self.state["jobs"].as_array_mut().unwrap();
                let name = &event["job"]["name"];
                match jobs.iter_mut().find(|job| job["name"] == *name) {
                    Some(job) => *job = event["job"].clone(),
                    None => jobs.push(event["job"].clone()),
                }
            }
            "run_started" | "run_ended" => {}
            kind => panic!("unexpected jobs event {kind}"),
        }
        event
    }

    /// Events until one of `kind` (returned); the earlier ones applied.
    async fn until_kind(&mut self, client: &mut Client, kind: &str) -> Value {
        loop {
            let event = self.next(client).await;
            if event["kind"] == kind {
                return event;
            }
        }
    }

    fn job(&self, name: &str) -> &Value {
        self.state["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["name"] == name)
            .unwrap_or_else(|| panic!("no job {name} in {}", self.state))
    }

    /// Events until the job's status is `status`.
    async fn until_status(&mut self, client: &mut Client, name: &str, status: &str) {
        while self.state["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["name"] == name)
            .is_none_or(|job| job["status"] != status)
        {
            self.next(client).await;
        }
    }
}

/// The lines of `job/<id>` until `text` arrived.
async fn until_line(client: &mut Client, run_id: &str, text: &str) -> Vec<String> {
    let topic = format!("job/{run_id}");
    let result = client.ok("subscribe", json!({ "topic": topic })).await;
    let mut lines: Vec<String> = result["state"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["text"].as_str().unwrap().to_owned())
        .collect();
    while !lines.iter().any(|line| line == text) {
        let event = client.event(&topic).await["event"].clone();
        if event["kind"] == "job_output" {
            lines.push(event["text"].as_str().unwrap().to_owned());
        }
    }
    lines
}

// spec: DMN-327, DMN-324, DMN-336, PRT-200, PRT-201, PRT-202, PRT-203, PRT-204, PRT-205
#[tokio::test(flavor = "multi_thread")]
async fn a_command_job_runs_in_its_cwd_once_at_a_time_and_is_killed_through_the_protocol() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let (ws, folder) = workspace(&mut client).await;
    std::fs::create_dir(folder.join("sub")).unwrap();
    let mut jobs = Jobs::subscribe(&mut client, &ws).await;
    assert_eq!(jobs.state, json!({ "jobs": [], "error": null }));

    let view = define(
        &mut client,
        &ws,
        "hold",
        json!({ "description": "Waits", "command": format!("pwd; echo ready; {}", daemon.hold("gate")), "cwd": "sub" }),
    )
    .await;

    assert_eq!(view["status"], "idle");
    assert_eq!(
        view["job"],
        json!({ "description": "Waits", "trigger": "manual", "missed": "skip",
                "command": format!("pwd; echo ready; {}", daemon.hold("gate")), "cwd": "sub",
                "restart": "always", "enabled": true })
    );
    assert_eq!(
        jobs.until_kind(&mut client, "jobs_set").await["jobs"][0],
        view
    );

    let started = run(&mut client, &ws, "hold").await.unwrap();
    assert_eq!(started["status"], "running");
    assert_eq!(started["trigger"], "manual");
    assert_eq!(started["job"], "hold");
    assert_eq!(started["workspaceId"], json!(ws));
    assert_eq!(started["startedBy"], json!({ "kind": "user" }));
    let run_id = started["id"].as_str().unwrap().to_owned();
    let event = jobs.next(&mut client).await;
    assert_eq!(event, json!({ "kind": "run_started", "run": started }));
    jobs.until_status(&mut client, "hold", "running").await;
    assert_eq!(jobs.job("hold")["run"], started);

    let error = run(&mut client, &ws, "hold").await.unwrap_err();
    assert_eq!(error["code"], "conflict");
    let lines = until_line(&mut client, &run_id, "ready").await;
    assert_eq!(
        lines[0],
        folder.join("sub").canonicalize().unwrap().to_str().unwrap()
    );

    client.ok("jobs.kill", json!({ "jobId": run_id })).await;

    let ended = jobs.until_kind(&mut client, "run_ended").await["run"].clone();
    assert_eq!(ended["status"], "killed");
    assert_eq!(ended["id"], json!(run_id));
    jobs.until_status(&mut client, "hold", "idle").await;
    assert_eq!(jobs.job("hold")["lastRun"], ended);
    assert_eq!(
        client.ok("jobs.list", json!({ "workspaceId": ws })).await,
        jobs.state
    );
    let runs = client
        .ok("jobs.runs", json!({ "workspaceId": ws, "job": "hold" }))
        .await;
    assert_eq!(runs, json!({ "runs": [ended], "more": false }));
}

// spec: PRT-204, PRT-205, PRT-206, PRT-207, DMN-332
#[tokio::test(flavor = "multi_thread")]
async fn job_requests_refuse_unknown_disabled_and_invalid_jobs() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let (ws, folder) = workspace(&mut client).await;
    define(&mut client, &ws, "build", json!({ "command": "true" })).await;

    let refusals = [
        (
            "jobs.run",
            json!({ "workspaceId": ws, "job": "nope" }),
            "not_found",
        ),
        (
            "jobs.remove",
            json!({ "workspaceId": ws, "name": "nope" }),
            "not_found",
        ),
        (
            "jobs.enable",
            json!({ "workspaceId": ws, "name": "nope", "enabled": false }),
            "not_found",
        ),
        (
            "jobs.define",
            json!({ "workspaceId": ws, "name": "build", "job": { "command": "x" } }),
            "conflict",
        ),
        (
            "jobs.define",
            json!({ "workspaceId": ws, "name": "Bad", "job": { "command": "x" } }),
            "bad_request",
        ),
        (
            "jobs.define",
            json!({ "workspaceId": ws, "name": "x", "job": { "command": "x", "every": 1 } }),
            "bad_request",
        ),
        ("jobs.list", json!({ "workspaceId": "nope" }), "not_found"),
        ("subscribe", json!({ "topic": "jobs/nope" }), "not_found"),
    ];
    for (method, params, code) in refusals {
        let error = client.call(method, params.clone()).await.unwrap_err();
        assert_eq!(error["code"], code, "{method} {params}: {error}");
    }

    let view = client
        .ok(
            "jobs.enable",
            json!({ "workspaceId": ws, "name": "build", "enabled": false }),
        )
        .await;
    assert_eq!(view["status"], "disabled");
    assert_eq!(
        run(&mut client, &ws, "build").await.unwrap_err()["code"],
        "conflict"
    );
    let view = client
        .ok(
            "jobs.define",
            json!({ "workspaceId": ws, "name": "build", "job": { "command": "make" }, "replace": true }),
        )
        .await;
    assert_eq!(view["job"]["command"], "make");
    assert_eq!(
        std::fs::read_to_string(folder.join(".omfp/jobs.json")).unwrap(),
        "{\n  \"jobs\": {\n    \"build\": {\n      \"command\": \"make\"\n    }\n  }\n}\n"
    );
    assert_eq!(
        client
            .ok("jobs.runs", json!({ "workspaceId": ws, "job": "never" }))
            .await,
        json!({ "runs": [], "more": false })
    );
}

// spec: DMN-322, DMN-326, PRT-200, PRT-204, PRT-206
#[tokio::test(flavor = "multi_thread")]
async fn a_declared_service_starts_with_its_file_stays_stopped_once_killed_and_goes_with_its_job() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let (ws, folder) = workspace(&mut client).await;
    std::fs::create_dir_all(folder.join(".omfp")).unwrap();
    std::fs::write(
        folder.join(".omfp/jobs.json"),
        json!({ "jobs": { "watch": { "trigger": "service", "command": daemon.hold("gate") } } })
            .to_string(),
    )
    .unwrap();
    client
        .ok("workspaces.rescan", json!({ "workspaceId": ws }))
        .await;
    let mut jobs = Jobs::subscribe(&mut client, &ws).await;
    jobs.until_status(&mut client, "watch", "running").await;
    let first = jobs.job("watch")["run"].clone();
    assert_eq!(first["trigger"], "service");
    assert_eq!(first["startedBy"], Value::Null);

    client
        .ok("jobs.kill", json!({ "jobId": first["id"] }))
        .await;

    jobs.until_status(&mut client, "watch", "stopped").await;
    assert_eq!(jobs.job("watch")["lastRun"]["status"], "killed");
    assert_eq!(jobs.job("watch")["nextAt"], Value::Null);

    let second = run(&mut client, &ws, "watch").await.unwrap();
    assert_eq!(second["trigger"], "manual");
    jobs.until_status(&mut client, "watch", "running").await;

    client
        .ok("jobs.remove", json!({ "workspaceId": ws, "name": "watch" }))
        .await;

    let ended = jobs.until_kind(&mut client, "run_ended").await["run"].clone();
    assert_eq!(
        (&ended["id"], &ended["status"]),
        (&second["id"], &json!("killed"))
    );
    assert_eq!(
        std::fs::read_to_string(folder.join(".omfp/jobs.json")).unwrap(),
        "{\n  \"jobs\": {}\n}\n"
    );
    // Its runs stay readable until the workspace goes (PRT-206).
    let runs = client
        .ok("jobs.runs", json!({ "workspaceId": ws, "job": "watch" }))
        .await;
    assert_eq!(runs["runs"].as_array().unwrap().len(), 2);
}

// spec: DMN-322, DMN-326
#[tokio::test(flavor = "multi_thread")]
async fn a_changed_running_service_starts_again_with_its_new_definition() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let (ws, _) = workspace(&mut client).await;
    let old = format!("echo old; {}", daemon.hold("gate"));
    let new = format!("echo new; {}", daemon.hold("gate"));
    define(
        &mut client,
        &ws,
        "watch",
        json!({ "trigger": "service", "command": old }),
    )
    .await;
    let mut jobs = Jobs::subscribe(&mut client, &ws).await;
    jobs.until_status(&mut client, "watch", "running").await;
    let first = jobs.job("watch")["run"]["id"].clone();

    client
        .ok(
            "jobs.define",
            json!({ "workspaceId": ws, "name": "watch", "job": { "trigger": "service", "command": new }, "replace": true }),
        )
        .await;

    let ended = jobs.until_kind(&mut client, "run_ended").await["run"].clone();
    assert_eq!((&ended["id"], &ended["status"]), (&first, &json!("killed")));
    let started = jobs.until_kind(&mut client, "run_started").await["run"].clone();
    assert_eq!(started["trigger"], "service");
    let lines = until_line(&mut client, started["id"].as_str().unwrap(), "new").await;
    assert_eq!(lines, ["new"]);
    client
        .ok("jobs.kill", json!({ "jobId": started["id"] }))
        .await;
}

// spec: DMN-335, PRT-210, DMN-321
#[tokio::test(flavor = "multi_thread")]
async fn the_index_carries_each_workspaces_job_counts() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let (ws, folder) = workspace(&mut client).await;
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    let mine = |workspaces: &Value| {
        workspaces
            .as_array()
            .unwrap()
            .iter()
            .find(|workspace| workspace["id"] == json!(ws))
            .cloned()
            .unwrap()
    };
    assert!(mine(&index["state"]["workspaces"]).get("jobs").is_none());

    define(&mut client, &ws, "build", json!({ "command": "true" })).await;
    define(
        &mut client,
        &ws,
        "nightly",
        json!({ "trigger": "schedule", "schedule": "0 3 * * *", "command": "true" }),
    )
    .await;

    let counts = loop {
        let event = client.event("index").await["event"].clone();
        if event["kind"] == "workspace_upserted"
            && event["workspace"]["id"] == json!(ws)
            && event["workspace"]["jobs"]["total"] == 2
        {
            break event["workspace"]["jobs"].clone();
        }
    };
    assert_eq!(
        counts,
        json!({ "total": 2, "manual": 1, "schedule": 1, "service": 0, "running": 0, "failing": 0, "error": false })
    );

    std::fs::write(folder.join(".omfp/jobs.json"), "{").unwrap();
    client
        .ok("workspaces.rescan", json!({ "workspaceId": ws }))
        .await;
    loop {
        let event = client.event("index").await["event"].clone();
        if event["kind"] == "workspace_upserted" && event["workspace"]["jobs"]["error"] == true {
            assert_eq!(event["workspace"]["jobs"]["total"], 2);
            break;
        }
    }
}

// spec: DMN-328, DMN-334, PRT-209
#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_job_makes_a_thread_the_index_names_after_the_job() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let (ws, _) = workspace(&mut client).await;
    define(
        &mut client,
        &ws,
        "ask",
        json!({ "prompt": { "text": "Say hi" } }),
    )
    .await;
    daemon.launcher.push_run(fixture("text_reply"));
    let mut jobs = Jobs::subscribe(&mut client, &ws).await;

    let started = run(&mut client, &ws, "ask").await.unwrap();

    let thread_id = started["threadId"].as_str().unwrap().to_owned();
    let ended = jobs.until_kind(&mut client, "run_ended").await["run"].clone();
    assert_eq!(ended["status"], "succeeded");
    assert_eq!(ended["threadId"], json!(thread_id));
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    let thread = index["state"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == json!(thread_id))
        .unwrap()
        .clone();
    assert_eq!(
        thread["provenance"],
        json!({ "createdBy": { "kind": "job", "job": "ask" } })
    );
    assert!(thread["title"].as_str().unwrap().starts_with("ask · "));
}

// spec: DMN-329, DMN-081
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_workspace_tears_its_jobs_down_before_deleting_the_folder() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let (ws, folder) = workspace(&mut client).await;
    let command = format!(
        r#"trap 'test -d "$PWD" && echo folder-present; exit 0' TERM; echo ready; {}"#,
        daemon.hold("gate")
    );
    define(
        &mut client,
        &ws,
        "watch",
        json!({ "trigger": "service", "command": command }),
    )
    .await;
    let mut jobs = Jobs::subscribe(&mut client, &ws).await;
    jobs.until_status(&mut client, "watch", "running").await;
    let run_id = jobs.job("watch")["run"]["id"].as_str().unwrap().to_owned();
    let topic = format!("job/{run_id}");
    until_line(&mut client, &run_id, "ready").await;

    client
        .ok("workspaces.remove", json!({ "workspaceId": ws }))
        .await;

    assert!(!folder.exists());
    loop {
        let event = client.event(&topic).await["event"].clone();
        if event["kind"] == "job_output" && event["text"] == "folder-present" {
            break;
        }
    }
    let runs = client
        .ok("jobs.runs", json!({ "workspaceId": ws, "job": "watch" }))
        .await;
    assert_eq!(runs, json!({ "runs": [], "more": false }));
}

// spec: DMN-330, DMN-327
#[tokio::test(flavor = "multi_thread")]
async fn a_run_stays_readable_after_the_daemon_restarts() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let (ws, _) = workspace(&mut client).await;
    define(
        &mut client,
        &ws,
        "two",
        json!({ "command": "echo one; echo two; exit 3" }),
    )
    .await;
    define(
        &mut client,
        &ws,
        "hold",
        json!({ "command": daemon.hold("gate") }),
    )
    .await;
    let mut jobs = Jobs::subscribe(&mut client, &ws).await;
    let finished = run(&mut client, &ws, "two").await.unwrap();
    let ended = loop {
        let event = jobs.until_kind(&mut client, "run_ended").await;
        if event["run"]["id"] == finished["id"] {
            break event["run"].clone();
        }
    };
    assert_eq!(
        (&ended["status"], &ended["exitCode"]),
        (&json!("failed"), &json!(3))
    );
    let held = run(&mut client, &ws, "hold").await.unwrap();
    drop(client);

    let daemon = daemon.restart().await;
    let mut client = daemon.client("test").await;

    let result = client
        .ok(
            "subscribe",
            json!({ "topic": format!("job/{}", finished["id"].as_str().unwrap()) }),
        )
        .await;
    assert_eq!(
        result["state"]["lines"],
        json!([{ "stream": "stdout", "text": "one" }, { "stream": "stdout", "text": "two" }])
    );
    assert_eq!(result["state"]["dropped"], 0);
    assert_eq!(result["state"]["job"]["status"]["code"], 3);
    let list = client.ok("jobs.list", json!({ "workspaceId": ws })).await;
    let hold = list["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["name"] == "hold")
        .unwrap()
        .clone();
    // The shutdown killed the run and recorded it.
    assert_eq!(hold["lastRun"]["id"], held["id"]);
    assert_eq!(hold["lastRun"]["status"], "killed");
    assert_eq!(hold["status"], "idle");
}
