//! Prompt jobs against the pinned omp and the fake LLM (L3, DMN-328, DMN-334, PRT-209): a `new` job's run makes a thread
//! named after the job and ends with omp's `prompt_result`; a `reuse` job's runs go to one thread, and a due time while
//! that thread works is a `skipped` run. The scheduler follows a clock the test moves; a gated model reply keeps the
//! thread working, so nothing depends on timing (TST-003).

mod client;
mod common;

use std::sync::Arc;

use client::{Client, Follower};
use common::{Harness, omp_daemon_with};
use fake_llm::{Reply, Step};
use omfp_daemon::DaemonHandle;
use omfp_daemon::jobs::declared::{Clock, Sleep};
use omfp_testkit::{FAKE_MODEL, FAKE_PROVIDER};
use serde_json::{Value, json};
use tokio::sync::watch;

/// A clock that moves only when the test sets it.
#[derive(Clone)]
struct TestClock {
    now: Arc<watch::Sender<i64>>,
}

impl TestClock {
    fn new(now_ms: i64) -> Self {
        Self {
            now: Arc::new(watch::Sender::new(now_ms)),
        }
    }

    fn set(&self, now_ms: i64) {
        self.now.send_replace(now_ms);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        *self.now.borrow()
    }

    fn sleep_until(&self, at_ms: i64) -> Sleep {
        let mut now = self.now.subscribe();
        Box::pin(async move {
            // The sender lives as long as the daemon's clock.
            let _ = now.wait_for(|now| *now >= at_ms).await;
        })
    }
}

/// `jobs/<id>` on a connection of its own (a client skips the frames of topics it does not wait on): its snapshot,
/// then its events one by one.
struct Jobs {
    client: Client,
    topic: String,
    state: Value,
}

impl Jobs {
    async fn subscribe(daemon: &DaemonHandle, workspace_id: &str) -> Self {
        let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "jobs").await;
        let topic = format!("jobs/{workspace_id}");
        let result = client.ok("subscribe", json!({ "topic": topic })).await;
        assert_eq!(result["mode"], "snapshot");
        Self {
            client,
            topic,
            state: result["state"].clone(),
        }
    }

    /// The next `run_ended` of `job`; job views on the way are kept (PRT-200).
    async fn ended(&mut self, job: &str) -> Value {
        loop {
            let event = self.client.event(&self.topic).await["event"].clone();
            match event["kind"].as_str().unwrap() {
                "jobs_set" => {
                    self.state = json!({ "jobs": event["jobs"], "error": event["error"] });
                }
                "job_updated" => {
                    let jobs = self.state["jobs"].as_array_mut().unwrap();
                    let name = &event["job"]["name"];
                    match jobs.iter_mut().find(|view| view["name"] == *name) {
                        Some(view) => *view = event["job"].clone(),
                        None => jobs.push(event["job"].clone()),
                    }
                }
                "run_ended" if event["run"]["job"] == job => return event["run"].clone(),
                _ => {}
            }
        }
    }

    fn view(&self, job: &str) -> &Value {
        self.state["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|view| view["name"] == job)
            .unwrap_or_else(|| panic!("no job {job} in {}", self.state))
    }
}

async fn define(client: &mut Client, workspace_id: &str, name: &str, job: Value) {
    client
        .ok(
            "jobs.define",
            json!({ "workspaceId": workspace_id, "name": name, "job": job }),
        )
        .await;
}

async fn run(client: &mut Client, workspace_id: &str, job: &str) -> Result<Value, Value> {
    client
        .call(
            "jobs.run",
            json!({ "workspaceId": workspace_id, "job": job }),
        )
        .await
}

async fn thread(client: &mut Client, workspace_id: &str, thread_id: &str) -> Value {
    let list = client
        .ok("threads.list", json!({ "workspaceId": workspace_id }))
        .await;
    list["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == thread_id)
        .unwrap_or_else(|| panic!("no thread {thread_id} in {list}"))
        .clone()
}

/// `<YYYY-MM-DD HH:MM>` of `at_ms` in the host's time zone, as a `new` job's thread title has it (DMN-328).
fn local_minute(at_ms: i64) -> String {
    jiff::Timestamp::from_millisecond(at_ms)
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
}

// spec: DMN-328, DMN-334, PRT-209, PRT-201, PRT-204, DMN-324
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // One acceptance scenario, step by step.
async fn prompt_jobs_make_or_reuse_a_thread_and_skip_a_due_time_while_it_works() {
    const BUSY_STEP: usize = 3;
    let harness = Harness::new(vec![
        Step::reply(Reply::text("The report is ready.")),
        Step::reply(Reply::text("First digest.")),
        Step::reply(Reply::text("Second digest.")),
        Step::reply(Reply::text("Answered at last.")).gated(),
    ])
    .await;
    let origin = jiff::Timestamp::now().as_millisecond();
    let clock = TestClock::new(origin);
    let (daemon, _data) = omp_daemon_with(&harness, &["--no-title-generation"], |ports| {
        ports.clock = Arc::new(clock.clone());
    })
    .await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let workspace = client.ok("workspaces.create", json!({})).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let model = format!("{FAKE_PROVIDER}/{FAKE_MODEL}");
    define(
        &mut client,
        &workspace,
        "report",
        json!({ "prompt": { "text": "Write the report", "model": model } }),
    )
    .await;
    define(
        &mut client,
        &workspace,
        "digest",
        json!({
            "trigger": "schedule",
            "schedule": "* * * * *",
            "prompt": { "text": "Digest the news", "thread": "reuse", "model": model },
        }),
    )
    .await;
    let mut jobs = Jobs::subscribe(&daemon, &workspace).await;

    // `new`: a thread named after the job and its start, made by the job, the run ended by omp's result.
    let started = run(&mut client, &workspace, "report").await.unwrap();
    let report_thread = started["threadId"].as_str().unwrap().to_owned();
    let ended = jobs.ended("report").await;
    assert_eq!(
        (&ended["id"], &ended["status"], &ended["threadId"]),
        (&started["id"], &json!("succeeded"), &json!(report_thread)),
        "{ended}"
    );
    let made = thread(&mut client, &workspace, &report_thread).await;
    assert_eq!(made["title"], format!("report · {}", local_minute(origin)));
    assert_eq!(
        made["provenance"],
        json!({ "createdBy": { "kind": "job", "job": "report" } })
    );
    let mut report = Follower::snapshot(&mut client, &report_thread, None).await;
    report
        .until(&mut client, |state| {
            state.settled && state.transcript.len() >= 2
        })
        .await;

    // `reuse`: the first run makes the job's thread, the second goes to it.
    let first = run(&mut client, &workspace, "digest").await.unwrap();
    let digest_thread = first["threadId"].as_str().unwrap().to_owned();
    assert_ne!(digest_thread, report_thread);
    assert_eq!(jobs.ended("digest").await["status"], "succeeded");
    let mut digest = Follower::snapshot(&mut client, &digest_thread, None).await;
    digest
        .until(&mut client, |state| {
            state.settled && state.transcript.len() >= 2
        })
        .await;
    let second = run(&mut client, &workspace, "digest").await.unwrap();
    assert_eq!(second["threadId"], json!(digest_thread));
    assert_eq!(jobs.ended("digest").await["status"], "succeeded");
    digest
        .until(&mut client, |state| {
            state.settled && state.transcript.len() >= 4
        })
        .await;
    let reused = thread(&mut client, &workspace, &digest_thread).await;
    assert_eq!(reused["title"], "digest");
    assert_eq!(
        reused["provenance"],
        json!({ "createdBy": { "kind": "job", "job": "digest" } })
    );

    // The job's thread works on someone's prompt: a run by hand is refused, a due time is skipped.
    client
        .ok(
            "thread.prompt",
            json!({ "threadId": digest_thread, "text": "One more thing", "images": [] }),
        )
        .await;
    harness.fake.wait_for_gate(BUSY_STEP).await;
    let refused = run(&mut client, &workspace, "digest").await.unwrap_err();
    assert_eq!(refused["code"], "conflict", "{refused}");
    let due = jobs.view("digest")["nextAt"].as_i64().unwrap();
    clock.set(due);
    let skipped = jobs.ended("digest").await;
    assert_eq!(
        (
            &skipped["trigger"],
            &skipped["status"],
            &skipped["reason"],
            &skipped["threadId"]
        ),
        (
            &json!("schedule"),
            &json!("skipped"),
            &json!("busy"),
            &json!(digest_thread)
        ),
        "{skipped}"
    );

    harness.fake.release_gate(BUSY_STEP);
    digest
        .until(&mut client, |state| {
            state.settled && state.transcript.len() >= 6
        })
        .await;
    assert_eq!(harness.fake.requests().len(), BUSY_STEP + 1);
    harness.assert_no_stray_network();
    daemon.shutdown().await.unwrap();
}
